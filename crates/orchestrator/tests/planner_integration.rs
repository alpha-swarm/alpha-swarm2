//! Integration test: planner decomposes a goal into sub-tasks.
//! Requires: Ollama running with qwen2.5-coder:14b or qwen3:32b.

#[cfg(feature = "native")]
mod tests {
    use std::sync::Arc;
    use inference_client::{InferenceRouter, OllamaBackend};
    use swarm_orchestrator::planner_types::{parse_plan, SubTask};

    fn ollama_url() -> String {
        std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".into())
    }

    fn planner_model() -> String {
        std::env::var("PLANNER_MODEL").unwrap_or_else(|_| "qwen2.5-coder:14b".into())
    }

    #[tokio::test]
    #[ignore] // Run with: cargo test -p swarm-orchestrator --test planner_integration -- --ignored
    async fn test_plan_simple_goal() {
        let router = Arc::new(InferenceRouter::new().add_backend(OllamaBackend::new(&ollama_url())));
        let tier = swarm_config::TierConfig {
            model: planner_model(),
            ..swarm_config::TierConfig::orchestrator()
        };

        let repo_files = vec![
            "src/lib.rs".to_string(),
            "src/main.rs".to_string(),
            "Cargo.toml".to_string(),
        ];

        let result = swarm_orchestrator::plan_goal(
            &router,
            "Add a hello() function to src/lib.rs that returns the string 'hello world'",
            &repo_files,
            &tier,
            None,
            None,
        ).await;

        assert!(result.is_ok(), "Planning failed: {:?}", result.err());
        let tasks = result.unwrap();
        assert!(!tasks.is_empty(), "No tasks generated");
        assert!(tasks.len() <= 5, "Too many tasks: {}", tasks.len());

        // At least one task should reference src/lib.rs
        let has_lib = tasks.iter().any(|t| t.files.iter().any(|f| f.contains("lib.rs")));
        assert!(has_lib, "No task references lib.rs: {:?}", tasks);
    }

    #[tokio::test]
    #[ignore]
    async fn test_plan_with_dependencies() {
        let router = Arc::new(InferenceRouter::new().add_backend(OllamaBackend::new(&ollama_url())));
        let tier = swarm_config::TierConfig {
            model: planner_model(),
            ..swarm_config::TierConfig::orchestrator()
        };

        let repo_files = vec![
            "src/lib.rs".to_string(),
            "src/types.rs".to_string(),
            "tests/test.rs".to_string(),
        ];

        let result = swarm_orchestrator::plan_goal(
            &router,
            "Add a struct User { name: String, age: u32 } to src/types.rs, then add a test for it in tests/test.rs",
            &repo_files,
            &tier,
            None,
            None,
        ).await;

        assert!(result.is_ok());
        let tasks = result.unwrap();
        assert!(tasks.len() >= 1);
    }

    #[test]
    fn test_parse_plan_with_edit_field() {
        let json = r#"[{
            "id": "task-1",
            "description": "Add import",
            "files": ["src/lib.rs"],
            "complexity": "simple",
            "depends_on": [],
            "edit": {"path": "src/lib.rs", "old": "use std::io;", "new": "use std::io;\nuse std::fmt;"}
        }]"#;

        let repo_files = vec!["src/lib.rs".to_string()];
        let tasks = parse_plan(json, &repo_files).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].edit.is_some());
        let edit = tasks[0].edit.as_ref().unwrap();
        assert_eq!(edit.path, "src/lib.rs");
        assert!(edit.new.contains("std::fmt"));
    }

    #[test]
    fn test_parse_plan_with_new_files() {
        let json = r#"[{
            "id": "task-1",
            "description": "Create new module",
            "files": ["src/new_module.rs"],
            "complexity": "simple",
            "depends_on": []
        }]"#;

        let repo_files = vec!["src/lib.rs".to_string()];
        let tasks = parse_plan(json, &repo_files).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].files.contains(&"src/new_module.rs".to_string()));
    }
}
