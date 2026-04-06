use sysinfo::System;
use tracing::{info, warn};

use swarm_config::ResourceConfig;

/// Snapshot of current system resource usage.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResourceSnapshot {
    pub cpu_percent: f64,
    pub ram_total_mb: u64,
    pub ram_used_mb: u64,
    pub ram_percent: f64,
    pub disk_total_gb: f64,
    pub disk_free_gb: f64,
    pub disk_percent: f64,
}

/// Check current resource usage. Returns a snapshot.
pub fn check_resources() -> ResourceSnapshot {
    let mut sys = System::new_all();
    sys.refresh_all();

    let cpu_percent = sys.global_cpu_usage() as f64;
    let ram_total = sys.total_memory();
    let ram_used = sys.used_memory();
    let ram_percent = if ram_total > 0 { (ram_used as f64 / ram_total as f64) * 100.0 } else { 0.0 };

    // Disk usage for root partition
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let (disk_total, disk_free) = disks.list().first()
        .map(|d| (d.total_space() as f64 / 1_073_741_824.0, d.available_space() as f64 / 1_073_741_824.0))
        .unwrap_or((0.0, 0.0));
    let disk_percent = if disk_total > 0.0 { ((disk_total - disk_free) / disk_total) * 100.0 } else { 0.0 };

    ResourceSnapshot {
        cpu_percent,
        ram_total_mb: ram_total / 1_048_576,
        ram_used_mb: ram_used / 1_048_576,
        ram_percent,
        disk_total_gb: disk_total,
        disk_free_gb: disk_free,
        disk_percent,
    }
}

/// Check if resources are available to run a new task.
pub fn can_schedule(config: &ResourceConfig) -> bool {
    let snap = check_resources();

    if snap.cpu_percent > config.max_cpu_percent {
        warn!(
            cpu = format!("{:.1}%", snap.cpu_percent),
            limit = format!("{:.1}%", config.max_cpu_percent),
            "CPU too high, deferring task"
        );
        return false;
    }

    if snap.ram_percent > config.max_ram_percent {
        warn!(
            ram = format!("{:.1}%", snap.ram_percent),
            limit = format!("{:.1}%", config.max_ram_percent),
            "RAM too high, deferring task"
        );
        return false;
    }

    info!(
        cpu = format!("{:.1}%", snap.cpu_percent),
        ram = format!("{:.1}% ({}/{}MB)", snap.ram_percent, snap.ram_used_mb, snap.ram_total_mb),
        disk = format!("{:.1}GB free", snap.disk_free_gb),
        "Resources OK"
    );
    true
}
