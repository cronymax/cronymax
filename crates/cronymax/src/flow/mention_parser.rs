//! Parse `@mention` occurrences from document bodies.
//!
//! ## Rules (mirrors `app/flow/MentionParser`)
//!
//! * Match `@<name>` where `<name>` starts with a letter or `_` and may
//!   contain letters, digits, `_`, or `-` (to support kebab-case agent IDs
//!   like `tech-lead`). The `@` must NOT be immediately preceded by another
//!   word character (so `email@example.com` is ignored).
//! * Skip any `@` that appears inside a fenced code block (lines between
//!   ```` ``` ```` markers at the start of a line).
//! * `name` in [`ParsedMention`] excludes the leading `@`.
//! * Line and column numbers are 1-based.

use regex::Regex;

/// One `@mention` occurrence in a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedMention {
    /// The mention name without the leading `@`.
    pub name: String,
    /// Byte offset of the `@` in the original text.
    pub byte_offset: usize,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number (byte-offset within the line).
    pub column: usize,
}

/// Parses `@mention` occurrences from `text`.
///
/// Agent IDs may contain letters, digits, underscores, and hyphens
/// (e.g. `@tech-lead`, `@my_agent`).
pub fn parse_mentions(text: &str) -> Vec<ParsedMention> {
    // Pre-compute which lines are inside a fenced code block.
    let fenced = fenced_lines(text);

    // Match @<slug> where <slug> = [a-zA-Z_][a-zA-Z0-9_-]*
    let re =
        Regex::new(r"@([a-zA-Z_][a-zA-Z0-9_-]*)").expect("mention regex is valid");

    let mut mentions = Vec::new();
    let bytes = text.as_bytes();

    for m in re.find_iter(text) {
        let byte_offset = m.start();

        // Reject email-style `word@word` (preceding char is alphanumeric or `_`).
        if byte_offset > 0 {
            let prev = bytes[byte_offset - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                continue;
            }
        }

        // Compute line / column.
        let before = &text[..byte_offset];
        let line = before.bytes().filter(|&b| b == b'\n').count() + 1;
        let last_newline = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let column = byte_offset - last_newline + 1;

        // Skip if inside a fenced code block.
        if fenced.contains(&line) {
            continue;
        }

        let name = m.as_str()[1..].to_owned(); // strip '@'
        mentions.push(ParsedMention { name, byte_offset, line, column });
    }

    mentions
}

/// Returns the set of 1-based line numbers that are inside fenced code blocks.
fn fenced_lines(text: &str) -> std::collections::HashSet<usize> {
    let mut inside = false;
    let mut fenced = std::collections::HashSet::new();
    for (idx, line) in text.lines().enumerate() {
        let line_no = idx + 1;
        if line.starts_with("```") {
            inside = !inside;
            fenced.insert(line_no); // the fence line itself is also excluded
            continue;
        }
        if inside {
            fenced.insert(line_no);
        }
    }
    fenced
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_mention() {
        let mentions = parse_mentions("Hello @alice, please review.");
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].name, "alice");
        assert_eq!(mentions[0].line, 1);
        assert_eq!(mentions[0].column, 7);
    }

    #[test]
    fn kebab_case_agent_name() {
        let mentions = parse_mentions("Please review @tech-lead.");
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].name, "tech-lead");
    }

    #[test]
    fn multiple_mentions() {
        let mentions = parse_mentions("@alice and @bob should collaborate.");
        assert_eq!(mentions.len(), 2);
        assert_eq!(mentions[0].name, "alice");
        assert_eq!(mentions[1].name, "bob");
    }

    #[test]
    fn email_address_ignored() {
        let mentions = parse_mentions("contact user@example.com for details");
        assert!(mentions.is_empty(), "email address should not be a mention");
    }

    #[test]
    fn mention_inside_fenced_block_ignored() {
        let text = "Outside @alice\n```\n@bob inside fence\n```\nOutside @carol";
        let names: Vec<_> = parse_mentions(text).into_iter().map(|m| m.name).collect();
        assert!(names.contains(&"alice".to_owned()));
        assert!(names.contains(&"carol".to_owned()));
        assert!(!names.contains(&"bob".to_owned()), "bob is inside a fence");
    }

    #[test]
    fn at_start_of_word_matched() {
        let mentions = parse_mentions("Review by@alice"); // 'y' precedes '@'
        assert!(mentions.is_empty(), "by@alice is email-style");
    }

    #[test]
    fn multiline_positions() {
        let text = "line one\n@dave is here\nline three";
        let mentions = parse_mentions(text);
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].name, "dave");
        assert_eq!(mentions[0].line, 2);
        assert_eq!(mentions[0].column, 1);
    }
}

