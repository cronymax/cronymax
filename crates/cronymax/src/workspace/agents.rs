//! Agent definition registry.
//!
//! Mirrors `app/document/agent_registry.h` + `agent_definition.h`.
//! Scans `<workspace>/.cronymax/agents/*.agent.yaml`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::fs;

/// Parsed `*.agent.yaml` definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDef {
    pub name: String,
    /// `"worker"` | `"reviewer"`. Defaults to `"worker"`.
    #[serde(default = "default_worker")]
    pub kind: String,
    /// Legacy scalar llm field (model name). May be empty.
    #[serde(default)]
    pub llm: String,
    /// Structured llm.provider (e.g. `"copilot"`, `"openai"`).
    #[serde(default)]
    pub llm_provider: String,
    /// Structured llm.model.
    #[serde(default)]
    pub llm_model: String,
    /// Short one-line description of the agent's role (shown in `${agents}` listings).
    /// If empty, the first sentence of `system_prompt` is used as fallback.
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub system_prompt: String,
    /// Defaults to `name` if empty.
    #[serde(default)]
    pub memory_namespace: String,
    #[serde(default)]
    pub tools: Vec<String>,
}

fn default_worker() -> String {
    "worker".to_owned()
}

/// Raw YAML shape — handles both scalar and map `llm` fields.
#[derive(Debug, Deserialize)]
struct RawAgentYaml {
    name: String,
    #[serde(default = "default_worker")]
    kind: String,
    #[serde(default)]
    llm: serde_yml::Value,
    #[serde(default)]
    description: String,
    #[serde(default)]
    system_prompt: String,
    #[serde(default)]
    memory_namespace: String,
    #[serde(default)]
    tools: Vec<String>,
}

fn parse_agent_yaml(yaml: &str) -> Option<AgentDef> {
    let raw: RawAgentYaml = serde_yml::from_str(yaml).ok()?;
    if raw.name.is_empty() {
        return None;
    }

    let (llm, llm_provider, llm_model) = match &raw.llm {
        serde_yml::Value::Mapping(m) => {
            let provider = m
                .get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let model = m
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            (model.clone(), provider, model)
        }
        serde_yml::Value::String(s) => (s.clone(), String::new(), s.clone()),
        _ => (String::new(), String::new(), String::new()),
    };

    let kind = if raw.kind == "reviewer" {
        "reviewer".to_owned()
    } else {
        "worker".to_owned()
    };
    let memory_namespace = if raw.memory_namespace.is_empty() {
        raw.name.clone()
    } else {
        raw.memory_namespace
    };

    Some(AgentDef {
        name: raw.name,
        kind,
        llm,
        llm_provider,
        llm_model,
        description: raw.description,
        system_prompt: raw.system_prompt,
        memory_namespace,
        tools: raw.tools,
    })
}

/// In-memory registry of [`AgentDef`]s for one workspace.
#[derive(Debug, Default)]
pub struct AgentRegistry {
    agents_dir: PathBuf,
    agents: HashMap<String, AgentDef>,
}

impl AgentRegistry {
    pub fn new(agents_dir: impl Into<PathBuf>) -> Self {
        Self {
            agents_dir: agents_dir.into(),
            agents: HashMap::new(),
        }
    }

    /// Reload from disk. Returns the number of agents loaded.
    pub async fn refresh(&mut self) -> usize {
        let mut next: HashMap<String, AgentDef> = HashMap::new();
        if let Ok(mut rd) = fs::read_dir(&self.agents_dir).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                let path = entry.path();
                // Must end in .agent.yaml
                let fname = path.file_name().unwrap_or_default().to_string_lossy();
                if !fname.ends_with(".agent.yaml") {
                    continue;
                }
                let basename = fname.trim_end_matches(".agent.yaml").to_owned();
                if basename.is_empty() {
                    continue;
                }
                if let Ok(text) = fs::read_to_string(&path).await {
                    if let Some(def) = parse_agent_yaml(&text) {
                        next.insert(basename, def);
                    }
                }
            }
        }
        let count = next.len();
        self.agents = next;
        count
    }

    pub fn get(&self, name: &str) -> Option<&AgentDef> {
        self.agents.get(name)
    }

    /// Sorted list of agent names.
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<_> = self.agents.keys().cloned().collect();
        v.sort();
        v
    }

    /// Sorted list of `(name, description)` pairs for use in the `${agents}`
    /// prompt variable. When `description` is empty, the first sentence of
    /// `system_prompt` is used as a fallback.
    pub fn entries(&self) -> Vec<(String, String)> {
        let mut v: Vec<_> = self
            .agents
            .iter()
            .map(|(name, def)| {
                let desc = if !def.description.is_empty() {
                    def.description.clone()
                } else {
                    // Derive from first sentence of system_prompt.
                    def.system_prompt
                        .split(['.', '\n'])
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_owned()
                };
                (name.clone(), desc)
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// Write an agent definition to disk and refresh.
    pub async fn save(&mut self, name: &str, yaml: &str) -> anyhow::Result<()> {
        validate_safe_name(name)?;
        let path = self.agents_dir.join(format!("{name}.agent.yaml"));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let tmp = path.with_extension("yaml.tmp");
        fs::write(&tmp, yaml).await?;
        fs::rename(&tmp, &path).await?;
        self.refresh().await;
        Ok(())
    }

    /// Delete an agent file from disk and refresh.
    pub async fn delete(&mut self, name: &str) -> anyhow::Result<()> {
        validate_safe_name(name)?;
        let path = self.agents_dir.join(format!("{name}.agent.yaml"));
        fs::remove_file(&path).await.or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(e)
            }
        })?;
        self.agents.remove(name);
        Ok(())
    }

    pub fn agents_dir(&self) -> &Path {
        &self.agents_dir
    }
}

fn validate_safe_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty() || name.len() > 64 {
        anyhow::bail!("invalid agent name length");
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        anyhow::bail!("agent name contains invalid characters");
    }
    if name.starts_with('-') {
        anyhow::bail!("agent name must not start with '-'");
    }
    Ok(())
}
