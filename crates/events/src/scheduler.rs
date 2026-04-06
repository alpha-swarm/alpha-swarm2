use std::time::Duration;

use anyhow::{Context, Result};
use async_nats::jetstream::kv;
use serde::{Deserialize, Serialize};
use tracing::{info, debug};

/// Lease TTL — if a daemon dies, its leases expire after this.
const LEASE_TTL: Duration = Duration::from_secs(600); // 10 minutes
/// Resource entries expire after 1 minute if not refreshed.
const RESOURCE_TTL: Duration = Duration::from_secs(60);

/// KV bucket names
const TASKS_BUCKET: &str = "swarm-tasks";
const LEASES_BUCKET: &str = "swarm-leases";
const RESOURCES_BUCKET: &str = "swarm-resources";

/// A task entry in the KV store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEntry {
    pub run_id: String,
    pub project: String,
    pub goal: String,
    pub status: String,
    pub created_at: String,
}

/// A lease entry — who's working on what.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseEntry {
    pub daemon_id: String,
    pub run_id: String,
    pub claimed_at: String,
}

/// Resource snapshot published by each daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostResources {
    pub daemon_id: String,
    pub host: String,
    pub cpu_percent: f64,
    pub ram_percent: f64,
    pub disk_percent: f64,
    pub available_models: Vec<String>,
    pub updated_at: String,
}

/// Distributed task scheduler backed by NATS KV.
pub struct NatsScheduler {
    tasks: kv::Store,
    leases: kv::Store,
    resources: kv::Store,
    daemon_id: String,
}

impl NatsScheduler {
    /// Connect to NATS and create/open KV buckets.
    pub async fn connect(nats_url: &str, daemon_id: &str) -> Result<Self> {
        let client = async_nats::connect(nats_url)
            .await
            .with_context(|| format!("NATS scheduler: failed to connect to {nats_url}"))?;

        let js = async_nats::jetstream::new(client);

        let tasks = js.create_key_value(kv::Config {
            bucket: TASKS_BUCKET.into(),
            history: 5,
            ..Default::default()
        }).await.context("Failed to create swarm-tasks KV bucket")?;

        let leases = js.create_key_value(kv::Config {
            bucket: LEASES_BUCKET.into(),
            max_age: LEASE_TTL,
            ..Default::default()
        }).await.context("Failed to create swarm-leases KV bucket")?;

        let resources = js.create_key_value(kv::Config {
            bucket: RESOURCES_BUCKET.into(),
            max_age: RESOURCE_TTL,
            ..Default::default()
        }).await.context("Failed to create swarm-resources KV bucket")?;

        info!(daemon_id, "NATS scheduler connected");
        Ok(Self { tasks, leases, resources, daemon_id: daemon_id.into() })
    }

    /// Submit a new task to the distributed queue.
    pub async fn submit_task(&self, entry: &TaskEntry) -> Result<()> {
        let key = format!("task.{}", sanitize_key(&entry.run_id));
        let value = serde_json::to_vec(entry)?;
        self.tasks.put(&key, value.into()).await
            .context("Failed to submit task to KV")?;
        info!(run_id = %entry.run_id, "Task submitted to NATS KV");
        Ok(())
    }

    /// Try to claim a task atomically. Returns Ok(true) if claimed, Ok(false) if already taken.
    pub async fn try_claim(&self, run_id: &str) -> Result<bool> {
        let key = format!("lease.{}", sanitize_key(run_id));
        let lease = LeaseEntry {
            daemon_id: self.daemon_id.clone(),
            run_id: run_id.into(),
            claimed_at: chrono::Utc::now().to_rfc3339(),
        };
        let value = serde_json::to_vec(&lease)?;

        // create() fails if key already exists — atomic claiming
        match self.leases.create(&key, value.into()).await {
            Ok(_) => {
                info!(run_id, daemon = %self.daemon_id, "Task claimed via NATS KV");
                Ok(true)
            }
            Err(_) => {
                debug!(run_id, "Task already claimed by another daemon");
                Ok(false)
            }
        }
    }

    /// Release a lease (task completed or failed).
    pub async fn release_lease(&self, run_id: &str) -> Result<()> {
        let key = format!("lease.{}", sanitize_key(run_id));
        self.leases.purge(&key).await
            .context("Failed to release lease")?;
        Ok(())
    }

    /// Renew a lease (heartbeat — prevents expiry during long tasks).
    pub async fn renew_lease(&self, run_id: &str) -> Result<()> {
        let key = format!("lease.{}", sanitize_key(run_id));
        let lease = LeaseEntry {
            daemon_id: self.daemon_id.clone(),
            run_id: run_id.into(),
            claimed_at: chrono::Utc::now().to_rfc3339(),
        };
        let value = serde_json::to_vec(&lease)?;
        self.leases.put(&key, value.into()).await
            .context("Failed to renew lease")?;
        Ok(())
    }

    /// Publish this daemon's resource snapshot.
    pub async fn publish_resources(&self, res: &HostResources) -> Result<()> {
        let key = format!("host.{}", sanitize_key(&self.daemon_id));
        let value = serde_json::to_vec(res)?;
        self.resources.put(&key, value.into()).await
            .context("Failed to publish resources")?;
        Ok(())
    }

    /// Get all available daemons and their resources.
    pub async fn list_available_hosts(&self) -> Result<Vec<HostResources>> {
        use futures::StreamExt;
        let mut hosts = Vec::new();
        let mut keys = self.resources.keys().await.context("Failed to list resource keys")?;

        while let Some(key_result) = keys.next().await {
            let Ok(key) = key_result else { continue };
            if let Ok(Some(value)) = self.resources.get(&key).await {
                if let Ok(host) = serde_json::from_slice::<HostResources>(&value) {
                    hosts.push(host);
                }
            }
        }
        Ok(hosts)
    }

    /// Watch for new tasks. Returns a stream of task entries.
    pub async fn watch_tasks(&self) -> Result<impl futures::Stream<Item = TaskEntry>> {
        use futures::StreamExt;
        let watcher = self.tasks.watch_all().await
            .context("Failed to watch tasks")?;

        Ok(watcher.filter_map(|entry| async {
            let entry = entry.ok()?;
            if entry.operation != kv::Operation::Put { return None; }
            serde_json::from_slice::<TaskEntry>(&entry.value).ok()
        }))
    }

    pub fn daemon_id(&self) -> &str {
        &self.daemon_id
    }
}

/// Sanitize a key for NATS KV — replace colons with dots, remove invalid chars.
fn sanitize_key(s: &str) -> String {
    s.chars().map(|c| match c {
        ':' => '.',
        c if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' => c,
        _ => '_',
    }).collect()
}
