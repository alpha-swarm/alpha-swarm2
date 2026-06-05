//! alpha-swarm Leptos CSR dashboard. Talks to the daemon's /sql shim
//! (same-origin :8001). Polls every POLL_MS; degrades to empty on any error
//! (never panics the UI).

use gloo_net::http::Request;
use gloo_timers::callback::Interval;
use leptos::*;
use serde_json::Value;

const SQL_URL: &str = "/sql";
const POLL_MS: u32 = 3000;

/// POST a SurrealQL statement; return the first statement's result rows.
async fn sql(query: String) -> Vec<Value> {
    let built = Request::post(SQL_URL)
        .header("Content-Type", "text/plain")
        .body(query);
    let Ok(req) = built else { return vec![] };
    let Ok(resp) = req.send().await else { return vec![] };
    let Ok(json) = resp.json::<Value>().await else { return vec![] };
    // SurrealDB /sql shape: [{ "status":"OK", "result":[...] }, ...]
    json.as_array()
        .and_then(|arr| arr.first())
        .and_then(|stmt| stmt.get("result"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default()
}

/// String field accessor (handles string + non-string JSON values).
fn field(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn short_time(ts: &str) -> String {
    ts.chars().take(19).collect::<String>().replace('T', " ")
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s.to_string()
    }
}

#[component]
fn App() -> impl IntoView {
    let (tick, set_tick) = create_signal(0u32);
    Interval::new(POLL_MS, move || set_tick.update(|t| *t += 1)).forget();

    view! {
        <header>
            <span class="dot"></span>
            <h1>"alpha-swarm"</h1>
            <span class="muted">"local agent swarm — live"</span>
        </header>
        <main>
            <SubmitGoal set_tick=set_tick/>
            <Runs tick=tick/>
            <div class="grid2">
                <Routing tick=tick/>
                <Recent tick=tick/>
            </div>
        </main>
    }
}

#[component]
fn SubmitGoal(set_tick: WriteSignal<u32>) -> impl IntoView {
    let (project, set_project) = create_signal("alpha-swarm2".to_string());
    let (goal, set_goal) = create_signal(String::new());
    let (status, set_status) = create_signal(String::new());

    let submit = move |_| {
        let p = project.get().replace('\'', "");
        let g = goal.get();
        if g.trim().is_empty() {
            set_status.set("goal is empty".into());
            return;
        }
        let ge = g.replace('\'', "");
        set_status.set("submitting…".into());
        spawn_local(async move {
            let q = format!(
                "CREATE autopilot_goal SET project = '{p}', goal = '{ge}', status = 'queued', created_at = time::now()"
            );
            let _ = sql(q).await;
            set_status.set("queued ✓".into());
            set_goal.set(String::new());
            set_tick.update(|t| *t += 1);
        });
    };

    view! {
        <div class="panel">
            <h2>"Submit goal"</h2>
            <div class="row" style="margin-bottom:10px">
                <input
                    prop:value=move || project.get()
                    on:input=move |e| set_project.set(event_target_value(&e))
                    style="width:220px"/>
                <span class="muted">{move || status.get()}</span>
            </div>
            <textarea
                prop:value=move || goal.get()
                on:input=move |e| set_goal.set(event_target_value(&e))
                placeholder="e.g. Add a doc comment to the X function in crates/.../y.rs"></textarea>
            <div class="row" style="margin-top:10px">
                <button on:click=submit>"Queue goal"</button>
            </div>
        </div>
    }
}

#[component]
fn Runs(tick: ReadSignal<u32>) -> impl IntoView {
    let runs = create_local_resource(
        move || tick.get(),
        |_| async move {
            sql("SELECT id, status, task_description, progress_message, created_at FROM agent_run ORDER BY created_at DESC LIMIT 25".into()).await
        },
    );
    view! {
        <div class="panel">
            <h2>"Runs"</h2>
            <table>
                <thead><tr><th>"Status"</th><th>"Goal"</th><th>"Progress"</th><th>"When"</th></tr></thead>
                <tbody>
                    {move || runs.get().unwrap_or_default().into_iter().map(|r| {
                        let st = field(&r, "status");
                        let cls = format!("badge s-{st}");
                        view! {
                            <tr>
                                <td><span class=cls>{st}</span></td>
                                <td class="goal">{truncate(&field(&r, "task_description"), 90)}</td>
                                <td class="muted">{truncate(&field(&r, "progress_message"), 60)}</td>
                                <td class="muted">{short_time(&field(&r, "created_at"))}</td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
}

#[component]
fn Routing(tick: ReadSignal<u32>) -> impl IntoView {
    let stats = create_local_resource(
        move || tick.get(),
        |_| async move {
            sql("SELECT shape, tier, attempts, successes FROM routing_stats ORDER BY shape".into()).await
        },
    );
    view! {
        <div class="panel">
            <h2>"Learned routing (UCB)"</h2>
            <table>
                <thead><tr><th>"Shape"</th><th>"Tier"</th><th>"Pass"</th><th>"N"</th></tr></thead>
                <tbody>
                    {move || stats.get().unwrap_or_default().into_iter().map(|r| {
                        let a = r.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0);
                        let su = r.get("successes").and_then(|v| v.as_i64()).unwrap_or(0);
                        let rate = if a > 0 { format!("{:.0}%", su as f64 / a as f64 * 100.0) } else { "—".into() };
                        view! {
                            <tr>
                                <td>{field(&r, "shape")}</td>
                                <td>{field(&r, "tier")}</td>
                                <td>{rate}</td>
                                <td class="muted">{a}</td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
}

#[component]
fn Recent(tick: ReadSignal<u32>) -> impl IntoView {
    let fails = create_local_resource(
        move || tick.get(),
        |_| async move {
            sql("SELECT task_description, error_message, created_at FROM agent_run WHERE status = 'failed' ORDER BY created_at DESC LIMIT 8".into()).await
        },
    );
    view! {
        <div class="panel">
            <h2>"Recent rejects"</h2>
            <table>
                <thead><tr><th>"Goal"</th><th>"Reason"</th></tr></thead>
                <tbody>
                    {move || fails.get().unwrap_or_default().into_iter().map(|r| {
                        view! {
                            <tr>
                                <td class="goal">{truncate(&field(&r, "task_description"), 70)}</td>
                                <td class="muted">{truncate(&field(&r, "error_message"), 120)}</td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
}

fn main() {
    mount_to_body(App);
}
