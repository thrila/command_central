use rusqlite::{params, Connection};
use serenity::all::{
    CommandDataOptionValue, CommandOptionType, CreateCommand, CreateCommandOption,
    CreateInteractionResponse, CreateInteractionResponseMessage, GuildId, Interaction, Ready,
};
use serenity::{async_trait, prelude::*};
use std::env;
use std::process::Command as ProcessCommand;
use tokio::sync::{mpsc, oneshot};

enum Task {
    Sif(String),
    Nmap(String),
    Mail(String, String),
    History(oneshot::Sender<Vec<(String, String)>>),
}

struct Handler {
    tx: mpsc::Sender<Task>,
    guild_id: u64,
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

        let reply = match command.data.name.as_str() {
            "ping" => "I'm up!".to_string(),

            "sif" => {
                let url = match command.data.options.first().map(|o| &o.value) {
                    Some(CommandDataOptionValue::String(s)) => s.clone(),
                    _ => "missing url".to_string(),
                };
                let _ = self.tx.send(Task::Sif(url)).await;
                "Queued SIF task".to_string()
            }

            "nmap" => {
                let target = match command.data.options.first().map(|o| &o.value) {
                    Some(CommandDataOptionValue::String(s)) => s.clone(),
                    _ => "missing target".to_string(),
                };
                let _ = self.tx.send(Task::Nmap(target)).await;
                "Queued NMAP task".to_string()
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
                let _ = self.tx.send(Task::Mail(dest, body)).await;
                "Queued mail task".to_string()
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

                let (resp_tx, resp_rx) = oneshot::channel();
                let _ = self.tx.send(Task::History(resp_tx)).await;

                match resp_rx.await {
                    Ok(rows) => {
                        let rows: Vec<_> = rows.into_iter().take(limit as usize).collect();
                        if rows.is_empty() {
                            "No task history yet.".to_string()
                        } else {
                            let log = rows
                                .iter()
                                .map(|(kind, status)| format!("{kind} -> {status}"))
                                .collect::<Vec<_>>()
                                .join("\n");
                            format!("```sh\n{log}\n```")
                        }
                    }
                    Err(_) => "Failed to fetch history.".to_string(),
                }
            }

            _ => "Unknown command".to_string(),
        };

        let data = CreateInteractionResponseMessage::new().content(reply);
        let builder = CreateInteractionResponse::Message(data);
        let _ = command.create_response(&ctx.http, builder).await;
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let token = env::var("DISCORD_TOKEN")?;
    let guild_id: u64 = env::var("DISCORD_GUILD_ID")?.parse()?;

    let (tx, mut rx) = mpsc::channel::<Task>(100);

    std::thread::spawn(move || {
        let conn = Connection::open("tasks.db").unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS tasks (id INTEGER PRIMARY KEY, kind TEXT, status TEXT)",
            [],
        )
        .unwrap();

        while let Some(task) = rx.blocking_recv() {
            match task {
                Task::Sif(url) => {
                    let kind = format!("sif:{url}");
                    conn.execute(
                        "INSERT INTO tasks (kind,status) VALUES (?1,'running')",
                        params![kind],
                    )
                    .unwrap();
                    let _ = ProcessCommand::new("./sif")
                        .args(["-u", &url, "-am", "api"])
                        .output();
                }
                Task::Nmap(target) => {
                    let kind = format!("nmap:{target}");
                    conn.execute(
                        "INSERT INTO tasks (kind,status) VALUES (?1,'running')",
                        params![kind],
                    )
                    .unwrap();
                    let _ = ProcessCommand::new("nmap").args(["-sV", &target]).output();
                }
                Task::Mail(dest, body) => {
                    let kind = format!("mail:{dest}");
                    conn.execute(
                        "INSERT INTO tasks (kind,status) VALUES (?1,'running')",
                        params![kind],
                    )
                    .unwrap();
                    let _ = ProcessCommand::new("mail")
                        .args(["-s", "Discord Bot", &dest])
                        .spawn();
                    println!("{}", body);
                }
                Task::History(resp_tx) => {
                    let mut stmt = conn
                        .prepare("SELECT kind, status FROM tasks ORDER BY id DESC LIMIT 50")
                        .unwrap();
                    let rows = stmt
                        .query_map([], |row| {
                            let kind: String = row.get(0)?;
                            let status: String = row.get(1)?;
                            Ok((kind, status))
                        })
                        .unwrap()
                        .filter_map(Result::ok)
                        .collect::<Vec<_>>();
                    let _ = resp_tx.send(rows);
                }
            }
        }
    });

    // Slash commands don't need MESSAGE_CONTENT or GUILD_MESSAGES at all.
    let intents = GatewayIntents::empty();

    let mut client = Client::builder(&token, intents)
        .event_handler(Handler { tx, guild_id })
        .await?;

    client.start().await?;
    Ok(())
}
