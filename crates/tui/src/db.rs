use crate::app::Goal;
use reqwest::Client;
use serde_json::Value;

const SURREAL_URL: &str = "http://127.0.0.1:8001/sql";
const SURREAL_NS: &str = "alpha_swarm";
const SURREAL_DB: &str = "swarm";

/// NATS url for the daemon DB bridge (preferred path).
const NATS_URL: &str = "nats://127.0.0.1:4223";
/// Canonical subjects — see `knowledge_base::bridge_client`.
const DB_QUERY_SUBJECT: &str = "swarm.db.query";
const DB_EXEC_SUBJECT: &str = "swarm.db.exec";
const BRIDGE_TIMEOUT_SECS: u64 = 10;

pub async fn fetch_goals(project: &str) -> Vec<Goal> {
    let q = format!(
        "SELECT * FROM agent_run WHERE project = '{}' ORDER BY created_at DESC LIMIT 30",
        project
    );
    let rows = query(&q).await;
    rows.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect()
}

pub async fn submit_goal(project: &str, goal: &str) -> bool {
    let q = format!(
        "CREATE agent_run SET project = '{}', task_description = '{}', \
         status = 'planning', agent_id = 'tui', model_used = 'auto', created_at = time::now(), \
         files_modified = [], tokens_input = 0, tokens_output = 0, duration_ms = 0",
        project.replace('\'', ""),
        goal.replace('\'', ""),
    );
    !query(&q).await.is_empty()
}

pub async fn approve_goal(run_id: &str) -> bool {
    let q = format!("UPDATE {} SET status = 'approved'", run_id);
    !query(&q).await.is_empty()
}

pub async fn delete_goal(run_id: &str) -> bool {
    let q = format!("DELETE {}", run_id);
    query(&q).await;
    true
}

/// Bridge-first: query the daemon-owned DB over NATS request-reply; fall back
/// to the legacy SurrealDB HTTP endpoint while the external server exists.
async fn query(q: &str) -> Vec<Value> {
    if let Some(rows) = bridge_query(q).await {
        return rows;
    }
    http_query(q).await
}

async fn bridge_query(q: &str) -> Option<Vec<Value>> {
    let client = async_nats::connect(NATS_URL).await.ok()?;
    let verb = q.trim_start().split_whitespace().next().unwrap_or("").to_uppercase();
    let subject = if verb == "SELECT" { DB_QUERY_SUBJECT } else { DB_EXEC_SUBJECT };
    let payload = serde_json::json!({ "query": q }).to_string();
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(BRIDGE_TIMEOUT_SECS),
        client.request(subject.to_string(), payload.into()),
    ).await.ok()?.ok()?;
    let parsed: Value = serde_json::from_slice(&resp.payload).ok()?;
    if parsed.get("ok")?.as_bool()? {
        Some(parsed.get("rows")?.as_array()?.clone())
    } else {
        None
    }
}

async fn http_query(q: &str) -> Vec<Value> {
    let full = format!("USE NS {} DB {}; {}", SURREAL_NS, SURREAL_DB, q);
    let client = Client::new();
    let resp = client.post(SURREAL_URL)
        .basic_auth("root", Some("root"))
        .header("Content-Type", "text/plain")
        .body(full)
        .send().await;

    let Ok(resp) = resp else { return vec![]; };
    let Ok(body) = resp.text().await else { return vec![]; };
    let Ok(parsed) = serde_json::from_str::<Vec<Value>>(&body) else { return vec![]; };

    // SurrealDB returns [{result: {ns/db}}, {result: [...]}] — take last array result
    for entry in parsed.iter().rev() {
        if let Some(result) = entry.get("result") {
            if let Some(arr) = result.as_array() {
                return arr.clone();
            }
        }
    }
    vec![]
}
