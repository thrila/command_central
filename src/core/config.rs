use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub llm: LlmSection,
    #[serde(default)]
    pub discord: DiscordSection,
    #[serde(default)]
    pub paths: PathsSection,
    #[serde(default)]
    pub mcp: McpSection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSection {
    pub provider: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
}

impl Default for LlmSection {
    fn default() -> Self {
        Self {
            provider: Some("openai".to_string()),
            api_key: None,
            model: Some("gpt-4".to_string()),
            base_url: Some("https://api.openai.com/v1".to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordSection {
    pub token: Option<String>,
    pub channel_id: Option<String>,
}

impl Default for DiscordSection {
    fn default() -> Self {
        Self {
            token: None,
            channel_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathsSection {
    pub atomic_repo: Option<String>,
    pub opencode_bin: Option<String>,
}

impl Default for PathsSection {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/user".to_string());
        Self {
            atomic_repo: Some(format!("{home}/Atomic")),
            opencode_bin: Some(format!("{home}/.opencode/bin/opencode")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpSection {
    #[serde(default)]
    pub servers: Vec<McpServerEntry>,
}

impl Default for McpSection {
    fn default() -> Self {
        Self {
            servers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerEntry {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

impl Config {
    pub fn load() -> Self {
        let paths = [
            "config.toml",
            "command_central.toml",
            &std::env::var("HOME").unwrap_or_default(),
        ];

        for path in &paths {
            if Path::new(path).exists() {
                if let Ok(content) = std::fs::read_to_string(path) {
                    if let Ok(config) = toml::from_str::<Config>(&content) {
                        println!("Loaded config from {path}");
                        return config.merge_env();
                    }
                }
            }
        }

        // Try $HOME/.config/command_central/config.toml
        let home_config = format!(
            "{}/.config/command_central/config.toml",
            std::env::var("HOME").unwrap_or_default()
        );
        if Path::new(&home_config).exists() {
            if let Ok(content) = std::fs::read_to_string(&home_config) {
                if let Ok(config) = toml::from_str::<Config>(&content) {
                    println!("Loaded config from {home_config}");
                    return config.merge_env();
                }
            }
        }

        // Fallback: defaults + env vars
        Config::default().merge_env()
    }

    /// Environment variables override config file values
    fn merge_env(mut self) -> Self {
        if let Ok(v) = std::env::var("LLM_PROVIDER") {
            self.llm.provider = Some(v);
        }
        if let Ok(v) = std::env::var("LLM_API_KEY") {
            self.llm.api_key = Some(v);
        }
        if let Ok(v) = std::env::var("LLM_MODEL") {
            self.llm.model = Some(v);
        }
        if let Ok(v) = std::env::var("LLM_BASE_URL") {
            self.llm.base_url = Some(v);
        }
        if let Ok(v) = std::env::var("DISCORD_TOKEN") {
            self.discord.token = Some(v);
        }
        if let Ok(v) = std::env::var("DISCORD_CHANNEL_ID") {
            self.discord.channel_id = Some(v);
        }
        if let Ok(v) = std::env::var("ATOMIC_REPO_PATH") {
            self.paths.atomic_repo = Some(v);
        }
        if let Ok(v) = std::env::var("OPENCODE_BIN") {
            self.paths.opencode_bin = Some(v);
        }
        if let Ok(v) = std::env::var("MCP_SERVERS") {
            self.mcp = parse_mcp_env(&v);
        }
        self
    }

    pub fn save(&self) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write("config.toml", content)?;
        Ok(())
    }

    pub fn get_llm_config(&self) -> crate::agent::llm::LlmConfig {
        crate::agent::llm::LlmConfig {
            provider: self
                .llm
                .provider
                .clone()
                .unwrap_or_else(|| "openai".to_string()),
            api_key: self.llm.api_key.clone().unwrap_or_default(),
            model: self
                .llm
                .model
                .clone()
                .unwrap_or_else(|| "gpt-4".to_string()),
            base_url: self
                .llm
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
        }
    }

    pub fn format_report(&self) -> String {
        let mut r = String::from("**Config**\n");
        r.push_str(&format!(
            "LLM: {} / {} / {}\n",
            self.llm.provider.as_deref().unwrap_or("?"),
            self.llm.model.as_deref().unwrap_or("?"),
            if self.llm.api_key.is_some() {
                "key set ✓"
            } else {
                "no key ✗"
            }
        ));
        r.push_str(&format!(
            "Discord: {}",
            if self.discord.token.is_some() {
                "connected ✓"
            } else {
                "not set ✗"
            }
        ));
        if let Some(ref cid) = self.discord.channel_id {
            r.push_str(&format!(" (channel: {cid})"));
        }
        r.push('\n');
        r.push_str(&format!(
            "Paths: Atomic={}, Opencode={}\n",
            self.paths.atomic_repo.as_deref().unwrap_or("?"),
            self.paths.opencode_bin.as_deref().unwrap_or("?")
        ));
        r.push_str(&format!("MCP servers: {}\n", self.mcp.servers.len()));
        for s in &self.mcp.servers {
            r.push_str(&format!("  - {}: {} {:?}\n", s.name, s.command, s.args));
        }
        r
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            llm: LlmSection::default(),
            discord: DiscordSection::default(),
            paths: PathsSection::default(),
            mcp: McpSection::default(),
        }
    }
}

fn parse_mcp_env(val: &str) -> McpSection {
    let mut servers = Vec::new();
    for entry in val.split(';') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let parts: Vec<&str> = entry.splitn(3, '|').collect();
        if parts.len() < 2 {
            continue;
        }
        let args: Vec<String> = parts
            .get(2)
            .unwrap_or(&"")
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.trim().to_string())
            .collect();
        servers.push(McpServerEntry {
            name: parts[0].trim().to_string(),
            command: parts[1].trim().to_string(),
            args,
        });
    }
    McpSection { servers }
}
