use super::gate::{ApprovalGate, ToolCategory};
use super::tools::{execute_builtin_tool, get_builtin_tool_definitions, ToolDefinition};
use crate::core::mcp::McpClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub provider: String,
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

impl LlmConfig {
    pub fn from_env() -> Self {
        Self {
            provider: std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "openai".to_string()),
            api_key: std::env::var("LLM_API_KEY").unwrap_or_default(),
            model: std::env::var("LLM_MODEL").unwrap_or_else(|_| "gpt-4".to_string()),
            base_url: std::env::var("LLM_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub enum ProgressEvent {
    Thinking(String),
    ToolStart {
        name: String,
        args: String,
    },
    ToolEnd {
        name: String,
        success: bool,
        summary: String,
    },
    Completed(String),
    Error(String),
    ApprovalNeeded {
        tool: String,
        args: String,
        message: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    tools: Option<Vec<ToolDef>>,
    tool_choice: Option<String>,
    max_tokens: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct ToolDef {
    #[serde(rename = "type")]
    tool_type: String,
    function: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Choice {
    message: ResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<ResponseToolCall>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ResponseToolCall {
    id: Option<String>,
    #[serde(rename = "type")]
    call_type: Option<String>,
    function: ResponseFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct ResponseFunction {
    name: String,
    arguments: String,
}

pub async fn chat_with_agent(
    config: &LlmConfig,
    system_prompt: &str,
    messages: &[Message],
    mcp_clients: &[Arc<McpClient>],
    http_client: &reqwest::Client,
    cancel_rx: Option<watch::Receiver<bool>>,
    approval_gate: Option<&ApprovalGate>,
    progress_tx: Option<&tokio::sync::mpsc::UnboundedSender<ProgressEvent>>,
) -> Result<String> {
    let all_tools = collect_tool_definitions(mcp_clients);
    let client = http_client;

    let mut all_messages = vec![Message {
        role: "system".to_string(),
        content: system_prompt.to_string(),
    }];
    all_messages.extend_from_slice(messages);

    let tool_defs: Vec<ToolDef> = all_tools
        .iter()
        .map(|t| ToolDef {
            tool_type: "function".to_string(),
            function: json!({
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters,
            }),
        })
        .collect();

    if let Some(ref tx) = progress_tx {
        let _ = tx.send(ProgressEvent::Thinking("Asking agent...".to_string()));
    }

    let req = ChatRequest {
        model: config.model.clone(),
        messages: all_messages.clone(),
        tools: Some(tool_defs),
        tool_choice: Some("auto".to_string()),
        max_tokens: 8192,
    };

    if let Some(ref rx) = cancel_rx {
        if *rx.borrow() {
            return Err(anyhow::anyhow!("Cancelled"));
        }
    }

    let resp = client
        .post(format!(
            "{}/chat/completions",
            config.base_url.trim_end_matches('/')
        ))
        .header("Authorization", format!("Bearer {}", config.api_key))
        .json(&req)
        .send()
        .await?;

    if let Some(ref rx) = cancel_rx {
        if *rx.borrow() {
            return Err(anyhow::anyhow!("Cancelled"));
        }
    }

    let chat_resp: ChatResponse = resp.json().await?;
    let choice = chat_resp
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No response from LLM"))?;

    if let Some(tool_calls) = choice.message.tool_calls {
        let mut results = Vec::new();

        for tc in &tool_calls {
            if let Some(ref rx) = cancel_rx {
                if *rx.borrow() {
                    return Err(anyhow::anyhow!("Cancelled"));
                }
            }

            let args: serde_json::Value =
                serde_json::from_str(&tc.function.arguments).unwrap_or_default();

            let cat = ToolCategory::for_tool(&tc.function.name);
            let args_preview = if tc.function.arguments.len() > 200 {
                format!("{}...", &tc.function.arguments[..200])
            } else {
                tc.function.arguments.clone()
            };

            if let Some(ref tx) = progress_tx {
                let _ = tx.send(ProgressEvent::ToolStart {
                    name: tc.function.name.clone(),
                    args: args_preview.clone(),
                });
            }

            if let Some(gate) = approval_gate {
                if let Some(msg) = gate.check(
                    &tc.function.name,
                    &args_preview,
                    &format!("{} operation", cat.description()),
                ) {
                    if let Some(ref tx) = progress_tx {
                        let _ = tx.send(ProgressEvent::ApprovalNeeded {
                            tool: tc.function.name.clone(),
                            args: args_preview.clone(),
                            message: msg.clone(),
                        });
                    }
                    results.push(format!(
                        "Tool `{}` requires approval: {msg}",
                        tc.function.name
                    ));
                    continue;
                }
            }

            let result =
                execute_dynamic_tool(&tc.function.name, &args, mcp_clients, cancel_rx.as_ref())
                    .await;
            let summary = if result.output.len() > 300 {
                format!(
                    "{}... ({} chars)",
                    &result.output[..300],
                    result.output.len()
                )
            } else {
                result.output.clone()
            };

            if let Some(ref tx) = progress_tx {
                let _ = tx.send(ProgressEvent::ToolEnd {
                    name: result.name.clone(),
                    success: result.success,
                    summary: summary.clone(),
                });
            }

            results.push(format!(
                "Result of `{}` ({}):\n{}",
                result.name,
                if result.success { "ok" } else { "failed" },
                result.output
            ));
        }

        let tool_output = results.join("\n---\n");

        if let Some(ref rx) = cancel_rx {
            if *rx.borrow() {
                return Err(anyhow::anyhow!("Cancelled"));
            }
        }

        let follow_up_req = ChatRequest {
            model: config.model.clone(),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                Message {
                    role: "assistant".to_string(),
                    content: format!(
                        "I'll use my tools to work on this.\n\nTool results:\n{}",
                        tool_output
                    ),
                },
                Message {
                    role: "user".to_string(),
                    content: format!(
                        "Continue based on the tool results above. If you've completed the task, respond with a concise summary. Use the `task_complete` signal: end your response with [DONE] when finished.\n\nTool results:\n{}",
                        tool_output
                    ),
                },
            ],
            tools: None,
            tool_choice: None,
            max_tokens: 8192,
        };

        let follow_up_resp = client
            .post(format!(
                "{}/chat/completions",
                config.base_url.trim_end_matches('/')
            ))
            .header("Authorization", format!("Bearer {}", config.api_key))
            .json(&follow_up_req)
            .send()
            .await?;

        let follow_up_chat: ChatResponse = follow_up_resp.json().await?;
        let content = follow_up_chat
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .unwrap_or_else(|| "Done.".to_string());

        if let Some(ref tx) = progress_tx {
            let _ = tx.send(ProgressEvent::Completed(content.clone()));
            let _ = tx.send(ProgressEvent::Completed(
                "[DONE] Task complete.".to_string(),
            ));
        }

        Ok(content)
    } else {
        let content = choice
            .message
            .content
            .unwrap_or_else(|| "No response".to_string());

        if let Some(ref tx) = progress_tx {
            let _ = tx.send(ProgressEvent::Completed(content.clone()));
        }

        Ok(content)
    }
}

fn collect_tool_definitions(mcp_clients: &[Arc<McpClient>]) -> Vec<ToolDefinition> {
    let mut tools = get_builtin_tool_definitions();
    for client in mcp_clients {
        tools.extend(client.tool_definitions());
    }
    tools
}

async fn execute_dynamic_tool(
    name: &str,
    args: &serde_json::Value,
    mcp_clients: &[Arc<McpClient>],
    cancel_rx: Option<&watch::Receiver<bool>>,
) -> super::tools::ToolResult {
    if let Some(ref rx) = cancel_rx {
        if *rx.borrow() {
            return super::tools::ToolResult {
                name: name.to_string(),
                output: "Cancelled by user.".to_string(),
                success: false,
            };
        }
    }

    if name.starts_with("mcp_") {
        for client in mcp_clients {
            let defs = client.tool_definitions();
            if defs.iter().any(|t| t.name == name) {
                if let (Some(server_name), Some(tool_name)) = (
                    defs.iter()
                        .find(|t| t.name == name)
                        .and_then(|t| t.mcp_server.clone()),
                    defs.iter()
                        .find(|t| t.name == name)
                        .and_then(|t| t.mcp_tool.clone()),
                ) {
                    if client.name == server_name {
                        match client.call_tool(&tool_name, args.clone()) {
                            Ok(output) => {
                                return super::tools::ToolResult {
                                    name: name.to_string(),
                                    output,
                                    success: true,
                                };
                            }
                            Err(e) => {
                                return super::tools::ToolResult {
                                    name: name.to_string(),
                                    output: format!("MCP error: {e}"),
                                    success: false,
                                };
                            }
                        }
                    }
                }
            }
        }
        return super::tools::ToolResult {
            name: name.to_string(),
            output: format!("No MCP client found for {name}"),
            success: false,
        };
    }

    execute_builtin_tool(name, args).await
}
