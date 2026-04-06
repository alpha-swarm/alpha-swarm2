use leptos::prelude::*;
use crate::types::*;
use crate::api;

#[component]
pub fn KanbanBoard(
    goals: Vec<Goal>,
    #[prop(optional, into)] rerun_project: String,
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
            <div style="min-width:280px;max-width:340px;flex-shrink:0">
                <div style=format!("display:flex;align-items:center;gap:8px;padding:10px 0;font-size:13px;font-weight:600;border-bottom:2px solid {color};margin-bottom:8px")>
                    {label}" "<span style="color:var(--muted);font-weight:400">{count}</span>
                </div>
                {col_goals.into_iter().map(|g| {
                    let rp = rerun_project.clone();
                    view! { <GoalCard goal=g rerun_project=rp /> }
                }).collect_view()}
            </div>
        })
    }).collect_view().into_any()
}

#[component]
fn GoalCard(
    goal: Goal,
    #[prop(into)] rerun_project: String,
) -> impl IntoView {
    let expanded = RwSignal::new(false);
    let goal_text = goal.goal.clone();
    let agents = goal.agents.clone();
    let agent_count = goal.total;
    let rerun_text = goal.goal.clone();
    let can_rerun = !rerun_project.is_empty();

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
            <div style="font-size:12px;color:var(--muted);margin-top:4px">{agent_count}" agents"</div>

            {move || expanded.get().then(|| {
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
}
