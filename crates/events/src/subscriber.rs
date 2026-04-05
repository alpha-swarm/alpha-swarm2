use anyhow::{Context, Result};
use futures::StreamExt;
use tracing::{info, warn};

use crate::SwarmEvent;

/// Subscribes to NATS events for a project or all projects.
pub struct EventSubscriber {
    client: async_nats::Client,
}

impl EventSubscriber {
    pub async fn connect(nats_url: &str) -> Result<Self> {
        let client = async_nats::connect(nats_url)
            .await
            .with_context(|| format!("Failed to connect to NATS at {nats_url}"))?;

        info!(url = nats_url, "Event subscriber connected to NATS");
        Ok(Self { client })
    }

    /// Subscribe to all events for a specific project.
    pub async fn subscribe_project(&self, project: &str) -> Result<EventStream> {
        let subject = format!("alpha-swarm.{project}.>");
        let sub = self.client.subscribe(subject.clone())
            .await
            .with_context(|| format!("Failed to subscribe to {subject}"))?;

        info!(subject = %subject, "Subscribed to project events");
        Ok(EventStream { inner: sub })
    }

    /// Subscribe to all events across all projects.
    pub async fn subscribe_all(&self) -> Result<EventStream> {
        let subject = "alpha-swarm.>";
        let sub = self.client.subscribe(subject.to_string())
            .await
            .context("Failed to subscribe to all events")?;

        info!("Subscribed to all alpha-swarm events");
        Ok(EventStream { inner: sub })
    }
}

pub struct EventStream {
    inner: async_nats::Subscriber,
}

impl EventStream {
    /// Get the next event (blocking).
    pub async fn next(&mut self) -> Option<SwarmEvent> {
        loop {
            let msg = self.inner.next().await?;
            match serde_json::from_slice::<SwarmEvent>(&msg.payload) {
                Ok(event) => return Some(event),
                Err(e) => {
                    warn!(
                        subject = %msg.subject,
                        error = %e,
                        "Failed to parse event, skipping"
                    );
                    continue;
                }
            }
        }
    }
}
