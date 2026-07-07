use crate::utils::shell;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::process::Command;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    #[serde(skip)]
    pub mcp_server: Option<String>,
    #[serde(skip)]
    pub mcp_tool: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolResult {
    pub name: String,
    pub output: String,
    pub success: bool,
}

pub fn get_builtin_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "shell".to_string(),
            description: "Execute any shell command on the system. Use this for running code, scripts, git, npm, cargo, docker, or any CLI tool.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    }
                },
                "required": ["command"]
            }),
            mcp_server: None,
            mcp_tool: None,
        },
        ToolDefinition {
            name: "read_file".to_string(),
            description: "Read the contents of any file on the system".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative path to the file"
                    }
                },
                "required": ["path"]
            }),
            mcp_server: None,
            mcp_tool: None,
        },
        ToolDefinition {
            name: "write_file".to_string(),
            description: "Write or overwrite content to a file. Creates parent directories if needed.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the file"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write"
                    }
                },
                "required": ["path", "content"]
            }),
            mcp_server: None,
            mcp_tool: None,
        },
        ToolDefinition {
            name: "grep_search".to_string(),
            description: "Search for a regex pattern in files within a directory".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Regex pattern to search for"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to search in (default: .)"
                    }
                },
                "required": ["pattern"]
            }),
            mcp_server: None,
            mcp_tool: None,
        },
        ToolDefinition {
            name: "system_info".to_string(),
            description: "Get detailed system information: OS, CPU, memory, disk, uptime, load".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            mcp_server: None,
            mcp_tool: None,
        },
        ToolDefinition {
            name: "list_directory".to_string(),
            description: "List contents of a directory with details".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path (default: .)"
                    }
                },
                "required": []
            }),
            mcp_server: None,
            mcp_tool: None,
        },
        ToolDefinition {
            name: "web_fetch".to_string(),
            description: "Fetch and read content from a URL. Use this to browse documentation, APIs, or any web page.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch"
                    },
                    "format": {
                        "type": "string",
                        "enum": ["markdown", "text", "html"],
                        "description": "Output format (default: markdown)"
                    }
                },
                "required": ["url"]
            }),
            mcp_server: None,
            mcp_tool: None,
        },
        ToolDefinition {
            name: "web_search".to_string(),
            description: "Search the web for information. Use this to find docs, answers, news, or any online content.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "count": {
                        "type": "integer",
                        "description": "Number of results (default: 5)"
                    }
                },
                "required": ["query"]
            }),
            mcp_server: None,
            mcp_tool: None,
        },
    ]
}

pub async fn execute_builtin_tool(name: &str, args: &serde_json::Value) -> ToolResult {
    match name {
        "shell" => {
            let cmd = args["command"].as_str().unwrap_or("");
            match shell::run_shell(cmd) {
                Ok((stdout, stderr, status)) => {
                    let output = if status.success() {
                        stdout
                    } else {
                        format!(
                            "Exit: {:?}\nSTDERR: {}\nSTDOUT: {}",
                            status.code(),
                            stderr,
                            stdout
                        )
                    };
                    ToolResult {
                        name: name.to_string(),
                        output,
                        success: status.success(),
                    }
                }
                Err(e) => ToolResult {
                    name: name.to_string(),
                    output: format!("Error: {e}"),
                    success: false,
                },
            }
        }

        "read_file" => {
            let path = args["path"].as_str().unwrap_or("");
            match tokio::fs::read_to_string(path).await {
                Ok(content) => ToolResult {
                    name: name.to_string(),
                    output: content,
                    success: true,
                },
                Err(e) => ToolResult {
                    name: name.to_string(),
                    output: format!("Error reading file: {e}"),
                    success: false,
                },
            }
        }

        "write_file" => {
            let path = args["path"].as_str().unwrap_or("");
            let content = args["content"].as_str().unwrap_or("");
            // Ensure parent dir exists
            if let Some(parent) = std::path::Path::new(path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match tokio::fs::write(path, content).await {
                Ok(_) => ToolResult {
                    name: name.to_string(),
                    output: format!("Written {} bytes to {}", content.len(), path),
                    success: true,
                },
                Err(e) => ToolResult {
                    name: name.to_string(),
                    output: format!("Error writing file: {e}"),
                    success: false,
                },
            }
        }

        "grep_search" => {
            let pattern = args["pattern"].as_str().unwrap_or("");
            let path = args["path"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(".");
            let output = Command::new("grep").args(["-rne", pattern, path]).output();
            match output {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                    let text = if o.status.success() {
                        stdout
                    } else if !stderr.is_empty() {
                        format!("No matches. Error: {stderr}")
                    } else {
                        "No matches.".to_string()
                    };
                    ToolResult {
                        name: name.to_string(),
                        output: text,
                        success: o.status.success(),
                    }
                }
                Err(e) => ToolResult {
                    name: name.to_string(),
                    output: format!("Error: {e}"),
                    success: false,
                },
            }
        }

        "system_info" => {
            let uname = shell::run_shell("uname -a").ok();
            let mem = shell::run_shell("free -h 2>/dev/null || echo 'N/A'").ok();
            let disk = shell::run_shell("df -h / 2>/dev/null || echo 'N/A'").ok();
            let cpu = shell::run_shell(
                "echo \"Cores: $(nproc 2>/dev/null)\"; cat /proc/cpuinfo 2>/dev/null | grep 'model name' | head -1 || echo 'N/A'",
            ).ok();
            let uptime =
                shell::run_shell("uptime -p 2>/dev/null || uptime 2>/dev/null || echo 'N/A'").ok();
            let load = shell::run_shell("cat /proc/loadavg 2>/dev/null || echo 'N/A'").ok();
            let output = format!(
                "OS: {}\nMemory:\n{}\nDisk:\n{}\nCPU:\n{}\nUptime: {}\nLoad: {}",
                uname
                    .map(|(s, _, _)| s.trim().to_string())
                    .unwrap_or_default(),
                mem.map(|(s, _, _)| s).unwrap_or_default(),
                disk.map(|(s, _, _)| s).unwrap_or_default(),
                cpu.map(|(s, _, _)| s).unwrap_or_default(),
                uptime
                    .map(|(s, _, _)| s.trim().to_string())
                    .unwrap_or_default(),
                load.map(|(s, _, _)| s.trim().to_string())
                    .unwrap_or_default(),
            );
            ToolResult {
                name: name.to_string(),
                output,
                success: true,
            }
        }

        "list_directory" => {
            let path = args["path"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(".");
            let output = Command::new("ls").args(["-la", path]).output();
            match output {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                    let text = if o.status.success() { stdout } else { stderr };
                    ToolResult {
                        name: name.to_string(),
                        output: text,
                        success: o.status.success(),
                    }
                }
                Err(e) => ToolResult {
                    name: name.to_string(),
                    output: format!("Error: {e}"),
                    success: false,
                },
            }
        }

        "web_fetch" => {
            let url = args["url"].as_str().unwrap_or("");
            if url.is_empty() {
                return ToolResult {
                    name: name.to_string(),
                    output: "No URL provided".to_string(),
                    success: false,
                };
            }
            match fetch_url(url).await {
                Ok(content) => ToolResult {
                    name: name.to_string(),
                    output: content,
                    success: true,
                },
                Err(e) => ToolResult {
                    name: name.to_string(),
                    output: format!("Error fetching URL: {e}"),
                    success: false,
                },
            }
        }

        "web_search" => {
            let query = args["query"].as_str().unwrap_or("");
            if query.is_empty() {
                return ToolResult {
                    name: name.to_string(),
                    output: "No query provided".to_string(),
                    success: false,
                };
            }
            let count = args["count"].as_i64().unwrap_or(5).max(1).min(20) as usize;
            match search_web(query, count).await {
                Ok(results) => ToolResult {
                    name: name.to_string(),
                    output: results,
                    success: true,
                },
                Err(e) => ToolResult {
                    name: name.to_string(),
                    output: format!("Search error: {e}"),
                    success: false,
                },
            }
        }

        _ => ToolResult {
            name: name.to_string(),
            output: format!("Unknown tool: {name}"),
            success: false,
        },
    }
}

async fn fetch_url(url: &str) -> Result<String, anyhow::Error> {
    let client = reqwest::Client::builder()
        .user_agent("command_central/0.2.0")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let resp = client.get(url).send().await?;
    let status = resp.status();
    let text = resp.text().await?;

    // Strip HTML tags for cleaner output
    let cleaned = strip_html(&text);
    let max_len = 8000;
    let truncated = utf8_truncate(&cleaned, max_len);
    let content = if cleaned.len() > max_len {
        format!(
            "[Status: {status}]\n{}...\n[Truncated: {} chars]",
            truncated,
            cleaned.len()
        )
    } else {
        format!("[Status: {status}]\n{}", cleaned)
    };
    Ok(content)
}

fn strip_html(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let chars: Vec<char> = html.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if !in_tag && i + 6 < chars.len() {
            let lower: String = chars[i..i + 7].iter().collect::<String>().to_lowercase();
            if lower.starts_with("<script") {
                in_script = true;
                in_tag = true;
                i += 1;
                continue;
            }
            if lower.starts_with("<style") {
                in_style = true;
                in_tag = true;
                i += 1;
                continue;
            }
        }
        if in_script && i + 8 < chars.len() {
            if chars[i..i + 9].iter().collect::<String>().to_lowercase() == "</script>" {
                in_script = false;
                in_tag = true;
                i += 1;
                continue;
            }
        }
        if in_style && i + 7 < chars.len() {
            if chars[i..i + 8].iter().collect::<String>().to_lowercase() == "</style>" {
                in_style = false;
                in_tag = true;
                i += 1;
                continue;
            }
        }
        if in_script || in_style {
            i += 1;
            continue;
        }

        if chars[i] == '<' {
            in_tag = true;
            i += 1;
            continue;
        }
        if chars[i] == '>' {
            in_tag = false;
            i += 1;
            continue;
        }
        if !in_tag {
            if chars[i] == '&' {
                // Skip HTML entities
                while i < chars.len() && chars[i] != ';' {
                    i += 1;
                }
                i += 1;
                continue;
            }
            result.push(chars[i]);
        }
        i += 1;
    }

    // Collapse whitespace
    let mut collapsed = String::new();
    let mut prev_space = false;
    for c in result.chars() {
        if c.is_whitespace() {
            if !prev_space {
                collapsed.push(' ');
                prev_space = true;
            }
        } else {
            collapsed.push(c);
            prev_space = false;
        }
    }
    collapsed.trim().to_string()
}

async fn search_web(query: &str, count: usize) -> Result<String, anyhow::Error> {
    // Use DuckDuckGo's instant answer API (no key needed)
    let url = format!(
        "https://api.duckduckgo.com/?q={}&format=json&no_html=1",
        urlencoding(&query)
    );

    let client = reqwest::Client::builder()
        .user_agent("command_central/0.2.0")
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let resp = client.get(&url).send().await?;
    let data: serde_json::Value = resp.json().await?;

    let mut results = String::new();

    // Abstract / answer
    if let Some(answer) = data["AbstractText"].as_str() {
        if !answer.is_empty() {
            results.push_str(&format!("**Summary:** {}\n\n", answer));
        }
    }

    // Results
    if let Some(results_arr) = data["RelatedTopics"].as_array() {
        let mut count_found = 0;
        for item in results_arr {
            if count_found >= count {
                break;
            }
            if let Some(text) = item["Text"].as_str() {
                if let Some(url) = item["FirstURL"].as_str() {
                    results.push_str(&format!("• {} — {}\n", text, url));
                    count_found += 1;
                }
            }
            if let Some(topics) = item["Topics"].as_array() {
                for topic in topics {
                    if count_found >= count {
                        break;
                    }
                    if let Some(text) = topic["Text"].as_str() {
                        if let Some(url) = topic["FirstURL"].as_str() {
                            results.push_str(&format!("• {} — {}\n", text, url));
                            count_found += 1;
                        }
                    }
                }
            }
        }
    }

    if results.is_empty() {
        // Fallback: try direct fetch of the search page
        results = format!(
            "No structured results for '{}'. Try using web_fetch on a search engine URL.",
            query
        );
    }

    Ok(results)
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "+".to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}

/// Truncate a string at a UTF-8 safe boundary near max_len bytes
fn utf8_truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    // Find the nearest char boundary at or before max
    let mut idx = max;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    &s[..idx]
}
