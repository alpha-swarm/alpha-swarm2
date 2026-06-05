//! Pre-land security scan — the final quality-gate tier.
//!
//! Runs INSIDE `run_quality_gate`, after `cargo check -p` + `cargo test -p`
//! already passed, so it only ever sees compiling, test-passing diffs and can
//! only raise the bar (return Err) — never let something through that the cargo
//! tiers rejected. A blocking finding fails the run exactly like a test failure:
//! the diff never lands and never feeds distillation (gate_passed = false).
//!
//! Scans ONLY lines new to each changed file (diff vs the gate worktree's HEAD)
//! — scanning whole files would flag pre-existing `unsafe`/secrets and punish a
//! run for sins it didn't commit. Deterministic rule pass; no network, no LLM.

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use tracing::warn;

use swarm_config::SecurityConfig;

const RULE_ID_SECRET: &str = "SEC001-hardcoded-secret";
const RULE_ID_UNSAFE: &str = "SEC002-new-unsafe-block";
const RULE_ID_CMD_INJECT: &str = "SEC003-command-injection";
const RULE_ID_PATH_TRAVERSAL: &str = "SEC004-path-traversal";
const RULE_ID_NET_BIND: &str = "SEC005-unrestricted-bind";

/// Max chars of a flagged line echoed into the report.
const SCAN_SNIPPET_MAX_CHARS: usize = 160;
/// Cap total findings so a pathological diff can't build a huge report.
const SCAN_FINDINGS_MAX: usize = 40;
/// Min RHS string length to treat a `key = "..."` assignment as a real secret.
const SCAN_MIN_SECRET_LEN: usize = 20;
/// File extensions worth scanning (source + config + scripts).
const SCAN_SCANNABLE_EXTS: &[&str] = &["rs", "ts", "js", "go", "py", "sh", "toml", "yaml", "yml", "env"];
/// High-confidence secret markers (provider key prefixes, PEM, bearer header).
const SECRET_PREFIXES: &[&str] = &["AKIA", "ghp_", "gho_", "github_pat_", "-----BEGIN ", "Authorization: Bearer "];
/// Substrings that mark a line as an example/placeholder (skip for secret rules).
const PLACEHOLDER_MARKERS: &[&str] = &["example", "dummy", "placeholder", "your_", "changeme", "redacted", "xxxxx", "env::var", "getenv"];
/// Identifier substrings that make a string assignment "secret-shaped".
const SECRET_KEYWORDS: &[&str] = &["key", "token", "secret", "password", "passwd", "apikey", "api_key", "credential"];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

/// Parse the configured threshold; unknown → High (fail-safe-strict).
fn parse_severity(s: &str) -> Severity {
    match s.trim().to_lowercase().as_str() {
        "low" => Severity::Low,
        "medium" => Severity::Medium,
        "critical" => Severity::Critical,
        _ => Severity::High,
    }
}

struct Finding {
    rule_id: &'static str,
    severity: Severity,
    file: String,
    line: usize,
    snippet: String,
}

/// Scan a run's changed files for risky NEW lines. Returns `Err(report)` if any
/// finding meets/exceeds `sec.fail_severity`; below-threshold findings only log.
pub fn scan(files: &[(String, Vec<u8>)], gate_worktree: &Path, sec: &SecurityConfig) -> Result<(), String> {
    let threshold = parse_severity(&sec.fail_severity);
    let mut findings: Vec<Finding> = Vec::new();

    'files: for (path, new_bytes) in files {
        let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("");
        if !SCAN_SCANNABLE_EXTS.contains(&ext) {
            continue;
        }
        let Ok(new_text) = std::str::from_utf8(new_bytes) else { continue };
        // Lines already in HEAD are pre-existing — only scan what this run added.
        let old_text = head_version(gate_worktree, path);
        let old_lines: HashSet<&str> = old_text.lines().map(str::trim).collect();
        for (idx, raw) in new_text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || old_lines.contains(line) {
                continue;
            }
            if let Some((rule_id, severity)) = classify(line, ext) {
                findings.push(Finding {
                    rule_id,
                    severity,
                    file: path.clone(),
                    line: idx + 1,
                    snippet: line.chars().take(SCAN_SNIPPET_MAX_CHARS).collect(),
                });
                if findings.len() >= SCAN_FINDINGS_MAX {
                    break 'files;
                }
            }
        }
    }

    let blocking: Vec<&Finding> = findings.iter().filter(|f| f.severity >= threshold).collect();
    if blocking.is_empty() {
        for f in &findings {
            warn!(rule = f.rule_id, file = %f.file, line = f.line, "security scan: below-threshold finding");
        }
        return Ok(());
    }
    let report = blocking
        .iter()
        .map(|f| format!("{} @ {}:{}: {}", f.rule_id, f.file, f.line, f.snippet))
        .collect::<Vec<_>>()
        .join("; ");
    Err(format!("security scan: {} blocking finding(s): {}", blocking.len(), report))
}

/// The committed (HEAD) content of `path` in the gate worktree, or empty if the
/// file is new / unreadable (then every line counts as added).
fn head_version(worktree: &Path, path: &str) -> String {
    Command::new("git")
        .args(["show", &format!("HEAD:{path}")])
        .current_dir(worktree)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Classify a single added line. Returns the first matching rule, or None.
fn classify(line: &str, ext: &str) -> Option<(&'static str, Severity)> {
    let lower = line.to_lowercase();
    let is_placeholder = PLACEHOLDER_MARKERS.iter().any(|m| lower.contains(m));

    // SEC001 hardcoded secret.
    if !is_placeholder {
        if SECRET_PREFIXES.iter().any(|p| line.contains(p)) {
            return Some((RULE_ID_SECRET, Severity::High));
        }
        if let Some(val) = assigned_string_value(line) {
            if val.len() >= SCAN_MIN_SECRET_LEN && SECRET_KEYWORDS.iter().any(|k| lower.contains(k)) {
                return Some((RULE_ID_SECRET, Severity::High));
            }
        }
    }

    // SEC002 new unsafe block (Rust only).
    if ext == "rs" && (line.contains("unsafe {") || line.contains("unsafe fn")) {
        return Some((RULE_ID_UNSAFE, Severity::High));
    }

    // SEC003 command injection — spawn/exec built from a non-literal (interpolated).
    let spawns = line.contains("Command::new(") || line.contains(".arg(") || lower.contains("exec(") || lower.contains("system(");
    if spawns && (line.contains("format!") || line.contains("{}") || line.contains(" + ")) {
        return Some((RULE_ID_CMD_INJECT, Severity::Medium));
    }

    // SEC004 path traversal — a `..` path built via interpolation.
    if lower.contains("..") && (line.contains(".join(") || lower.contains("fs::")) && line.contains("format!") {
        return Some((RULE_ID_PATH_TRAVERSAL, Severity::Medium));
    }

    // SEC005 unrestricted network bind.
    if line.contains("0.0.0.0") && (lower.contains("bind") || lower.contains("listen")) {
        return Some((RULE_ID_NET_BIND, Severity::Medium));
    }

    None
}

/// Best-effort RHS of a `... = "value"` assignment.
fn assigned_string_value(line: &str) -> Option<&str> {
    let after_eq = &line[line.find('=')?  + 1..];
    let open = after_eq.find('"')? + 1;
    let rest = &after_eq[open..];
    let close = rest.find('"')?;
    Some(&rest[..close])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(sev: &str) -> SecurityConfig {
        SecurityConfig { rules_enabled: true, fail_severity: sev.into() }
    }

    #[test]
    fn flags_added_secret_only() {
        // empty worktree → head_version returns "" → every line is "added"; the
        // git call fails harmlessly in tests.
        let files = vec![("crates/x/src/a.rs".to_string(), b"let api_key = \"AKIA1234567890ABCDEF\";\n".to_vec())];
        let r = scan(&files, Path::new("/nonexistent"), &cfg("high"));
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("SEC001"));
    }

    #[test]
    fn ignores_placeholder_secret() {
        let files = vec![("a.rs".to_string(), b"let key = \"your_api_key_example_here\";\n".to_vec())];
        assert!(scan(&files, Path::new("/nonexistent"), &cfg("high")).is_ok());
    }

    #[test]
    fn new_unsafe_is_high() {
        let files = vec![("a.rs".to_string(), b"unsafe { ptr.write(0); }\n".to_vec())];
        assert!(scan(&files, Path::new("/nonexistent"), &cfg("high")).is_err());
    }

    #[test]
    fn threshold_lets_medium_pass_at_high() {
        // command-injection is Medium; at fail_severity=high it should pass.
        let files = vec![("a.rs".to_string(), b"Command::new(format!(\"{}\", x));\n".to_vec())];
        assert!(scan(&files, Path::new("/nonexistent"), &cfg("high")).is_ok());
        assert!(scan(&files, Path::new("/nonexistent"), &cfg("medium")).is_err());
    }

    #[test]
    fn benign_line_passes() {
        let files = vec![("a.rs".to_string(), b"let total = a + b;\n".to_vec())];
        assert!(scan(&files, Path::new("/nonexistent"), &cfg("high")).is_ok());
    }
}
