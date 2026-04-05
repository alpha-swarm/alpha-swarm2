use anyhow::Result;
use async_trait::async_trait;

use crate::types::*;

#[async_trait]
pub trait InferenceBackend: Send + Sync {
    fn kind(&self) -> BackendKind;

    async fn health_check(&self) -> Result<()>;

    async fn list_models(&self) -> Result<Vec<ModelInfo>>;

    async fn chat(
        &self,
        model: &str,
        messages: &[ChatMessage],
        options: &InferenceOptions,
    ) -> Result<InferenceResponse>;

    async fn generate(
        &self,
        model: &str,
        prompt: &str,
        options: &InferenceOptions,
    ) -> Result<InferenceResponse> {
        let messages = vec![ChatMessage::user(prompt)];
        self.chat(model, &messages, options).await
    }
}
