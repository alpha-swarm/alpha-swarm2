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
}
