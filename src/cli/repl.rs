use crate::agent::llm::{chat_with_agent, LlmConfig, Message};
use crate::core::task::Task;
use crate::utils::shell;
use tokio::sync::mpsc;

pub async fn run_repl(tx: mpsc::Sender<Task>) -> anyhow::Result<()> {
    let config = LlmConfig::from_env();
    let agent_available = config.is_configured();

    let http_client = reqwest::Client::builder()
        .user_agent("command_central/0.2.0")
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    println!("╔══════════════════════════════════════════╗");
    println!("║     Command Central — Agent Terminal     ║");
    println!("╠══════════════════════════════════════════╣");
    println!("║ Commands:                                ║");
    println!("║  shell <cmd>    Run a shell command      ║");
    println!("║  ask <query>    Ask the AI agent         ║");
    println!("║  history       Show task history         ║");
    println!("║  help          Show this help            ║");
    println!("║  exit          Exit                      ║");
    println!("╚══════════════════════════════════════════╝");
    if !agent_available {
        println!("[!] LLM not configured — 'ask' mode requires LLM_API_KEY in .env");
    }
    println!();

    loop {
        let input = {
            let mut line = String::new();
            std::io::Write::flush(&mut std::io::stdout())?;
            let bytes = std::io::stdin().read_line(&mut line)?;
            if bytes == 0 {
                break;
            }
            line.trim().to_string()
        };

        if input.is_empty() {
            continue;
        }

        let (cmd, rest) = input
            .split_once(char::is_whitespace)
            .unwrap_or((&input, ""));

        match cmd {
            "exit" | "quit" => {
                println!("Goodbye.");
                break;
            }
            "help" => {
                println!("shell <cmd>     Run a shell command");
                println!("ask <query>     Ask the AI agent (needs LLM config)");
                println!("history        Show task history");
                println!("help           Show this help");
                println!("exit           Exit");
            }
            "shell" => {
                let rest = rest.trim();
                if rest.is_empty() {
                    println!("Usage: shell <command>");
                    continue;
                }
                match shell::run_shell(rest) {
                    Ok((stdout, stderr, status)) => {
                        if !stdout.is_empty() {
                            println!("{stdout}");
                        }
                        if !stderr.is_empty() {
                            eprintln!("STDERR: {stderr}");
                        }
                        println!("Exit: {:?}", status.code());
                    }
                    Err(e) => eprintln!("Error: {e}"),
                }
            }
            "ask" => {
                let rest = rest.trim();
                if rest.is_empty() {
                    println!("Usage: ask <question>");
                    continue;
                }
                if !agent_available {
                    println!("LLM not configured. Set LLM_API_KEY in .env");
                    continue;
                }
                let system_prompt = "You are a coding and utility AI agent running in a terminal. You have access to tools: shell, read_file, write_file, grep_search, system_info, list_directory, web_fetch, web_search. Use them to help the user with their tasks. Be concise and direct.";
                let messages = vec![Message {
                    role: "user".to_string(),
                    content: rest.to_string(),
                }];

                eprint!("Thinking... ");
                match chat_with_agent(
                    &config,
                    system_prompt,
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
                    Err(e) => eprintln!("\nAgent error: {e}"),
                }
            }
            "history" => {
                let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
                let _ = tx.send(Task::History(resp_tx)).await;
                match resp_rx.await {
                    Ok(rows) => {
                        if rows.is_empty() {
                            println!("No task history yet.");
                        } else {
                            for (kind, status, id) in &rows {
                                println!("  [{id}] {kind:40} {status}");
                            }
                        }
                    }
                    Err(_) => eprintln!("Failed to fetch history."),
                }
            }
            _ => {
                println!("Unknown command: {cmd}. Type 'help' for available commands.");
            }
        }
    }

    Ok(())
}
