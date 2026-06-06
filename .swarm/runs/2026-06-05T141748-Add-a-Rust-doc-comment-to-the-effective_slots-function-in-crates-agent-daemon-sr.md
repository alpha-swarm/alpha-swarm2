# Add a Rust doc comment to the effective_slots function in crates/agent-daemon/src/resources.rs explaining how it adapts run slots to live RAM headroom.

- model: qwen2.5-coder:14b
- files: crates/agent-daemon/src/resources.rs
- when: 2026-06-05T14:17:48.939825+00:00

## Agent output

```
<<<EDIT crates/agent-daemon/src/resources.rs
--- OLD
/// Effective concurrent-run cap from LIVE local resources, bounded by
/// `max_concurrent_runs` (never above it). Opt-in via `dynamic_slots`; off →
/// the static cap. Counts how many `PER_RUN_RAM_PERCENT` chunks fit in the
/// headroom below `max_ram_percent`, clamped to `[1, max_concurrent_runs]` — so
/// it only ever LOWERS concurrency under memory pressure, never raises it.
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
--- NEW
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
>>>
```

## Diff

```diff
<<<EDIT crates/agent-daemon/src/resources.rs
--- OLD
/// Effective concurrent-run cap from LIVE local resources, bounded by
/// `max_concurrent_runs` (never above it). Opt-in via `dynamic_slots`; off →
/// the static cap. Counts how many `PER_RUN_RAM_PERCENT` chunks fit in the
/// headroom below `max_ram_percent`, clamped to `[1, max_concurrent_runs]` — so
/// it only ever LOWERS concurrency under memory pressure, never raises it.
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
--- NEW
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
>>>
```
