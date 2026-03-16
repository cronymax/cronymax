// Schedule service — cross-session scheduled task management, scoped by profile.
//
// Defines scheduled tasks (cron-based automations), execution records,
// and the CRUD task store. The actual execution engine lives in `ai::scheduler::runtime`.
#![allow(dead_code)]

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ─── Scheduled Task ──────────────────────────────────────────────────────────

/// A user-created scheduled automation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub name: String,
    /// 5-field POSIX cron expression.
    pub cron: String,
    /// What to execute when triggered.
    pub action_type: String,
    /// The value: prompt text or shell command.
    pub action_value: String,
    /// Optional agent to invoke (empty = none).
    #[serde(default)]
    pub agent_name: String,
    /// Profile whose AI/sandbox config to use.
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    /// Whether the task is active.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// If true, auto-disable after first execution (one-shot task).
    #[serde(default)]
    pub run_once: bool,
    /// ISO 8601 creation timestamp.
    #[serde(default)]
    pub created_at: String,
}

fn default_profile_id() -> String {
    "default".into()
}
fn default_true() -> bool {
    true
}

/// The action to execute.
#[derive(Debug, Clone)]
pub enum TaskAction {
    /// Send text to AI chat as if user typed `? {text}`.
    Prompt { text: String },
    /// Execute a shell command directly.
    Command { command: String },
}

impl ScheduledTask {
    /// Parse the action from the stored fields.
    pub fn action(&self) -> TaskAction {
        match self.action_type.as_str() {
            "command" => TaskAction::Command {
                command: self.action_value.clone(),
            },
            _ => TaskAction::Prompt {
                text: self.action_value.clone(),
            },
        }
    }
}

// ─── Execution Record ────────────────────────────────────────────────────────

/// Log entry for a scheduled task run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRecord {
    pub task_id: String,
    pub timestamp: String,
    pub duration_ms: u64,
    pub status: String,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub error: Option<String>,
}

/// Execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatus {
    Success,
    Failure,
    Timeout,
}

impl ExecutionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Timeout => "timeout",
        }
    }
}

// ─── Task Store ──────────────────────────────────────────────────────────────

/// Persisted file format for `tasks.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TasksFile {
    #[serde(default)]
    pub tasks: Vec<ScheduledTask>,
}

/// CRUD store for scheduled tasks — persisted to `tasks.toml` in the config directory.
pub struct ScheduledTaskStore {
    /// Path to tasks.toml.
    pub path: PathBuf,
    /// In-memory tasks.
    pub tasks: Vec<ScheduledTask>,
}

impl ScheduledTaskStore {
    /// Create a new store pointing at the default tasks.toml path.
    pub fn new() -> Self {
        let path = crate::renderer::platform::config_dir().join("tasks.toml");
        Self {
            path,
            tasks: Vec::new(),
        }
    }

    /// Load tasks from disk.
    pub fn load(&mut self) -> anyhow::Result<()> {
        if !self.path.exists() {
            self.tasks.clear();
            return Ok(());
        }
        let contents = std::fs::read_to_string(&self.path)?;
        let file: TasksFile = toml::from_str(&contents)?;
        self.tasks = file.tasks;
        Ok(())
    }

    /// Save tasks to disk.
    pub fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = TasksFile {
            tasks: self.tasks.clone(),
        };
        let contents = toml::to_string_pretty(&file)?;
        std::fs::write(&self.path, contents)?;
        Ok(())
    }

    /// Create a new scheduled task.
    pub fn create(&mut self, task: ScheduledTask) -> anyhow::Result<()> {
        if self.tasks.len() >= 100 {
            anyhow::bail!("Maximum of 100 scheduled tasks reached");
        }
        self.tasks.push(task);
        self.save()?;
        Ok(())
    }

    /// Update an existing task by ID.
    pub fn update(&mut self, id: &str, task: ScheduledTask) -> anyhow::Result<()> {
        let idx = self
            .tasks
            .iter()
            .position(|t| t.id == id)
            .ok_or_else(|| anyhow::anyhow!("Task '{}' not found", id))?;
        self.tasks[idx] = task;
        self.save()?;
        Ok(())
    }

    /// Delete a task by ID.
    pub fn delete(&mut self, id: &str) -> anyhow::Result<()> {
        let idx = self
            .tasks
            .iter()
            .position(|t| t.id == id)
            .ok_or_else(|| anyhow::anyhow!("Task '{}' not found", id))?;
        self.tasks.remove(idx);
        self.save()?;
        Ok(())
    }

    /// List all tasks.
    pub fn list(&self) -> &[ScheduledTask] {
        &self.tasks
    }

    /// Toggle a task's enabled state.
    pub fn toggle_enabled(&mut self, id: &str) -> anyhow::Result<bool> {
        let task = self
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| anyhow::anyhow!("Task '{}' not found", id))?;
        task.enabled = !task.enabled;
        let new_state = task.enabled;
        self.save()?;
        Ok(new_state)
    }

    /// Get a task by ID.
    pub fn get(&self, id: &str) -> Option<&ScheduledTask> {
        self.tasks.iter().find(|t| t.id == id)
    }
}

impl Default for ScheduledTaskStore {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Cron Validation ─────────────────────────────────────────────────────────

/// Validate a cron expression. Returns Ok if valid.
pub fn validate_cron(expr: &str) -> Result<(), String> {
    croner::Cron::new(expr)
        .parse()
        .map(|_| ())
        .map_err(|e| format!("Invalid cron expression: {}", e))
}
