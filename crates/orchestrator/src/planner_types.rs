//! WASI-portable planner types and constants.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use inference_client::Complexity;

/// A direct edit that can be applied without LLM inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectEdit {
    pub path: String,
    pub old: String,
    pub new: String,
}

/// A sub-task decomposed from a high-level goal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    pub id: String,
    pub description: String,
    pub files: Vec<String>,
    pub complexity: Complexity,
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// If set, this task can be applied directly without an LLM agent.
    #[serde(default)]
    pub edit: Option<DirectEdit>,
    /// Graph template: "edit", "create", "refactor", "doc". If set, uses graph executor instead of chat loop.
    #[serde(default)]
    pub template: Option<String>,
}

/// System prompt for the planner LLM.
pub const PLANNER_SYSTEM: &str = r#"You decompose a goal into the MINIMUM number of tasks. Correctness matters more than speed.

RULES:
- Create as FEW tasks as possible. If the goal can be done in 1 task, output 1 task.
- For EDITING existing files: list files from the repository file list below.
- For CREATING new files: you MAY list file paths that don't exist yet. The agent will use <<<CREATE>>> to make them.
- Each task lists the specific files it will read, modify, or create.
- Classify complexity: simple (1-2 files, small change), medium (2-4 files), complex (4+ files).
- If task-2 needs output from task-1, set depends_on: ["task-1"]. Tasks without depends_on run in parallel.
- Tasks that run in parallel MUST NOT modify the same file. Use depends_on if they share files.
- Maximum 5 tasks. Most goals need 1-3.

For TRIVIAL single-line changes (adding an import, renaming, inserting a line):
- Set complexity to "simple"
- Include an "edit" field with {"path":"file.rs","old":"existing line","new":"replacement line"}
- Tasks with an edit field execute instantly without an LLM — be precise with old/new text.

Set a "template" for each task to optimize execution:
- "edit": modifying 1 existing code file
- "create": creating a new file
- "refactor": modifying 2+ files together
- "doc": editing .md/.toml/.yaml (no build check needed)

OUTPUT FORMAT (JSON array only, no other text):
[{"id":"task-1","description":"what to do","files":["file.rs"],"complexity":"simple","depends_on":[],"edit":null,"template":"edit"}]"#;

/// Maximum number of sub-tasks the planner can create.
pub const MAX_TASKS: usize = 5;

/// Parse a planner response into sub-tasks, validating against known repo files.
/// Tolerant: tries JSON directly, then extracts from markdown fences, then falls back to single task.
pub fn parse_plan(json_str: &str, repo_files: &[String]) -> Result<Vec<SubTask>, String> {
    let input = json_str.trim();

    // Try 1: direct JSON parse
    let mut tasks: Vec<SubTask> = match serde_json::from_str(input) {
        Ok(t) => t,
        Err(_) => {
            // Try 2: extract from markdown code fence ```json ... ```
            let extracted = if let Some(start) = input.find("```json") {
                let after = &input[start + 7..];
                if let Some(end) = after.find("```") {
                    after[..end].trim()
                } else { "" }
            } else if let Some(start) = input.find("```") {
                let after = &input[start + 3..];
                if let Some(end) = after.find("```") {
                    after[..end].trim()
                } else { "" }
            } else { "" };

            if !extracted.is_empty() {
                serde_json::from_str(extracted).unwrap_or_default()
            } else {
                Vec::new()
            }
        }
    };

    // Try 3: if still empty, find JSON array in the text
    if tasks.is_empty() {
        if let Some(start) = input.find('[') {
            if let Some(end) = input.rfind(']') {
                let _ = serde_json::from_str::<Vec<SubTask>>(&input[start..=end])
                    .map(|t| tasks = t);
            }
        }
    }

    // Fallback: create single task from the goal (model didn't produce JSON)
    if tasks.is_empty() {
        // Extract file paths mentioned in the goal text (look for path-like strings)
        let mut files: Vec<String> = extract_file_paths_from_text(json_str);

        // Also include existing repo files that are referenced in the goal
        for repo_file in repo_files {
            if json_str.contains(repo_file.as_str()) && !files.contains(repo_file) {
                files.push(repo_file.clone());
            }
        }

        // If still empty, pick first 5 repo files as fallback
        if files.is_empty() {
            files = repo_files.iter()
                .filter(|f| !f.starts_with("target/") && !f.starts_with(".git/"))
                .take(5)
                .cloned()
                .collect();
        }

        return Ok(vec![SubTask {
            id: "task-1".into(),
            description: json_str.chars().take(500).collect(),
            files,
            complexity: inference_client::Complexity::Medium,
            depends_on: Vec::new(),
            edit: None,
            template: None,
        }]);
    }

    // Cap tasks
    if tasks.len() > MAX_TASKS {
        tasks.truncate(MAX_TASKS);
    }

    // Validate files: existing files must match repo, new files are allowed (agent will CREATE them)
    for task in &mut tasks {
        let (existing, new): (Vec<_>, Vec<_>) = task.files.iter()
            .partition(|f| repo_files.iter().any(|rf| rf == *f));

        // Keep all files — existing ones for EDIT, new ones for CREATE
        // But if a task has ONLY non-existent files and none look like valid paths, flag it
        if existing.is_empty() && !new.is_empty() {
            // Ensure new file paths look reasonable (have an extension, no spaces)
            task.files.retain(|f| f.contains('.') && !f.contains(' '));
        }
    }

    // Remove tasks with no files left
    tasks.retain(|t| !t.files.is_empty());

    if tasks.is_empty() {
        return Err("All planned tasks have no valid files".into());
    }

    // Cycle detection on dependency graph
    if detect_cycle(&tasks) {
        return Err("Dependency cycle detected in planned tasks".into());
    }

    // Warn about parallel tasks sharing files (don't error)
    warn_parallel_file_overlap(&tasks);

    Ok(tasks)
}

/// Detect cycles in the task dependency graph using DFS.
/// Returns `true` if a cycle exists.
fn detect_cycle(tasks: &[SubTask]) -> bool {
    let ids: HashSet<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for task in tasks {
        adj.entry(task.id.as_str()).or_default();
        for dep in &task.depends_on {
            if ids.contains(dep.as_str()) {
                adj.entry(task.id.as_str()).or_default().push(dep.as_str());
            }
        }
    }

    // 0 = unvisited, 1 = in-stack, 2 = done
    let mut state: HashMap<&str, u8> = ids.iter().map(|id| (*id, 0u8)).collect();

    fn dfs<'a>(
        node: &'a str,
        adj: &HashMap<&'a str, Vec<&'a str>>,
        state: &mut HashMap<&'a str, u8>,
    ) -> bool {
        state.insert(node, 1);
        if let Some(neighbors) = adj.get(node) {
            for &neighbor in neighbors {
                match state.get(neighbor) {
                    Some(1) => return true, // back edge = cycle
                    Some(0) => {
                        if dfs(neighbor, adj, state) {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
        }
        state.insert(node, 2);
        false
    }

    for &id in &ids {
        if state[id] == 0 && dfs(id, &adj, &mut state) {
            return true;
        }
    }

    false
}

/// Warn (via eprintln) if parallel tasks share files.
/// Two tasks are parallel if neither transitively depends on the other.
fn warn_parallel_file_overlap(tasks: &[SubTask]) {
    // Build transitive dependency sets for each task
    let id_to_idx: HashMap<&str, usize> = tasks.iter().enumerate().map(|(i, t)| (t.id.as_str(), i)).collect();

    // Compute transitive ancestors (all tasks this task depends on, directly or transitively)
    let mut ancestors: Vec<HashSet<usize>> = vec![HashSet::new(); tasks.len()];
    // Topological order: simple iterative fixpoint since task count is capped at MAX_TASKS
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..tasks.len() {
            for dep in &tasks[i].depends_on {
                if let Some(&dep_idx) = id_to_idx.get(dep.as_str()) {
                    if ancestors[i].insert(dep_idx) {
                        changed = true;
                    }
                    let dep_ancestors: Vec<usize> = ancestors[dep_idx].iter().copied().collect();
                    for a in dep_ancestors {
                        if ancestors[i].insert(a) {
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    for i in 0..tasks.len() {
        for j in (i + 1)..tasks.len() {
            // Parallel = neither is ancestor of the other
            let i_depends_on_j = ancestors[i].contains(&j);
            let j_depends_on_i = ancestors[j].contains(&i);
            if !i_depends_on_j && !j_depends_on_i {
                let shared: Vec<&String> = tasks[i].files.iter()
                    .filter(|f| tasks[j].files.contains(f))
                    .collect();
                if !shared.is_empty() {
                    eprintln!(
                        "WARNING: parallel tasks {} and {} share files: {:?}. Consider adding depends_on.",
                        tasks[i].id, tasks[j].id, shared
                    );
                }
            }
        }
    }
}

/// Extract file paths from free text (e.g. goal descriptions).
/// Looks for strings that look like file paths: contain '/' and end with a known extension.
fn extract_file_paths_from_text(text: &str) -> Vec<String> {
    let extensions = [".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".toml", ".json", ".yaml", ".yml", ".md", ".css", ".html"];
    let mut paths = Vec::new();

    for word in text.split_whitespace() {
        // Strip punctuation from edges
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '.' && c != '_' && c != '-');
        if clean.contains('/') && extensions.iter().any(|ext| clean.ends_with(ext)) {
            if !paths.contains(&clean.to_string()) {
                paths.push(clean.to_string());
            }
        }
    }

    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plan_allows_new_files() {
        let repo_files = vec!["src/lib.rs".into(), "Cargo.toml".into()];
        let json = r#"[{"id":"task-1","description":"Create types","files":["src/lib.rs","dashboard/src/types/swarm.ts"],"complexity":"simple"}]"#;
        let tasks = parse_plan(json, &repo_files).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].files.len(), 2);
        assert!(tasks[0].files.contains(&"dashboard/src/types/swarm.ts".to_string()));
    }

    #[test]
    fn parse_plan_rejects_invalid_new_paths() {
        let repo_files = vec!["src/lib.rs".into()];
        let json = r#"[{"id":"task-1","description":"Bad","files":["no extension"],"complexity":"simple"}]"#;
        let result = parse_plan(json, &repo_files);
        assert!(result.is_err());
    }

    #[test]
    fn parse_plan_keeps_existing_files() {
        let repo_files = vec!["src/lib.rs".into(), "src/main.rs".into()];
        let json = r#"[{"id":"task-1","description":"Edit","files":["src/lib.rs"],"complexity":"simple"}]"#;
        let tasks = parse_plan(json, &repo_files).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].files, vec!["src/lib.rs"]);
    }

    #[test]
    fn extract_paths_from_goal_text() {
        let text = "Read crates/knowledge-base/src/schema.rs then CREATE dashboard/src/types/swarm.ts";
        let paths = extract_file_paths_from_text(text);
        assert!(paths.contains(&"crates/knowledge-base/src/schema.rs".to_string()));
        assert!(paths.contains(&"dashboard/src/types/swarm.ts".to_string()));
    }

    #[test]
    fn fallback_uses_goal_paths() {
        let repo_files = vec!["crates/knowledge-base/src/schema.rs".into(), "src/lib.rs".into()];
        // This text won't parse as JSON, triggering fallback
        let text = "Read crates/knowledge-base/src/schema.rs and create dashboard/src/types/swarm.ts";
        let tasks = parse_plan(text, &repo_files).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].files.contains(&"crates/knowledge-base/src/schema.rs".to_string()));
        assert!(tasks[0].files.contains(&"dashboard/src/types/swarm.ts".to_string()));
    }

    #[test]
    fn parse_plan_only_new_files() {
        let repo_files = vec!["src/lib.rs".into()];
        let json = r#"[{"id":"task-1","description":"Create","files":["new/file.ts"],"complexity":"simple"}]"#;
        let tasks = parse_plan(json, &repo_files).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].files, vec!["new/file.ts"]);
    }

    #[test]
    fn test_depends_on_parsed() {
        let repo_files = vec!["src/lib.rs".into(), "src/main.rs".into()];
        let json = r#"[
            {"id":"task-1","description":"Write lib","files":["src/lib.rs"],"complexity":"simple","depends_on":[]},
            {"id":"task-2","description":"Wire up main","files":["src/main.rs"],"complexity":"simple","depends_on":["task-1"]}
        ]"#;
        let tasks = parse_plan(json, &repo_files).unwrap();
        assert_eq!(tasks.len(), 2);
        assert!(tasks[0].depends_on.is_empty());
        assert_eq!(tasks[1].depends_on, vec!["task-1"]);
    }

    #[test]
    fn test_cycle_detection() {
        let tasks = vec![
            SubTask {
                id: "task-1".into(),
                description: "A".into(),
                files: vec!["a.rs".into()],
                complexity: Complexity::Simple,
                depends_on: vec!["task-2".into()],
                edit: None,
                template: None,
            },
            SubTask {
                id: "task-2".into(),
                description: "B".into(),
                files: vec!["b.rs".into()],
                complexity: Complexity::Simple,
                depends_on: vec!["task-1".into()],
                edit: None,
                template: None,
            },
        ];
        assert!(detect_cycle(&tasks));
    }

    #[test]
    fn test_no_cycle() {
        let tasks = vec![
            SubTask {
                id: "task-1".into(),
                description: "A".into(),
                files: vec!["a.rs".into()],
                complexity: Complexity::Simple,
                depends_on: vec![],
                edit: None,
                template: None,
            },
            SubTask {
                id: "task-2".into(),
                description: "B".into(),
                files: vec!["b.rs".into()],
                complexity: Complexity::Simple,
                depends_on: vec!["task-1".into()],
                edit: None,
                template: None,
            },
            SubTask {
                id: "task-3".into(),
                description: "C".into(),
                files: vec!["c.rs".into()],
                complexity: Complexity::Simple,
                depends_on: vec!["task-1".into(), "task-2".into()],
                edit: None,
                template: None,
            },
        ];
        assert!(!detect_cycle(&tasks));
    }
}
