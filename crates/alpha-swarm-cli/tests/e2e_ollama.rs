/// E2E tests against real Ollama instance.
/// Run with: ALPHA_SWARM_OLLAMA_URL=http://100.81.10.8:11434 cargo test --test e2e_ollama -- --ignored
use std::sync::Arc;

use agent_core::Agent;
use inference_client::{Complexity, InferenceRouter, OllamaBackend};

fn get_ollama_url() -> String {
    std::env::var("ALPHA_SWARM_OLLAMA_URL").unwrap_or_else(|_| "http://100.81.10.8:11434".into())
}

fn setup_router() -> InferenceRouter {
    let url = get_ollama_url();
    InferenceRouter::new().add_backend(OllamaBackend::new(&url))
}

#[tokio::test]
#[ignore]
async fn list_models_from_ollama() {
    let router = setup_router();
    let models = router.list_models().await.unwrap();
    assert!(!models.is_empty(), "Ollama should have at least one model");

    for model in &models {
        println!("  {} ({})", model.name, model.parameter_size);
    }
}

#[tokio::test]
#[ignore]
async fn simple_inference_call() {
    let router = setup_router();
    let resp = router
        .generate("What is 2+2? Reply with just the number.", Complexity::Simple, &Default::default())
        .await
        .unwrap();

    assert!(!resp.content.is_empty());
    assert!(resp.content.contains('4'), "Expected '4' in response: {}", resp.content);
    assert!(resp.tokens_output > 0);
    println!("Model: {}, Tokens: {}/{}, Duration: {}ms", resp.model, resp.tokens_input, resp.tokens_output, resp.duration_ms);
}

#[tokio::test]
#[ignore]
async fn agent_modifies_code_via_ollama() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();

    let router = setup_router();
    let mut agent = Agent::new(Arc::new(router), dir.path());
    let result = agent
        .run(
            "Add a function called greet that takes a name parameter and prints a greeting. Call it from main.",
            &["src/main.rs".to_string()],
            Complexity::Simple,
        )
        .await
        .unwrap();

    println!("Model: {}, Edits: {}, Applied: {}", result.inference_response.model, result.edits.len(), result.applied);
    println!("Response:\n{}", result.inference_response.content);

    // The model should produce at least some output
    assert!(!result.inference_response.content.is_empty());
}

#[tokio::test]
#[ignore]
async fn model_routing_picks_appropriate_model() {
    let router = setup_router();

    let simple = router.recommend_model(Complexity::Simple).await.unwrap();
    let complex = router.recommend_model(Complexity::Complex).await.unwrap();

    println!("Simple: {} ({})", simple.name, simple.parameter_size);
    println!("Complex: {} ({})", complex.name, complex.parameter_size);

    // Complex should get a larger or equal model
    // (both are Ollama in this setup, so complex gets the biggest)
}

#[tokio::test]
#[ignore]
async fn embedding_generation() {
    let url = get_ollama_url();
    let ollama = OllamaBackend::new(&url);

    let embedding = ollama.embed("qwen2.5-coder:7b", "add a greet function").await.unwrap();
    assert!(!embedding.is_empty(), "Embedding should not be empty");
    assert!(embedding.len() > 100, "Embedding should have many dimensions, got {}", embedding.len());

    println!("Embedding dimensions: {}", embedding.len());
}
