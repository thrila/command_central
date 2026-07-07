use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command as StdCommand, Stdio};
use std::sync::Mutex;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct McpInitializeResult {
    protocol_version: String,
    capabilities: Value,
    server_info: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct ListToolsResult {
    tools: Vec<McpTool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CallToolResult {
    content: Vec<McpContentItem>,
    is_error: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct McpContentItem {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
    data: Option<String>,
    mime_type: Option<String>,
}

pub struct McpClient {
    stdin: Mutex<Box<dyn Write + Send + Sync>>,
    reader_rx: Mutex<tokio::sync::mpsc::Receiver<String>>,
    reader_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    next_id: Mutex<u64>,
    pub tools: Mutex<Vec<McpTool>>,
    pub name: String,
}

impl Drop for McpClient {
    fn drop(&mut self) {
        if let Some(handle) = self.reader_handle.lock().unwrap().take() {
            handle.abort();
        }
    }
}

impl McpClient {
    pub fn spawn(command: &str, args: &[&str], name: &str) -> Result<Self> {
        let mut child = StdCommand::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().ok_or_else(|| anyhow!("No stdout"))?;
        let stdin: Box<dyn Write + Send + Sync> = Box::new(
            child.stdin.take().ok_or_else(|| anyhow!("No stdin"))?,
        );

        let (reader_tx, reader_rx) = tokio::sync::mpsc::channel::<String>(256);

        let reader_handle = tokio::task::spawn_blocking(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        if reader_tx.blocking_send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let client = Self {
            stdin: Mutex::new(stdin),
            reader_rx: Mutex::new(reader_rx),
            reader_handle: Mutex::new(Some(reader_handle)),
            next_id: Mutex::new(1),
            tools: Mutex::new(Vec::new()),
            name: name.to_string(),
        };

        client.initialize()?;
        client.discover_tools()?;

        Ok(client)
    }

    fn next_id(&self) -> u64 {
        let mut id = self.next_id.lock().unwrap();
        let current = *id;
        *id += 1;
        current
    }

    fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.next_id();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        };

        let req_str = serde_json::to_string(&req)?;
        let mut stdin = self.stdin.lock().unwrap();
        writeln!(stdin, "{req_str}")?;
        stdin.flush()?;
        drop(stdin);

        let mut line = String::new();
        while line.trim().is_empty() {
            line = self
                .reader_rx
                .lock()
                .unwrap()
                .blocking_recv()
                .ok_or_else(|| anyhow!("MCP server closed connection"))?;
        }

        if line.trim().is_empty() {
            return Err(anyhow!("Empty response from MCP server"));
        }

        let resp: JsonRpcResponse = serde_json::from_str(line.trim())?;

        if let Some(e) = resp.error {
            return Err(anyhow!("MCP error {}: {}", e.code, e.message));
        }

        resp.result.ok_or_else(|| anyhow!("No result in response"))
    }

    fn initialize(&self) -> Result<()> {
        let params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "command_central",
                "version": "0.2.0"
            }
        });
        let result = self.send_request("initialize", Some(params))?;
        let _: McpInitializeResult = serde_json::from_value(result)?;

        let notif = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });
        let mut stdin = self.stdin.lock().unwrap();
        writeln!(stdin, "{}", serde_json::to_string(&notif)?)?;
        stdin.flush()?;
        drop(stdin);

        Ok(())
    }

    fn discover_tools(&self) -> Result<()> {
        let result = self.send_request("tools/list", None)?;
        let list: ListToolsResult = serde_json::from_value(result)?;
        let mut tools = self.tools.lock().unwrap();
        *tools = list.tools;
        Ok(())
    }

    pub fn call_tool(&self, name: &str, arguments: Value) -> Result<String> {
        let params = json!({
            "name": name,
            "arguments": arguments
        });
        let result = self.send_request("tools/call", Some(params))?;
        let call_result: CallToolResult = serde_json::from_value(result)?;

        let mut output = String::new();
        for item in &call_result.content {
            match item.content_type.as_str() {
                "text" | "resource" | _ => {
                    if let Some(ref text) = item.text {
                        output.push_str(text);
                    }
                }
            }
            output.push('\n');
        }
        Ok(output.trim().to_string())
    }

    pub fn tool_definitions(&self) -> Vec<crate::agent::tools::ToolDefinition> {
        let tools = self.tools.lock().unwrap();
        tools
            .iter()
            .map(|t| {
                let schema = t
                    .input_schema
                    .clone()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
                crate::agent::tools::ToolDefinition {
                    name: format!("mcp_{}_{}", self.name, t.name),
                    description: t
                        .description
                        .clone()
                        .unwrap_or_else(|| format!("MCP tool: {}", t.name)),
                    parameters: schema,
                    mcp_server: Some(self.name.clone()),
                    mcp_tool: Some(t.name.clone()),
                }
            })
            .collect()
    }
}

pub fn load_mcp_servers() -> Vec<McpClient> {
    let config_str = std::env::var("MCP_SERVERS").unwrap_or_default();
    let mut clients = Vec::new();

    for entry in config_str.split(';') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let parts: Vec<&str> = entry.splitn(3, '|').collect();
        if parts.len() < 2 {
            continue;
        }
        let name = parts[0].trim();
        let command = parts[1].trim();
        let args_str = parts.get(2).unwrap_or(&"");
        let args: Vec<&str> = if args_str.is_empty() {
            vec![]
        } else {
            args_str.split(',').map(|s| s.trim()).collect()
        };

        match McpClient::spawn(command, &args, name) {
            Ok(client) => {
                println!("MCP connected: {name} ({command})");
                clients.push(client);
            }
            Err(e) => {
                eprintln!("MCP failed to start {name}: {e}");
            }
        }
    }

    clients
}

pub fn load_mcp_servers_from_config(config: &crate::core::config::Config) -> Vec<McpClient> {
    let mut clients = Vec::new();
    for server in &config.mcp.servers {
        let args: Vec<&str> = server.args.iter().map(|s| s.as_str()).collect();
        match McpClient::spawn(&server.command, &args, &server.name) {
            Ok(client) => {
                println!("MCP connected: {} ({})", server.name, server.command);
                clients.push(client);
            }
            Err(e) => {
                eprintln!("MCP failed to start {}: {e}", server.name);
            }
        }
    }
    clients
}
