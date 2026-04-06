/// A single file edit produced by an agent.
#[derive(Debug, Clone)]
pub enum FileEdit {
    Edit {
        path: String,
        old: String,
        new: String,
    },
    Create {
        path: String,
        content: String,
    },
    Delete {
        path: String,
    },
}

/// A tool call parsed from the model's response.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub params_json: String,
}

/// An agent action — either a tool call, a file edit, or a sub-agent spawn.
#[derive(Debug, Clone)]
pub enum AgentAction {
    Tool(ToolCall),
    Edit(FileEdit),
    Agent { description: String, files: Vec<String>, complexity: String },
    Done { summary: String },
}

/// Error type for parse failures (no external deps).
#[derive(Debug)]
pub struct ParseError(pub String);

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Parse an LLM response into a list of file edits.
///
/// Expects edit blocks in the format:
/// ```text
/// <<<EDIT path/to/file
/// --- OLD
/// lines to replace
/// --- NEW
/// replacement lines
/// >>>
/// ```
pub fn parse_edits(response: &str) -> Result<Vec<FileEdit>, ParseError> {
    let mut edits = Vec::new();
    let mut pos = 0;

    while pos < response.len() {
        let Some(start) = response[pos..].find("<<<") else { break };
        let block_start = pos + start + 3;

        let Some(end_offset) = response[block_start..].find(">>>") else {
            return Err(ParseError(format!(
                "Unclosed edit block starting at position {}", pos + start
            )));
        };
        let block_end = block_start + end_offset;
        let block = response[block_start..block_end].trim();

        let edit = parse_single_block(block)?;
        edits.push(edit);

        pos = block_end + 3;
    }

    Ok(edits)
}

/// Parse all action blocks from a model response: TOOL, EDIT, CREATE, DELETE, AGENT, DONE.
pub fn parse_actions(response: &str) -> Result<Vec<AgentAction>, ParseError> {
    let mut actions = Vec::new();
    let mut pos = 0;

    while pos < response.len() {
        let Some(start) = response[pos..].find("<<<") else { break };
        let block_start = pos + start + 3;

        let Some(end_offset) = response[block_start..].find(">>>") else {
            return Err(ParseError(format!(
                "Unclosed block starting at position {}", pos + start
            )));
        };
        let block_end = block_start + end_offset;
        let block = response[block_start..block_end].trim();

        let first_newline = block.find('\n').unwrap_or(block.len());
        let header = block[..first_newline].trim();
        let body = if first_newline < block.len() { &block[first_newline + 1..] } else { "" };

        if let Some(tool_name) = header.strip_prefix("TOOL ") {
            actions.push(AgentAction::Tool(ToolCall {
                name: tool_name.trim().to_string(),
                params_json: body.trim().to_string(),
            }));
        } else if header.starts_with("EDIT ") || header.starts_with("CREATE ") || header.starts_with("DELETE ") {
            let edit = parse_single_block(block)?;
            actions.push(AgentAction::Edit(edit));
        } else if header == "AGENT" {
            // Parse agent spawn: {"description": "...", "files": [...], "complexity": "..."}
            let desc = extract_json_field(body, "description");
            let complexity = extract_json_field(body, "complexity");
            let files: Vec<String> = extract_json_array(body, "files");
            actions.push(AgentAction::Agent { description: desc, files, complexity });
        } else if header == "DONE" {
            actions.push(AgentAction::Done { summary: body.trim().to_string() });
        } else {
            // Try as edit block for backwards compatibility
            match parse_single_block(block) {
                Ok(edit) => actions.push(AgentAction::Edit(edit)),
                Err(_) => {} // Skip unknown blocks
            }
        }

        pos = block_end + 3;
    }

    Ok(actions)
}

/// Simple JSON field extraction (no serde dependency in edit-parser).
fn extract_json_field(json: &str, field: &str) -> String {
    let needle = format!("\"{}\"", field);
    let Some(idx) = json.find(&needle) else { return String::new() };
    let after_key = &json[idx + needle.len()..];
    let Some(colon) = after_key.find(':') else { return String::new() };
    let after_colon = after_key[colon + 1..].trim_start();
    if after_colon.starts_with('"') {
        let content = &after_colon[1..];
        let end = content.find('"').unwrap_or(content.len());
        content[..end].to_string()
    } else {
        String::new()
    }
}

fn extract_json_array(json: &str, field: &str) -> Vec<String> {
    let needle = format!("\"{}\"", field);
    let Some(idx) = json.find(&needle) else { return Vec::new() };
    let after_key = &json[idx + needle.len()..];
    let Some(bracket) = after_key.find('[') else { return Vec::new() };
    let after_bracket = &after_key[bracket + 1..];
    let Some(end) = after_bracket.find(']') else { return Vec::new() };
    let inner = &after_bracket[..end];
    inner.split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_single_block(block: &str) -> Result<FileEdit, ParseError> {
    let first_newline = block.find('\n').unwrap_or(block.len());
    let header = block[..first_newline].trim();
    let body = if first_newline < block.len() { &block[first_newline + 1..] } else { "" };

    if let Some(path) = header.strip_prefix("EDIT ") {
        parse_edit_block(path.trim(), body)
    } else if let Some(path) = header.strip_prefix("CREATE ") {
        Ok(FileEdit::Create {
            path: path.trim().to_string(),
            content: body.to_string(),
        })
    } else if let Some(path) = header.strip_prefix("DELETE ") {
        Ok(FileEdit::Delete {
            path: path.trim().to_string(),
        })
    } else {
        Err(ParseError(format!("Unknown edit block type: {header}")))
    }
}

fn parse_edit_block(path: &str, body: &str) -> Result<FileEdit, ParseError> {
    let Some(old_start) = body.find("--- OLD") else {
        return Err(ParseError(format!("EDIT block for {path} missing --- OLD marker")));
    };
    let Some(new_start) = body.find("--- NEW") else {
        return Err(ParseError(format!("EDIT block for {path} missing --- NEW marker")));
    };

    Ok(FileEdit::Edit {
        path: path.to_string(),
        old: body[old_start + 7..new_start].trim().to_string(),
        new: body[new_start + 7..].trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_edit() {
        let response = "<<<EDIT src/main.rs\n--- OLD\nfn main() {}\n--- NEW\nfn main() { println!(\"hi\"); }\n>>>";
        let edits = parse_edits(response).unwrap();
        assert_eq!(edits.len(), 1);
        assert!(matches!(&edits[0], FileEdit::Edit { path, .. } if path == "src/main.rs"));
    }

    #[test]
    fn test_parse_create() {
        let response = "<<<CREATE src/new.rs\nfn new_function() -> bool { true }\n>>>";
        let edits = parse_edits(response).unwrap();
        assert_eq!(edits.len(), 1);
        assert!(matches!(&edits[0], FileEdit::Create { path, .. } if path == "src/new.rs"));
    }

    #[test]
    fn test_parse_delete() {
        let response = "<<<DELETE src/old.rs\n>>>";
        let edits = parse_edits(response).unwrap();
        assert_eq!(edits.len(), 1);
        assert!(matches!(&edits[0], FileEdit::Delete { path } if path == "src/old.rs"));
    }

    #[test]
    fn test_parse_multiple() {
        let response = "<<<EDIT a.rs\n--- OLD\nx\n--- NEW\ny\n>>>\n<<<CREATE b.rs\nnew\n>>>\n<<<DELETE c.rs\n>>>";
        let edits = parse_edits(response).unwrap();
        assert_eq!(edits.len(), 3);
    }

    #[test]
    fn test_unclosed_block() {
        let response = "<<<EDIT a.rs\n--- OLD\nx\n--- NEW\ny";
        assert!(parse_edits(response).is_err());
    }

    #[test]
    fn test_empty_response() {
        let edits = parse_edits("").unwrap();
        assert!(edits.is_empty());
    }

    #[test]
    fn test_text_around_blocks_ignored() {
        let response = "Here is my solution:\n\n<<<EDIT src/main.rs\n--- OLD\nold\n--- NEW\nnew\n>>>\n\nHope this helps!";
        let edits = parse_edits(response).unwrap();
        assert_eq!(edits.len(), 1);
    }

    #[test]
    fn test_code_with_angle_brackets() {
        let response = "<<<EDIT src/lib.rs\n--- OLD\nfn foo() -> Vec<String> { vec![] }\n--- NEW\nfn foo() -> Vec<String> { vec![\"hello\".into()] }\n>>>";
        let edits = parse_edits(response).unwrap();
        assert_eq!(edits.len(), 1);
        if let FileEdit::Edit { new, .. } = &edits[0] {
            assert!(new.contains("Vec<String>"));
        }
    }

    #[test]
    fn test_missing_old_marker() {
        let response = "<<<EDIT a.rs\n--- NEW\nnew content\n>>>";
        assert!(parse_edits(response).is_err());
    }

    #[test]
    fn test_edit_preserves_content() {
        let response = "<<<EDIT src/main.rs\n--- OLD\nlet x = 1;\nlet y = 2;\n--- NEW\nlet x = 10;\nlet y = 20;\n>>>";
        let edits = parse_edits(response).unwrap();
        if let FileEdit::Edit { old, new, .. } = &edits[0] {
            assert!(old.contains("let x = 1;"));
            assert!(new.contains("let x = 10;"));
        } else {
            panic!("Expected Edit");
        }
    }
}
