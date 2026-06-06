# Refactor crates/agent-daemon/src/resources.rs: extract the per-run RAM headroom slot calculation inside effective_slots into a small pure helper fn for testability, keeping behavior identical

- model: unknown
- files: crates/agent-daemon/src/resources.rs
- when: 2026-06-05T22:44:15.594369+00:00

## Agent output

```

```

## Diff

```diff
--- a/crates/agent-daemon/src/resources.rs
+++ b/crates/agent-daemon/src/resources.rs
@@ -130,6 +130,10 @@
 /// the configured maximum RAM usage percentage (`max_ram_percent`).
 const PER_RUN_RAM_PERCENT: f64 = 25.0;
 
+fn calculate_per_run_headroom_slot(headroom: f64) -> usize {
+    ((headroom / PER_RUN_RAM_PERCENT).floor() as usize).clamp(1, usize::MAX)
+}
+
 /// Calculates the effective number of concurrent run slots based on live RAM headroom.
 ///
 /// This function adapts the number of possible concurrent runs by considering
@@ -153,8 +157,7 @@
     }
     let snap = check_local("local");
     let headroom = (config.max_ram_percent - snap.ram_percent).max(0.0);
-    let fit = (headroom / PER_RUN_RAM_PERCENT).floor() as usize;
-    let slots = fit.clamp(1, max);
+    let slots = calculate_per_run_headroom_slot(headroom).clamp(1, max);
     if slots < max {
         warn!(
             ram = format!("{:.1}%", snap.ram_percent),

```
