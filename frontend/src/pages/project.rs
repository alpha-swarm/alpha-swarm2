use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_params_map};
use crate::api;
use crate::types::*;
use crate::components::card::StatCard;
use crate::components::badge::StatusBadge;
use crate::components::kanban::KanbanBoard;
use crate::components::detail_panel::RunDetailPanel;

#[component]
pub fn ProjectDetailPage() -> impl IntoView {
    let params = use_params_map();
    let name = move || params.read().get("name").unwrap_or_default();

    let metrics = RwSignal::new(ProjectMetrics::default());
    let goals = RwSignal::new(Vec::<Goal>::new());
    let runs = RwSignal::new(Vec::<AgentRun>::new());
    let run_detail = RwSignal::new(Option::<AgentRun>::None);

    let pn = name();
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(m) = api::get_metrics(&pn).await { metrics.set(m); }
        if let Ok(g) = api::get_goals(&pn).await { goals.set(g); }
        if let Ok(r) = api::list_runs(&pn).await { runs.set(r); }
    });

    let on_delete = {
        let pn = name();
        move |_| {
            let pn = pn.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = api::delete_project(&pn).await;
                use_navigate()("/projects", Default::default());
            });
        }
    };

    let rerun_project = name();

    let pass_rate = Signal::derive(move || format!("{}%", (metrics.get().pass_rate * 100.0) as u32));
    let total_runs = Signal::derive(move || metrics.get().total_runs.to_string());
    let avg_dur = Signal::derive(move || crate::types::format_duration(metrics.get().avg_duration_ms));

    view! {
        <div style="display:flex;align-items:center;gap:12px;margin-bottom:4px">
            <a href="/projects" class="btn" style="padding:4px 8px;text-decoration:none">"←"</a>
            <h1 style="flex:1">{name}</h1>
            <a href="/submit" class="btn btn-primary" style="font-size:13px;text-decoration:none">"Submit Task"</a>
            <button class="btn" style="color:var(--error);font-size:13px" on:click=on_delete>"Delete"</button>
        </div>

        <div class="grid grid-3" style="margin:20px 0">
            <StatCard title="Pass Rate" value=pass_rate />
            <StatCard title="Total Runs" value=total_runs />
            <StatCard title="Avg Duration" value=avg_dur />
        </div>

        <h2 style="font-size:15px;font-weight:600;margin:28px 0 12px">"Task Board"</h2>
        <div style="display:flex;gap:16px;overflow-x:auto;min-height:150px">
            {move || view! { <KanbanBoard goals=goals.get() rerun_project=rerun_project.clone() run_detail=run_detail /> }}
        </div>

        <h2 style="font-size:15px;font-weight:600;margin:28px 0 12px">"Run History"</h2>
        <div style="display:flex;gap:16px">
            <div style="flex:1;min-width:0">
                <RunTable runs=runs run_detail=run_detail />
            </div>
            {move || run_detail.get().map(|d| view! {
                <RunDetailPanel run=d on_close=move || run_detail.set(None) />
            })}
        </div>
    }
}

#[component]
fn RunTable(runs: RwSignal<Vec<AgentRun>>, run_detail: RwSignal<Option<AgentRun>>) -> impl IntoView {
    move || {
        let list = runs.get();
        if list.is_empty() {
            return view! { <p class="empty">"No runs yet"</p> }.into_any();
        }
        view! {
            <table style="width:100%;border-collapse:collapse">
                <tr>
                    {["Status","Model","Task","Tokens","Duration"].into_iter().map(|h| view! {
                        <th style="text-align:left;padding:10px 12px;font-size:12px;color:var(--muted);border-bottom:1px solid var(--border)">{h}</th>
                    }).collect_view()}
                </tr>
                {list.into_iter().map(|r| {
                    let rc = r.clone();
                    let st = r.status.clone();
                    let model = r.model_used.clone();
                    let task = if r.task_description.len() > crate::types::TASK_PREVIEW_CHARS { format!("{}...", &r.task_description[..crate::types::TASK_PREVIEW_CHARS]) } else { r.task_description.clone() };
                    let tokens = r.tokens_output;
                    let dur = r.duration_human();
                    view! {
                        <tr style="cursor:pointer" on:click=move |_| {
                            let rc = rc.clone();
                            let id = rc.id.clone().unwrap_or_default();
                            run_detail.set(Some(rc));
                            wasm_bindgen_futures::spawn_local(async move {
                                if let Ok(d) = api::get_run_detail(&id).await { run_detail.set(Some(d)); }
                            });
                        }>
                            <td style="padding:12px;border-bottom:1px solid var(--border)"><StatusBadge status=st /></td>
                            <td style="padding:12px;font-size:13px;border-bottom:1px solid var(--border)">{model}</td>
                            <td style="padding:12px;font-size:13px;border-bottom:1px solid var(--border)">{task}</td>
                            <td style="padding:12px;font-size:13px;border-bottom:1px solid var(--border)">{tokens}</td>
                            <td style="padding:12px;font-size:13px;border-bottom:1px solid var(--border)">{dur}</td>
                        </tr>
                    }
                }).collect_view()}
            </table>
        }.into_any()
    }
}
