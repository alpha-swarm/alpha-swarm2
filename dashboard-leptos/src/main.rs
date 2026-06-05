//! alpha-swarm Leptos CSR dashboard. Talks to the daemon's /sql shim
//! (same-origin :8001). Polls every POLL_MS; degrades to empty on any error
//! (never panics the UI).

use gloo_net::http::Request;
use gloo_timers::callback::Interval;
use leptos::*;
use serde_json::Value;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

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
    // Fallback poll.
    Interval::new(POLL_MS, move || set_tick.update(|t| *t += 1)).forget();
    // Live: bump tick on each swarm event (real-time refresh; poll is the
    // fallback if SSE drops). Best-effort — ignore if EventSource is unavailable.
    if let Ok(es) = web_sys::EventSource::new("/events") {
        let cb = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |_e: web_sys::MessageEvent| {
            set_tick.update(|t| *t += 1);
        });
        es.set_onmessage(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
        std::mem::forget(es); // keep the connection open for the app lifetime
    }
    // Currently-selected run id (clicked in the Runs table) → drives the detail panel.
    let (selected, set_selected) = create_signal::<Option<String>>(None);

    view! {
        <header>
            <span class="dot"></span>
            <h1>"alpha-swarm"</h1>
            <span class="muted">"local agent swarm — live"</span>
        </header>
        <main>
            <SubmitGoal set_tick=set_tick/>
            <Runs tick=tick set_selected=set_selected/>
            <Detail selected=selected tick=tick/>
            <div class="grid2">
                <Routing tick=tick/>
                <Recent tick=tick/>
            </div>
            <Memory tick=tick/>
        </main>
    }
}

#[component]
fn Memory(tick: ReadSignal<u32>) -> impl IntoView {
    let counts = create_local_resource(
        move || tick.get(),
        |_| async move {
            sql("SELECT namespace, count() AS c FROM memory_entry GROUP BY namespace".into()).await
        },
    );
    let patterns = create_local_resource(
        move || tick.get(),
        |_| async move {
            sql("SELECT content, use_count FROM memory_entry WHERE namespace = 'patterns' ORDER BY use_count DESC LIMIT 12".into()).await
        },
    );
    view! {
        <div class="panel">
            <h2>"Learned memory (SONA)"</h2>
            <div class="row" style="gap:16px; margin-bottom:12px">
                {move || counts.get().unwrap_or_default().into_iter().map(|r| {
                    let c = r.get("c").and_then(|v| v.as_i64()).unwrap_or(0);
                    view! { <span class="muted">{field(&r, "namespace")}": "<b style="color:var(--fg)">{c}</b></span> }
                }).collect_view()}
            </div>
            <table>
                <thead><tr><th>"Distilled pattern"</th><th>"Reused"</th></tr></thead>
                <tbody>
                    {move || patterns.get().unwrap_or_default().into_iter().map(|r| {
                        let uc = r.get("use_count").and_then(|v| v.as_i64()).unwrap_or(0);
                        view! {
                            <tr>
                                <td class="goal">{truncate(&field(&r, "content"), 160)}</td>
                                <td class="muted">{format!("{uc}×")}</td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </div>
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
fn Runs(tick: ReadSignal<u32>, set_selected: WriteSignal<Option<String>>) -> impl IntoView {
    let runs = create_local_resource(
        move || tick.get(),
        |_| async move {
            sql("SELECT id, status, task_description, progress_message, created_at FROM agent_run ORDER BY created_at DESC LIMIT 25".into()).await
        },
    );
    view! {
        <div class="panel">
            <h2>"Runs — click a row for detail"</h2>
            <table>
                <thead><tr><th>"Status"</th><th>"Goal"</th><th>"Progress"</th><th>"When"</th></tr></thead>
                <tbody>
                    {move || runs.get().unwrap_or_default().into_iter().map(|r| {
                        let st = field(&r, "status");
                        let cls = format!("badge s-{st}");
                        let id = field(&r, "id");
                        view! {
                            <tr style="cursor:pointer" on:click=move |_| set_selected.set(Some(id.clone()))>
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
fn Detail(selected: ReadSignal<Option<String>>, tick: ReadSignal<u32>) -> impl IntoView {
    // Refetch when the selection changes OR on each poll tick (live progress).
    let data = create_local_resource(
        move || (selected.get(), tick.get()),
        |(sel, _)| async move {
            let Some(id) = sel else { return None };
            let run = sql(format!("SELECT * FROM {id}")).await.into_iter().next();
            // NOTE: SurrealDB requires ORDER BY fields in the projection.
            let plan = sql(format!(
                "SELECT sub_tasks, version FROM goal_plan WHERE run_id = '{id}' ORDER BY version DESC LIMIT 1"
            )).await.into_iter().next();
            let tasks = plan
                .and_then(|p| p.get("sub_tasks").and_then(|v| v.as_array()).cloned())
                .unwrap_or_default();
            run.map(|r| (r, tasks))
        },
    );

    view! {
        <div class="panel">
            <h2>"Run detail"</h2>
            {move || match data.get().flatten() {
                None => view! {
                    <p class="muted">"Click a run above to see its plan, files, model, and gate result."</p>
                }.into_view(),
                Some((r, tasks)) => {
                    let files = r.get("files_modified").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                    let err = field(&r, "error_message");
                    let diff = field(&r, "diff");
                    let num = |k: &str| r.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
                    view! {
                        <p style="margin:0 0 12px"><b>{field(&r, "task_description")}</b></p>
                        <div class="row" style="gap:18px; margin-bottom:14px">
                            <span><span class=format!("badge s-{}", field(&r,"status"))>{field(&r,"status")}</span></span>
                            <span class="muted">"model: "{field(&r, "model_used")}</span>
                            <span class="muted">"tokens: "{num("tokens_input")}" / "{num("tokens_output")}</span>
                            <span class="muted">{format!("{}ms", num("duration_ms"))}</span>
                        </div>
                        <h2>"Plan"</h2>
                        <ul>
                            {tasks.iter().map(|t| view!{
                                <li>
                                    <b>{field(t, "id")}</b>": "{field(t, "description")}
                                    <span class="muted">" "{field(t, "complexity")}</span>
                                </li>
                            }).collect_view()}
                        </ul>
                        <h2>"Files modified"</h2>
                        <ul>
                            {files.iter().filter_map(|f| f.as_str().map(|s| s.to_string())).map(|f| view!{
                                <li class="muted">{f}</li>
                            }).collect_view()}
                        </ul>
                        {(!err.is_empty()).then(|| view!{
                            <div><h2>"Failure / gate reason"</h2><pre style="white-space:pre-wrap; color:var(--fail)">{err}</pre></div>
                        })}
                        {(!diff.is_empty()).then(|| view!{
                            <div><h2>"Diff"</h2><pre style="white-space:pre-wrap; max-height:340px; overflow:auto">{truncate(&diff, 4000)}</pre></div>
                        })}
                    }.into_view()
                }
            }}
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
