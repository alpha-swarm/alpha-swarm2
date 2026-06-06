# Add a Rust doc comment to the PER_RUN_RAM_PERCENT constant in crates/agent-daemon/src/resources.rs

- model: unknown
- files: crates/agent-daemon/src/resources.rs
- when: 2026-06-05T14:47:49.838715+00:00

## Agent output

```

```

## Diff

```diff
--- a/crates/agent-daemon/src/resources.rs
+++ b/crates/agent-daemon/src/resources.rs
@@ -121,8 +121,9 @@
     snap
 }
 
-/// Estimated local RAM headroom (% of total) one parallel run+gate consumes.
-/// Used to derive how many runs fit under `max_ram_percent`.
+/// Estimated local RAM headroom (% of total) that a single parallel run and gate consumes.
+/// This constant is used to calculate the number of concurrent runs that can fit within
+/// the configured maximum RAM usage percentage (`max_ram_percent`).
 const PER_RUN_RAM_PERCENT: f64 = 25.0;
 
 /// Calculates the effective number of concurrent run slots based on live RAM headroom.

```
