use super::gate::ApprovalGate;
use super::llm::{chat_with_agent, LlmConfig, Message, ProgressEvent};
use super::tools::get_builtin_tool_definitions;
use crate::core::mcp::McpClient;
use std::sync::Arc;
use tokio::sync::watch;

pub struct AgentChat {
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub config: LlmConfig,
    pub mcp_clients: Vec<Arc<McpClient>>,
    pub cancel_tx: Option<watch::Sender<bool>>,
    pub approval_gate: Arc<ApprovalGate>,
    pub http_client: reqwest::Client,
    pub progress_tx: Option<tokio::sync::mpsc::UnboundedSender<ProgressEvent>>,
}

impl AgentChat {
    pub fn new(
        config: LlmConfig,
        mcp_clients: Vec<Arc<McpClient>>,
        http_client: reqwest::Client,
        dynamic_context: Option<&str>,
    ) -> Self {
        let (cancel_tx, _cancel_rx) = watch::channel(false);

        let mut tools_desc = String::new();
        let builtin = get_builtin_tool_definitions();
        for t in &builtin {
            tools_desc.push_str(&format!("- `{}`: {}\n", t.name, t.description));
        }
        for client in &mcp_clients {
            for t in client.tool_definitions() {
                tools_desc.push_str(&format!(
                    "- `{}` (MCP/{}): {}\n",
                    t.name,
                    t.mcp_server.as_deref().unwrap_or("?"),
                    t.description
                ));
            }
        }

        let os_info = std::env::consts::OS;
        let hostname = std::env::var("HOSTNAME")
            .unwrap_or_else(|_| std::env::var("HOST").unwrap_or_else(|_| "unknown".to_string()));
        let user = std::env::var("USER")
            .unwrap_or_else(|_| std::env::var("USERNAME").unwrap_or_else(|_| "user".to_string()));
        let pwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".to_string());

        let ctx_block = dynamic_context.unwrap_or("");

        let system_prompt = format!(
            "You are Command Central — a powerful coding and utility AI agent running on a self-hosted Linux server.\n\n\
            == ENVIRONMENT ==\n\
            OS: {os_info}\n\
            User: {user}\n\
            Hostname: {hostname}\n\
            Working directory: {pwd}\n\
            {ctx_block}\n\n\
            == CORE IDENTITY ==\n\
            You are an autonomous engineering agent. Your job is to help the user with coding, system administration, \
            automation, and any technical task. You have full shell access to the machine.\n\n\
            == TOOLS AVAILABLE ==\n\
            You have the following tools at your disposal. Use them freely to accomplish tasks:\n\n\
            {tools_desc}\n\
            \n\
            == SAFETY RULES ==\n\
            1. Before running destructive commands (rm, mv, git push --force, docker rm, etc.), warn the user.\n\
            2. Before writing to files outside the current project directory, ask for confirmation.\n\
            3. When uncertain about a command's effect, explain your concern before executing.\n\
            4. Never delete or modify files in ~/.ssh, ~/.gnupg, or other security directories.\n\
            5. Never expose API keys, tokens, or secrets in output.\n\n\
            == HOW TO USE TOOLS ==\n\
            Always think step by step. When a task requires multiple steps, use tools sequentially. \
            For example: to fix a bug, first search the code, read the files, then write the fix. \
            To check system health, use shell commands to inspect processes, logs, and resources.\n\n\
            == BEHAVIOR RULES ==\n\
            1. Be concise and direct. No fluff.\n\
            2. When given a task, immediately start working on it using your tools.\n\
            3. If you need more info, ask the user clearly.\n\
            4. Show the output/results of your actions.\n\
            5. For coding tasks: read relevant files first, understand the codebase, make changes, verify.\n\
            6. For system tasks: check current state, make changes, verify the result.\n\
            7. You can browse the web (web_fetch, web_search) to look up documentation or solutions.\n\
            8. You can run any shell command — compilers, tests, git, docker, npm, cargo, etc.\n\
            9. Always verify your work when possible.\n\
            10. If something fails, diagnose and retry.\n\
            11. If you're uncertain about something, say so instead of guessing.\n\
            12. End your response with [DONE] when the task is complete.\n\
            \n\
            Remember: you're talking to a human who can see your responses. Keep them informed of what you're doing."
        );

        Self {
            system_prompt,
            messages: Vec::new(),
            config,
            mcp_clients,
            cancel_tx: Some(cancel_tx),
            approval_gate: Arc::new(ApprovalGate::new()),
            http_client,
            progress_tx: None,
        }
    }

    pub fn is_configured(&self) -> bool {
        self.config.is_configured()
    }

    pub fn cancel(&self) {
        if let Some(ref tx) = self.cancel_tx {
            let _ = tx.send(true);
        }
    }

    pub fn new_cancel_token(&mut self) -> watch::Receiver<bool> {
        let (tx, rx) = watch::channel(false);
        self.cancel_tx = Some(tx);
        rx
    }

    pub async fn send_message(&mut self, user_input: &str) -> String {
        if self.messages.len() > 100 {
            let mut tail = self.messages.split_off(self.messages.len() - 50);
            std::mem::swap(&mut self.messages, &mut tail);
        }

        self.messages.push(Message {
            role: "user".to_string(),
            content: user_input.to_string(),
        });

        let cancel_rx = self.new_cancel_token();

        let result = chat_with_agent(
            &self.config,
            &self.system_prompt,
            &self.messages,
            &self.mcp_clients,
            &self.http_client,
            Some(cancel_rx),
            Some(&self.approval_gate),
            self.progress_tx.as_ref(),
        )
        .await;

        match result {
            Ok(response) => {
                self.messages.push(Message {
                    role: "assistant".to_string(),
                    content: response.clone(),
                });
                response
            }
            Err(e) => {
                let err_msg = format!("Agent error: {e}");
                self.messages.push(Message {
                    role: "assistant".to_string(),
                    content: err_msg.clone(),
                });
                err_msg
            }
        }
    }

    pub fn reset(&mut self) {
        self.messages.clear();
        self.approval_gate.reset_approvals();
    }
}
