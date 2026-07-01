use std::process::Command;

#[derive(Debug, Clone)]
pub struct AgentInfo {
    pub name: String,
    pub path: Option<String>,
    pub version: Option<String>,
    pub kind: AgentKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentKind {
    CodingAgent,
    DevTool,
    Shell,
}

pub fn detect_agents() -> Vec<AgentInfo> {
    let candidates = vec![
        ("opencode", AgentKind::CodingAgent),
        ("aider", AgentKind::CodingAgent),
        ("cursor", AgentKind::CodingAgent),
        ("claude", AgentKind::CodingAgent),
        ("copilot", AgentKind::CodingAgent),
        ("gh", AgentKind::DevTool),
        ("git", AgentKind::DevTool),
        ("cargo", AgentKind::DevTool),
        ("rustc", AgentKind::DevTool),
        ("node", AgentKind::DevTool),
        ("npm", AgentKind::DevTool),
        ("bun", AgentKind::DevTool),
        ("deno", AgentKind::DevTool),
        ("python3", AgentKind::DevTool),
        ("python", AgentKind::DevTool),
        ("go", AgentKind::DevTool),
        ("docker", AgentKind::DevTool),
        ("docker-compose", AgentKind::DevTool),
        ("tmux", AgentKind::DevTool),
        ("nvim", AgentKind::DevTool),
        ("vim", AgentKind::DevTool),
        ("code", AgentKind::DevTool),
        ("nmap", AgentKind::DevTool),
        ("curl", AgentKind::DevTool),
        ("wget", AgentKind::DevTool),
        ("jq", AgentKind::DevTool),
        ("yq", AgentKind::DevTool),
        ("htop", AgentKind::DevTool),
        ("btm", AgentKind::DevTool),
        ("lazygit", AgentKind::DevTool),
        ("delta", AgentKind::DevTool),
        ("fzf", AgentKind::DevTool),
        ("rg", AgentKind::DevTool),
        ("bat", AgentKind::DevTool),
        ("zellij", AgentKind::DevTool),
        ("screenfetch", AgentKind::DevTool),
        ("neofetch", AgentKind::DevTool),
        ("fastfetch", AgentKind::DevTool),
    ];

    let mut agents = Vec::new();
    for (name, kind) in candidates {
        let info = check_binary(name, &kind);
        agents.push(info);
    }
    agents
}

fn check_binary(name: &str, kind: &AgentKind) -> AgentInfo {
    let path = which(name);

    let version = if let Some(ref p) = path {
        get_version(name, p)
    } else {
        None
    };

    // Also check common non-PATH locations
    let path = path.or_else(|| check_common_locations(name));

    AgentInfo {
        name: name.to_string(),
        path,
        version,
        kind: kind.clone(),
    }
}

fn which(name: &str) -> Option<String> {
    Command::new("which")
        .arg(name)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        })
}

fn get_version(name: &str, path: &str) -> Option<String> {
    let flags = match name {
        "opencode" => &["--version"][..],
        "aider" => &["--version"],
        "nvim" => &["--version", "--headless", "-c", "qall"],
        "vim" => &["--version"],
        "python3" | "python" => &["--version"],
        "node" => &["--version"],
        "npm" => &["--version"],
        "bun" => &["--version"],
        "deno" => &["--version"],
        "cargo" => &["--version"],
        "rustc" => &["--version"],
        "go" => &["version"],
        "gh" => &["--version"],
        "git" => &["--version"],
        "docker" => &["--version"],
        "docker-compose" => &["--version"],
        "code" => &["--version"],
        "tmux" => &["-V"],
        _ => &["--version"],
    };

    Command::new(path)
        .args(flags)
        .output()
        .ok()
        .and_then(|o| {
            let out = String::from_utf8_lossy(&o.stdout).to_string()
                + &String::from_utf8_lossy(&o.stderr);
            let first = out.lines().next().unwrap_or("").trim().to_string();
            if first.is_empty() { None } else { Some(first) }
        })
}

fn check_common_locations(name: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let paths = vec![
        format!("{home}/.opencode/bin/{name}"),
        format!("{home}/.local/bin/{name}"),
        format!("{home}/.cargo/bin/{name}"),
        format!("{home}/go/bin/{name}"),
        format!("{home}/.bun/bin/{name}"),
        format!("{home}/.deno/bin/{name}"),
        format!("{home}/.npm-global/bin/{name}"),
        format!("/usr/local/bin/{name}"),
        format!("/opt/homebrew/bin/{name}"),
    ];
    for p in &paths {
        if std::path::Path::new(p).exists() {
            return Some(p.clone());
        }
    }
    None
}

pub fn format_agent_report(agents: &[AgentInfo]) -> String {
    let mut report = String::from("**Installed Agents & Tools**\n");
    let mut coding = Vec::new();
    let mut tools = Vec::new();

    for a in agents {
        match a.kind {
            AgentKind::CodingAgent => coding.push(a),
            AgentKind::DevTool => tools.push(a),
            AgentKind::Shell => {}
        }
    }

    if !coding.is_empty() {
        report.push_str("\n**Coding Agents:**\n");
        for a in &coding {
            let status = if a.path.is_some() { "✅" } else { "❌" };
            let ver = a.version.as_deref().unwrap_or("-");
            report.push_str(&format!("  {status} {}: {ver}\n", a.name));
        }
    }

    if !tools.is_empty() {
        report.push_str("\n**Dev Tools:**\n");
        for a in &tools {
            let status = if a.path.is_some() { "✅" } else { "❌" };
            let ver = a.version.as_deref().unwrap_or("-");
            let _path_str = a.path.as_deref().unwrap_or("");
            report.push_str(&format!("  {status} {}: {ver}\n", a.name));
        }
    }

    report
}
