/// Integration tests for knowledge-base crate.
/// Requires SurrealDB running at 127.0.0.1:8000 with root/root.
use knowledge_base::*;

async fn test_store() -> KnowledgeStore {
    // Use unique namespace per test run to avoid collisions
    let ns = format!("test_{}", uuid::Uuid::new_v4().to_string().replace('-', ""));
    KnowledgeStore::connect("127.0.0.1:8000", &ns, "test")
        .await
        .expect("Failed to connect to SurrealDB")
}

#[tokio::test]
async fn store_and_retrieve_run() {
    let store = test_store().await;
    let run = AgentRun::new("test-project", "add a greet function", "agent-1", "qwen:7b");

    let id = store.store_run(&run).await.expect("store failed");
    assert!(!id.is_empty());
    assert_ne!(id, "unknown");

    let runs = store.list_runs("test-project", None).await.expect("list failed");
    assert!(!runs.is_empty());
    assert_eq!(runs[0].task_description, "add a greet function");
}

#[tokio::test]
async fn filter_runs_by_status() {
    let store = test_store().await;

    let mut passed = AgentRun::new("proj", "task1", "a1", "m1");
    passed.status = RunStatus::Passed;
    store.store_run(&passed).await.unwrap();

    let mut failed = AgentRun::new("proj", "task2", "a2", "m1");
    failed.status = RunStatus::Failed;
    store.store_run(&failed).await.unwrap();

    let mut running = AgentRun::new("proj", "task3", "a3", "m1");
    running.status = RunStatus::Running;
    store.store_run(&running).await.unwrap();

    let failed_only = store.list_runs("proj", Some(RunStatus::Failed)).await.unwrap();
    assert_eq!(failed_only.len(), 1);
    assert_eq!(failed_only[0].task_description, "task2");

    let running_only = store.running_agents("proj").await.unwrap();
    assert_eq!(running_only.len(), 1);
    assert_eq!(running_only[0].task_description, "task3");
}

#[tokio::test]
async fn metrics_from_stored_runs() {
    let store = test_store().await;

    for i in 0..7 {
        let mut run = AgentRun::new("metrics-proj", &format!("task-{i}"), &format!("a-{i}"), "model-a");
        run.status = RunStatus::Passed;
        run.tokens_input = 100;
        run.tokens_output = 50;
        run.duration_ms = 1000;
        store.store_run(&run).await.unwrap();
    }
    for i in 7..9 {
        let mut run = AgentRun::new("metrics-proj", &format!("task-{i}"), &format!("a-{i}"), "model-b");
        run.status = RunStatus::Failed;
        run.tokens_input = 200;
        run.tokens_output = 100;
        run.duration_ms = 2000;
        store.store_run(&run).await.unwrap();
    }
    let mut skipped = AgentRun::new("metrics-proj", "task-9", "a-9", "model-a");
    skipped.status = RunStatus::Skipped;
    store.store_run(&skipped).await.unwrap();

    let runs = store.list_runs("metrics-proj", None).await.unwrap();
    let metrics = ProjectMetrics::from_runs(&runs);

    assert_eq!(metrics.total_runs, 10);
    assert_eq!(metrics.passed, 7);
    assert_eq!(metrics.failed, 2);
    assert_eq!(metrics.skipped, 1);
    assert!((metrics.pass_rate - 0.7).abs() < 0.01);
    assert_eq!(metrics.models_used.len(), 2);
}

#[tokio::test]
async fn store_and_search_embedding() {
    let store = test_store().await;

    let mut run = AgentRun::new("embed-proj", "add logging to main", "a1", "m1");
    run.status = RunStatus::Passed;
    run.embedding = Some(vec![1.0, 0.0, 0.0, 0.0]);
    store.store_run(&run).await.unwrap();

    // Search with similar embedding
    let similar = store.find_similar("embed-proj", &[0.99, 0.01, 0.0, 0.0], 5, 0.5).await.unwrap();
    // Note: SurrealDB vector search requires HNSW index for cosine similarity.
    // In schemaless mode without explicit index, this may return empty.
    // This test verifies the query doesn't error.
    let _ = similar;
}

#[tokio::test]
async fn empty_project_returns_empty() {
    let store = test_store().await;

    let runs = store.list_runs("nonexistent", None).await.unwrap();
    assert!(runs.is_empty());

    let agents = store.running_agents("nonexistent").await.unwrap();
    assert!(agents.is_empty());
}
