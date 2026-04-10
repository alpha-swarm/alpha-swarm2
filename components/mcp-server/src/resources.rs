//! MCP Resources — read-only data endpoints for swarm state.

use serde_json::{json, Value};

// --- Resource URI scheme ---

const URI_PREFIX: &str = "swarm://";

/// Return the resource definitions (MCP resources/list response).
pub fn list_resources() -> Vec<Value> {
    vec![
        resource_def("projects", "Projects", "All registered projects with repo URLs and status."),
        resource_def("models", "Available Models", "Ollama models available for agent tasks."),
        resource_def("resources", "System Resources", "CPU, RAM, disk utilization per host."),
        resource_def("health", "Health Status", "System health check."),
        // Dynamic resources (use with project name / run ID):
        resource_def("projects/{project}/runs", "Project Runs", "Agent runs for a project (replace {project})."),
        resource_def("projects/{project}/goals", "Project Goals", "Goal summaries grouped by task description."),
        resource_def("projects/{project}/metrics", "Project Metrics", "Aggregated pass rate, tokens, duration."),
        resource_def("runs/{id}", "Run Detail", "Full detail of a single run (replace {id})."),
        resource_def("runs/{id}/sub-runs", "Sub-Runs", "Child agent runs under a parent run."),
        resource_def("runs/{id}/plans", "Plans", "Plan versions for a run."),
        resource_def("runs/{id}/timeline", "Run Timeline", "Ordered tool calls + phase transitions for a run."),
        resource_def("live", "Live Agents", "Currently running agents with progress messages."),
        resource_def("dashboard", "Dashboard", "Global aggregate stats across all projects."),
    ]
}

fn resource_def(path: &str, name: &str, description: &str) -> Value {
    json!({
        "uri": format!("{URI_PREFIX}{path}"),
        "name": name,
        "description": description,
        "mimeType": "application/json",
    })
}

/// Read a resource by URI.
pub fn read_resource(uri: &str) -> Result<Value, String> {
    let path = uri.strip_prefix(URI_PREFIX).unwrap_or(uri);
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    match segments.as_slice() {
        ["projects"] => query_resource(uri, "SELECT * FROM project ORDER BY name"),
        ["projects", project, "runs"] => query_resource(
            uri,
            &project_query(project, "SELECT id, task_description, status, model_used, duration_ms, \
                quality_gate_passed, progress_message, phase_timings, created_at \
                FROM agent_run WHERE project = '{project}' ORDER BY created_at DESC LIMIT 20"),
        ),
        ["projects", project, "goals"] => query_resource(
            uri,
            &project_query(project, "SELECT task_description, count() as total, \
                math::sum(IF quality_gate_passed = true THEN 1 ELSE 0 END) as passed \
                FROM agent_run WHERE project = '{project}' AND parent_run_id = NONE \
                GROUP BY task_description ORDER BY total DESC LIMIT 20"),
        ),
        ["projects", project, "metrics"] => query_resource(
            uri,
            &project_query(project, "SELECT count() as total_runs, \
                math::sum(IF status = 'passed' THEN 1 ELSE 0 END) as passed, \
                math::sum(IF status = 'failed' THEN 1 ELSE 0 END) as failed, \
                math::sum(tokens_input) as total_tokens_in, \
                math::sum(tokens_output) as total_tokens_out, \
                math::sum(duration_ms) as total_duration_ms \
                FROM agent_run WHERE project = '{project}'"),
        ),
        ["runs", id] => {
            let ref_ = if id.contains(':') { id.to_string() } else { format!("type::thing('agent_run', '{}')", id.replace('\'', "")) };
            query_resource(uri, &format!("SELECT * FROM {ref_}"))
        }
        ["runs", id, "sub-runs"] => query_resource(
            uri,
            &format!("SELECT id, task_description, status, model_used, duration_ms, progress_message \
                FROM agent_run WHERE parent_run_id = '{}' ORDER BY created_at", id.replace('\'', "")),
        ),
        ["runs", id, "plans"] => query_resource(
            uri,
            &format!("SELECT * FROM goal_plan WHERE run_id = '{}' ORDER BY version DESC", id.replace('\'', "")),
        ),
        ["runs", id, "timeline"] => resource_run_timeline(uri, id),
        ["live"] => resource_live(uri),
        ["dashboard"] => resource_dashboard(uri),
        ["models"] => resource_models(uri),
        ["resources"] => query_resource(uri, "SELECT * FROM resource_snapshot ORDER BY timestamp DESC LIMIT 5"),
        ["health"] => Ok(content_response(uri, r#"{"status":"ok"}"#)),
        _ => Err(format!("Unknown resource URI: {uri}")),
    }
}

// --- Helpers ---

/// Substitute `{project}` placeholder and sanitize.
fn project_query(project: &str, template: &str) -> String {
    template.replace("{project}", &project.replace('\'', ""))
}

/// Run a SurrealDB query and wrap in MCP resource response.
fn query_resource(uri: &str, query: &str) -> Result<Value, String> {
    let result = crate::surreal_query(query)?;
    Ok(content_response(uri, &result))
}

fn content_response(uri: &str, text: &str) -> Value {
    json!({
        "contents": [{ "uri": uri, "mimeType": "application/json", "text": text }]
    })
}

fn resource_models(uri: &str) -> Result<Value, String> {
    let text = crate::http_get(crate::OLLAMA_HOST, "/api/tags")?;
    Ok(content_response(uri, &text))
}

fn resource_run_timeline(uri: &str, id: &str) -> Result<Value, String> {
    let safe_id = id.replace('\'', "");
    let ref_ = if id.contains(':') { id.to_string() } else { format!("type::thing('agent_run', '{safe_id}')") };

    // Fetch the run with tool_calls and attempts
    let query = format!("SELECT tool_calls, attempts, phase_timings, progress_message, status FROM {ref_}");
    let result = crate::surreal_query(&query)?;
    Ok(content_response(uri, &result))
}

fn resource_live(uri: &str) -> Result<Value, String> {
    let result = crate::surreal_query(
        "SELECT id, project, task_description, status, progress_message, last_activity_at \
         FROM agent_run WHERE status = 'running' OR status = 'planning'"
    )?;
    Ok(content_response(uri, &result))
}

fn resource_dashboard(uri: &str) -> Result<Value, String> {
    let result = crate::surreal_query(
        "SELECT \
            count() as total_runs, \
            math::sum(IF status = 'running' THEN 1 ELSE 0 END) as active, \
            math::sum(IF status = 'passed' THEN 1 ELSE 0 END) as passed, \
            math::sum(IF status = 'failed' THEN 1 ELSE 0 END) as failed, \
            math::sum(IF status = 'pending' THEN 1 ELSE 0 END) as pending, \
            math::sum(tokens_input) as total_tokens_in, \
            math::sum(tokens_output) as total_tokens_out, \
            math::sum(duration_ms) as total_duration_ms \
         FROM agent_run"
    )?;
    Ok(content_response(uri, &result))
}
