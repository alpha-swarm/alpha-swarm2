use inference_client::ChatMessage;

const SYSTEM_PROMPT: &str = r#"You are a code modification agent. You receive a task description and file contents, then output precise file edits.

RULES:
- Only modify files that need to change
- Output edits in the exact format specified below
- Do not add unnecessary changes, comments, or formatting
- If the task is unclear or impossible, explain why instead of guessing

OUTPUT FORMAT:
For each file you want to modify, output a block like this:

<<<EDIT path/to/file.rs
--- OLD
the exact lines to replace (include enough context to be unique)
--- NEW
the replacement lines
>>>

For new files:

<<<CREATE path/to/new_file.rs
file contents here
>>>

For deleted files:

<<<DELETE path/to/file.rs
>>>

Output ONLY edit blocks. No explanation before or after unless the task cannot be done."#;

pub fn build_prompt(
    task_description: &str,
    files: &[(String, String)],
) -> Vec<ChatMessage> {
    let mut file_context = String::new();
    for (path, content) in files {
        file_context.push_str(&format!("=== {path} ===\n{content}\n\n"));
    }

    let user_message = format!(
        "TASK: {task_description}\n\nFILES:\n{file_context}"
    );

    vec![
        ChatMessage::system(SYSTEM_PROMPT),
        ChatMessage::user(user_message),
    ]
}
