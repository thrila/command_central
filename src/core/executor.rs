use super::config::Config;
use super::db::Database;
use super::detector;
use super::monitor;
use super::scheduler::Scheduler;
use super::task::Task;
use crate::utils::shell;
use std::process::Command as ProcessCommand;
use std::sync::Arc;
use tokio::sync::mpsc;

pub fn spawn_worker(
    mut rx: mpsc::Receiver<Task>,
    db_path: String,
    scheduler: Arc<Scheduler>,
    config: Config,
) {
    std::thread::spawn(move || {
        let db = match Database::open(&db_path) {
            Ok(db) => Arc::new(db),
            Err(e) => {
                eprintln!("Failed to open database: {e}");
                return;
            }
        };

        while let Some(task) = rx.blocking_recv() {
            match task {
                Task::Shell(cmd) => {
                    let label = format!("shell:{cmd}");
                    if let Ok(id) = db.insert_task(&label) {
                        match shell::run_shell(&cmd) {
                            Ok((stdout, stderr, status)) => {
                                let output = if status.success() {
                                    stdout
                                } else {
                                    format!("STDERR:\n{stderr}\nSTDOUT:\n{stdout}")
                                };
                                let status_str = if status.success() { "ok" } else { "failed" };
                                let _ = db.update_task_status(id, status_str, Some(&output));
                            }
                            Err(e) => {
                                let _ = db.update_task_status(id, "error", Some(&e.to_string()));
                            }
                        }
                    }
                }

                Task::Run(cmd) => {
                    let label = format!("run:{}", &cmd[..cmd.len().min(50)]);
                    if let Ok(id) = db.insert_task(&label) {
                        match shell::run_shell(&cmd) {
                            Ok((stdout, stderr, status)) => {
                                let output = if status.success() {
                                    stdout
                                } else {
                                    format!("STDERR:\n{stderr}\nSTDOUT:\n{stdout}")
                                };
                                let status_str = if status.success() { "ok" } else { "failed" };
                                let _ = db.update_task_status(id, status_str, Some(&output));
                            }
                            Err(e) => {
                                let _ = db.update_task_status(id, "error", Some(&e.to_string()));
                            }
                        }
                    }
                }

                Task::Sif(url) => {
                    let label = format!("sif:{url}");
                    if let Ok(id) = db.insert_task(&label) {
                        let result = ProcessCommand::new("./sif")
                            .args(["-u", &url, "-am", "api"])
                            .output();
                        match result {
                            Ok(o) => {
                                let out = String::from_utf8_lossy(&o.stdout).to_string();
                                let s = if o.status.success() { "ok" } else { "failed" };
                                let _ = db.update_task_status(id, s, Some(&out));
                            }
                            Err(e) => {
                                let _ = db.update_task_status(id, "error", Some(&e.to_string()));
                            }
                        }
                    }
                }

                Task::Nmap(target) => {
                    let label = format!("nmap:{target}");
                    if let Ok(id) = db.insert_task(&label) {
                        let result = ProcessCommand::new("nmap")
                            .args(["-sV", &target])
                            .output();
                        match result {
                            Ok(o) => {
                                let out = String::from_utf8_lossy(&o.stdout).to_string();
                                let s = if o.status.success() { "ok" } else { "failed" };
                                let _ = db.update_task_status(id, s, Some(&out));
                            }
                            Err(e) => {
                                let _ = db.update_task_status(id, "error", Some(&e.to_string()));
                            }
                        }
                    }
                }

                Task::Mail(dest, body) => {
                    let label = format!("mail:{dest}");
                    if let Ok(id) = db.insert_task(&label) {
                        let mut child = match ProcessCommand::new("mail")
                            .args(["-s", "Command Central", &dest])
                            .stdin(std::process::Stdio::piped())
                            .spawn()
                        {
                            Ok(c) => c,
                            Err(e) => {
                                let _ = db.update_task_status(id, "error", Some(&e.to_string()));
                                continue;
                            }
                        };
                        if let Some(mut stdin) = child.stdin.take() {
                            use std::io::Write;
                            let _ = stdin.write_all(body.as_bytes());
                        }
                        match child.wait() {
                            Ok(status) if status.success() => {
                                let _ = db.update_task_status(id, "ok", Some(&body));
                            }
                            Ok(status) => {
                                let _ = db.update_task_status(id, "failed", Some(&format!("exit: {:?}", status.code())));
                            }
                            Err(e) => {
                                let _ = db.update_task_status(id, "error", Some(&e.to_string()));
                            }
                        }
                    }
                }

                Task::Workon(task) => {
                    let label = format!("workon:{}", &task[..task.len().min(50)]);
                    if let Ok(id) = db.insert_task(&label) {
                        let atomic_path = config.paths.atomic_repo.clone()
                            .unwrap_or_else(|| "/home/david/Atomic".to_string());
                        let opencode_bin = config.paths.opencode_bin.clone()
                            .unwrap_or_else(|| "/home/david/.opencode/bin/opencode".to_string());
                        let result = ProcessCommand::new(&opencode_bin)
                            .args(["run", &task])
                            .current_dir(&atomic_path)
                            .output();
                        match result {
                            Ok(o) => {
                                let out = String::from_utf8_lossy(&o.stdout).to_string();
                                let err = String::from_utf8_lossy(&o.stderr).to_string();
                                let output = if err.is_empty() {
                                    out
                                } else {
                                    format!("{out}\nSTDERR: {err}")
                                };
                                let s = if o.status.success() { "ok" } else { "failed" };
                                let _ = db.update_task_status(id, s, Some(&output));
                            }
                            Err(e) => {
                                let _ = db.update_task_status(id, "error", Some(&e.to_string()));
                            }
                        }
                    }
                }

                Task::Agents => {
                    let agents = detector::detect_agents();
                    let report = detector::format_agent_report(&agents);
                    let _ = db.insert_task("agents");
                    println!("{report}");
                }

                Task::Services => {
                    let report = monitor::list_services();
                    let _ = db.insert_task("services");
                    println!("{report}");
                }

                Task::MonitorProcess(name) => {
                    let report = monitor::find_process(&name);
                    let label = format!("monitor:process:{}", name);
                    let _ = db.insert_task(&label);
                    println!("{report}");
                }

                Task::MonitorService(name) => {
                    let report = monitor::monitor_service(&name);
                    let label = format!("monitor:service:{}", name);
                    let _ = db.insert_task(&label);
                    println!("{report}");
                }

                Task::AddLoop {
                    name,
                    command,
                    interval_secs,
                } => {
                    let report = scheduler.add_loop(&name, &command, interval_secs);
                    let label = format!("loop:{}", name);
                    let _ = db.insert_task(&label);
                    println!("{report}");
                }

                Task::CancelTask(id) => {
                    let result = db.cancel_task(id);
                    println!(
                        "{}",
                        if result.unwrap_or(false) {
                            format!("Task {id} cancelled.")
                        } else {
                            format!("Task {id} not found or not running.")
                        }
                    );
                }

                Task::DeleteTask(id) => {
                    let result = db.delete_task(id);
                    println!(
                        "{}",
                        if result.unwrap_or(false) {
                            format!("Task {id} deleted.")
                        } else {
                            format!("Task {id} not found.")
                        }
                    );
                }

                Task::RetryTask(id) => {
                    match db.retry_task(id) {
                        Ok(Some(new_id)) => {
                            println!("Task {id} retried as task {new_id}.");
                        }
                        Ok(None) => {
                            println!("Task {id} not found or cannot be retried.");
                        }
                        Err(e) => {
                            println!("Error retrying task {id}: {e}");
                        }
                    }
                }

                Task::History(resp_tx) => {
                    let rows = db
                        .get_history(50)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(id, kind, status, _output)| (kind, status, id))
                        .collect();
                    let _ = resp_tx.send(rows);
                }
            }
        }
    });
}
