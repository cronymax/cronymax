//! Crony — the builtin agent that is always available in the Cronymax runtime.
//!
//! `CronyBuiltin::def()` constructs a fully-formed [`AgentDef`] without
//! touching the filesystem. The system prompt is sealed inside the binary
//! via `include_str!` and cannot be overwritten by the user.
//!
//! Users may create `.cronymax/agents/crony.agent.yaml` to override a
//! restricted set of peripheral fields: `reflection`, `vars`, and
//! `memory_namespace`. The prompt source is always `PromptSource::Builtin`
//! regardless of any override file.

use crate::agent_loop::react::{ReflectionConfig, ReflectionTrigger};
use crate::capability::agent_loader::{AgentDef, AgentKind, PromptSource};

pub mod prompts {

    /// Embedded Crony system prompt (sealed at compile time).
    pub const SYSTEM_PROMPT: &str = include_str!("prompts/system.md");

    /// Embedded Crony reflection prompt template (sealed at compile time).
    pub const REFLECT_PROMPT: &str = include_str!("prompts/reflect.md");

    /// System prompt for the thread-compaction LLM call.
    ///
    /// Instructs the model to produce a concise prose summary of the
    /// conversation excerpt being compressed.
    pub const COMPACTION_SUMMARIZE: &str = r#"You are a conversation summariser.
    Produce a concise plain-text summary (≤ 300 words) of the conversation excerpts below.
    Preserve key decisions, file paths, facts, and tool results.
    Do NOT use lists or headings — write flowing prose."#;

    /// System prompt for the reflection-decay LLM call.
    ///
    /// Used when `flush_thread_with_reflections` summarises older
    /// `[REFLECTION]` messages before the thread grows too long.
    pub const REFLECTION_DECAY_SUMMARIZE: &str = r#"You are a concise summariser.
    Output only the summary paragraph — no preamble, no lists, no headings."#;
}

/// The builtin Crony agent definition.
pub struct CronyBuiltin;

impl CronyBuiltin {
    /// Return the canonical Crony [`AgentDef`].
    ///
    /// Reflection is on by default (`EveryNTurns(4)`) per design decision D8.
    /// The reflection prompt template is `None` — the loop will use the
    /// embedded `REFLECT_PROMPT` when it fires.
    pub fn def() -> AgentDef {
        AgentDef {
            name: "Crony".to_owned(),
            kind: AgentKind::Worker,
            llm_provider: String::new(),
            llm_model: String::new(),
            system_prompt: prompts::SYSTEM_PROMPT.to_owned(),
            prompt_source: PromptSource::Builtin,
            memory_namespace: String::new(),
            tools: Vec::new(),
            inject_workspace: true,
            vars: std::collections::HashMap::new(),
            reflection: Some(ReflectionConfig {
                trigger: ReflectionTrigger::EveryNTurns(4),
                prompt_template: None,
                enabled: true,
            }),
        }
    }

    /// The agent id used to select the Crony builtin.
    pub const ID: &'static str = "crony";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::agent_loader::PromptSource;

    #[test]
    fn crony_def_prompt_is_sealed() {
        let def = CronyBuiltin::def();
        assert!(
            !def.system_prompt.is_empty(),
            "system prompt must not be empty"
        );
        assert_eq!(def.prompt_source, PromptSource::Builtin);
        assert_eq!(def.name, "crony");
    }

    #[test]
    fn crony_def_reflection_enabled_by_default() {
        let def = CronyBuiltin::def();
        let cfg = def.reflection.expect("reflection must be present");
        assert!(cfg.enabled);
        assert!(matches!(cfg.trigger, ReflectionTrigger::EveryNTurns(4)));
    }
}
