use super::*;

pub struct SchedulerRuntime {
    /// Store for scheduled tasks.
    pub store: ScheduledTaskStore,
    /// Last time we checked for due tasks.
    pub last_check: chrono::DateTime<Local>,
    /// Last execution timestamp per task ID.
    pub last_runs: HashMap<String, chrono::DateTime<Local>>,
    /// History file path.
    pub history_path: PathBuf,
}

impl SchedulerRuntime {
    pub fn new(store: ScheduledTaskStore) -> Self {
        let history_path = crate::renderer::platform::config_dir().join("task-history.jsonl");
        Self {
            store,
            last_check: Local::now(),
            last_runs: HashMap::new(),
            history_path,
        }
    }

    /// Check for due tasks and return their IDs.
    pub fn check_due_tasks(&mut self) -> Vec<String> {
        let now = Local::now();
        let mut due = Vec::new();

        for task in &self.store.tasks {
            if !task.enabled {
                continue;
            }
            // Try to parse cron and find next occurrence.
            let cron = match croner::Cron::new(&task.cron).parse() {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Ok(next) = cron.find_next_occurrence(&self.last_check, false)
                && next <= now
            {
                // Check last_run to avoid duplicate execution.
                let should_run = match self.last_runs.get(&task.id) {
                    Some(last) => *last < next,
                    None => true,
                };
                if should_run {
                    due.push(task.id.clone());
                    self.last_runs.insert(task.id.clone(), now);
                }
            }
        }

        self.last_check = now;
        due
    }

    /// Execute a task by ID. Returns the execution record.
    pub async fn execute_task(&self, task_id: &str) -> anyhow::Result<ExecutionRecord> {
        let task = self
            .store
            .get(task_id)
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", task_id))?;

        let start = std::time::Instant::now();
        let timestamp = Utc::now().to_rfc3339();

        let result = match task.action() {
            TaskAction::Prompt { text } => {
                // The primary execution path routes through
                // `AppEvent::ScheduledTaskFire` → `submit_chat()` on the main
                // thread (see `src/app/events/misc.rs`).  This standalone path
                // is used when no UI event loop is available (headless mode).
                // For now record the prompt text; a future iteration can wire
                // in `complete_chat()` from `channels/agent_loop.rs`.
                Ok(format!("[Scheduled] Prompt dispatched: {}", text))
            }
            TaskAction::Command { command } => execute_command(&command).await,
        };

        let duration = start.elapsed();
        let (status, output, error) = match result {
            Ok(out) => (ExecutionStatus::Success, out, None),
            Err(e) => (ExecutionStatus::Failure, String::new(), Some(e.to_string())),
        };

        let record = ExecutionRecord {
            task_id: task_id.to_string(),
            timestamp,
            duration_ms: duration.as_millis() as u64,
            status: status.as_str().to_string(),
            output,
            error,
        };

        // Append to history.
        if let Err(e) = self.append_history(&record) {
            log::error!("Failed to write execution history: {}", e);
        }

        Ok(record)
    }

    /// Append an execution record to the JSONL history file.
    fn append_history(&self, record: &ExecutionRecord) -> anyhow::Result<()> {
        use std::io::Write;

        // Check file size for rotation (10MB max).
        if self.history_path.exists() {
            let metadata = std::fs::metadata(&self.history_path)?;
            if metadata.len() > 10 * 1024 * 1024 {
                // Rotate: rename existing to .bak and start fresh.
                let bak = self.history_path.with_extension("jsonl.bak");
                let _ = std::fs::rename(&self.history_path, &bak);
            }
        }

        if let Some(parent) = self.history_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.history_path)?;
        let json = serde_json::to_string(record)?;
        writeln!(file, "{}", json)?;
        Ok(())
    }

    /// Load execution history for a specific task (most recent first).
    pub fn load_history(&self, task_id: &str, limit: usize) -> Vec<ExecutionRecord> {
        if !self.history_path.exists() {
            return Vec::new();
        }
        let contents = match std::fs::read_to_string(&self.history_path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        let mut records: Vec<ExecutionRecord> = contents
            .lines()
            .filter_map(|line| serde_json::from_str::<ExecutionRecord>(line).ok())
            .filter(|r| r.task_id == task_id)
            .collect();

        records.reverse();
        records.truncate(limit);
        records
    }

    /// Load all execution history (most recent first).
    pub fn load_all_history(&self, limit: usize) -> Vec<ExecutionRecord> {
        if !self.history_path.exists() {
            return Vec::new();
        }
        let contents = match std::fs::read_to_string(&self.history_path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        let mut records: Vec<ExecutionRecord> = contents
            .lines()
            .filter_map(|line| serde_json::from_str::<ExecutionRecord>(line).ok())
            .collect();

        records.reverse();
        records.truncate(limit);
        records
    }
}

/// Execute a shell command and return its output.
async fn execute_command(command: &str) -> anyhow::Result<String> {
    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        Ok(format!("{}{}", stdout, stderr))
    } else {
        anyhow::bail!(
            "Command failed (exit {}): {}{}",
            output.status,
            stdout,
            stderr
        )
    }
}

/// Start the scheduler polling loop as a background tokio task.
pub fn start_scheduler_loop(
    runtime: &tokio::runtime::Runtime,
    store: ScheduledTaskStore,
    proxy: winit::event_loop::EventLoopProxy<crate::ai::stream::AppEvent>,
    poll_interval_secs: u64,
    running: Arc<std::sync::atomic::AtomicBool>,
) {
    let mut scheduler = SchedulerRuntime::new(store);

    runtime.spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(poll_interval_secs));

        loop {
            interval.tick().await;

            if !running.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            // Reload tasks from disk each cycle to pick up changes.
            if let Err(e) = scheduler.store.load() {
                log::error!("Failed to reload scheduled tasks: {}", e);
                continue;
            }

            let due_ids = scheduler.check_due_tasks();
            for task_id in due_ids {
                let task = match scheduler.store.get(&task_id) {
                    Some(t) => t.clone(),
                    None => continue,
                };

                // Notify UI that task started.
                let _ = proxy.send_event(crate::ai::stream::AppEvent::ScheduledTaskStarted {
                    task_id: task_id.clone(),
                    task_name: task.name.clone(),
                });

                // For prompt tasks, fire an event so the main thread can call submit_chat.
                // For command tasks, execute here and report results.
                match task.action() {
                    TaskAction::Prompt { text } => {
                        let _ = proxy.send_event(crate::ai::stream::AppEvent::ScheduledTaskFire {
                            task_id: task_id.clone(),
                            task_name: task.name.clone(),
                            action_type: "prompt".into(),
                            action_value: text.clone(),
                        });

                        // Write execution history for prompt tasks too.
                        let record = ExecutionRecord {
                            task_id: task_id.clone(),
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            duration_ms: 0,
                            status: "dispatched".to_string(),
                            output: format!("Prompt dispatched: {}", text),
                            error: None,
                        };
                        if let Err(e) = scheduler.append_history(&record) {
                            log::error!("Failed to write prompt task history: {}", e);
                        }
                    }
                    TaskAction::Command { command: _ } => {
                        let record = scheduler.execute_task(&task_id).await;
                        let (status, duration_ms) = match &record {
                            Ok(r) => (r.status.clone(), r.duration_ms),
                            Err(_e) => ("failure".to_string(), 0),
                        };

                        let output = match &record {
                            Ok(r) => r.output.clone(),
                            Err(e) => e.to_string(),
                        };
                        let _ =
                            proxy.send_event(crate::ai::stream::AppEvent::ScheduledTaskCompleted {
                                task_id: task_id.clone(),
                                task_name: task.name.clone(),
                                status,
                                duration_ms,
                                output,
                            });
                    }
                }

                // Auto-disable run_once tasks after firing.
                if task.run_once {
                    if let Some(t) = scheduler.store.tasks.iter_mut().find(|t| t.id == task_id) {
                        t.enabled = false;
                    }
                    let _ = scheduler.store.save();
                }
            }
        }
    });
}
