use rusqlite::{params, Connection};
use serenity::all::{
    CommandDataOptionValue, CommandOptionType, CreateCommand, CreateCommandOption,
    EditInteractionResponse, GuildId, Interaction, Ready,
};
use serenity::{async_trait, prelude::*};
use std::env;
use std::process::Command as ProcessCommand;
use std::time::Duration;

// Absolute path to the project root (where Cargo.toml lives), baked in at compile time.
// Resolves ./sif and ./nmap correctly no matter what directory the bot is launched from.
const PROJECT_ROOT: &str = env!("CARGO_MANIFEST_DIR");

// Discord hard-caps message content at 2000 chars. Leave headroom for the
// "```sh\n...\n```" wrapper and a status line.
const MAX_OUTPUT_CHARS: usize = 1800;

type TaskResult = (bool, String, String); // (succeeded, stdout, stderr)

struct Handler {
    guild_id: u64,
}

/// Opens a connection to tasks.db with WAL mode + a busy timeout, so
/// concurrent tasks writing at the same time retry instead of erroring.
fn open_db() -> rusqlite::Result<Connection> {
    let db_path = format!("{PROJECT_ROOT}/tasks.db");
    let conn = Connection::open(&db_path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS tasks (id INTEGER PRIMARY KEY, kind TEXT, status TEXT)",
        [],
    )?;
    Ok(conn)
}

/// Wraps output in a fenced code block, truncating so it stays under Discord's message cap.
fn format_output_block(label: &str, ok: bool, stdout: &str, stderr: &str) -> String {
    let status = if ok { "done" } else { "failed" };
    let mut combined = if stderr.trim().is_empty() {
        stdout.to_string()
    } else if stdout.trim().is_empty() {
        stderr.to_string()
    } else {
        format!("{stdout}\n--- stderr ---\n{stderr}")
    };

    if combined.trim().is_empty() {
        combined = "(no output)".to_string();
    }

    if combined.len() > MAX_OUTPUT_CHARS {
        combined.truncate(MAX_OUTPUT_CHARS);
        combined.push_str("\n... (truncated)");
    }

    format!("{label} {status}\n```sh\n{combined}\n```")
}

/// Runs `program args...` under a hard timeout via the `timeout` coreutil,
/// capturing stdout/stderr instead of discarding them.
fn run_with_timeout(timeout_secs: u32, program: &str, args: &[&str]) -> TaskResult {
    let mut full_args = vec![timeout_secs.to_string(), program.to_string()];
    full_args.extend(args.iter().map(|s| s.to_string()));

    match ProcessCommand::new("timeout").args(&full_args).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let timed_out = output.status.code() == Some(124);
            let stderr = if timed_out {
                format!("{stderr}\n(killed: exceeded {timeout_secs}s timeout)")
            } else {
                stderr
            };
            (output.status.success(), stdout, stderr)
        }
        Err(e) => (
            false,
            String::new(),
            format!("failed to spawn process: {e}"),
        ),
    }
}

/// Inserts a 'running' row, runs the command, updates the row with the final
/// status, and returns the result. Meant to be called inside spawn_blocking.
fn execute_task(kind_label: String, timeout_secs: u32, program: &str, args: &[&str]) -> TaskResult {
    let conn = match open_db() {
        Ok(c) => c,
        Err(e) => return (false, String::new(), format!("db open error: {e}")),
    };

    if let Err(e) = conn.execute(
        "INSERT INTO tasks (kind,status) VALUES (?1,'running')",
        params![kind_label],
    ) {
        return (false, String::new(), format!("db insert error: {e}"));
    }
    let id = conn.last_insert_rowid();

    let (ok, stdout, stderr) = run_with_timeout(timeout_secs, program, args);

    let status = if ok { "done" } else { "failed" };
    let _ = conn.execute(
        "UPDATE tasks SET status = ?1 WHERE id = ?2",
        params![status, id],
    );

    (ok, stdout, stderr)
}

fn fetch_history(limit: i64) -> Vec<(String, String)> {
    let conn = match open_db() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut stmt = match conn.prepare("SELECT kind, status FROM tasks ORDER BY id ASC LIMIT ?1") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    stmt.query_map(params![limit], |row| {
        let kind: String = row.get(0)?;
        let status: String = row.get(1)?;
        Ok((kind, status))
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        println!("{} is online", ready.user.name);

        let guild_id = GuildId::new(self.guild_id);

        let commands = guild_id
            .set_commands(
                &ctx.http,
                vec![
                    CreateCommand::new("ping").description("Check if the bot is alive"),
                    CreateCommand::new("sif")
                        .description("Queue a SIF task")
                        .add_option(
                            CreateCommandOption::new(
                                CommandOptionType::String,
                                "url",
                                "Target URL",
                            )
                            .required(true),
                        ),
                    CreateCommand::new("nmap")
                        .description("Queue an nmap scan")
                        .add_option(
                            CreateCommandOption::new(
                                CommandOptionType::String,
                                "target",
                                "Scan target",
                            )
                            .required(true),
                        ),
                    CreateCommand::new("mail")
                        .description("Queue a mail send")
                        .add_option(
                            CreateCommandOption::new(
                                CommandOptionType::String,
                                "dest",
                                "Recipient",
                            )
                            .required(true),
                        )
                        .add_option(
                            CreateCommandOption::new(
                                CommandOptionType::String,
                                "body",
                                "Message body",
                            )
                            .required(true),
                        ),
                    CreateCommand::new("history")
                        .description("Show recent task history")
                        .add_option(
                            CreateCommandOption::new(
                                CommandOptionType::Integer,
                                "limit",
                                "How many entries to show (default 10, max 50)",
                            )
                            .required(false),
                        ),
                ],
            )
            .await;

        if let Err(why) = commands {
            println!("Failed to register commands: {why:?}");
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let Interaction::Command(command) = interaction else {
            return;
        };

        // Ack within Discord's 3s window immediately; the real work happens
        // on a spawn_blocking task independent of every other in-flight command.
        let _ = command.defer(&ctx.http).await;

        let content: String = match command.data.name.as_str() {
            "ping" => "ジュジュWelcome.".to_string(),

            "sif" => {
                let url = match command.data.options.first().map(|o| &o.value) {
                    Some(CommandDataOptionValue::String(s)) => s.clone(),
                    _ => String::new(),
                };
                if url.is_empty() {
                    "Missing required `url` option.".to_string()
                } else {
                    let sif_bin = format!("{PROJECT_ROOT}/sif");
                    let result = tokio::task::spawn_blocking(move || {
                        let kind = format!("sif:{url}");
                        execute_task(kind, 30000, &sif_bin, &["-u", &url, "-am", "api"])
                    })
                    .await;

                    match result {
                        Ok((ok, stdout, stderr)) => {
                            format_output_block("SIF", ok, &stdout, &stderr)
                        }
                        Err(_) => "SIF task panicked or was cancelled.".to_string(),
                    }
                }
            }

            "nmap" => {
                let target = match command.data.options.first().map(|o| &o.value) {
                    Some(CommandDataOptionValue::String(s)) => s.clone(),
                    _ => String::new(),
                };
                if target.is_empty() {
                    "Missing required `target` option.".to_string()
                } else {
                    let nmap_bin = format!("{PROJECT_ROOT}/nmap");
                    let result = tokio::task::spawn_blocking(move || {
                        let kind = format!("nmap:{target}");
                        execute_task(
                            kind,
                            30000,
                            &nmap_bin,
                            &["-sV", "-T4", "--top-ports", "1000", &target],
                        )
                    })
                    .await;

                    match result {
                        Ok((ok, stdout, stderr)) => {
                            format_output_block("Nmap", ok, &stdout, &stderr)
                        }
                        Err(_) => "Nmap task panicked or was cancelled.".to_string(),
                    }
                }
            }

            "mail" => {
                let mut dest = String::new();
                let mut body = String::new();
                for opt in &command.data.options {
                    match (opt.name.as_str(), &opt.value) {
                        ("dest", CommandDataOptionValue::String(s)) => dest = s.clone(),
                        ("body", CommandDataOptionValue::String(s)) => body = s.clone(),
                        _ => {}
                    }
                }
                if dest.is_empty() || body.is_empty() {
                    "Missing required `dest` or `body` option.".to_string()
                } else {
                    let result = tokio::task::spawn_blocking(move || {
                        let kind = format!("mail:{dest}");
                        let res = execute_task(kind, 60, "mail", &["-s", "Discord Bot", &dest]);
                        println!("mail body for {dest}: {body}");
                        res
                    })
                    .await;

                    match result {
                        Ok((ok, stdout, stderr)) => {
                            format_output_block("Mail", ok, &stdout, &stderr)
                        }
                        Err(_) => "Mail task panicked or was cancelled.".to_string(),
                    }
                }
            }

            "history" => {
                let limit = command
                    .data
                    .options
                    .first()
                    .and_then(|o| match &o.value {
                        CommandDataOptionValue::Integer(n) => Some(*n),
                        _ => None,
                    })
                    .unwrap_or(10)
                    .clamp(1, 50);

                let rows = tokio::task::spawn_blocking(move || fetch_history(limit))
                    .await
                    .unwrap_or_default();

                if rows.is_empty() {
                    "No task history yet.".to_string()
                } else {
                    let mut log = rows
                        .iter()
                        .map(|(kind, status)| format!("{kind} -> {status}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if log.len() > MAX_OUTPUT_CHARS {
                        log.truncate(MAX_OUTPUT_CHARS);
                        log.push_str("\n... (truncated)");
                    }
                    format!("```sh\n{log}\n```")
                }
            }

            _ => "Unknown command".to_string(),
        };

        let _ = command
            .edit_response(&ctx.http, EditInteractionResponse::new().content(content))
            .await;
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let token = env::var("DISCORD_TOKEN")?;
    let guild_id: u64 = env::var("DISCORD_GUILD_ID")?.parse()?;

    // Make sure the DB (and WAL mode) is initialized before the bot starts
    // accepting interactions, rather than lazily on first task.
    let _ = open_db()?;

    let intents = GatewayIntents::empty();

    let mut client = Client::builder(&token, intents)
        .event_handler(Handler { guild_id })
        .await?;

    client.start().await?;
    Ok(())
}
