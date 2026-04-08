use anyhow::{Context, Result};
use tracing::info;

use crate::SwarmEvent;

/// Publishes events to NATS subjects.
pub struct EventPublisher {
    client: async_nats::Client,
}

impl EventPublisher {
    pub async fn connect(nats_url: &str) -> Result<Self> {
        let client = async_nats::connect(nats_url)
            .await
            .with_context(|| format!("Failed to connect to NATS at {nats_url}"))?;

        info!(url = nats_url, "Event publisher connected to NATS");
        Ok(Self { client })
    }

    pub async fn publish(&self, event: &SwarmEvent) -> Result<()> {
        let subject = event.nats_subject();
        let payload = serde_json::to_vec(event)
            .context("Failed to serialize event")?;

        self.client.publish(subject.clone(), payload.into())
            .await
            .with_context(|| format!("Failed to publish to {subject}"))?;

        info!(subject = %subject, "Published event");
        Ok(())
    }

    /// Publish and flush (ensures delivery before returning).
    pub async fn publish_flush(&self, event: &SwarmEvent) -> Result<()> {
        self.publish(event).await?;
        self.client.flush().await.context("NATS flush failed")?;
        Ok(())
    }
}
