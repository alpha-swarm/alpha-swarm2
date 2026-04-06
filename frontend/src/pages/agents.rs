use leptos::prelude::*;
use crate::state::AppState;
use crate::api;
use crate::types::*;
use crate::components::badge::StatusBadge;
use crate::components::agent_card::AgentRow;
use crate::components::detail_panel::RunDetailPanel;

#[component]
pub fn AgentsPage() -> impl IntoView {
    let state = expect_context::<AppState>();
    let all_goals = RwSignal::new(Vec::<(String, Vec<Goal>)>::new());
    let selected_run = RwSignal::new(Option::<AgentRun>::None);

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
        <div style="display:flex;gap:16px">
            <div style="flex:1;min-width:0">
                {move || {
                    let goals = all_goals.get();
                    if goals.is_empty() {
                        return view! { <p class="empty">"No agent activity yet"</p> }.into_any();
                    }
                    goals.into_iter().flat_map(|(project, goals)| {
                        goals.into_iter().map(move |goal| {
                            let project = project.clone();
                            view! { <GoalTree project=project goal=goal selected_run=selected_run /> }
                        }).collect::<Vec<_>>()
                    }).collect_view().into_any()
                }}
            </div>
            {move || selected_run.get().map(|d| view! {
                <RunDetailPanel run=d on_close=move || selected_run.set(None) />
            })}
        </div>
    }
}

#[component]
fn GoalTree(
    #[prop(into)] project: String,
    goal: Goal,
    selected_run: RwSignal<Option<AgentRun>>,
) -> impl IntoView {
    let expanded = RwSignal::new(false);
    let status = goal.status.clone();
    let goal_text = goal.goal.clone();
    let agents = goal.agents.clone();
    let agent_count = goal.total;
    let passed = goal.passed;
    let failed = goal.failed;

    view! {
        <div class="card" style="margin-bottom:12px">
            <div style="cursor:pointer" on:click=move |_| expanded.set(!expanded.get())>
                <div style="display:flex;align-items:center;gap:8px;margin-bottom:4px">
                    <StatusBadge status=status.clone() />
                    <span style="font-size:13px;color:var(--muted)">{project}</span>
                </div>
                <div style="font-size:15px;font-weight:600;margin-bottom:4px">{goal_text}</div>
                <div style="font-size:13px;color:var(--muted)">
                    {agent_count}" agents"
                    {(passed > 0).then(|| format!(" · {} passed", passed))}
                    {(failed > 0).then(|| format!(" · {} failed", failed))}
                </div>
            </div>

            {move || expanded.get().then(|| {
                let agents = agents.clone();
                view! {
                    <div style="margin-top:12px;border-top:1px solid var(--border);padding-top:12px">
                        {agents.into_iter().map(|a| {
                            let ac = a.clone();
                            view! {
                                <div on:click=move |_| {
                                    let ac = ac.clone();
                                    let id = ac.id.clone().unwrap_or_default();
                                    selected_run.set(Some(ac));
                                    wasm_bindgen_futures::spawn_local(async move {
                                        if let Ok(detail) = api::get_run_detail(&id).await {
                                            selected_run.set(Some(detail));
                                        }
                                    });
                                }>
                                    <AgentRow run=a />
                                </div>
                            }
                        }).collect_view()}
                    </div>
                }
            })}
        </div>
    }
}
