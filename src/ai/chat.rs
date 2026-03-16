// Chat logic — system prompt building, memory injection.
#![allow(dead_code)]

use crate::ai::context::TokenCounter;
use crate::ai::skills::loader::ExternalSkill;

/// Build the system prompt, optionally injecting memory context and external skills.
///
/// If memory content is provided and non-empty, it's appended as a `<memory>` block.
/// If external skills are provided, their instruction text is appended as labeled sections.
pub fn build_system_prompt(
    base: &str,
    memory_block: Option<&str>,
    _counter: &TokenCounter,
    _model: &str,
    _max_memory_tokens: usize,
) -> String {
    let mut prompt = base.to_string();

    if let Some(memory) = memory_block
        && !memory.is_empty()
    {
        prompt.push_str("\n\n");
        prompt.push_str(memory);
    }

    prompt
}

/// Build the system prompt with external skill instructions injected.
///
/// Calls `build_system_prompt` for base + memory, then appends any
/// external skill instructions if the profile allows "external" category.
pub fn build_system_prompt_with_skills(
    base: &str,
    memory_block: Option<&str>,
    counter: &TokenCounter,
    model: &str,
    max_memory_tokens: usize,
    external_skills: &[ExternalSkill],
    allowed_skills: &[String],
) -> String {
    let mut prompt = build_system_prompt(base, memory_block, counter, model, max_memory_tokens);

    // Inject external skill instructions only if "external" category is allowed.
    if allowed_skills.iter().any(|c| c == "external") {
        for ext in external_skills {
            if !ext.instructions.is_empty() {
                prompt.push_str(&format!(
                    "\n\n--- Skill: {} ---\n{}",
                    ext.frontmatter.name, ext.instructions
                ));
            }
        }
    }

    prompt
}
