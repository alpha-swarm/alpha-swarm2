use crate::app::Goal;
use reqwest::Client;
use serde_json::Value;

const SURREAL_URL: &str = "http://127.0.0.1:8001/sql";
const SURREAL_NS: &str = "alpha_swarm";
const SURREAL_DB: &str = "swarm";

pub async fn fetch_goals(project: &str) -> Vec<Goal> {
    let q = format!(
        "USE NS {} DB {}; SELECT * FROM agent_run WHERE project = '{}' ORDER BY created_at DESC LIMIT 30",
        SURREAL_NS, SURREAL_DB, project
    );
    let rows = query(&q).await;
    rows.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect()
}

pub async fn submit_goal(project: &str, goal: &str) -> bool {
    let q = format!(
        "USE NS {} DB {}; CREATE agent_run SET project = '{}', task_description = '{}', \
         status = 'planning', agent_id = 'tui', model_used = 'auto', created_at = time::now(), \
         files_modified = [], tokens_input = 0, tokens_output = 0, duration_ms = 0",
        SURREAL_NS, SURREAL_DB,
        project.replace('\'', ""),
        goal.replace('\'', ""),
    );
    !query(&q).await.is_empty()
}

pub async fn approve_goal(run_id: &str) -> bool {
    let q = format!("USE NS {} DB {}; UPDATE {} SET status = 'approved'", SURREAL_NS, SURREAL_DB, run_id);
    !query(&q).await.is_empty()
}

pub async fn delete_goal(run_id: &str) -> bool {
    let q = format!("USE NS {} DB {}; DELETE {}", SURREAL_NS, SURREAL_DB, run_id);
    query(&q).await;
    true
}

async fn query(q: &str) -> Vec<Value> {
    let client = Client::new();
    let resp = client.post(SURREAL_URL)
        .basic_auth("root", Some("root"))
        .header("Content-Type", "text/plain")
        .body(q.to_string())
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
