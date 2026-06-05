use serde::{Deserialize, Serialize};
use sysinfo::System;
use tracing::warn;

use swarm_config::ResourceConfig;

/// This module contains the logic for checking and managing system resources.
/// It provides functions to snapshot resource usage for both local machines and remote Ollama instances,
/// as well as determining if there is enough available capacity to schedule new tasks based on configured limits.
#[derive(Debug, Clone, Serialize)]
pub struct ResourceSnapshot {
    pub host: String,
    pub host_type: String,
    pub cpu_percent: f64,
    pub ram_total_mb: u64,
    pub ram_used_mb: u64,
    pub ram_percent: f64,
    pub disk_total_gb: f64,
    pub disk_free_gb: f64,
    pub disk_percent: f64,
    pub ollama_models: Vec<OllamaModelStatus>,
}

// Remove the duplicate definition

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaModelStatus {
    pub name: String,
    pub size_mb: u64,
    pub expires_at: String,
}

/// Check resources for all configured hosts.
pub async fn check_all_hosts(config: &ResourceConfig) -> Vec<ResourceSnapshot> {
    let mut snapshots = Vec::new();
    for host in &config.hosts {
        match host.host_type.as_str() {
            "local" => snapshots.push(check_local(&host.name)),
            "ollama" => snapshots.push(check_ollama(&host.name, &host.ollama_url).await),
            other => warn!(host = %host.name, host_type = other, "Unknown host type"),
        }
    }
    snapshots
}

/// Check local machine resources via sysinfo.
pub fn check_local(name: &str) -> ResourceSnapshot {
    let mut sys = System::new_all();
    sys.refresh_all();

    let cpu_percent = sys.global_cpu_usage() as f64;
    let ram_total = sys.total_memory();
    let ram_used = sys.used_memory();
    let ram_percent = if ram_total > 0 { (ram_used as f64 / ram_total as f64) * 100.0 } else { 0.0 };

    let disks = sysinfo::Disks::new_with_refreshed_list();
    let (disk_total, disk_free) = disks.list().first()
        .map(|d| (d.total_space() as f64 / 1_073_741_824.0, d.available_space() as f64 / 1_073_741_824.0))
        .unwrap_or((0.0, 0.0));
    let disk_percent = if disk_total > 0.0 { ((disk_total - disk_free) / disk_total) * 100.0 } else { 0.0 };

    ResourceSnapshot {
        host: name.to_string(),
        host_type: "local".to_string(),
        cpu_percent,
        ram_total_mb: ram_total / 1_048_576,
        ram_used_mb: ram_used / 1_048_576,
        ram_percent,
        disk_total_gb: disk_total,
        disk_free_gb: disk_free,
        disk_percent,
        ollama_models: vec![],
    }
}

/// Check remote Ollama instance — query /api/ps for loaded models.
async fn check_ollama(name: &str, ollama_url: &str) -> ResourceSnapshot {
    let mut snap = ResourceSnapshot {
        host: name.to_string(),
        host_type: "ollama".to_string(),
        cpu_percent: 0.0,
        ram_total_mb: 0,
        ram_used_mb: 0,
        ram_percent: 0.0,
        disk_total_gb: 0.0,
        disk_free_gb: 0.0,
        disk_percent: 0.0,
        ollama_models: vec![],
    };

    // Query /api/ps for running models
    let ps_url = format!("{}/api/ps", ollama_url);
    match reqwest::get(&ps_url).await {
        Ok(resp) => {
            if let Ok(body) = resp.json::<serde_json::Value>().await
                && let Some(models) = body.get("models").and_then(|m| m.as_array()) {
                    let mut total_size: u64 = 0;
                    for m in models {
                        let model_name = m.get("name").and_then(|n| n.as_str()).unwrap_or("unknown").to_string();
                        let size = m.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                        let size_mb = size / 1_048_576;
                        total_size += size_mb;
                        let expires = m.get("expires_at").and_then(|e| e.as_str()).unwrap_or("").to_string();
                        snap.ollama_models.push(OllamaModelStatus {
                            name: model_name,
                            size_mb,
                            expires_at: expires,
                        });
                    }
                    // Estimate RAM usage from loaded model sizes
                    snap.ram_used_mb = total_size;
            }
        }
        Err(e) => warn!(host = name, url = %ps_url, "Failed to query Ollama: {e}"),
    }

    // Query /api/tags for available models count
    let tags_url = format!("{}/api/tags", ollama_url);
    if let Ok(resp) = reqwest::get(&tags_url).await
        && let Ok(body) = resp.json::<serde_json::Value>().await
        && let Some(models) = body.get("models").and_then(|m| m.as_array()) {
            snap.disk_total_gb = models.len() as f64; // repurpose as "models available"
    }

    snap
}

/// Estimated local RAM headroom (% of total) that a single parallel run and gate consumes.
/// This constant is used to calculate the number of concurrent runs that can fit within
/// the configured maximum RAM usage percentage (`max_ram_percent`).
const PER_RUN_RAM_PERCENT: f64 = 25.0;

/// Calculates the effective number of concurrent run slots based on live RAM headroom.
///
/// This function adapts the number of possible concurrent runs by considering
/// the available RAM headroom. It calculates how many `PER_RUN_RAM_PERCENT`
/// chunks can fit within the headroom below the configured maximum RAM usage
/// percentage (`max_ram_percent`). The result is clamped between 1 and
/// `max_concurrent_runs`, ensuring that it only ever lowers concurrency under
/// memory pressure, never raises it.
///
/// # Parameters
/// - `config`: A reference to the resource configuration containing settings
///   such as `max_concurrent_runs` and `dynamic_slots`.
///
/// # Returns
/// The effective number of concurrent run slots that can be utilized given
/// the current system's RAM usage.
pub fn effective_slots(config: &ResourceConfig) -> usize {
    let max = config.max_concurrent_runs.max(1);
    if !config.dynamic_slots {
        return max;
    }
    let snap = check_local("local");
    let headroom = (config.max_ram_percent - snap.ram_percent).max(0.0);
    let fit = (headroom / PER_RUN_RAM_PERCENT).floor() as usize;
    let slots = fit.clamp(1, max);
    if slots < max {
        warn!(
            ram = format!("{:.1}%", snap.ram_percent),
            cap = format!("{:.1}%", config.max_ram_percent),
            slots, max, "dynamic admission: lowering concurrent-run slots"
        );
    }
    slots
}

/// Check if LOCAL resources are available to run a new task.
pub fn can_schedule(config: &ResourceConfig) -> bool {
    let snap = check_local("local");

    if snap.cpu_percent > config.max_cpu_percent {
        warn!(cpu = format!("{:.1}%", snap.cpu_percent), limit = format!("{:.1}%", config.max_cpu_percent), "CPU too high");
        return false;
    }
    if snap.ram_percent > config.max_ram_percent {
        warn!(ram = format!("{:.1}%", snap.ram_percent), limit = format!("{:.1}%", config.max_ram_percent), "RAM too high");
        return false;
    }
    const MAX_DISK_PERCENT: f64 = 90.0;
    if snap.disk_percent > MAX_DISK_PERCENT {
        warn!(disk = format!("{:.1}%", snap.disk_percent), limit = format!("{:.1}%", MAX_DISK_PERCENT), "Disk too full");
        return false;
    }
    true
}
