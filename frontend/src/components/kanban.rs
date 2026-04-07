use leptos::prelude::*;
use crate::types::*;
use crate::api;
use crate::components::badge::StatusBadge;

#[component]
pub fn KanbanBoard(
    goals: Vec<Goal>,
    #[prop(optional, into)] rerun_project: String,
    #[prop(optional)] run_detail: Option<RwSignal<Option<AgentRun>>>,
) -> impl IntoView {
    if goals.is_empty() {
        return view! { <p class="empty">"No tasks yet. Submit a task to get started."</p> }.into_any();
    }

    let columns = vec![
        ("running", "In Progress", "var(--warning)"),
        ("passed", "Completed", "var(--success)"),
        ("failed", "Failed", "var(--error)"),
    ];

    columns.into_iter().filter_map(|(status, label, color)| {
        let col_goals: Vec<Goal> = goals.iter()
            .filter(|g| g.status == status || (status == "running" && (g.status == "partial" || g.status == "planning")))
            .cloned()
            .collect();
        if col_goals.is_empty() { return None; }
        let count = col_goals.len();
        let rerun_project = rerun_project.clone();

        Some(view! {
            <div style="min-width:320px;flex:1">
                <div style=format!("display:flex;align-items:center;gap:8px;padding:10px 0;font-size:13px;font-weight:600;border-bottom:2px solid {color};margin-bottom:8px")>
                    {label}" "<span style="color:var(--muted);font-weight:400">{count}</span>
                </div>
                {col_goals.into_iter().map(|g| {
                    let rp = rerun_project.clone();
                    view! { <GoalCard goal=g rerun_project=rp run_detail=run_detail /> }
                }).collect_view()}
            </div>
        })
    }).collect_view().into_any()
}

#[component]
fn GoalCard(
    goal: Goal,
    #[prop(into)] rerun_project: String,
    run_detail: Option<RwSignal<Option<AgentRun>>>,
) -> impl IntoView {
    let expanded = RwSignal::new(false);
    let goal_text = goal.goal.clone();
    let agents = goal.agents.clone();
    let agent_count = goal.total;
    let rerun_text = goal.goal.clone();
    let can_rerun = !rerun_project.is_empty();
    // Show progress from the first running/planning agent
    let progress = agents.iter()
        .find(|a| a.status == "running" || a.status == "planning")
        .and_then(|a| a.progress_message.clone());
    // Check if any agent is in "planned" status (awaiting approval)
    let planned_agent = agents.iter()
        .find(|a| a.status == "planned")
        .and_then(|a| a.id.clone());

    view! {
        <div class="card" style="margin-bottom:8px;padding:12px 14px">
            <div style="display:flex;justify-content:space-between;align-items:start">
                <div
                    style="font-size:14px;font-weight:500;cursor:pointer;flex:1"
                    on:click=move |_| expanded.set(!expanded.get())
                >
                    {goal_text}
                </div>
                {can_rerun.then(|| {
                    let text = rerun_text.clone();
                    let project = rerun_project.clone();
                    view! {
                        <button
                            class="btn"
                            style="padding:2px 8px;font-size:11px"
                            on:click=move |_| {
                                let t = text.clone();
                                let p = project.clone();
                                wasm_bindgen_futures::spawn_local(async move {
                                    let _ = api::submit_task(&api::SubmitTask { task: t, project: p, files: vec![] }).await;
                                });
                            }
                        >"Re-run"</button>
                    }
                })}
            </div>
            <div style="font-size:12px;color:var(--muted);margin-top:4px">
                {agent_count}" agents"
                {progress.map(|p| view! {
                    <span style="margin-left:8px;color:var(--accent);font-style:italic">{p}</span>
                })}
            </div>
            {planned_agent.map(|id| {
                let href = format!("/plan/{}", id);
                view! {
                    <a href=href class="btn btn-primary" style="margin-top:8px;font-size:12px;text-decoration:none;display:inline-block">
                        "Review Plan"
                    </a>
                }
            })}

            {move || expanded.get().then(|| {
                let agents = agents.clone();
                view! {
                    <div style="margin-top:8px;border-top:1px solid var(--border);padding-top:8px">
                        <SubAgentTable agents=agents run_detail=run_detail />
                    </div>
                }
            })}
        </div>
    }
}

#[component]
fn SubAgentTable(
    agents: Vec<AgentRun>,
    run_detail: Option<RwSignal<Option<AgentRun>>>,
) -> impl IntoView {
    let headers = ["Status", "Model", "Task", "Tokens", "Duration"];

    view! {
        <table style="width:100%;border-collapse:collapse;font-size:12px">
            <tr>
                {headers.into_iter().map(|h| view! {
                    <th style="text-align:left;padding:6px 8px;color:var(--muted);border-bottom:1px solid var(--border);font-weight:500">{h}</th>
                }).collect_view()}
            </tr>
            {agents.into_iter().map(|a| {
                let st = a.status.clone();
                let model = a.model_used.clone();
                let task = if a.task_description.len() > TASK_PREVIEW_CHARS {
                    format!("{}...", &a.task_description[..TASK_PREVIEW_CHARS])
                } else {
                    a.task_description.clone()
                };
                let tokens = format!("{}/{}", a.tokens_input, a.tokens_output);
                let dur = a.duration_human();
                let clickable = run_detail.is_some();
                let ac = a.clone();

                view! {
                    <tr
                        style=if clickable { "cursor:pointer" } else { "" }
                        on:click=move |_| {
                            if let Some(rd) = run_detail {
                                let ac = ac.clone();
                                let id = ac.id.clone().unwrap_or_default();
                                rd.set(Some(ac));
                                wasm_bindgen_futures::spawn_local(async move {
                                    if let Ok(detail) = api::get_run_detail(&id).await {
                                        rd.set(Some(detail));
                                    }
                                });
                            }
                        }
                    >
                        <td style="padding:6px 8px;border-bottom:1px solid var(--border)"><StatusBadge status=st /></td>
                        <td style="padding:6px 8px;border-bottom:1px solid var(--border)">{model}</td>
                        <td style="padding:6px 8px;border-bottom:1px solid var(--border)">{task}</td>
                        <td style="padding:6px 8px;border-bottom:1px solid var(--border)">{tokens}</td>
                        <td style="padding:6px 8px;border-bottom:1px solid var(--border)">{dur}</td>
                    </tr>
                }
            }).collect_view()}
        </table>
    }
}
