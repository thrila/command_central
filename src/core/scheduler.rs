use crate::core::db::Database;
use crate::utils::shell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct ScheduledTask {
    pub id: u64,
    pub name: String,
    pub command: String,
    pub interval_secs: u64,
    pub active: bool,
}

struct LoopHandle {
    cancel: Arc<AtomicBool>,
}

pub struct Scheduler {
    tasks: Mutex<HashMap<u64, (ScheduledTask, Arc<AtomicBool>)>>,
    next_id: Mutex<u64>,
    db_path: String,
}

impl Scheduler {
    pub fn new(db_path: &str) -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            next_id: Mutex::new(1),
            db_path: db_path.to_string(),
        }
    }

    pub fn add_loop(&self, name: &str, command: &str, interval_secs: u64) -> String {
        let mut tasks = self.tasks.lock().unwrap();
        let mut next_id = self.next_id.lock().unwrap();
        let id = *next_id;
        *next_id += 1;

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();

        let task = ScheduledTask {
            id,
            name: name.to_string(),
            command: command.to_string(),
            interval_secs,
            active: true,
        };

        let name_clone = name.to_string();
        let command_clone = command.to_string();
        let db_path = self.db_path.clone();

        tasks.insert(id, (task, cancel));

        tokio::spawn(async move {
            if let Ok(db) = Database::open(&db_path) {
                loop {
                    if cancel_clone.load(Ordering::Relaxed) {
                        if let Ok(task_id) =
                            db.insert_task(&format!("loop:{}:cancelled", &name_clone))
                        {
                            let _ = db.update_task_status(
                                task_id,
                                "ok",
                                Some(&format!("Loop `{}` stopped.", &name_clone)),
                            );
                        }
                        break;
                    }

                    tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;

                    if cancel_clone.load(Ordering::Relaxed) {
                        break;
                    }

                    let output = match shell::run_shell(&command_clone) {
                        Ok((stdout, stderr, status)) => {
                            let s = if status.success() { "ok" } else { "failed" };
                            let out = if stderr.is_empty() {
                                stdout
                            } else {
                                format!("{stdout}\nSTDERR: {stderr}")
                            };
                            format!("[{s}] {out}")
                        }
                        Err(e) => format!("[error] {e}"),
                    };

                    let label = format!("loop:{}", &name_clone);
                    if let Ok(task_id) = db.insert_task(&label) {
                        let status_str = if output.starts_with("[ok]") {
                            "ok"
                        } else {
                            "failed"
                        };
                        let _ = db.update_task_status(task_id, status_str, Some(&output));
                    }
                }
            }
        });

        format!("Loop `{name}` created (ID: {id}) — running `{command}` every {interval_secs}s")
    }

    pub fn stop_loop(&self, id: u64) -> String {
        let tasks = self.tasks.lock().unwrap();
        if let Some((task, cancel)) = tasks.get(&id) {
            cancel.store(true, Ordering::Relaxed);
            format!("Stopping loop `{}` (ID: {id})", task.name)
        } else {
            format!("Loop with ID {id} not found")
        }
    }

    pub fn remove_loop(&self, id: u64) -> String {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some((task, cancel)) = tasks.remove(&id) {
            cancel.store(true, Ordering::Relaxed);
            format!("Removed loop `{}` (ID: {id})", task.name)
        } else {
            format!("Loop with ID {id} not found")
        }
    }

    pub fn list_loops(&self) -> String {
        let tasks = self.tasks.lock().unwrap();
        if tasks.is_empty() {
            return "No active loops.".to_string();
        }
        let mut out = String::from("**Active Loops:**\n");
        for (_, (task, cancel)) in tasks.iter() {
            let active = if cancel.load(Ordering::Relaxed) {
                "stopping"
            } else {
                "running"
            };
            out.push_str(&format!(
                "  [{}] {} ({}) — `{}` every {}s\n",
                task.id, task.name, active, task.command, task.interval_secs
            ));
        }
        out
    }
}
