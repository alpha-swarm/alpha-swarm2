use leptos::prelude::*;
use crate::state::AppState;
use crate::api;
use crate::types::*;
use crate::components::badge::StatusBadge;
use crate::components::agent_card::AgentRow;
use crate::components::detail_panel::RunDetailPanel;

const GOALS_PER_PAGE: usize = 10;

#[derive(Clone, Copy, PartialEq)]
enum SortField {
    Status,
    Project,
    Agents,
}

const SORT_OPTIONS: &[(SortField, &str)] = &[
    (SortField::Status, "Status"),
    (SortField::Project, "Project"),
    (SortField::Agents, "Agent Count"),
];

/// Priority for status sorting — running first
fn status_priority(status: &str) -> u8 {
    match status {
        "running" | "partial" | "planning" => 0,
        "failed" => 1,
        "passed" => 2,
        _ => 3,
    }
}

#[component]
pub fn AgentsPage() -> impl IntoView {
    let state = expect_context::<AppState>();
    let all_goals = RwSignal::new(Vec::<(String, Goal)>::new());
    let selected_run = RwSignal::new(Option::<AgentRun>::None);
    let sort_by = RwSignal::new(SortField::Status);
    let sort_asc = RwSignal::new(true);
    let page = RwSignal::new(0usize);

    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(projects) = api::list_projects().await {
            let mut result = Vec::new();
            for p in &projects {
                if let Ok(goals) = api::get_goals(&p.name).await {
                    for g in goals {
                        result.push((p.name.clone(), g));
                    }
                }
            }
            all_goals.set(result);
            state.projects.set(projects);
        }
    });

    let sorted_goals = move || {
        let mut goals = all_goals.get();
        let field = sort_by.get();
        let asc = sort_asc.get();

        goals.sort_by(|(pa, ga), (pb, gb)| {
            let ord = match field {
                SortField::Status => status_priority(&ga.status).cmp(&status_priority(&gb.status)),
                SortField::Project => pa.cmp(pb),
                SortField::Agents => ga.total.cmp(&gb.total),
            };
            if asc { ord } else { ord.reverse() }
        });
        goals
    };

    let total_pages = move || {
        let len = sorted_goals().len();
        if len == 0 { 1 } else { (len + GOALS_PER_PAGE - 1) / GOALS_PER_PAGE }
    };

    let paged_goals = move || {
        let goals = sorted_goals();
        let start = page.get() * GOALS_PER_PAGE;
        goals.into_iter().skip(start).take(GOALS_PER_PAGE).collect::<Vec<_>>()
    };

    view! {
        <h1>"Agents"</h1>
        <p class="subtitle">"Hierarchical view — goals and their sub-agents"</p>

        // Sort controls
        <div style="display:flex;align-items:center;gap:8px;margin-bottom:16px;font-size:13px">
            <span style="color:var(--muted)">"Sort by:"</span>
            {SORT_OPTIONS.iter().map(|(field, label)| {
                let f = *field;
                let l = *label;
                view! {
                    <button
                        class="btn"
                        style=move || {
                            if sort_by.get() == f {
                                "font-size:13px;font-weight:600;color:var(--accent)"
                            } else {
                                "font-size:13px"
                            }
                        }
                        on:click=move |_| {
                            if sort_by.get() == f {
                                sort_asc.set(!sort_asc.get());
                            } else {
                                sort_by.set(f);
                                sort_asc.set(true);
                            }
                            page.set(0);
                        }
                    >
                        {l}
                        {move || if sort_by.get() == f { if sort_asc.get() { " ↑" } else { " ↓" } } else { "" }}
                    </button>
                }
            }).collect_view()}
        </div>

        <div style="display:flex;gap:16px">
            <div style="flex:1;min-width:0">
                {move || {
                    let goals = paged_goals();
                    if goals.is_empty() {
                        return view! { <p class="empty">"No agent activity yet"</p> }.into_any();
                    }
                    goals.into_iter().map(|(project, goal)| {
                        view! { <GoalTree project=project goal=goal selected_run=selected_run /> }
                    }).collect_view().into_any()
                }}

                // Pagination
                {move || {
                    let tp = total_pages();
                    let _tp = tp;
                    (tp > 1).then(|| view! {
                        <div style="display:flex;align-items:center;justify-content:center;gap:8px;margin-top:16px;font-size:13px">
                            <button
                                class="btn"
                                style="padding:4px 10px"
                                disabled=move || page.get() == 0
                                on:click=move |_| page.set(page.get().saturating_sub(1))
                            >"← Prev"</button>
                            <span style="color:var(--muted)">
                                "Page "{move || page.get() + 1}" of "{tp}
                            </span>
                            <button
                                class="btn"
                                style="padding:4px 10px"
                                disabled=move || page.get() + 1 >= tp
                                on:click=move |_| page.set(page.get() + 1)
                            >"Next →"</button>
                        </div>
                    })
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
