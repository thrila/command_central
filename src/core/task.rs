use tokio::sync::oneshot;

#[derive(Debug)]
pub enum Task {
    Sif(String),
    Nmap(String),
    Mail(String, String),
    Shell(String),
    Workon(String),
    Run(String),
    Agents,
    Services,
    MonitorProcess(String),
    MonitorService(String),
    AddLoop {
        name: String,
        command: String,
        interval_secs: u64,
    },
    CancelTask(i64),
    DeleteTask(i64),
    RetryTask(i64),
    History(oneshot::Sender<Vec<(String, String, i64)>>),
}

impl Task {
    pub fn kind(&self) -> &str {
        match self {
            Task::Sif(_) => "sif",
            Task::Nmap(_) => "nmap",
            Task::Mail(_, _) => "mail",
            Task::Shell(_) => "shell",
            Task::Workon(_) => "workon",
            Task::Run(_) => "run",
            Task::Agents => "agents",
            Task::Services => "services",
            Task::MonitorProcess(_) => "monitor",
            Task::MonitorService(_) => "monitor",
            Task::AddLoop { .. } => "loop",
            Task::CancelTask(_) => "cancel",
            Task::DeleteTask(_) => "delete",
            Task::RetryTask(_) => "retry",
            Task::History(_) => "history",
        }
    }

    pub fn label(&self) -> String {
        match self {
            Task::Sif(url) => format!("sif:{}", url),
            Task::Nmap(target) => format!("nmap:{}", target),
            Task::Mail(dest, _) => format!("mail:{}", dest),
            Task::Shell(cmd) => format!("shell:{}", cmd),
            Task::Workon(task) => format!("workon:{}", &task[..task.len().min(50)]),
            Task::Run(cmd) => format!("run:{}", &cmd[..cmd.len().min(50)]),
            Task::Agents => "agents".to_string(),
            Task::Services => "services".to_string(),
            Task::MonitorProcess(t) => format!("monitor:process:{}", t),
            Task::MonitorService(t) => format!("monitor:service:{}", t),
            Task::AddLoop { ref name, .. } => format!("loop:{}", name),
            Task::CancelTask(id) => format!("cancel:{}", id),
            Task::DeleteTask(id) => format!("delete:{}", id),
            Task::RetryTask(id) => format!("retry:{}", id),
            Task::History(_) => "history".to_string(),
        }
    }
}
