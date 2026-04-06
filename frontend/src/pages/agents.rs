use leptos::prelude::*;
use crate::state::AppState;
use crate::api;
use crate::types::*;

#[component]
pub fn AgentsPage() -> impl IntoView {
    let state = expect_context::<AppState>();
    let all_goals = RwSignal::new(Vec::<(String, Vec<Goal>)>::new());
    let expanded_goal = RwSignal::new(Option::<String>::None);
    let expanded_agent = RwSignal::new(Option::<String>::None);
    let agent_detail = RwSignal::new(Option::<AgentRun>::None);

    // Load goals for all projects
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(projects) = api::list_projects().await {
            let mut result = Vec::new();
            for p in &projects {
                if let Ok(goals) = api::get_goals(&p.name).await {
                    if !goals.is_empty() {
                        result.push((p.name.clone(), goals));
                    }
                }
            }
            all_goals.set(result);
            state.projects.set(projects);
        }
    });

    view! {
        <h1>"Agents"</h1>
        <p class="subtitle">"Hierarchical view — goals and their sub-agents"</p>
        <div>
            {move || {
                let goals = all_goals.get();
                if goals.is_empty() {
                    return view! { <p class="empty">"No agent activity yet"</p> }.into_any();
                }
                goals.into_iter().flat_map(|(project, goals)| {
                    goals.into_iter().map(move |goal| {
                        let project = project.clone();
                        let goal_key = format!("{}:{}", project, goal.goal);
                        let gk = goal_key.clone();
                        let is_expanded = move || expanded_goal.get().as_ref() == Some(&goal_key);
                        let status = goal.status.clone();
                        let status_class = format!("badge {status}");
                        let goal_text = goal.goal.clone();
                        let agents = goal.agents.clone();
                        let agent_count = goal.total;
                        let passed = goal.passed;
                        let failed = goal.failed;

                        view! {
                            <div class="card" style="margin-bottom:12px">
                                <div
                                    style="cursor:pointer"
                                    on:click=move |_| {
                                        let gk = gk.clone();
                                        if expanded_goal.get().as_ref() == Some(&gk) {
                                            expanded_goal.set(None);
                                        } else {
                                            expanded_goal.set(Some(gk));
                                        }
                                    }
                                >
                                    <div style="display:flex;align-items:center;gap:8px;margin-bottom:4px">
                                        <span class=status_class>{status.clone()}</span>
                                        <span style="font-size:13px;color:var(--muted)">{project.clone()}</span>
                                    </div>
                                    <div style="font-size:15px;font-weight:600;margin-bottom:4px">{goal_text}</div>
                                    <div style="font-size:13px;color:var(--muted)">
                                        {agent_count}" agents"
                                        {(passed > 0).then(|| format!(" · {} passed", passed))}
                                        {(failed > 0).then(|| format!(" · {} failed", failed))}
                                    </div>
                                </div>

                                {move || is_expanded().then(|| {
                                    let agents = agents.clone();
                                    view! {
                                        <div style="margin-top:12px;border-top:1px solid var(--border);padding-top:12px">
                                            {agents.into_iter().map(|a| {
                                                let aid = a.id.clone().unwrap_or_default();
                                                let a_status = a.status.clone();
                                                let a_class = format!("badge {a_status}");
                                                let a_agent = a.agent_id.clone();
                                                let a_model = a.model_used.clone();
                                                let a_dur = format!("{:.1}s", a.duration_secs());
                                                let a_tokens = a.tokens_output;
                                                let a_task = a.task_description.clone();
                                                let a_error = a.error_message.clone();
                                                let a_files = a.files_modified.clone();
                                                let aid_click = aid.clone();

                                                view! {
                                                    <div
                                                        class="card"
                                                        style="margin-bottom:8px;padding:10px 14px;cursor:pointer"
                                                        on:click=move |_| {
                                                            let id = aid_click.clone();
                                                            if expanded_agent.get().as_ref() == Some(&id) {
                                                                expanded_agent.set(None);
                                                                agent_detail.set(None);
                                                            } else {
                                                                expanded_agent.set(Some(id.clone()));
                                                                wasm_bindgen_futures::spawn_local(async move {
                                                                    if let Ok(detail) = api::get_run_detail(&id).await {
                                                                        agent_detail.set(Some(detail));
                                                                    }
                                                                });
                                                            }
                                                        }
                                                    >
                                                        <div style="display:flex;align-items:center;gap:8px">
                                                            <span class=a_class style="font-size:11px">{a_status}</span>
                                                            <span style="font-size:13px;font-weight:500">{a_agent}</span>
                                                            <span style="font-size:12px;color:var(--muted)">{a_model}</span>
                                                            <span style="margin-left:auto;font-size:12px;color:var(--muted)">{a_tokens}" tok · "{a_dur}</span>
                                                        </div>
                                                        {(!a_task.is_empty()).then(|| view! {
                                                            <div style="font-size:13px;color:var(--text-secondary);margin-top:4px">{a_task}</div>
                                                        })}
                                                        {a_error.clone().map(|err| view! {
                                                            <div style="font-size:12px;color:var(--error);margin-top:4px;background:var(--error-bg);padding:6px 10px;border-radius:var(--radius-sm)">{err}</div>
                                                        })}
                                                        {(!a_files.is_empty()).then(|| {
                                                            let files_str = a_files.join(", ");
                                                            view! {
                                                                <div style="font-size:12px;color:var(--muted);margin-top:4px">"Files: "{files_str}</div>
                                                            }
                                                        })}

                                                        // Expanded detail with diff
                                                        {move || {
                                                            let is_this = expanded_agent.get().as_ref() == Some(&aid);
                                                            is_this.then(|| {
                                                                agent_detail.get().map(|d| {
                                                                    let diff = d.diff.clone().unwrap_or_default();
                                                                    let has_pr = diff.starts_with("PR: ");
                                                                    let pr_url = if has_pr { diff[4..].to_string() } else { String::new() };
                                                                    view! {
                                                                        <div style="margin-top:8px;border-top:1px solid var(--border);padding-top:8px">
                                                                            {has_pr.then(|| view! {
                                                                                <a href=pr_url target="_blank" class="btn btn-primary" style="font-size:13px;text-decoration:none;margin-bottom:8px;display:inline-flex">"View PR on GitHub →"</a>
                                                                            })}
                                                                            {(!has_pr && !diff.is_empty()).then(|| {
                                                                                let diff_display = if diff.len() > 2000 { format!("{}...", &diff[..2000]) } else { diff.clone() };
                                                                                view! {
                                                                                    <div>
                                                                                        <div style="font-size:12px;font-weight:500;color:var(--muted);margin-bottom:4px">"Diff"</div>
                                                                                        <pre style="background:var(--bg-secondary);border:1px solid var(--border);border-radius:var(--radius-sm);padding:10px;font-size:11px;overflow-x:auto;max-height:300px;color:var(--text-secondary)">{diff_display}</pre>
                                                                                    </div>
                                                                                }
                                                                            })}
                                                                        </div>
                                                                    }
                                                                })
                                                            }).flatten()
                                                        }}
                                                    </div>
                                                }
                                            }).collect_view()}
                                        </div>
                                    }
                                })}
                            </div>
                        }
                    }).collect::<Vec<_>>()
                }).collect_view().into_any()
            }}
        </div>
    }
}
