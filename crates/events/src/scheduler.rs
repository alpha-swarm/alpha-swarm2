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
/// The bucket name for storing resource snapshots.
const RESOURCES_BUCKET: &str = "swarm-resources";
/// Key for the global execution lock (only 1 goal runs across all daemons).
const EXECUTION_LOCK_KEY: &str = "goal-execution-lock";
/// Execution lock TTL — auto-releases if daemon dies.
#[allow(dead_code)]
const EXECUTION_LOCK_TTL: Duration = Duration::from_secs(900); // 15 minutes

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
    /// Publishes the resources of a host to the KV store.
///
/// # Arguments
///
/// * `res` - A reference to the `HostResources` struct containing the resource information.
///
/// # Returns
///
/// A `Result` indicating success or failure.
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
            if let Ok(Some(value)) = self.resources.get(&key).await
                && let Ok(host) = serde_json::from_slice::<HostResources>(&value) {
                    hosts.push(host);
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

    /// Try to acquire per-project execution lock. Different projects can run in parallel.
    pub async fn try_acquire_execution_lock(&self, run_id: &str) -> Result<bool> {
        self.try_acquire_project_lock(EXECUTION_LOCK_KEY, run_id).await
    }

    /// Per-project lock — allows parallel execution across projects.
    pub async fn try_acquire_project_lock(&self, project: &str, run_id: &str) -> Result<bool> {
        let key = format!("{}.{}", EXECUTION_LOCK_KEY, sanitize_key(project));
        let value = serde_json::to_vec(&serde_json::json!({
            "daemon_id": self.daemon_id,
            "run_id": run_id,
            "acquired_at": chrono::Utc::now().to_rfc3339(),
        }))?;

        match self.leases.create(&key, value.into()).await {
            Ok(_) => {
                info!(run_id, project, daemon = %self.daemon_id, "Acquired execution lock");
                Ok(true)
            }
            Err(_) => {
                debug!(run_id, project, "Execution lock held");
                Ok(false)
            }
        }
    }

    /// Release execution lock (global or per-project).
    pub async fn release_execution_lock(&self) -> Result<()> {
        self.release_project_lock(EXECUTION_LOCK_KEY).await
    }

    pub async fn release_project_lock(&self, project: &str) -> Result<()> {
        let key = format!("{}.{}", EXECUTION_LOCK_KEY, sanitize_key(project));
        self.leases.purge(&key).await.context("Failed to release lock")?;
        info!(project, daemon = %self.daemon_id, "Released execution lock");
        Ok(())
    }

    /// Renew execution lock.
    pub async fn renew_execution_lock(&self, run_id: &str) -> Result<()> {
        self.renew_project_lock(EXECUTION_LOCK_KEY, run_id).await
    }

    pub async fn renew_project_lock(&self, project: &str, run_id: &str) -> Result<()> {
        let key = format!("{}.{}", EXECUTION_LOCK_KEY, sanitize_key(project));
        let value = serde_json::to_vec(&serde_json::json!({
            "daemon_id": self.daemon_id, "run_id": run_id,
            "acquired_at": chrono::Utc::now().to_rfc3339(),
        }))?;
        self.leases.put(&key, value.into()).await.context("Failed to renew lock")?;
        Ok(())
    }

    /// Check if execution lock is free (backward compat — checks global key).
    pub async fn is_execution_lock_free(&self) -> bool {
        let key = format!("{}.{}", EXECUTION_LOCK_KEY, sanitize_key(EXECUTION_LOCK_KEY));
        matches!(self.leases.get(&key).await, Ok(None) | Err(_))
    }

    /// Acquire any free execution slot in `[0, slots)`, returning its index.
    /// Slots are independent KV keys with atomic `create()`, so two pollers (or
    /// daemons) never grab the same slot — the cap on concurrent executions.
    /// Backed by the leases bucket (TTL), so a dead daemon's slot auto-frees.
    /// Attempts to acquire an execution slot for the given run ID.
///
/// # Arguments
///
/// * `run_id` - A unique identifier for the run attempting to acquire a slot.
/// * `slots` - The number of slots requested.
///
/// # Returns
///
/// * `Ok(Some(slot))` if a slot is successfully acquired, with the slot index.
/// * `Ok(None)` if no slots are available.
/// * `Err(e)` if an error occurs during the acquisition process.
pub async fn try_acquire_execution_slot(&self, run_id: &str, slots: usize) -> Result<Option<usize>> {
        for i in 0..slots.max(1) {
            let key = format!("{EXECUTION_LOCK_KEY}.slot-{i}");
            let value = serde_json::to_vec(&serde_json::json!({
                "daemon_id": self.daemon_id, "run_id": run_id,
                "acquired_at": chrono::Utc::now().to_rfc3339(),
            }))?;
            if self.leases.create(&key, value.into()).await.is_ok() {
                info!(run_id, slot = i, daemon = %self.daemon_id, "Acquired execution slot");
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    /// Release a previously-acquired execution slot.
    pub async fn release_execution_slot(&self, slot: usize) -> Result<()> {
        let key = format!("{EXECUTION_LOCK_KEY}.slot-{slot}");
        let _ = self.leases.purge(&key).await;
        debug!(slot, daemon = %self.daemon_id, "Released execution slot");
        Ok(())
    }

    /// Renew an execution slot (heartbeat so it doesn't TTL-expire mid-run).
    pub async fn renew_execution_slot(&self, slot: usize, run_id: &str) -> Result<()> {
        let key = format!("{EXECUTION_LOCK_KEY}.slot-{slot}");
        let value = serde_json::to_vec(&serde_json::json!({
            "daemon_id": self.daemon_id, "run_id": run_id,
            "acquired_at": chrono::Utc::now().to_rfc3339(),
        }))?;
        self.leases.put(&key, value.into()).await.context("Failed to renew execution slot")?;
        Ok(())
    }

    pub fn daemon_id(&self) -> &str {
        &self.daemon_id
    }
}

/// Sanitize a key for NATS KV — replace colons with dots, remove invalid chars.
/// Makes a string safe as a NATS KV key.
///
/// This function ensures that the input string can be used as a valid key in a NATS KeyValue (KV)
/// store by replacing any characters that are not allowed in KV keys with underscores (`_`).
fn sanitize_key(s: &str) -> String {
    s.chars().map(|c| match c {
        ':' => '.',
        c if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' => c,
        _ => '_',
    }).collect()
}
