mod agent;
mod cli;
mod core;
mod discord;
#[cfg(test)]
mod tests;
mod utils;

use clap::{Parser, Subcommand};
use core::config::Config;
use core::executor;
use core::mcp;
use core::scheduler::Scheduler;
use core::task::Task;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(
    name = "command_central",
    version,
    about = "Coding & utility agent command terminal"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a shell command
    Shell { command: String },
    /// Ask the AI agent
    Ask { query: String },
    /// Launch the interactive TUI
    Tui,
    /// Launch the REPL
    Repl,
    /// Start the Discord bot
    Discord,
    /// View or edit configuration
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    /// Interactive setup wizard
    Setup,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show current config
    Show,
    /// Set a config value: llm.api_key sk-..., discord.token ..., mcp.add name|cmd|args
    Set { key: String, value: String },
    /// Remove an MCP server
    McpRemove { name: String },
    /// Open config file in editor
    Edit,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::load();

    let cli = Cli::parse();

    let http_client = reqwest::Client::builder()
        .user_agent("command_central/0.2.0")
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()?;

    let (tx, rx) = mpsc::channel::<Task>(100);
    let scheduler = Arc::new(Scheduler::new("tasks.db"));
    let scheduler_clone = scheduler.clone();
    executor::spawn_worker(rx, "tasks.db".to_string(), scheduler_clone, config.clone());

    match cli.command {
        Some(Commands::Shell { command }) => {
            println!("Running: {command}");
            match crate::utils::shell::run_shell(&command) {
                Ok((stdout, stderr, status)) => {
                    if !stdout.is_empty() {
                        println!("{stdout}");
                    }
                    if !stderr.is_empty() {
                        eprintln!("{stderr}");
                    }
                    println!("Exit: {:?}", status.code());
                }
                Err(e) => eprintln!("Error: {e}"),
            }
        }
        Some(Commands::Ask { query }) => {
            let llm_cfg = config.get_llm_config();
            if !llm_cfg.is_configured() {
                eprintln!(
                    "LLM not configured. Run `command_central setup` or set llm.api_key in config."
                );
                eprintln!("Set LLM_API_KEY in .env or config.toml to use the AI agent.");
                return Ok(());
            }
            let messages = vec![agent::llm::Message {
                role: "user".to_string(),
                content: query,
            }];
            eprint!("Thinking... ");
            match agent::llm::chat_with_agent(
                &llm_cfg,
                &agent_system_prompt(),
                &messages,
                &[],
                &http_client,
                None,
                None,
                None,
            )
            .await
            {
                Ok(response) => {
                    eprintln!("done.");
                    println!("{response}");
                }
                Err(e) => {
                    eprintln!("\nAgent error: {e}\nCheck your LLM_API_KEY and base_url in config.")
                }
            }
        }
        Some(Commands::Tui) => {
            cli::tui::run_tui(tx).await?;
        }
        Some(Commands::Repl) => {
            cli::repl::run_repl(tx).await?;
        }
        Some(Commands::Discord) | None => {
            let token = config.discord.token.clone()
                .or_else(|| std::env::var("DISCORD_TOKEN").ok())
                .expect("Discord token not set. Run `command_central setup` or set discord.token in config.");

            let channel_id: Option<u64> = config
                .discord
                .channel_id
                .clone()
                .or_else(|| std::env::var("DISCORD_CHANNEL_ID").ok())
                .and_then(|s| s.parse().ok());

            let mcp_clients: Vec<_> = mcp::load_mcp_servers_from_config(&config)
                .into_iter()
                .map(Arc::new)
                .collect();
            println!("MCP servers loaded: {}", mcp_clients.len());
            discord::bot::run(&token, tx, channel_id, mcp_clients).await?;
        }
        Some(Commands::Config { action }) => match action {
            Some(ConfigAction::Show) | None => {
                println!("{}", config.format_report());
            }
            Some(ConfigAction::Set { key, value }) => {
                handle_config_set(key, value)?;
            }
            Some(ConfigAction::McpRemove { name }) => {
                let mut cfg = Config::load();
                cfg.mcp.servers.retain(|s| s.name != name);
                cfg.save()?;
                println!("Removed MCP server: {name}");
            }
            Some(ConfigAction::Edit) => {
                let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
                std::process::Command::new(&editor)
                    .arg("config.toml")
                    .status()?;
            }
        },
        Some(Commands::Setup) => {
            run_setup_wizard().await?;
        }
    }

    Ok(())
}

fn handle_config_set(key: String, value: String) -> anyhow::Result<()> {
    let mut cfg = Config::load();

    match key.as_str() {
        "llm.provider" => cfg.llm.provider = Some(value),
        "llm.api_key" => cfg.llm.api_key = Some(value),
        "llm.model" => cfg.llm.model = Some(value),
        "llm.base_url" => cfg.llm.base_url = Some(value),
        "discord.token" => cfg.discord.token = Some(value),
        "discord.channel_id" => cfg.discord.channel_id = Some(value),
        "paths.atomic_repo" => cfg.paths.atomic_repo = Some(value),
        "paths.opencode_bin" => cfg.paths.opencode_bin = Some(value),
        _ if key.starts_with("mcp.add") => {
            let parts: Vec<&str> = value.splitn(3, '|').collect();
            if parts.len() < 2 {
                return Err(anyhow::anyhow!("Format: name|command|arg1,arg2"));
            }
            let args: Vec<String> = parts.get(2)
                .unwrap_or(&"")
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            cfg.mcp.servers.push(core::config::McpServerEntry {
                name: parts[0].to_string(),
                command: parts[1].to_string(),
                args,
            });
            cfg.save()?;
            println!("Added MCP server: {}", parts[0]);
            return Ok(());
        }
        _ => return Err(anyhow::anyhow!("Unknown config key: {key}. Try: llm.provider, llm.api_key, llm.model, discord.token, paths.atomic_repo, mcp.add")),
    }

    cfg.save()?;
    println!("Set {key}");
    Ok(())
}

async fn run_setup_wizard() -> anyhow::Result<()> {
    println!("╔══════════════════════════════════════════╗");
    println!("║   Command Central Setup Wizard           ║");
    println!("╚══════════════════════════════════════════╝");
    println!();

    let mut cfg = Config::load();

    println!("[1/4] AI Provider");
    println!("  Supported: openai, anthropic, or any OpenAI-compatible API");
    cfg.llm.provider = Some(read_input(
        &format!(
            "Provider [{}]: ",
            cfg.llm.provider.as_deref().unwrap_or("openai")
        ),
        cfg.llm.provider.clone(),
    ));
    cfg.llm.api_key = Some(read_input(
        &format!("API key [{}]: ", mask_key(cfg.llm.api_key.as_deref())),
        cfg.llm.api_key.clone(),
    ));
    cfg.llm.model = Some(read_input(
        &format!("Model [{}]: ", cfg.llm.model.as_deref().unwrap_or("gpt-4")),
        cfg.llm.model.clone(),
    ));
    cfg.llm.base_url = Some(read_input(
        &format!(
            "Base URL [{}]: ",
            cfg.llm
                .base_url
                .as_deref()
                .unwrap_or("https://api.openai.com/v1")
        ),
        cfg.llm.base_url.clone(),
    ));
    println!();

    println!("[2/4] Discord Bot");
    cfg.discord.token = Some(read_input(
        &format!("Bot token [{}]: ", mask_key(cfg.discord.token.as_deref())),
        cfg.discord.token.clone(),
    ));
    let cid = read_input(
        &format!(
            "Channel ID (or empty for all channels) [{}]: ",
            cfg.discord.channel_id.as_deref().unwrap_or("")
        ),
        cfg.discord.channel_id.clone(),
    );
    if cid.is_empty() {
        cfg.discord.channel_id = None;
    } else {
        cfg.discord.channel_id = Some(cid);
    }
    println!();

    println!("[3/4] Paths");
    cfg.paths.atomic_repo = Some(read_input(
        &format!(
            "Atomic repo path [{}]: ",
            cfg.paths.atomic_repo.as_deref().unwrap_or("~/Atomic")
        ),
        cfg.paths.atomic_repo.clone(),
    ));
    cfg.paths.opencode_bin = Some(read_input(
        &format!(
            "Opencode binary [{}]: ",
            cfg.paths
                .opencode_bin
                .as_deref()
                .unwrap_or("~/.opencode/bin/opencode")
        ),
        cfg.paths.opencode_bin.clone(),
    ));
    println!();

    println!("[4/4] MCP Servers");
    println!("  Add servers in format: name|command|arg1,arg2");
    println!("  Example: my_tools|/usr/bin/my-mcp|--port,8080");
    println!("  Empty line to skip.");
    loop {
        let entry = read_input("Add MCP server (or blank to finish): ", None);
        if entry.trim().is_empty() {
            break;
        }
        let parts: Vec<&str> = entry.splitn(3, '|').collect();
        if parts.len() < 2 {
            println!("  Format: name|command|arg1,arg2");
            continue;
        }
        let args: Vec<String> = parts
            .get(2)
            .unwrap_or(&"")
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        cfg.mcp.servers.push(core::config::McpServerEntry {
            name: parts[0].to_string(),
            command: parts[1].to_string(),
            args,
        });
        println!("  Added: {}", parts[0]);
    }

    cfg.save()?;
    println!("\n✓ Config saved to config.toml");
    println!("Run `command_central discord` to start the bot.");
    Ok(())
}

fn read_input(prompt: &str, default: Option<String>) -> String {
    use std::io::Write;
    print!("{prompt}");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok();
    let val = line.trim().to_string();
    if val.is_empty() {
        default.unwrap_or_default()
    } else {
        val
    }
}

fn mask_key(key: Option<&str>) -> String {
    match key {
        Some(k) if k.len() > 8 => format!("{}...{}", &k[..4], &k[k.len() - 4..]),
        Some(k) => k.to_string(),
        None => "not set".to_string(),
    }
}

fn agent_system_prompt() -> String {
    let os = std::env::consts::OS;
    let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());
    let pwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".to_string());

    format!(
        "You are a coding and utility AI agent running in a terminal.\n\
         Environment: OS={os}, user={user}, host={host}, cwd={pwd}\n\
         You have access to tools: shell, read_file, write_file, grep_search, system_info, list_directory, web_fetch, web_search.\n\
         Use them to help the user with their tasks. Be concise and direct.\n\
         Safety: before running destructive commands, warn the user. Never expose API keys or secrets."
    )
}
