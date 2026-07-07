use crate::agent::chat::AgentChat;
use crate::agent::llm::LlmConfig;
use crate::core::config::Config;
use crate::core::detector;
use crate::core::mcp::McpClient;
use crate::core::monitor;
use crate::core::task::Task;
use serenity::all::{ChannelId, Message};
use serenity::{async_trait, prelude::*};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::sync::{mpsc, Mutex};

struct SessionState {
    chat: AgentChat,
}

pub struct Handler {
    pub tx: mpsc::Sender<Task>,
    pub channel_id: Option<u64>,
    pub sessions: Arc<Mutex<HashMap<u64, SessionState>>>,
    pub mcp_clients: Vec<Arc<McpClient>>,
    pub http_client: reqwest::Client,
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        if let Some(cid) = self.channel_id {
            if msg.channel_id != ChannelId::new(cid) {
                return;
            }
        }

        let content = msg.content.trim().to_string();

        if content.is_empty() {
            return;
        }

        let typing = msg.channel_id.start_typing(&ctx.http);

        let reply = if content == "ping" {
            "I'm up!".to_string()
        } else if content == "agents" {
            let agents = detector::detect_agents();
            truncate(&detector::format_agent_report(&agents), 1900)
        } else if content == "services" {
            truncate(&monitor::list_services(), 1900)
        } else if content == "help" {
            help_text()
        } else if content == "config" {
            let config = Config::load();
            config.format_report()
        } else if content.starts_with("config ") {
            handle_discord_config(&content).await
        } else if content == "reset" {
            let mut sessions = self.sessions.lock().await;
            sessions.remove(&msg.channel_id.get());
            "Conversation reset. I've forgotten our previous messages.".to_string()
        } else if content == "cancel" || content == "stop" {
            let sessions = self.sessions.lock().await;
            if let Some(session) = sessions.get(&msg.channel_id.get()) {
                session.chat.cancel();
                "Cancelling current agent operation...".to_string()
            } else {
                "No active agent session to cancel.".to_string()
            }
        } else if content.starts_with("approve ") {
            let tool = content[8..].trim();
            let sessions = self.sessions.lock().await;
            if let Some(session) = sessions.get(&msg.channel_id.get()) {
                session.chat.approval_gate.approve_tool(tool);
                format!("Approved `{tool}`. You can now re-send your request.")
            } else {
                "No active agent session.".to_string()
            }
        } else if content.starts_with("deny ") {
            let tool = content[5..].trim();
            let sessions = self.sessions.lock().await;
            if let Some(session) = sessions.get(&msg.channel_id.get()) {
                session.chat.approval_gate.deny_tool(tool)
            } else {
                "No active agent session.".to_string()
            }
        } else if content.starts_with("run ")
            || content.starts_with("sif ")
            || content.starts_with("nmap ")
            || content.starts_with("mail ")
            || content.starts_with("history")
            || content.starts_with("monitor ")
            || content.starts_with("loop ")
            || content.starts_with("workon ")
            || content.starts_with("cancel ")
            || content.starts_with("delete ")
            || content.starts_with("retry ")
        {
            process_raw_command(&content, &self.tx).await
        } else {
            let mut sessions = self.sessions.lock().await;
            let config = LlmConfig::from_env();
            if !config.is_configured() {
                "LLM not configured. Set LLM_API_KEY in .env to use the AI agent, or use `help` for available commands.".to_string()
            } else {
                let session =
                    sessions
                        .entry(msg.channel_id.get())
                        .or_insert_with(|| SessionState {
                            chat: AgentChat::new(
                                config.clone(),
                                self.mcp_clients.clone(),
                                self.http_client.clone(),
                                Some(&format!(
                                    "Discord channel: {}\nUser: {}",
                                    msg.channel_id, msg.author.name
                                )),
                            ),
                        });
                let channel_msg = format!("[{}]: {}", msg.author.name, content);
                session.chat.send_message(&channel_msg).await
            }
        };

        drop(typing);

        for chunk in chunk_message(&reply, 1900) {
            if let Err(e) = msg.channel_id.say(&ctx.http, &chunk).await {
                eprintln!("Failed to send reply: {e}");
                break;
            }
        }
    }
}

async fn handle_discord_config(content: &str) -> String {
    let parts: Vec<&str> = content.splitn(3, char::is_whitespace).collect();
    if parts.len() < 2 {
        return Config::load().format_report();
    }
    match parts[1] {
        "show" => Config::load().format_report(),
        "set" => {
            if parts.len() < 4 {
                return "Usage: `config set <key> <value>`\nKeys: llm.api_key, llm.provider, llm.model, discord.token, discord.channel_id, paths.atomic_repo".to_string();
            }
            let key = parts[2];
            let value = parts[3];
            let mut cfg = Config::load();
            match key {
                "llm.api_key" => cfg.llm.api_key = Some(value.to_string()),
                "llm.provider" => cfg.llm.provider = Some(value.to_string()),
                "llm.model" => cfg.llm.model = Some(value.to_string()),
                "llm.base_url" => cfg.llm.base_url = Some(value.to_string()),
                "discord.token" => cfg.discord.token = Some(value.to_string()),
                "discord.channel_id" => cfg.discord.channel_id = Some(value.to_string()),
                "paths.atomic_repo" => cfg.paths.atomic_repo = Some(value.to_string()),
                "paths.opencode_bin" => cfg.paths.opencode_bin = Some(value.to_string()),
                _ => return format!("Unknown key: {key}"),
            }
            match cfg.save() {
                Ok(_) => format!("Set {key} ✓\nRestart for changes to take effect."),
                Err(e) => format!("Error: {e}"),
            }
        }
        "mcp" => {
            if parts.len() < 3 {
                return "Usage: `config mcp add name|cmd|args` or `config mcp list` or `config mcp remove <name>`".to_string();
            }
            match parts[2] {
                "add" => {
                    let spec = parts.get(3).unwrap_or(&"");
                    let spec_parts: Vec<&str> = spec.splitn(3, '|').collect();
                    if spec_parts.len() < 2 {
                        return "Format: `config mcp add name|command|arg1,arg2`".to_string();
                    }
                    let mut cfg = Config::load();
                    let args: Vec<String> = spec_parts
                        .get(2)
                        .unwrap_or(&"")
                        .split(',')
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .collect();
                    cfg.mcp.servers.push(crate::core::config::McpServerEntry {
                        name: spec_parts[0].to_string(),
                        command: spec_parts[1].to_string(),
                        args,
                    });
                    cfg.save().ok();
                    format!("Added MCP server: {} ✓", spec_parts[0])
                }
                "list" => {
                    let cfg = Config::load();
                    if cfg.mcp.servers.is_empty() {
                        "No MCP servers configured.".to_string()
                    } else {
                        let mut r = String::from("**MCP Servers:**\n");
                        for s in &cfg.mcp.servers {
                            r.push_str(&format!("  {}: {} {:?}\n", s.name, s.command, s.args));
                        }
                        r
                    }
                }
                "remove" => {
                    let name = parts.get(3).unwrap_or(&"");
                    if name.is_empty() {
                        return "Usage: `config mcp remove <name>`".to_string();
                    }
                    let mut cfg = Config::load();
                    cfg.mcp.servers.retain(|s| s.name != *name);
                    cfg.save().ok();
                    format!("Removed MCP server: {name}")
                }
                _ => "Usage: `config mcp add/list/remove`".to_string(),
            }
        }
        _ => Config::load().format_report(),
    }
}

fn help_text() -> String {
    let mut h = String::from("**Command Central — text commands**\n");
    h.push_str("`run <cmd>`         Run any shell command\n");
    h.push_str("`agents`            List installed coding agents & tools\n");
    h.push_str("`services`          List running system services\n");
    h.push_str("`monitor p <name>`  Check if a process is running\n");
    h.push_str("`monitor s <name>`  Check a system service\n");
    h.push_str("`loop <n> = <c> every <s>`  Create scheduled loop\n");
    h.push_str("`workon <task>`     Run opencode on Atomic repo\n");
    h.push_str("`history [n]`       Show last n tasks\n");
    h.push_str("`cancel <id>`        Cancel a running/pending task\n");
    h.push_str("`delete <id>`        Delete a task from history\n");
    h.push_str("`retry <id>`         Retry a failed task\n");
    h.push_str("`sif <url>`         Queue a SIF task\n");
    h.push_str("`nmap <target>`     Run an nmap scan\n");
    h.push_str("`mail <d> <b>`      Queue a mail send\n");
    h.push_str("`config`            Show full config\n");
    h.push_str("`config set <k> <v>` Set config (llm.api_key, discord.token, etc.)\n");
    h.push_str("`config mcp add <n|c|a>` Add MCP server (name|cmd|args)\n");
    h.push_str("`config mcp list`    List MCP servers\n");
    h.push_str("`config mcp rm <n>`  Remove MCP server\n");
    h.push_str("`reset`             Reset conversation with agent\n");
    h.push_str("`cancel` or `stop`  Cancel current agent operation\n");
    h.push_str("`approve <tool>`    Approve a pending tool execution\n");
    h.push_str("`deny <tool>`       Deny a pending tool execution\n");
    h.push_str("\n**AI Agent:** Anything else is sent to the AI agent with web, shell, file, and MCP tools.");
    h
}

async fn process_raw_command(content: &str, tx: &mpsc::Sender<Task>) -> String {
    let parts: Vec<&str> = content.splitn(2, char::is_whitespace).collect();
    let cmd = parts[0];
    let rest = parts.get(1).unwrap_or(&"").trim();

    match cmd {
        "run" => {
            if rest.is_empty() {
                return "Usage: `run <command>`".to_string();
            }
            let _ = tx.send(Task::Run(rest.to_string())).await;
            format!("Running: `{rest}`")
        }
        "workon" => {
            if rest.is_empty() {
                return "Usage: `workon <task>`".to_string();
            }
            let _ = tx.send(Task::Workon(rest.to_string())).await;
            format!("Opencode spawned: {rest}")
        }
        "sif" => {
            if rest.is_empty() {
                return "Usage: `sif <url>`".to_string();
            }
            let _ = tx.send(Task::Sif(rest.to_string())).await;
            "Queued SIF".to_string()
        }
        "nmap" => {
            if rest.is_empty() {
                return "Usage: `nmap <target>`".to_string();
            }
            let _ = tx.send(Task::Nmap(rest.to_string())).await;
            "Queued NMAP".to_string()
        }
        "mail" => {
            let mp: Vec<&str> = rest.splitn(2, char::is_whitespace).collect();
            let d = mp.first().unwrap_or(&"");
            let b = mp.get(1).unwrap_or(&"");
            if d.is_empty() || b.is_empty() {
                return "Usage: `mail <dest> <body>`".to_string();
            }
            let _ = tx.send(Task::Mail(d.to_string(), b.to_string())).await;
            "Queued mail".to_string()
        }
        "history" => {
            let limit: i64 = rest.parse().unwrap_or(10).clamp(1, 50);
            let (r_tx, r_rx) = oneshot::channel();
            let _ = tx.send(Task::History(r_tx)).await;
            match r_rx.await {
                Ok(rows) => {
                    let rows: Vec<_> = rows.into_iter().take(limit as usize).collect();
                    if rows.is_empty() {
                        "No history.".to_string()
                    } else {
                        let log = rows
                            .iter()
                            .map(|(k, s, id)| format!("[{id}] {k} -> {s}"))
                            .collect::<Vec<_>>()
                            .join("\n");
                        format!("```sh\n{log}\n```")
                    }
                }
                Err(_) => "Failed to fetch history.".to_string(),
            }
        }
        "cancel" => {
            let id: i64 = match rest.parse() {
                Ok(n) => n,
                Err(_) => {
                    let sessions_lock = crate::discord::bot::GLOBAL_CANCEL_FN.lock().unwrap();
                    if let Some(ref cancel_fn) = *sessions_lock {
                        cancel_fn();
                        return "Cancelling current agent operation.".to_string();
                    }
                    return "Usage: `cancel <task_id>` or send `cancel` without args to cancel agent.".to_string();
                }
            };
            let _ = tx.send(Task::CancelTask(id)).await;
            format!("Cancelling task {id}...")
        }
        "delete" => {
            let id: i64 = match rest.parse() {
                Ok(n) => n,
                Err(_) => return "Usage: `delete <task_id>`".to_string(),
            };
            let _ = tx.send(Task::DeleteTask(id)).await;
            format!("Deleting task {id}...")
        }
        "retry" => {
            let id: i64 = match rest.parse() {
                Ok(n) => n,
                Err(_) => return "Usage: `retry <task_id>`".to_string(),
            };
            let _ = tx.send(Task::RetryTask(id)).await;
            format!("Retrying task {id}...")
        }
        "monitor" => {
            let sub: Vec<&str> = rest.splitn(2, char::is_whitespace).collect();
            let mt = sub.first().unwrap_or(&"");
            let mn = sub.get(1).unwrap_or(&"");
            if mn.is_empty() {
                return "Usage: `monitor p <name>` or `monitor s <name>`".to_string();
            }
            match *mt {
                "p" | "process" => truncate(&monitor::find_process(mn), 1900),
                "s" | "service" => truncate(&monitor::monitor_service(mn), 1900),
                _ => "Usage: `monitor p <name>` or `monitor s <name>`".to_string(),
            }
        }
        "loop" => {
            let lp: Vec<&str> = rest.splitn(4, char::is_whitespace).collect();
            if lp.len() < 4 || lp.get(1) != Some(&"=") || lp.get(3) != Some(&"every") {
                return "Usage: `loop <name> = <command> every <secs>`".to_string();
            }
            let interval: u64 = lp.get(4).and_then(|s| s.parse().ok()).unwrap_or(60).max(10);
            let _ = tx
                .send(Task::AddLoop {
                    name: lp[0].to_string(),
                    command: lp[2].to_string(),
                    interval_secs: interval,
                })
                .await;
            "Loop created (check `history`)".to_string()
        }
        _ => format!("Unknown: `{cmd}`. Try `help`"),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut idx = max;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    format!("{}\n... (truncated)", &s[..idx])
}

fn chunk_message(msg: &str, max: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < msg.len() {
        let mut end = (start + max).min(msg.len());
        while end > start && !msg.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = (start + 1).min(msg.len());
        }
        chunks.push(msg[start..end].to_string());
        start = end;
    }
    chunks
}

use std::sync::LazyLock;
use std::sync::Mutex as StdMutex;
static GLOBAL_CANCEL_FN: LazyLock<StdMutex<Option<Box<dyn Fn() + Send + Sync>>>> =
    LazyLock::new(|| StdMutex::new(None));

pub async fn run(
    token: &str,
    tx: mpsc::Sender<Task>,
    channel_id: Option<u64>,
    mcp_clients: Vec<Arc<McpClient>>,
) -> anyhow::Result<()> {
    let intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT;

    let http_client = reqwest::Client::builder()
        .user_agent("command_central/0.2.0")
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()?;

    let sessions: Arc<Mutex<HashMap<u64, SessionState>>> = Arc::new(Mutex::new(HashMap::new()));

    let mut client = Client::builder(token, intents)
        .event_handler(Handler {
            tx,
            channel_id,
            sessions,
            mcp_clients,
            http_client,
        })
        .await?;

    client.start().await?;
    Ok(())
}
