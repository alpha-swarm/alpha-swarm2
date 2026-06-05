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

/// GET a JSON endpoint (e.g. /review); Null on any error.
async fn get_json(path: &str) -> Value {
    match Request::get(path).send().await {
        Ok(resp) => resp.json::<Value>().await.unwrap_or(Value::Null),
        Err(_) => Value::Null,
    }
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
            <Review tick=tick/>
            <Memory tick=tick/>
        </main>
    }
}

#[component]
fn Review(tick: ReadSignal<u32>) -> impl IntoView {
    let data = create_local_resource(move || tick.get(), |_| async move { get_json("/review").await });
    let commits = move || data.get().as_ref()
        .and_then(|d| d.get("commits")).and_then(|c| c.as_array()).cloned().unwrap_or_default();
    let prs = move || data.get().as_ref()
        .and_then(|d| d.get("prs")).and_then(|p| p.as_array()).cloned().unwrap_or_default();
    view! {
        <div class="panel">
            <h2>"Review — swarm/auto commits + open PRs"</h2>
            <div class="grid2">
                <div>
                    <b>"Loop commits (not yet in main)"</b>
                    <ul>
                        {move || commits().into_iter().filter_map(|c| c.as_str().map(String::from)).map(|c| view! {
                            <li class="muted" style="font-family:ui-monospace,monospace; font-size:12px">{c}</li>
                        }).collect_view()}
                    </ul>
                </div>
                <div>
                    <b>"Open PRs"</b>
                    <ul>
                        {move || prs().into_iter().map(|p| {
                            let n = p.get("number").and_then(|v| v.as_i64()).unwrap_or(0);
                            view! {
                                <li>"#"{n}" "{field(&p, "title")}
                                    <span class="muted">" ("{field(&p, "headRefName")}")"</span></li>
                            }
                        }).collect_view()}
                    </ul>
                </div>
            </div>
        </div>
    }
}

/// Render a run's expanded detail (model/tokens/duration, plan, files, gate, diff).
fn detail_view(r: Value, tasks: Vec<Value>) -> View {
    let files = r.get("files_modified").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let err = field(&r, "error_message");
    let diff = field(&r, "diff");
    let num = |k: &str| r.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
    view! {
        <div style="padding:6px 0 10px">
            <div class="row" style="gap:18px; margin-bottom:10px">
                <span class="muted">"model: "{field(&r, "model_used")}</span>
                <span class="muted">"tokens: "{num("tokens_input")}" / "{num("tokens_output")}</span>
                <span class="muted">{format!("{}ms", num("duration_ms"))}</span>
            </div>
            <b>"Plan"</b>
            <ul>
                {tasks.iter().map(|t| view! {
                    <li>{field(t, "id")}": "{field(t, "description")}
                        <span class="muted">" ["{field(t, "complexity")}"]"</span></li>
                }).collect_view()}
            </ul>
            <b>"Files modified"</b>
            <ul>
                {files.iter().filter_map(|f| f.as_str().map(String::from)).map(|f| view! {
                    <li class="muted">{f}</li>
                }).collect_view()}
            </ul>
            {(!err.is_empty()).then(|| view! {
                <div><b style="color:var(--fail)">"Gate / failure"</b>
                    <pre style="white-space:pre-wrap; color:var(--fail)">{err}</pre></div>
            })}
            {(!diff.is_empty()).then(|| view! {
                <div><b>"Diff"</b>
                    <pre style="white-space:pre-wrap; max-height:300px; overflow:auto">{truncate(&diff, 4000)}</pre></div>
            })}
        </div>
    }.into_view()
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
fn Runs(tick: ReadSignal<u32>) -> impl IntoView {
    let (expanded, set_expanded) = create_signal::<Option<String>>(None);
    let runs = create_local_resource(
        move || tick.get(),
        |_| async move {
            sql("SELECT id, status, task_description, progress_message, created_at FROM agent_run ORDER BY created_at DESC LIMIT 25".into()).await
        },
    );
    // Detail for the currently-expanded run; refetches on expand change + tick.
    let detail = create_local_resource(
        move || (expanded.get(), tick.get()),
        |(sel, _)| async move {
            let Some(id) = sel else { return None };
            let run = sql(format!("SELECT * FROM {id}")).await.into_iter().next();
            // SurrealDB requires ORDER BY fields in the projection.
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
            <h2>"Runs — click a row to expand"</h2>
            <table>
                <thead><tr><th style="width:16px"></th><th>"Status"</th><th>"Goal"</th><th>"Progress"</th><th>"When"</th></tr></thead>
                <tbody>
                    {move || runs.get().unwrap_or_default().into_iter().map(|r| {
                        let st = field(&r, "status");
                        let cls = format!("badge s-{st}");
                        let id = field(&r, "id");
                        let (id_click, id_caret, id_cond) = (id.clone(), id.clone(), id.clone());
                        view! {
                            <tr style="cursor:pointer" on:click=move |_| {
                                set_expanded.update(|e| {
                                    *e = if e.as_deref() == Some(id_click.as_str()) { None } else { Some(id_click.clone()) };
                                });
                            }>
                                <td class="muted">{move || if expanded.get().as_deref() == Some(id_caret.as_str()) { "▾" } else { "▸" }}</td>
                                <td><span class=cls>{st}</span></td>
                                <td class="goal">{truncate(&field(&r, "task_description"), 90)}</td>
                                <td class="muted">{truncate(&field(&r, "progress_message"), 60)}</td>
                                <td class="muted">{short_time(&field(&r, "created_at"))}</td>
                            </tr>
                            {move || (expanded.get().as_deref() == Some(id_cond.as_str())).then(|| view! {
                                <tr><td></td><td colspan="4">
                                    {move || match detail.get().flatten() {
                                        Some((rr, tasks)) => detail_view(rr, tasks),
                                        None => view! { <span class="muted">"loading…"</span> }.into_view(),
                                    }}
                                </td></tr>
                            })}
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
