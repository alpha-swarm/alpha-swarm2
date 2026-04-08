use inference_client::ChatMessage;

/// Agent specialization types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentType {
    General,
    LintFixer,
    TestWriter,
    Refactorer,
    FeatureAdder,
    BugFixer,
}

impl AgentType {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "lint" | "lint-fixer" | "linter" => Self::LintFixer,
            "test" | "test-writer" | "tester" => Self::TestWriter,
            "refactor" | "refactorer" => Self::Refactorer,
            "feature" | "feature-adder" => Self::FeatureAdder,
            "bug" | "bug-fixer" | "bugfix" => Self::BugFixer,
            _ => Self::General,
        }
    }

    /// Get the system prompt for this agent type (role description only, no output format).
    pub fn system_prompt(self) -> &'static str {
        self.role_description()
    }

    fn role_description(self) -> &'static str {
        match self {
            Self::General => "You are a code modification agent. You receive a task description and file contents, then output precise file edits.\n\nRULES:\n- Only modify files that need to change\n- Do not add unnecessary changes, comments, or formatting\n- If the task is unclear or impossible, explain why instead of guessing",
            Self::LintFixer => "You are a lint-fixing agent. You receive lint/clippy warnings and fix them precisely.\n\nRULES:\n- Fix ONLY the reported lint issues, nothing else\n- Use idiomatic patterns for the language\n- Do not refactor or change logic — only fix warnings\n- Prefer the simplest fix",
            Self::TestWriter => "You are a test-writing agent. You add tests for existing functions.\n\nRULES:\n- Write tests that cover the main behavior and edge cases\n- Use the project's existing test patterns and framework\n- Test both success and error paths\n- Do not modify the code under test — only add tests",
            Self::Refactorer => "You are a refactoring agent. You improve code structure without changing behavior.\n\nRULES:\n- Do NOT change external behavior or API\n- Extract functions, reduce duplication, improve naming\n- Keep changes minimal and focused\n- Preserve all existing tests",
            Self::FeatureAdder => "You are a feature implementation agent. You add new functionality as described.\n\nRULES:\n- Implement exactly what is requested\n- Follow existing code patterns and style\n- Add appropriate error handling\n- Keep the implementation minimal",
            Self::BugFixer => "You are a bug-fixing agent. You diagnose and fix bugs based on the description.\n\nRULES:\n- Fix the root cause, not just the symptom\n- Add a regression test if possible\n- Do not refactor unrelated code",
        }
    }
}

const EDIT_FORMAT: &str = "\nOUTPUT FORMAT:\nUse ONLY the exact file paths from the FILES section above. Never invent paths.\n\n<<<EDIT {actual_file_from_task}\n--- OLD\nexact existing lines to find\n--- NEW\nreplacement lines\n>>>\n\n<<<CREATE {actual_file_from_task}\nfile contents\n>>>\n\n<<<DELETE {actual_file_from_task}\n>>>\n\nRULES:\n- Use ONLY file paths listed in the FILES section or mentioned in the TASK\n- Never use example paths like path/to/file.rs\n- Output ONLY <<< blocks, no explanation";

/// Extended format for tool-use loop — includes TOOL and DONE blocks.
const TOOL_FORMAT: &str = "\nOUTPUT FORMAT:\nYou can call tools and edit files. Output ONLY <<< blocks.\n\n<<<TOOL tool_name\n{\"param\": \"value\"}\n>>>\n\n<<<EDIT {actual_file_from_task}\n--- OLD\nexact existing lines\n--- NEW\nreplacement lines\n>>>\n\n<<<CREATE {actual_file_from_task}\nfile contents\n>>>\n\n<<<DONE\nwhat was done\n>>>\n\nRULES:\n- Use ONLY file paths from the FILES section or TASK description\n- Never use example/placeholder paths\n- Start with <<< immediately, no explanation\n- Call tools first to read files, then edit, then <<<DONE>>>";

pub fn build_prompt(
    task_description: &str,
    files: &[(String, String)],
) -> Vec<ChatMessage> {
    build_prompt_with_type(task_description, files, AgentType::General)
}

/// Build a prompt for the tool-use loop with available tool names.
pub fn build_tool_prompt(
    task_description: &str,
    files: &[(String, String)],
    tool_names: &[&str],
) -> Vec<ChatMessage> {
    let file_context = build_file_context(files);

    let tools_list = tool_names.iter()
        .map(|n| format!("  - {n}"))
        .collect::<Vec<_>>()
        .join("\n");

    let system = format!(
        "{}\n\nAVAILABLE TOOLS:\n{}\n{}",
        AgentType::General.role_description(),
        tools_list,
        TOOL_FORMAT,
    );
    let user_message = format!("TASK: {task_description}\n\nFILES:\n{file_context}");

    vec![
        ChatMessage::system(system),
        ChatMessage::user(user_message),
    ]
}

/// Rough estimate of tokens from character count (1 token ≈ 4 chars for code).
#[allow(dead_code)]
const CHARS_PER_TOKEN: usize = 4;
/// Max chars for a single file before it gets summarized.
const MAX_FILE_CHARS: usize = 8_000;
/// Max total chars for all file context combined.
const MAX_TOTAL_CONTEXT_CHARS: usize = 24_000;

pub fn build_prompt_with_type(
    task_description: &str,
    files: &[(String, String)],
    agent_type: AgentType,
) -> Vec<ChatMessage> {
    let file_context = build_file_context(files);

    let system = format!("{}{}", agent_type.role_description(), EDIT_FORMAT);
    let user_message = format!("TASK: {task_description}\n\nFILES:\n{file_context}");

    vec![
        ChatMessage::system(system),
        ChatMessage::user(user_message),
    ]
}

/// Build file context with smart truncation:
/// - New files: just the header (no content to show)
/// - Small files: full content
/// - Large files: first chunk + signature summary
/// - Total context capped to prevent model confusion
fn build_file_context(files: &[(String, String)]) -> String {
    let mut context = String::new();
    let mut total_chars = 0;

    for (path, content) in files {
        if content.is_empty() {
            let entry = format!("=== {path} === [NEW FILE — does not exist yet, use <<<CREATE>>> to create it]\n\n");
            context.push_str(&entry);
            total_chars += entry.len();
            continue;
        }

        if total_chars > MAX_TOTAL_CONTEXT_CHARS {
            // Over budget — just list remaining files without content
            context.push_str(&format!("=== {path} === [{} lines, content omitted — context limit reached]\n\n", content.lines().count()));
            continue;
        }

        if content.len() <= MAX_FILE_CHARS {
            // Small file — include full content
            let entry = format!("=== {path} ===\n{content}\n\n");
            total_chars += entry.len();
            context.push_str(&entry);
        } else {
            // Large file — include first chunk + summary of structure
            let lines: Vec<&str> = content.lines().collect();
            let total_lines = lines.len();

            // Extract function/struct signatures as summary
            let signatures: Vec<String> = lines.iter()
                .enumerate()
                .filter(|(_, line)| crate::code_utils::is_signature_line(line))
                .map(|(i, line)| format!("  L{}: {}", i + 1, line.trim()))
                .collect();

            // Include first N lines for context
            let preview_lines = 50.min(total_lines);
            let preview: String = lines[..preview_lines].join("\n");

            let summary = if signatures.is_empty() {
                format!("=== {path} === [{total_lines} lines, showing first {preview_lines}]\n{preview}\n... ({} more lines)\n\n",
                    total_lines - preview_lines)
            } else {
                format!("=== {path} === [{total_lines} lines, showing first {preview_lines} + structure]\n{preview}\n\n--- Structure ({} items) ---\n{}\n... ({} more lines)\n\n",
                    signatures.len(), signatures.join("\n"), total_lines - preview_lines)
            };

            total_chars += summary.len();
            context.push_str(&summary);
        }
    }

    context
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_parses_all_variants() {
        assert_eq!(AgentType::from_str("lint"), AgentType::LintFixer);
        assert_eq!(AgentType::from_str("lint-fixer"), AgentType::LintFixer);
        assert_eq!(AgentType::from_str("linter"), AgentType::LintFixer);
        assert_eq!(AgentType::from_str("test"), AgentType::TestWriter);
        assert_eq!(AgentType::from_str("test-writer"), AgentType::TestWriter);
        assert_eq!(AgentType::from_str("refactor"), AgentType::Refactorer);
        assert_eq!(AgentType::from_str("feature"), AgentType::FeatureAdder);
        assert_eq!(AgentType::from_str("bug"), AgentType::BugFixer);
        assert_eq!(AgentType::from_str("bugfix"), AgentType::BugFixer);
        assert_eq!(AgentType::from_str("general"), AgentType::General);
        assert_eq!(AgentType::from_str("unknown"), AgentType::General);
        assert_eq!(AgentType::from_str(""), AgentType::General);
    }

    #[test]
    fn lint_fixer_prompt_is_constrained() {
        let msgs = build_prompt_with_type("fix lint", &[], AgentType::LintFixer);
        let system = &msgs[0].content;
        assert!(system.contains("lint-fixing agent"), "should mention lint-fixing");
        assert!(system.contains("ONLY"), "should mention ONLY");
        assert!(system.contains("Do not refactor"), "should prohibit refactoring");
    }

    #[test]
    fn test_writer_preserves_code() {
        let msgs = build_prompt_with_type("write tests", &[], AgentType::TestWriter);
        let system = &msgs[0].content;
        assert!(system.contains("Do not modify the code under test"));
    }

    #[test]
    fn refactorer_preserves_behavior() {
        let msgs = build_prompt_with_type("refactor", &[], AgentType::Refactorer);
        let system = &msgs[0].content;
        assert!(system.contains("Do NOT change external behavior"));
    }

    #[test]
    fn all_prompts_include_edit_format() {
        for agent_type in [
            AgentType::General, AgentType::LintFixer, AgentType::TestWriter,
            AgentType::Refactorer, AgentType::FeatureAdder, AgentType::BugFixer,
        ] {
            let msgs = build_prompt_with_type("task", &[], agent_type);
            let system = &msgs[0].content;
            assert!(system.contains("<<<EDIT"), "Missing EDIT format for {agent_type:?}");
            assert!(system.contains("--- OLD"), "Missing OLD marker for {agent_type:?}");
            assert!(system.contains("--- NEW"), "Missing NEW marker for {agent_type:?}");
        }
    }

    #[test]
    fn user_message_includes_all_files() {
        let files = vec![
            ("src/a.rs".to_string(), "fn a(){}".to_string()),
            ("src/b.rs".to_string(), "fn b(){}".to_string()),
        ];
        let msgs = build_prompt("fix both", &files);
        let user = &msgs[1].content;
        assert!(user.contains("=== src/a.rs ==="));
        assert!(user.contains("=== src/b.rs ==="));
        assert!(user.contains("fn a(){}"));
        assert!(user.contains("fn b(){}"));
        assert!(user.contains("TASK: fix both"));
    }
}
