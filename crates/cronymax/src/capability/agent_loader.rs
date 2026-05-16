//! Agent-definition loader — reads `<workspace>/.cronymax/agents/<agent>.agent.yaml`
//! and surfaces the fields the Rust runtime needs when spawning an agent loop.
//!
//! The on-disk format mirrors `app/document/agent_definition.h` (the C++ truth
//! source). We keep only the fields the runtime touches; unknown YAML keys are
//! silently ignored so future schema additions are backwards-compatible.
//!
//! ## Lookup order
//!
//! 1. `<workspace>/.cronymax/agents/<agent_id>.agent.yaml`
//! 2. If absent, returns a default `AgentDef` (empty `system_prompt`,
//!    `kind = "worker"`, no tools filter, no LLM override).
//!
//! The default is intentionally permissive so ad-hoc / legacy runs are not
//! broken by the absence of an agent definition file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tracing::warn;

use crate::agent_loop::react::ReflectionConfig;

// ── PromptSource ─────────────────────────────────────────────────────────────

/// Indicates whether an agent's system prompt came from a binary-embedded
/// builtin or a user-editable YAML file on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptSource {
    /// Prompt is sealed inside the binary via `include_str!`.
    Builtin,
    /// Prompt was loaded from the given on-disk YAML path.
    UserYaml(PathBuf),
}

// ── AgentDef ─────────────────────────────────────────────────────────────────

/// Parsed subset of `<agent>.agent.yaml` that the Rust runtime cares about.
#[derive(Clone, Debug)]
pub struct AgentDef {
    /// Human-readable name from the YAML `name:` field.
    /// Defaults to the file stem if absent.
    pub name: String,

    /// `"worker"` or `"reviewer"` (defaults to `"worker"` if absent or unknown).
    pub kind: AgentKind,

    /// Provider name from structured `llm.provider` form, e.g. `"copilot"`,
    /// `"openai"`. Empty means "use workspace default".
    pub llm_provider: String,

    /// Model identifier from structured `llm.model` form, e.g. `"gpt-4o"`.
    /// Also populated when the simple `llm:` string form is used.
    /// Empty means "use workspace default".
    pub llm_model: String,

    /// Pre-authored system prompt that describes the agent's role.
    /// The Rust runtime prepends this to the FlowRuntime invocation context
    /// message so the LLM understands both its persona and the flow context.
    pub system_prompt: String,

    /// Indicates where the system prompt originated. `Builtin` means
    /// the prompt is sealed inside the binary; `UserYaml` means it came
    /// from an editable YAML file.
    pub prompt_source: PromptSource,

    /// Optional memory namespace. Defaults to the agent id when empty.
    pub memory_namespace: String,

    /// Explicit tool allow-list. If empty the agent gets the full default
    /// tool set registered for the run. Non-empty means only the listed
    /// tool names will be available.
    pub tools: Vec<String>,

    /// OpenAI reasoning_effort hint (`minimal` | `low` | `medium` | `high`).
    /// Empty means "don't send the field" — let the model use its default.
    /// Read from a top-level `reasoning_effort:` YAML key, or from
    /// `llm.reasoning_effort` when the structured form is used.
    pub reasoning_effort: String,

    /// When `true` (default), the runtime appends a minimal workspace context
    /// block to the agent's system prompt before each run. Set to `false` in
    /// the YAML to opt out (e.g. pure document-processor agents).
    pub inject_workspace: bool,

    /// User-extensible template variables injected into the system prompt
    /// by the prompt var renderer (e.g. `${my_var}`).
    pub vars: HashMap<String, String>,

    /// Optional in-loop reflection configuration. When `Some`, the
    /// `ReactLoop` fires a self-assessment pass at the trigger interval.
    pub reflection: Option<ReflectionConfig>,
}

/// The two legal agent kinds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentKind {
    #[default]
    Worker,
    Reviewer,
}

impl AgentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentKind::Worker => "worker",
            AgentKind::Reviewer => "reviewer",
        }
    }
}

// ── YAML deserialization ──────────────────────────────────────────────────────

/// Raw deserialization target — maps 1:1 to the YAML keys we care about.
#[derive(Debug, Default, Deserialize)]
struct RawAgentDef {
    #[serde(default)]
    name: String,
    #[serde(default)]
    kind: String,
    /// Simple string form: `llm: gpt-4o`
    #[serde(default)]
    llm: Option<serde_yml::Value>,
    #[serde(default)]
    system_prompt: String,
    #[serde(default)]
    memory_namespace: String,
    #[serde(default)]
    tools: Vec<String>,
    /// Top-level shorthand: `reasoning_effort: high`.
    #[serde(default)]
    reasoning_effort: String,
    #[serde(default = "default_true")]
    inject_workspace: bool,
}

fn default_true() -> bool {
    true
}

/// Peripheral-only override for the Crony builtin agent. Only the fields
/// in the allowlist (`memory_namespace`, `vars`, `reflection.enabled`) may
/// be set. The prompt fields are intentionally absent.
#[derive(Debug, Default, Deserialize)]
struct RawCronyOverride {
    #[serde(default)]
    memory_namespace: Option<String>,
    #[serde(default)]
    vars: Option<HashMap<String, String>>,
    /// Shorthand for `reflection.enabled`. A full reflection object is not
    /// supported in the override — only the enabled flag.
    #[serde(default, rename = "reflection_enabled")]
    reflection_enabled: Option<bool>,
}

impl Default for AgentDef {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: AgentKind::Worker,
            llm_provider: String::new(),
            llm_model: String::new(),
            system_prompt: String::new(),
            prompt_source: PromptSource::UserYaml(PathBuf::new()),
            memory_namespace: String::new(),
            tools: Vec::new(),
            reasoning_effort: String::new(),
            inject_workspace: true,
            vars: HashMap::new(),
            reflection: None,
        }
    }
}

impl RawAgentDef {
    fn into_agent_def(self, fallback_name: &str, source_path: PathBuf) -> AgentDef {
        let name = if self.name.is_empty() {
            fallback_name.to_owned()
        } else {
            self.name
        };

        let kind = match self.kind.as_str() {
            "reviewer" => AgentKind::Reviewer,
            _ => AgentKind::Worker,
        };

        // The `llm` field can be:
        //   - a plain string: `llm: gpt-4o`
        //   - a mapping: `llm: {provider: copilot, model: gpt-4o, reasoning_effort: high}`
        let (llm_provider, llm_model, llm_reasoning) = match &self.llm {
            Some(serde_yml::Value::String(s)) => (String::new(), s.clone(), String::new()),
            Some(serde_yml::Value::Mapping(map)) => {
                let provider = map
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                let model = map
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                let reasoning = map
                    .get("reasoning_effort")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                (provider, model, reasoning)
            }
            _ => (String::new(), String::new(), String::new()),
        };

        // Top-level `reasoning_effort:` wins over `llm.reasoning_effort`
        // when both are set, since it's the more obvious place for users.
        let reasoning_effort = if !self.reasoning_effort.is_empty() {
            normalize_effort(&self.reasoning_effort)
        } else {
            normalize_effort(&llm_reasoning)
        };

        AgentDef {
            name,
            kind,
            llm_provider,
            llm_model,
            system_prompt: self.system_prompt,
            prompt_source: PromptSource::UserYaml(source_path),
            memory_namespace: self.memory_namespace,
            tools: self.tools,
            reasoning_effort,
            inject_workspace: self.inject_workspace,
            vars: HashMap::new(),
            reflection: None,
        }
    }
}

/// Lowercase + reject anything outside the OpenAI-supported set so we
/// don't forward typos to the API. Empty/unknown -> empty string.
///
/// As of GPT-5 the API accepts `minimal | low | medium | high | xhigh`
/// (plus `none`, which we represent as empty here). gpt-5.1 doesn't
/// accept `minimal`/`xhigh` but we leave that to the API to reject so
/// we don't have to track per-model support tables.
fn normalize_effort(s: &str) -> String {
    match s.trim().to_ascii_lowercase().as_str() {
        "minimal" | "low" | "medium" | "high" | "xhigh" => s.trim().to_ascii_lowercase(),
        _ => String::new(),
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Load an agent definition from `<workspace_root>/.cronymax/agents/<agent_id>.agent.yaml`.
///
/// Returns a default `AgentDef` (with `name = agent_id`) if:
/// * The file does not exist (agent definition is optional).
/// * The file cannot be read or parsed (logged at `warn` level).
///
/// This function never fails — a missing or malformed YAML just means
/// "use defaults", which is safe for ad-hoc runs.
pub async fn load_agent(workspace_root: &Path, agent_id: &str) -> AgentDef {
    let path = agents_dir(workspace_root).join(format!("{agent_id}.agent.yaml"));
    load_from_path(&path, agent_id).await
}

/// Load an agent definition, intercepting the builtin Crony agent id.
///
/// When `agent_id` is `""` or `"crony"` (case-insensitive), this returns
/// `CronyBuiltin::def()` augmented with any peripheral overrides from
/// `.cronymax/agents/crony.agent.yaml` (allowlist: `reflection`, `vars`,
/// `memory_namespace`). The `prompt_source` is always forced to
/// `PromptSource::Builtin` — the sealed prompt cannot be overridden.
///
/// For all other `agent_id` values, this delegates to `load_agent`.
pub async fn load_agent_with_builtin(workspace_root: &Path, agent_id: &str) -> AgentDef {
    if agent_id.is_empty() || agent_id.eq_ignore_ascii_case("crony") {
        let mut def = crate::crony::CronyBuiltin::def();

        // Apply optional peripheral overrides from Crony.agent.yaml.
        let override_path = agents_dir(workspace_root).join("Crony.agent.yaml");
        if let Ok(yaml) = tokio::fs::read_to_string(&override_path).await {
            match serde_yml::from_str::<RawCronyOverride>(&yaml) {
                Ok(ov) => {
                    if let Some(ns) = ov.memory_namespace {
                        if !ns.is_empty() {
                            def.memory_namespace = ns;
                        }
                    }
                    if let Some(vars) = ov.vars {
                        def.vars.extend(vars);
                    }
                    if let Some(enabled) = ov.reflection_enabled {
                        if let Some(ref mut r) = def.reflection {
                            r.enabled = enabled;
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        path = %override_path.display(),
                        error = %e,
                        "agent_loader: failed to parse crony.agent.yaml override"
                    );
                }
            }
        }
        // Always enforce sealed prompt source.
        def.prompt_source = PromptSource::Builtin;
        return def;
    }
    load_agent(workspace_root, agent_id).await
}

/// Build the agents directory path `<workspace_root>/.cronymax/agents/`.
pub fn agents_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".cronymax").join("agents")
}

async fn load_from_path(path: &Path, fallback_name: &str) -> AgentDef {
    let yaml = match tokio::fs::read_to_string(path).await {
        Ok(y) => y,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No definition file — silently use defaults.
            return AgentDef {
                name: fallback_name.to_owned(),
                ..Default::default()
            };
        }
        Err(e) => {
            warn!(path = %path.display(), error = %e, "agent_loader: failed to read agent yaml");
            return AgentDef {
                name: fallback_name.to_owned(),
                ..Default::default()
            };
        }
    };

    match serde_yml::from_str::<RawAgentDef>(&yaml) {
        Ok(raw) => raw.into_agent_def(fallback_name, path.to_path_buf()),
        Err(e) => {
            warn!(path = %path.display(), error = %e, "agent_loader: failed to parse agent yaml");
            AgentDef {
                name: fallback_name.to_owned(),
                ..Default::default()
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_llm() {
        let yaml = r#"
name: coder
kind: worker
llm: gpt-4o
system_prompt: "You are an expert software engineer."
tools:
  - run_shell
  - read_file
  - write_file
"#;
        let raw: RawAgentDef = serde_yml::from_str(yaml).unwrap();
        let def = raw.into_agent_def("coder", PathBuf::new());
        assert_eq!(def.name, "coder");
        assert_eq!(def.kind, AgentKind::Worker);
        assert_eq!(def.llm_model, "gpt-4o");
        assert_eq!(def.llm_provider, "");
        assert_eq!(def.system_prompt, "You are an expert software engineer.");
        assert_eq!(def.tools, vec!["run_shell", "read_file", "write_file"]);
    }

    #[test]
    fn parse_structured_llm() {
        let yaml = r#"
name: pm
kind: reviewer
llm:
  provider: copilot
  model: claude-3-5-sonnet
system_prompt: "Review the PRD for completeness."
"#;
        let raw: RawAgentDef = serde_yml::from_str(yaml).unwrap();
        let def = raw.into_agent_def("pm", PathBuf::new());
        assert_eq!(def.kind, AgentKind::Reviewer);
        assert_eq!(def.llm_provider, "copilot");
        assert_eq!(def.llm_model, "claude-3-5-sonnet");
        assert!(def.tools.is_empty());
    }

    #[test]
    fn empty_yaml_gives_defaults() {
        let yaml = "{}";
        let raw: RawAgentDef = serde_yml::from_str(yaml).unwrap();
        let def = raw.into_agent_def("my-agent", PathBuf::new());
        assert_eq!(def.name, "my-agent");
        assert_eq!(def.kind, AgentKind::Worker);
        assert!(def.system_prompt.is_empty());
        assert!(def.inject_workspace, "inject_workspace defaults to true");
    }

    #[test]
    fn inject_workspace_false_is_respected() {
        let yaml = "inject_workspace: false\nname: critic\n";
        let raw: RawAgentDef = serde_yml::from_str(yaml).unwrap();
        let def = raw.into_agent_def("critic", PathBuf::new());
        assert!(!def.inject_workspace);
    }
}
