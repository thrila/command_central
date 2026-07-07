use crate::utils::shell;
use std::process::Command;

#[derive(Debug)]
pub struct SystemStatus {
    pub cpu: String,
    pub memory: String,
    pub disk: String,
    pub uptime: String,
    pub processes: Vec<ProcessInfo>,
    pub services: Vec<ServiceInfo>,
}

#[derive(Debug)]
pub struct ProcessInfo {
    pub pid: String,
    pub name: String,
    pub cpu_pct: String,
    pub mem_pct: String,
}

#[derive(Debug)]
pub struct ServiceInfo {
    pub name: String,
    pub status: String,
    pub enabled: bool,
}

pub fn get_system_status() -> String {
    let cpu = shell::run_shell("top -bn1 2>/dev/null | grep 'Cpu(s)' | head -1 || echo 'N/A'")
        .map(|(s, _, _)| s.trim().to_string())
        .unwrap_or_default();
    let mem = shell::run_shell("free -h 2>/dev/null | grep Mem || echo 'N/A'")
        .map(|(s, _, _)| s.trim().to_string())
        .unwrap_or_default();
    let disk = shell::run_shell("df -h / 2>/dev/null | tail -1 || echo 'N/A'")
        .map(|(s, _, _)| s.trim().to_string())
        .unwrap_or_default();
    let uptime = shell::run_shell("uptime -p 2>/dev/null || uptime 2>/dev/null || echo 'N/A'")
        .map(|(s, _, _)| s.trim().to_string())
        .unwrap_or_default();
    let load = shell::run_shell("cat /proc/loadavg 2>/dev/null || echo 'N/A'")
        .map(|(s, _, _)| s.trim().to_string())
        .unwrap_or_default();

    format!(
        "**System Status**\n\
         CPU:  {}\n\
         Mem:  {}\n\
         Disk: {}\n\
         Uptime: {}\n\
         Load: {}",
        cpu, mem, disk, uptime, load
    )
}

pub fn find_process(name: &str) -> String {
    match Command::new("ps").args(["aux"]).output() {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout).to_string();
            let matches: Vec<&str> = stdout
                .lines()
                .filter(|line| {
                    line.to_lowercase().contains(&name.to_lowercase())
                        && !line.contains("grep")
                })
                .take(10)
                .collect();

            if matches.is_empty() {
                format!("No running process matching `{name}`")
            } else {
                format!(
                    "**Processes matching `{name}`:**\n```\n{}\n```",
                    matches.join("\n")
                )
            }
        }
        Err(e) => format!("Error: {e}"),
    }
}

pub fn monitor_service(name: &str) -> String {
    let systemctl = Command::new("systemctl")
        .args(["status", name])
        .output()
        .ok()
        .map(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout).to_string();
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            let combined = if stdout.is_empty() { stderr } else { stdout };
            let before = combined.len().min(2000);
            (combined[..before].to_string(), o.status.success())
        });

    match systemctl {
        Some((stdout, true)) => {
            format!("**Service: {name}**\n```\n{stdout}\n```")
        }
        _ => find_process(name),
    }
}

pub fn list_services() -> String {
    let result = shell::run_shell(
        "systemctl list-units --type=service --state=running --no-pager 2>/dev/null \
         | head -20 || echo 'systemctl not available'",
    );

    match result {
        Ok((stdout, _, _)) => {
            if stdout.trim().is_empty() || stdout.contains("not available") {
                "systemctl not available. Try `/monitor process <name>`.".to_string()
            } else {
                format!("**Running Services:**\n```\n{stdout}\n```")
            }
        }
        Err(e) => format!("Error: {e}"),
    }
}
