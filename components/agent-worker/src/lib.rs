wit_bindgen::generate!({
    world: "agent-worker",
    path: "wit",
});

use alpha_swarm::agent_worker::types::*;
use alpha_swarm::agent_worker::completions;
use alpha_swarm::agent_worker::repository;
use alpha_swarm::agent_worker::gate;

const SYSTEM_PROMPT: &str = r#"You are a code modification agent. You receive a task and files, then output precise edits.

OUTPUT FORMAT — for each file to modify:

<<<EDIT path/to/file
--- OLD
exact lines to replace
--- NEW
replacement lines
>>>

For new files: <<<CREATE path/to/file
content
>>>

Output ONLY edit blocks."#;

struct AgentWorker;

impl exports::alpha_swarm::agent_worker::handler::Guest for AgentWorker {
    fn handle_task(task: Task) -> Result<TaskResult, SwarmError> {
        let agent_id = format!("wasi-agent-{}", task.id);

        // 1. Read assigned files
        let files = repository::read_files(&task.repo, &agent_id, &task.assigned_files)?;

        // 2. Build prompt
        let mut file_context = String::new();
        for entry in &files {
            file_context.push_str(&format!("=== {} ===\n{}\n\n", entry.path, entry.content));
        }

        let user_msg = format!("TASK: {}\n\nFILES:\n{}", task.description, file_context);
        let messages = vec![
            completions::ChatMessage {
                role: "system".into(),
                content: SYSTEM_PROMPT.into(),
            },
            completions::ChatMessage {
                role: "user".into(),
                content: user_msg,
            },
        ];

        // 3. Call inference
        let response = completions::chat(&messages, task.complexity, None)?;

        // 4. Parse and apply edits
        let edits = parse_edits(&response.content);

        for edit in &edits {
            match edit {
                Edit::Modify { path, old, new } => {
                    let content = repository::read_file(&task.repo, &agent_id, path)?;
                    let updated = content.replacen(old.as_str(), new.as_str(), 1);
                    repository::write_file(&task.repo, &agent_id, path, &updated)?;
                }
                Edit::Create { path, content } => {
                    repository::write_file(&task.repo, &agent_id, path, content)?;
                }
            }
        }

        // 5. Run quality gate
        let checks = gate::check_all(&task.repo, &agent_id)?;
        let all_passed = checks.iter().all(|c| c.passed);

        // 6. Extract diff
        let diff = repository::extract_diff(&task.repo, &agent_id).ok();

        let status = if edits.is_empty() {
            TaskStatus::Skipped
        } else if all_passed {
            TaskStatus::Passed
        } else {
            TaskStatus::Failed
        };

        let error_message = if !all_passed {
            Some(checks.iter()
                .filter(|c| !c.passed)
                .map(|c| format!("{}: {}", c.check_name, c.stderr))
                .collect::<Vec<_>>()
                .join("\n"))
        } else {
            None
        };

        Ok(TaskResult {
            task_id: task.id,
            agent_id,
            status,
            diff,
            model_used: response.model,
            duration_ms: response.duration_ms,
            error_message,
        })
    }
}

export!(AgentWorker);

// --- Simple edit parser ---

enum Edit {
    Modify { path: String, old: String, new: String },
    Create { path: String, content: String },
}

fn parse_edits(response: &str) -> Vec<Edit> {
    let mut edits = Vec::new();
    let mut pos = 0;

    while pos < response.len() {
        let Some(start) = response[pos..].find("<<<") else { break };
        let block_start = pos + start + 3;
        let Some(end_offset) = response[block_start..].find(">>>") else { break };
        let block_end = block_start + end_offset;
        let block = response[block_start..block_end].trim();

        let first_newline = block.find('\n').unwrap_or(block.len());
        let header = block[..first_newline].trim();
        let body = if first_newline < block.len() { &block[first_newline + 1..] } else { "" };

        if let Some(path) = header.strip_prefix("EDIT ") {
            if let (Some(old_start), Some(new_start)) = (body.find("--- OLD"), body.find("--- NEW")) {
                let old = body[old_start + 7..new_start].trim().to_string();
                let new_content = body[new_start + 7..].trim().to_string();
                edits.push(Edit::Modify {
                    path: path.trim().to_string(),
                    old,
                    new: new_content,
                });
            }
        } else if let Some(path) = header.strip_prefix("CREATE ") {
            edits.push(Edit::Create {
                path: path.trim().to_string(),
                content: body.to_string(),
            });
        }

        pos = block_end + 3;
    }

    edits
}
