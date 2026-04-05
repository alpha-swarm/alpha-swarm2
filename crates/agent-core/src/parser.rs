use anyhow::{Result, bail};

/// A single file edit produced by the agent.
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

/// Parse the LLM response into a list of file edits.
pub fn parse_edits(response: &str) -> Result<Vec<FileEdit>> {
    let mut edits = Vec::new();
    let mut pos = 0;

    while pos < response.len() {
        // Find next <<<
        if let Some(start) = response[pos..].find("<<<") {
            let block_start = pos + start + 3;

            // Find matching >>>
            let Some(end_offset) = response[block_start..].find(">>>") else {
                bail!("Unclosed edit block starting at position {}", pos + start);
            };
            let block_end = block_start + end_offset;
            let block = &response[block_start..block_end];

            let edit = parse_single_block(block.trim())?;
            edits.push(edit);

            pos = block_end + 3;
        } else {
            break;
        }
    }

    Ok(edits)
}

fn parse_single_block(block: &str) -> Result<FileEdit> {
    // First line determines the type: EDIT, CREATE, or DELETE
    let first_newline = block.find('\n').unwrap_or(block.len());
    let header = block[..first_newline].trim();
    let body = if first_newline < block.len() {
        &block[first_newline + 1..]
    } else {
        ""
    };

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
        bail!("Unknown edit block type: {header}");
    }
}

fn parse_edit_block(path: &str, body: &str) -> Result<FileEdit> {
    let Some(old_start) = body.find("--- OLD") else {
        bail!("EDIT block for {path} missing --- OLD marker");
    };
    let Some(new_start) = body.find("--- NEW") else {
        bail!("EDIT block for {path} missing --- NEW marker");
    };

    let old_content = body[old_start + 7..new_start].trim().to_string();
    let new_content = body[new_start + 7..].trim().to_string();

    Ok(FileEdit::Edit {
        path: path.to_string(),
        old: old_content,
        new: new_content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_edit() {
        let response = r#"
<<<EDIT src/main.rs
--- OLD
fn main() {
    println!("hello");
}
--- NEW
fn main() {
    println!("hello, world!");
}
>>>
"#;
        let edits = parse_edits(response).unwrap();
        assert_eq!(edits.len(), 1);
        match &edits[0] {
            FileEdit::Edit { path, old, new } => {
                assert_eq!(path, "src/main.rs");
                assert!(old.contains("hello"));
                assert!(new.contains("hello, world!"));
            }
            _ => panic!("Expected Edit"),
        }
    }

    #[test]
    fn test_parse_create() {
        let response = r#"
<<<CREATE src/new.rs
fn new_function() -> bool {
    true
}
>>>
"#;
        let edits = parse_edits(response).unwrap();
        assert_eq!(edits.len(), 1);
        match &edits[0] {
            FileEdit::Create { path, content } => {
                assert_eq!(path, "src/new.rs");
                assert!(content.contains("new_function"));
            }
            _ => panic!("Expected Create"),
        }
    }

    #[test]
    fn test_parse_multiple() {
        let response = r#"
<<<EDIT src/a.rs
--- OLD
let x = 1;
--- NEW
let x = 2;
>>>

<<<DELETE src/old.rs
>>>

<<<CREATE src/b.rs
new file
>>>
"#;
        let edits = parse_edits(response).unwrap();
        assert_eq!(edits.len(), 3);
    }
}
