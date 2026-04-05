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

const EDIT_FORMAT: &str = "\nOUTPUT FORMAT:\nFor each file you want to modify, output a block like this:\n\n<<<EDIT path/to/file.rs\n--- OLD\nthe exact lines to replace (include enough context to be unique)\n--- NEW\nthe replacement lines\n>>>\n\nFor new files:\n\n<<<CREATE path/to/new_file.rs\nfile contents here\n>>>\n\nFor deleted files:\n\n<<<DELETE path/to/file.rs\n>>>\n\nOutput ONLY edit blocks. No explanation before or after unless the task cannot be done.";

pub fn build_prompt(
    task_description: &str,
    files: &[(String, String)],
) -> Vec<ChatMessage> {
    build_prompt_with_type(task_description, files, AgentType::General)
}

pub fn build_prompt_with_type(
    task_description: &str,
    files: &[(String, String)],
    agent_type: AgentType,
) -> Vec<ChatMessage> {
    let mut file_context = String::new();
    for (path, content) in files {
        file_context.push_str(&format!("=== {path} ===\n{content}\n\n"));
    }

    let system = format!("{}{}", agent_type.role_description(), EDIT_FORMAT);
    let user_message = format!("TASK: {task_description}\n\nFILES:\n{file_context}");

    vec![
        ChatMessage::system(system),
        ChatMessage::user(user_message),
    ]
}
