use anyhow::Result;
use std::process::Command as StdCommand;

pub fn run_command(cmd: &str, args: &[&str]) -> Result<(String, String, std::process::ExitStatus)> {
    let output = StdCommand::new(cmd).args(args).output()?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok((stdout, stderr, output.status))
}

pub fn run_shell(cmd: &str) -> Result<(String, String, std::process::ExitStatus)> {
    let output = if cfg!(target_os = "windows") {
        StdCommand::new("cmd").args(["/C", cmd]).output()?
    } else {
        StdCommand::new("sh").args(["-c", cmd]).output()?
    };
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok((stdout, stderr, output.status))
}
