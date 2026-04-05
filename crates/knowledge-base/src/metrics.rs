use serde::{Deserialize, Serialize};

/// Aggregated metrics for a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetrics {
    pub total_runs: u64,
    pub passed: u64,
    pub failed: u64,
    pub skipped: u64,
    pub pass_rate: f64,
    pub total_tokens_input: u64,
    pub total_tokens_output: u64,
    pub avg_duration_ms: u64,
    pub models_used: Vec<ModelMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMetrics {
    pub model: String,
    pub runs: u64,
    pub passed: u64,
    pub failed: u64,
    pub pass_rate: f64,
    pub avg_tokens_output: u64,
    pub avg_duration_ms: u64,
}

impl ProjectMetrics {
    pub fn from_runs(runs: &[crate::AgentRun]) -> Self {
        let total_runs = runs.len() as u64;
        let passed = runs.iter().filter(|r| r.status == crate::RunStatus::Passed).count() as u64;
        let failed = runs.iter().filter(|r| r.status == crate::RunStatus::Failed).count() as u64;
        let skipped = runs.iter().filter(|r| r.status == crate::RunStatus::Skipped).count() as u64;
        let pass_rate = if total_runs > 0 { passed as f64 / total_runs as f64 } else { 0.0 };

        let total_tokens_input: u64 = runs.iter().map(|r| r.tokens_input as u64).sum();
        let total_tokens_output: u64 = runs.iter().map(|r| r.tokens_output as u64).sum();
        let total_duration: u64 = runs.iter().map(|r| r.duration_ms).sum();
        let avg_duration_ms = if total_runs > 0 { total_duration / total_runs } else { 0 };

        // Per-model breakdown
        let mut model_map: std::collections::HashMap<String, Vec<&crate::AgentRun>> = std::collections::HashMap::new();
        for run in runs {
            if !run.model_used.is_empty() {
                model_map.entry(run.model_used.clone()).or_default().push(run);
            }
        }

        let mut models_used: Vec<ModelMetrics> = model_map.into_iter().map(|(model, model_runs)| {
            let runs_count = model_runs.len() as u64;
            let model_passed = model_runs.iter().filter(|r| r.status == crate::RunStatus::Passed).count() as u64;
            let model_failed = model_runs.iter().filter(|r| r.status == crate::RunStatus::Failed).count() as u64;
            let tokens_out: u64 = model_runs.iter().map(|r| r.tokens_output as u64).sum();
            let dur: u64 = model_runs.iter().map(|r| r.duration_ms).sum();

            ModelMetrics {
                model,
                runs: runs_count,
                passed: model_passed,
                failed: model_failed,
                pass_rate: if runs_count > 0 { model_passed as f64 / runs_count as f64 } else { 0.0 },
                avg_tokens_output: if runs_count > 0 { tokens_out / runs_count } else { 0 },
                avg_duration_ms: if runs_count > 0 { dur / runs_count } else { 0 },
            }
        }).collect();

        models_used.sort_by(|a, b| b.runs.cmp(&a.runs));

        ProjectMetrics {
            total_runs,
            passed,
            failed,
            skipped,
            pass_rate,
            total_tokens_input,
            total_tokens_output,
            avg_duration_ms,
            models_used,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentRun, RunStatus};

    fn make_run(model: &str, status: RunStatus, tokens_in: u32, tokens_out: u32, duration: u64) -> AgentRun {
        let mut run = AgentRun::new("test", "task", "agent-1", model);
        run.status = status;
        run.tokens_input = tokens_in;
        run.tokens_output = tokens_out;
        run.duration_ms = duration;
        run
    }

    #[test]
    fn empty_runs() {
        let m = ProjectMetrics::from_runs(&[]);
        assert_eq!(m.total_runs, 0);
        assert_eq!(m.pass_rate, 0.0);
        assert_eq!(m.avg_duration_ms, 0);
        assert!(m.models_used.is_empty());
    }

    #[test]
    fn counts_and_pass_rate() {
        let runs = vec![
            make_run("model-a", RunStatus::Passed, 100, 50, 1000),
            make_run("model-a", RunStatus::Passed, 100, 50, 1000),
            make_run("model-a", RunStatus::Failed, 100, 50, 1000),
            make_run("model-b", RunStatus::Passed, 200, 100, 2000),
            make_run("model-b", RunStatus::Skipped, 0, 0, 0),
        ];
        let m = ProjectMetrics::from_runs(&runs);
        assert_eq!(m.total_runs, 5);
        assert_eq!(m.passed, 3);
        assert_eq!(m.failed, 1);
        assert_eq!(m.skipped, 1);
        assert!((m.pass_rate - 0.6).abs() < 0.01);
    }

    #[test]
    fn token_sums() {
        let runs = vec![
            make_run("m", RunStatus::Passed, 100, 50, 1000),
            make_run("m", RunStatus::Passed, 200, 100, 2000),
        ];
        let m = ProjectMetrics::from_runs(&runs);
        assert_eq!(m.total_tokens_input, 300);
        assert_eq!(m.total_tokens_output, 150);
        assert_eq!(m.avg_duration_ms, 1500);
    }

    #[test]
    fn per_model_breakdown() {
        let runs = vec![
            make_run("model-a", RunStatus::Passed, 100, 50, 1000),
            make_run("model-a", RunStatus::Failed, 100, 50, 2000),
            make_run("model-b", RunStatus::Passed, 200, 100, 3000),
        ];
        let m = ProjectMetrics::from_runs(&runs);
        assert_eq!(m.models_used.len(), 2);
        // Sorted by run count descending
        assert_eq!(m.models_used[0].model, "model-a");
        assert_eq!(m.models_used[0].runs, 2);
        assert_eq!(m.models_used[0].passed, 1);
        assert_eq!(m.models_used[1].model, "model-b");
        assert_eq!(m.models_used[1].runs, 1);
    }
}
