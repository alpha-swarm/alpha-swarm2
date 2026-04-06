use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_params_map};
use crate::state::AppState;
use crate::api;
use crate::types::*;

#[component]
pub fn ProjectDetailPage() -> impl IntoView {
    let state = expect_context::<AppState>();
    let params = use_params_map();
    let name = move || params.read().get("name").unwrap_or_default();

    let metrics = RwSignal::new(ProjectMetrics::default());
    let goals = RwSignal::new(Vec::<Goal>::new());
    let runs = RwSignal::new(Vec::<AgentRun>::new());
    let selected_run = RwSignal::new(Option::<AgentRun>::None);
    let run_detail = RwSignal::new(Option::<AgentRun>::None);
    let expanded_goal = RwSignal::new(Option::<String>::None);

    // Load data
    let project_name = name();
    let pn = project_name.clone();
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(m) = api::get_metrics(&pn).await { metrics.set(m); }
        if let Ok(g) = api::get_goals(&pn).await { goals.set(g); }
        if let Ok(r) = api::list_runs(&pn).await { runs.set(r); }
    });

    let on_delete = {
        let pn = project_name.clone();
        move |_| {
            let pn = pn.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = api::delete_project(&pn).await;
                let nav = use_navigate();
                nav("/projects", Default::default());
            });
        }
    };

    let on_rerun_goal = move |goal_text: String| {
        let pn = name();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = api::submit_task(&api::SubmitTask { task: goal_text, project: pn, files: vec![] }).await;
        });
    };

    view! {
        <div style="display:flex;align-items:center;gap:12px;margin-bottom:4px">
            <a href="/projects" class="btn" style="padding:4px 8px;text-decoration:none">"←"</a>
            <div style="flex:1">
                <h1>{name}</h1>
            </div>
            <a href="/submit" class="btn btn-primary" style="font-size:13px;text-decoration:none">"Submit Task"</a>
            <button class="btn" style="color:var(--error);font-size:13px" on:click=on_delete>"Delete"</button>
        </div>

        // Metrics
        <div class="grid grid-3" style="margin:20px 0">
            <div class="card">
                <h3>"Pass Rate"</h3>
                <div class="value">{move || format!("{}%", (metrics.get().pass_rate * 100.0) as u32)}</div>
            </div>
            <div class="card">
                <h3>"Total Runs"</h3>
                <div class="value">{move || metrics.get().total_runs}</div>
            </div>
            <div class="card">
                <h3>"Avg Duration"</h3>
                <div class="value">{move || format!("{:.1}s", metrics.get().avg_duration_ms as f64 / 1000.0)}</div>
            </div>
        </div>

        // Kanban
        <h2 style="font-size:15px;font-weight:600;margin:28px 0 12px">"Task Board"</h2>
        <div style="display:flex;gap:16px;overflow-x:auto;min-height:150px">
            {move || {
                let all_goals = goals.get();
                if all_goals.is_empty() {
                    return view! { <p class="empty">"No tasks yet. Submit a task to get started."</p> }.into_any();
                }
                let columns = vec![
                    ("running", "In Progress", "var(--warning)"),
                    ("passed", "Completed", "var(--success)"),
                    ("failed", "Failed", "var(--error)"),
                ];
                columns.into_iter().filter_map(|(status, label, color)| {
                    let col_goals: Vec<Goal> = all_goals.iter()
                        .filter(|g| g.status == status || (status == "running" && g.status == "partial"))
                        .cloned()
                        .collect();
                    if col_goals.is_empty() { return None; }
                    let count = col_goals.len();
                    Some(view! {
                        <div style="min-width:280px;max-width:340px;flex-shrink:0">
                            <div style=format!("display:flex;align-items:center;gap:8px;padding:10px 0;font-size:13px;font-weight:600;border-bottom:2px solid {color};margin-bottom:8px")>
                                {label}" "<span style="color:var(--muted);font-weight:400">{count}</span>
                            </div>
                            {col_goals.into_iter().map(|g| {
                                let goal_text = g.goal.clone();
                                let goal_key = g.goal.clone();
                                let gk = goal_key.clone();
                                let agent_count = g.total;
                                let agents = g.agents.clone();
                                let is_expanded = move || expanded_goal.get().as_ref() == Some(&gk);
                                let rerun_text = goal_text.clone();

                                view! {
                                    <div class="card" style="margin-bottom:8px;padding:12px 14px">
                                        <div style="display:flex;justify-content:space-between;align-items:start">
                                            <div
                                                style="font-size:14px;font-weight:500;cursor:pointer;flex:1"
                                                on:click={
                                                    let gk = goal_key.clone();
                                                    move |_| {
                                                        let gk = gk.clone();
                                                        if expanded_goal.get().as_ref() == Some(&gk) {
                                                            expanded_goal.set(None);
                                                        } else {
                                                            expanded_goal.set(Some(gk));
                                                        }
                                                    }
                                                }
                                            >
                                                {goal_text}
                                            </div>
                                            <button
                                                class="btn"
                                                style="padding:2px 8px;font-size:11px"
                                                on:click={
                                                    let on_rerun = on_rerun_goal.clone();
                                                    let text = rerun_text.clone();
                                                    move |_| on_rerun(text.clone())
                                                }
                                            >"Re-run"</button>
                                        </div>
                                        <div style="font-size:12px;color:var(--muted);margin-top:4px">{agent_count}" agents"</div>

                                        {move || is_expanded().then(|| {
                                            let agents = agents.clone();
                                            view! {
                                                <div style="margin-top:8px;border-top:1px solid var(--border);padding-top:8px">
                                                    {agents.into_iter().map(|a| {
                                                        let st = a.status.clone();
                                                        let st_class = format!("badge {st}");
                                                        let model = a.model_used.clone();
                                                        let dur = format!("{:.1}s", a.duration_secs());
                                                        let agent_id = a.agent_id.clone();
                                                        view! {
                                                            <div style="display:flex;align-items:center;gap:6px;font-size:12px;padding:3px 0">
                                                                <span class=st_class style="font-size:10px;padding:1px 6px">{st}</span>
                                                                <span>{agent_id}</span>
                                                                <span style="color:var(--muted)">{model}</span>
                                                                <span style="margin-left:auto;color:var(--muted)">{dur}</span>
                                                            </div>
                                                        }
                                                    }).collect_view()}
                                                </div>
                                            }
                                        })}
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    })
                }).collect_view().into_any()
            }}
        </div>

        // Run History
        <h2 style="font-size:15px;font-weight:600;margin:28px 0 12px">"Run History"</h2>
        <div style="display:flex;gap:16px">
            <div style="flex:1;min-width:0">
                {move || {
                    let run_list = runs.get();
                    if run_list.is_empty() {
                        return view! { <p class="empty">"No runs yet"</p> }.into_any();
                    }
                    view! {
                        <table style="width:100%;border-collapse:collapse">
                            <tr>
                                <th style="text-align:left;padding:10px 12px;font-size:12px;font-weight:500;color:var(--muted);border-bottom:1px solid var(--border)">"Status"</th>
                                <th style="text-align:left;padding:10px 12px;font-size:12px;font-weight:500;color:var(--muted);border-bottom:1px solid var(--border)">"Model"</th>
                                <th style="text-align:left;padding:10px 12px;font-size:12px;font-weight:500;color:var(--muted);border-bottom:1px solid var(--border)">"Task"</th>
                                <th style="text-align:left;padding:10px 12px;font-size:12px;font-weight:500;color:var(--muted);border-bottom:1px solid var(--border)">"Tokens"</th>
                                <th style="text-align:left;padding:10px 12px;font-size:12px;font-weight:500;color:var(--muted);border-bottom:1px solid var(--border)">"Duration"</th>
                            </tr>
                            {run_list.into_iter().map(|r| {
                                let st = r.status.clone();
                                let st_class = format!("badge {st}");
                                let model = r.model_used.clone();
                                let task = if r.task_description.len() > 40 { format!("{}...", &r.task_description[..40]) } else { r.task_description.clone() };
                                let tokens = r.tokens_output;
                                let dur = format!("{:.1}s", r.duration_secs());
                                let run_clone = r.clone();
                                view! {
                                    <tr
                                        style="cursor:pointer"
                                        on:click=move |_| {
                                            selected_run.set(Some(run_clone.clone()));
                                            let id = run_clone.id.clone().unwrap_or_default();
                                            wasm_bindgen_futures::spawn_local(async move {
                                                if let Ok(detail) = api::get_run_detail(&id).await {
                                                    run_detail.set(Some(detail));
                                                }
                                            });
                                        }
                                    >
                                        <td style="padding:12px;font-size:14px;border-bottom:1px solid var(--border)"><span class=st_class>{st}</span></td>
                                        <td style="padding:12px;font-size:13px;border-bottom:1px solid var(--border)">{model}</td>
                                        <td style="padding:12px;font-size:13px;border-bottom:1px solid var(--border)">{task}</td>
                                        <td style="padding:12px;font-size:13px;border-bottom:1px solid var(--border)">{tokens}</td>
                                        <td style="padding:12px;font-size:13px;border-bottom:1px solid var(--border)">{dur}</td>
                                    </tr>
                                }
                            }).collect_view()}
                        </table>
                    }.into_any()
                }}
            </div>

            // Run detail panel
            {move || {
                run_detail.get().map(|d| {
                    let status = d.status.clone();
                    let st_class = format!("badge {status}");
                    let task = d.task_description.clone();
                    let model = d.model_used.clone();
                    let agent = d.agent_id.clone();
                    let tokens_in = d.tokens_input;
                    let tokens_out = d.tokens_output;
                    let dur = format!("{:.1}s", d.duration_secs());
                    let error = d.error_message.clone();
                    let diff = d.diff.clone().unwrap_or_default();
                    let has_pr = diff.starts_with("PR: ");
                    let pr_url = if has_pr { diff[4..].to_string() } else { String::new() };
                    let files = d.files_modified.clone();

                    view! {
                        <div style="width:380px;flex-shrink:0">
                            <div class="card" style="position:sticky;top:20px">
                                <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:12px">
                                    <span style="font-size:15px;font-weight:600">"Run Detail"</span>
                                    <button class="btn" style="padding:2px 8px" on:click=move |_| { run_detail.set(None); selected_run.set(None); }>"×"</button>
                                </div>
                                <span class=st_class>{status}</span>
                                <div style="font-size:14px;font-weight:500;margin:8px 0;line-height:1.4">{task}</div>
                                <div style="font-size:13px;color:var(--muted)">"Model: "{model}</div>
                                <div style="font-size:13px;color:var(--muted)">"Agent: "{agent}</div>
                                <div style="font-size:13px;color:var(--muted)">"Tokens: "{tokens_in}" in / "{tokens_out}" out"</div>
                                <div style="font-size:13px;color:var(--muted)">"Duration: "{dur}</div>
                                {(!files.is_empty()).then(|| {
                                    let f = files.join(", ");
                                    view! { <div style="font-size:13px;color:var(--muted)">"Files: "{f}</div> }
                                })}
                                {error.map(|e| view! {
                                    <div style="margin-top:8px;background:var(--error-bg);color:var(--error);padding:8px 12px;border-radius:var(--radius-sm);font-size:13px">{e}</div>
                                })}
                                {has_pr.then(|| view! {
                                    <a href=pr_url target="_blank" class="btn btn-primary" style="margin-top:8px;text-decoration:none;font-size:13px">"View PR →"</a>
                                })}
                                {(!has_pr && !diff.is_empty()).then(|| {
                                    let d = if diff.len() > 2000 { format!("{}...", &diff[..2000]) } else { diff.clone() };
                                    view! {
                                        <details style="margin-top:8px">
                                            <summary style="cursor:pointer;font-size:12px;color:var(--accent);font-weight:500">"Show Diff"</summary>
                                            <pre style="background:var(--bg-secondary);border:1px solid var(--border);border-radius:var(--radius-sm);padding:10px;font-size:11px;overflow-x:auto;max-height:300px;color:var(--text-secondary);margin-top:4px">{d}</pre>
                                        </details>
                                    }
                                })}
                            </div>
                        </div>
                    }
                })
            }}
        </div>
    }
}
