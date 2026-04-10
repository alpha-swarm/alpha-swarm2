//! MCP Tools — callable actions that control the swarm.

use serde_json::{json, Value};

// --- Tool definitions ---

/// Build a tool definition with required + optional properties.
fn tool_def(name: &str, description: &str, required: &[(&str, &str, &str)], optional: &[(&str, &str, &str)]) -> Value {
    let mut properties = json!({});
    let required_names: Vec<&str> = required.iter().map(|(n, _, _)| *n).collect();

    for (name, typ, desc) in required.iter().chain(optional.iter()) {
        properties[name] = json!({ "type": typ, "description": desc });
    }

    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required_names,
        }
    })
}

pub fn list_tools() -> Vec<Value> {
    vec![
        tool_def("submit_task",
            "Submit a new agent task. The swarm will plan and execute it, producing code changes and a PR.",
            &[("project", "string", "Project name (must exist)"),
              ("goal", "string", "What the agents should accomplish")],
            &[]),
        tool_def("create_plan",
            "Generate an execution plan for a goal without executing it. Returns sub-tasks for review.",
            &[("project", "string", "Project name"),
              ("goal", "string", "Goal to decompose into sub-tasks")],
            &[]),
        tool_def("approve_plan",
            "Approve a plan for execution. The swarm will begin running agents.",
            &[("run_id", "string", "The run ID containing the plan")],
            &[]),
        tool_def("plan_feedback",
            "Send feedback on a plan to trigger re-planning with improvements.",
            &[("run_id", "string", "The run ID"),
              ("feedback", "string", "What to change in the plan")],
            &[]),
        tool_def("create_project",
            "Register a new project with its git repository URL.",
            &[("name", "string", "Project name (unique identifier)"),
              ("repo_url", "string", "Git repository URL")],
            &[("description", "string", "Project description")]),
        tool_def("get_run_status",
            "Get the current status of an agent run, including progress, phase timings, and tool calls.",
            &[("run_id", "string", "The run ID to check")],
            &[]),
        tool_def("find_similar_runs",
            "Search for past agent runs in a project.",
            &[("project", "string", "Project name"),
              ("query", "string", "Description to search for")],
            &[("limit", "integer", "Max results (default 5)")]),
        tool_def("get_plans",
            "Get all plan versions for a run, including sub-tasks, feedback, and approval status.",
            &[("run_id", "string", "The run ID")],
            &[]),
        tool_def("edit_plan",
            "Directly edit the sub-tasks of a plan (replace the task list).",
            &[("run_id", "string", "The run ID"),
              ("sub_tasks", "string", "JSON array of sub-tasks: [{\"id\":\"task-1\",\"description\":\"...\",\"files\":[...],\"complexity\":\"simple\"}]")],
            &[]),
        tool_def("delete_project",
            "Remove a project and all its associated data.",
            &[("name", "string", "Project name to delete")],
            &[]),
    ]
}

// --- Tool dispatch ---

pub fn call_tool(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "submit_task" => tool_submit_task(args),
        "create_plan" => tool_create_plan(args),
        "approve_plan" => tool_approve_plan(args),
        "plan_feedback" => tool_plan_feedback(args),
        "create_project" => tool_create_project(args),
        "get_run_status" => tool_get_run_status(args),
        "find_similar_runs" => tool_find_similar_runs(args),
        "get_plans" => tool_get_plans(args),
        "edit_plan" => tool_edit_plan(args),
        "delete_project" => tool_delete_project(args),
        _ => Err(format!("Unknown tool: {name}")),
    }
}

// --- Helpers ---

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key).and_then(|v| v.as_str())
        .ok_or_else(|| format!("Missing required argument: {key}"))
}

/// Sanitize a string for safe SurrealQL interpolation.
fn sanitize(s: &str) -> String {
    s.replace('\'', "").replace('\\', "").replace('\n', " ")
}

fn text_result(text: impl Into<String>) -> Result<Value, String> {
    Ok(json!({ "content": [{ "type": "text", "text": text.into() }] }))
}

/// Build a SurrealQL reference for a run ID (handles both `agent_run:xxx` and bare `xxx`).
fn run_ref(run_id: &str) -> String {
    if run_id.contains(':') {
        run_id.to_string()
    } else {
        format!("type::thing('agent_run', '{}')", sanitize(run_id))
    }
}

fn create_run(project: &str, goal: &str, status: &str, agent_id: &str) -> Result<String, String> {
    let query = format!(
        "CREATE agent_run SET project = '{project}', task_description = '{goal}', \
         status = '{status}', agent_id = '{agent_id}', model_used = 'auto', \
         created_at = time::now(), files_modified = [], tokens_input = 0, \
         tokens_output = 0, duration_ms = 0",
        project = sanitize(project),
        goal = sanitize(goal),
    );
    let result = crate::surreal_query(&query)?;
    let parsed: Value = serde_json::from_str(&result).unwrap_or(json!([]));

    let id = parsed.get(0)
        .and_then(|r| r.get("result"))
        .and_then(|r| r.get(0))
        .and_then(|r| r.get("id"))
        .map(|v| v.to_string())
        .unwrap_or_else(|| "unknown".into());

    Ok(id)
}

// --- Tool implementations ---

fn tool_submit_task(args: &Value) -> Result<Value, String> {
    let project = require_str(args, "project")?;
    let goal = require_str(args, "goal")?;
    let id = create_run(project, goal, "pending", "mcp")?;
    text_result(format!("Task submitted. Run ID: {id}. The swarm will pick it up shortly."))
}

fn tool_create_plan(args: &Value) -> Result<Value, String> {
    let project = require_str(args, "project")?;
    let goal = require_str(args, "goal")?;
    let id = create_run(project, goal, "planning", "mcp-planner")?;
    text_result(format!("Planning started. Run ID: {id}. Use get_run_status to check, then approve_plan when ready."))
}

fn tool_approve_plan(args: &Value) -> Result<Value, String> {
    let run_id = require_str(args, "run_id")?;
    crate::surreal_query(&format!("UPDATE {} SET status = 'approved'", run_ref(run_id)))?;
    text_result(format!("Plan approved. Agents will begin execution for run {run_id}."))
}

fn tool_plan_feedback(args: &Value) -> Result<Value, String> {
    let run_id = require_str(args, "run_id")?;
    let feedback = require_str(args, "feedback")?;
    let query = format!(
        "UPDATE goal_plan SET user_feedback = '{feedback}', status = 'draft' \
         WHERE run_id = '{run_id}' ORDER BY version DESC LIMIT 1; \
         UPDATE {ref_} SET status = 'planning'",
        feedback = sanitize(feedback),
        run_id = sanitize(run_id),
        ref_ = run_ref(run_id),
    );
    crate::surreal_query(&query)?;
    text_result(format!("Feedback sent. The planner will re-plan for run {run_id}."))
}

fn tool_create_project(args: &Value) -> Result<Value, String> {
    let name = require_str(args, "name")?;
    let repo_url = require_str(args, "repo_url")?;
    let description = args.get("description").and_then(|v| v.as_str()).unwrap_or("");
    let query = format!(
        "CREATE project SET name = '{name}', repo_url = '{url}', description = '{desc}', \
         branch = 'main', status = 'active', created_at = time::now()",
        name = sanitize(name), url = sanitize(repo_url), desc = sanitize(description),
    );
    crate::surreal_query(&query)?;
    text_result(format!("Project '{name}' created with repo {repo_url}."))
}

fn tool_get_run_status(args: &Value) -> Result<Value, String> {
    let run_id = require_str(args, "run_id")?;
    let query = format!("SELECT * FROM {}", run_ref(run_id));
    let result = crate::surreal_query(&query)?;
    let parsed: Value = serde_json::from_str(&result).unwrap_or(json!([]));

    let run = parsed.get(0)
        .and_then(|r| r.get("result"))
        .and_then(|r| r.get(0))
        .cloned()
        .unwrap_or(json!(null));

    if run.is_null() {
        return text_result(format!("Run {run_id} not found."));
    }

    let get = |key: &str| run.get(key).and_then(|v| v.as_str()).unwrap_or("unknown");
    let tool_count = run.get("tool_calls").and_then(|t| t.as_array()).map(|a| a.len()).unwrap_or(0);

    let mut text = format!(
        "Run: {run_id}\nStatus: {}\nProgress: {}\nTool calls: {tool_count}",
        get("status"), get("progress_message"),
    );

    if let Some(err) = run.get("error_message").and_then(|v| v.as_str()) {
        text.push_str(&format!("\nError: {err}"));
    }
    if let Some(pt) = run.get("phase_timings") {
        if !pt.is_null() {
            text.push_str(&format!("\nPhase timings: {pt}"));
        }
    }

    // Sub-tasks
    let sub_query = format!(
        "SELECT id, status, task_description, progress_message FROM agent_run WHERE parent_run_id = '{}'",
        sanitize(run_id),
    );
    if let Ok(sub_result) = crate::surreal_query(&sub_query) {
        if let Ok(sub_parsed) = serde_json::from_str::<Value>(&sub_result) {
            if let Some(subs) = sub_parsed.get(0).and_then(|r| r.get("result")).and_then(|r| r.as_array()) {
                if !subs.is_empty() {
                    text.push_str(&format!("\n\nSub-tasks ({}):", subs.len()));
                    for sub in subs {
                        let sid = sub.get("id").and_then(|s| s.as_str()).unwrap_or("?");
                        let ss = sub.get("status").and_then(|s| s.as_str()).unwrap_or("?");
                        let sd = sub.get("task_description").and_then(|s| s.as_str()).unwrap_or("?");
                        text.push_str(&format!("\n  - {sid} [{ss}]: {}", &sd[..sd.len().min(80)]));
                    }
                }
            }
        }
    }

    text_result(text)
}

fn tool_find_similar_runs(args: &Value) -> Result<Value, String> {
    let project = require_str(args, "project")?;
    let _query_text = require_str(args, "query")?;
    let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(5);

    let query = format!(
        "SELECT id, task_description, status, model_used, duration_ms, quality_gate_passed, \
         error_message, created_at FROM agent_run WHERE project = '{}' \
         ORDER BY created_at DESC LIMIT {limit}",
        sanitize(project),
    );
    let result = crate::surreal_query(&query)?;
    let parsed: Value = serde_json::from_str(&result).unwrap_or(json!([]));

    let runs = parsed.get(0)
        .and_then(|r| r.get("result"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    let mut text = format!("Recent runs for '{project}' ({} results):", runs.len());
    for run in &runs {
        let id = run.get("id").and_then(|s| s.as_str()).unwrap_or("?");
        let desc = run.get("task_description").and_then(|s| s.as_str()).unwrap_or("?");
        let status = run.get("status").and_then(|s| s.as_str()).unwrap_or("?");
        let qg = run.get("quality_gate_passed").and_then(|b| b.as_bool());
        text.push_str(&format!("\n- {id} [{status}] QG:{qg:?}\n  {}", &desc[..desc.len().min(100)]));
    }

    text_result(text)
}

fn tool_get_plans(args: &Value) -> Result<Value, String> {
    let run_id = require_str(args, "run_id")?;
    let query = format!(
        "SELECT * FROM goal_plan WHERE run_id = '{}' ORDER BY version DESC",
        sanitize(run_id),
    );
    let result = crate::surreal_query(&query)?;
    let parsed: Value = serde_json::from_str(&result).unwrap_or(json!([]));

    let plans = parsed.get(0)
        .and_then(|r| r.get("result"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    if plans.is_empty() {
        return text_result(format!("No plans found for run {run_id}."));
    }

    let mut text = format!("Plans for run {run_id} ({} versions):", plans.len());
    for plan in &plans {
        let version = plan.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
        let status = plan.get("status").and_then(|s| s.as_str()).unwrap_or("?");
        let model = plan.get("model_used").and_then(|s| s.as_str()).unwrap_or("?");
        let tasks = plan.get("sub_tasks").and_then(|t| t.as_array()).map(|a| a.len()).unwrap_or(0);
        let feedback = plan.get("user_feedback").and_then(|s| s.as_str());

        text.push_str(&format!("\n\nv{version} [{status}] model={model} tasks={tasks}"));
        if let Some(fb) = feedback {
            text.push_str(&format!("\n  Feedback: {}", &fb[..fb.len().min(100)]));
        }
        if let Some(sub_tasks) = plan.get("sub_tasks").and_then(|t| t.as_array()) {
            for task in sub_tasks {
                let tid = task.get("id").and_then(|s| s.as_str()).unwrap_or("?");
                let desc = task.get("description").and_then(|s| s.as_str()).unwrap_or("?");
                let files = task.get("files").and_then(|f| f.as_array()).map(|a| a.len()).unwrap_or(0);
                let complexity = task.get("complexity").and_then(|s| s.as_str()).unwrap_or("?");
                text.push_str(&format!("\n  - {tid} [{complexity}, {files} files]: {}", &desc[..desc.len().min(80)]));
            }
        }
    }

    text_result(text)
}

fn tool_edit_plan(args: &Value) -> Result<Value, String> {
    let run_id = require_str(args, "run_id")?;
    let sub_tasks_json = require_str(args, "sub_tasks")?;

    // Validate JSON
    let _: Value = serde_json::from_str(sub_tasks_json)
        .map_err(|e| format!("Invalid sub_tasks JSON: {e}"))?;

    let query = format!(
        "UPDATE goal_plan SET sub_tasks = {sub_tasks}, status = 'draft' \
         WHERE run_id = '{run_id}' ORDER BY version DESC LIMIT 1",
        sub_tasks = sub_tasks_json,
        run_id = sanitize(run_id),
    );
    crate::surreal_query(&query)?;
    text_result(format!("Plan sub-tasks updated for run {run_id}. Use approve_plan to execute."))
}

fn tool_delete_project(args: &Value) -> Result<Value, String> {
    let name = require_str(args, "name")?;
    let safe_name = sanitize(name);
    let query = format!(
        "DELETE FROM project WHERE name = '{safe_name}'; \
         DELETE FROM agent_run WHERE project = '{safe_name}'; \
         DELETE FROM goal_plan WHERE project = '{safe_name}'; \
         DELETE FROM file_embedding WHERE project = '{safe_name}'"
    );
    crate::surreal_query(&query)?;
    text_result(format!("Project '{name}' and all associated data deleted."))
}
