// Skills system — built-in tool definitions and handlers.

pub mod browser;
mod browser_nav;
pub mod chat;
pub mod credentials;
pub mod general;
pub mod loader;
pub mod manager;
mod memory;
pub mod ollama;
pub mod onboarding;
pub mod sandbox;
pub mod scheduler;
pub mod tab;
pub mod terminal;
pub mod webview;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{Value, json};

/// Consolidated dependencies for skill registration.
///
/// Holds all shared state and communication primitives that skill handlers
/// need. Passed by reference to all `register_*_skills()` functions.
pub struct SkillDependencies {
    pub proxy: winit::event_loop::EventLoopProxy<crate::ai::stream::AppEvent>,
    pub pending_results: crate::ai::stream::PendingResultMap,
    pub webview_info: std::sync::Arc<std::sync::Mutex<Vec<crate::ui::types::BrowserViewInfo>>>,
    pub tab_info: std::sync::Arc<std::sync::Mutex<Vec<crate::ui::types::TabInfo>>>,
    pub terminal_info: std::sync::Arc<std::sync::Mutex<Vec<crate::ui::types::TerminalInfo>>>,
    pub onboarding_state: crate::ai::skills::onboarding::OnboardingState,
    /// Optional database store for persistent memory skills (chat category).
    pub db: Option<crate::ai::db::DbStore>,
    /// Profile ID for persistent memory skills.
    pub profile_id: String,
    /// Secret store for credential management skills.
    pub secret_store: Option<std::sync::Arc<crate::secret::SecretStore>>,
}

/// A tool skill definition (name, description, parameter JSON schema, category).
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub parameters_schema: Value,
    /// Skill category for per-profile filtering.
    /// Valid values: `sandbox`, `chat`, `browser`, `terminal`, `tab`, `webview`, `external`, `general`, `channels`, `scheduler`.
    pub category: String,
}

/// Async handler for a skill invocation.
pub type SkillHandler =
    Arc<dyn Fn(Value) -> Pin<Box<dyn Future<Output = anyhow::Result<Value>> + Send>> + Send + Sync>;

/// Registry of available skills.
pub struct SkillRegistry {
    skills: HashMap<String, (Skill, SkillHandler)>,
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
        }
    }

    /// Register a skill with its handler.
    pub fn register(&mut self, skill: Skill, handler: SkillHandler) {
        self.skills.insert(skill.name.clone(), (skill, handler));
    }

    /// Look up a skill by name.
    pub fn get(&self, name: &str) -> Option<&(Skill, SkillHandler)> {
        self.skills.get(name)
    }

    /// Convert all skills to OpenAI function-calling JSON schema format.
    pub fn to_openai_tools(&self) -> Vec<Value> {
        self.skills
            .values()
            .map(|(skill, _)| {
                json!({
                    "type": "function",
                    "function": {
                        "name": skill.name,
                        "description": skill.description,
                        "parameters": skill.parameters_schema,
                    }
                })
            })
            .collect()
    }

    /// Return OpenAI-format tool definitions filtered by allowed skill categories.
    ///
    /// Only includes skills whose `category` is present in `allowed_skills`.
    /// An empty `allowed_skills` slice returns no tools.
    pub fn to_openai_tools_filtered(&self, allowed_skills: &[String]) -> Vec<Value> {
        self.skills
            .values()
            .filter(|(skill, _)| allowed_skills.iter().any(|cat| cat == &skill.category))
            .map(|(skill, _)| {
                json!({
                    "type": "function",
                    "function": {
                        "name": skill.name,
                        "description": skill.description,
                        "parameters": skill.parameters_schema,
                    }
                })
            })
            .collect()
    }

    /// Get skill handlers filtered by allowed skill categories.
    ///
    /// Returns a map from tool function name → handler for skills whose
    /// category is present in `allowed_skills`.
    pub fn handlers_filtered(&self, allowed_skills: &[String]) -> HashMap<String, SkillHandler> {
        self.skills
            .iter()
            .filter(|(_, (skill, _))| allowed_skills.iter().any(|cat| cat == &skill.category))
            .map(|(name, (_, handler))| (name.clone(), handler.clone()))
            .collect()
    }

    /// Remove all skills with a given category.
    pub fn remove_by_category(&mut self, category: &str) {
        self.skills
            .retain(|_, (skill, _)| skill.category != category);
    }

    /// Register all built-in skills.
    pub fn register_builtins(&mut self) {
        self.register_run_command();
        self.register_read_file();
        self.register_write_file();
        self.register_show_diff();
    }

    fn register_run_command(&mut self) {
        let skill = Skill {
            name: "run_command".into(),
            description: "Run a shell command and return stdout, stderr, and exit code.".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Working directory (optional)"
                    }
                },
                "required": ["command"]
            }),
            category: "terminal".into(),
        };

        let handler: SkillHandler = Arc::new(|args: Value| {
            Box::pin(async move {
                let command = args["command"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'command' argument"))?;
                let cwd = args["cwd"].as_str();

                let mut cmd = tokio::process::Command::new("sh");
                cmd.arg("-c").arg(command);
                if let Some(dir) = cwd {
                    cmd.current_dir(dir);
                }
                cmd.stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());

                let result =
                    tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output()).await;

                match result {
                    Ok(Ok(output)) => Ok(json!({
                        "stdout": String::from_utf8_lossy(&output.stdout),
                        "stderr": String::from_utf8_lossy(&output.stderr),
                        "exit_code": output.status.code().unwrap_or(-1),
                    })),
                    Ok(Err(e)) => Ok(json!({
                        "error": format!("Failed to execute: {}", e),
                        "exit_code": -1,
                    })),
                    Err(_) => Ok(json!({
                        "error": "Command timed out after 30 seconds",
                        "exit_code": -1,
                    })),
                }
            })
        });

        self.register(skill, handler);
    }

    fn register_read_file(&mut self) {
        let skill = Skill {
            name: "read_file".into(),
            description: "Read the contents of a file, optionally a specific line range.".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the file"
                    },
                    "line_start": {
                        "type": "integer",
                        "description": "Start line (1-based, optional)"
                    },
                    "line_end": {
                        "type": "integer",
                        "description": "End line (1-based, inclusive, optional)"
                    }
                },
                "required": ["path"]
            }),
            category: "terminal".into(),
        };

        let handler: SkillHandler = Arc::new(|args: Value| {
            Box::pin(async move {
                let path = args["path"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;
                let content = tokio::fs::read_to_string(path).await?;

                let line_start = args["line_start"].as_u64().map(|n| n as usize);
                let line_end = args["line_end"].as_u64().map(|n| n as usize);

                let result = match (line_start, line_end) {
                    (Some(start), Some(end)) => content
                        .lines()
                        .skip(start.saturating_sub(1))
                        .take(end - start.saturating_sub(1))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    (Some(start), None) => content
                        .lines()
                        .skip(start.saturating_sub(1))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    _ => content,
                };

                Ok(json!({ "content": result }))
            })
        });

        self.register(skill, handler);
    }

    fn register_write_file(&mut self) {
        let skill = Skill {
            name: "write_file".into(),
            description: "Write content to a file.".into(),
            parameters_schema: json!({
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
            category: "terminal".into(),
        };

        let handler: SkillHandler = Arc::new(|args: Value| {
            Box::pin(async move {
                let path = args["path"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;
                let content = args["content"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'content' argument"))?;

                tokio::fs::write(path, content).await?;
                let bytes_written = content.len();

                Ok(json!({ "bytes_written": bytes_written }))
            })
        });

        self.register(skill, handler);
    }

    fn register_show_diff(&mut self) {
        let skill = Skill {
            name: "show_diff".into(),
            description: "Show a unified diff between two text blocks.".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "original": {
                        "type": "string",
                        "description": "Original text"
                    },
                    "modified": {
                        "type": "string",
                        "description": "Modified text"
                    }
                },
                "required": ["original", "modified"]
            }),
            category: "chat".into(),
        };

        let handler: SkillHandler = Arc::new(|args: Value| {
            Box::pin(async move {
                let original = args["original"].as_str().unwrap_or("");
                let modified = args["modified"].as_str().unwrap_or("");

                // Simple line-by-line diff.
                let orig_lines: Vec<&str> = original.lines().collect();
                let mod_lines: Vec<&str> = modified.lines().collect();

                let mut diff = String::new();
                diff.push_str("--- original\n");
                diff.push_str("+++ modified\n");

                let max_len = orig_lines.len().max(mod_lines.len());
                for i in 0..max_len {
                    match (orig_lines.get(i), mod_lines.get(i)) {
                        (Some(o), Some(m)) if o == m => {
                            diff.push_str(&format!(" {}\n", o));
                        }
                        (Some(o), Some(m)) => {
                            diff.push_str(&format!("-{}\n", o));
                            diff.push_str(&format!("+{}\n", m));
                        }
                        (Some(o), None) => {
                            diff.push_str(&format!("-{}\n", o));
                        }
                        (None, Some(m)) => {
                            diff.push_str(&format!("+{}\n", m));
                        }
                        (None, None) => {}
                    }
                }

                Ok(json!({ "diff": diff }))
            })
        });

        self.register(skill, handler);
    }
}

mod builtin_extra;
