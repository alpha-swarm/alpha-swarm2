use leptos::prelude::*;
use crate::types::AgentRun;
use super::badge::StatusBadge;

#[component]
pub fn AgentRow(run: AgentRun) -> impl IntoView {
    let status = run.status.clone();
    let agent_id = run.agent_id.clone();
    let model = run.model_used.clone();
    let dur = run.duration_human();
    let tokens = run.tokens_output;
    let task = run.task_description.clone();
    let error = run.error_message.clone();
    let files = run.files_modified.clone();

    view! {
        <div class="card" style="margin-bottom:8px;padding:10px 14px">
            <div style="display:flex;align-items:center;gap:8px">
                <StatusBadge status=status />
                <span style="font-size:13px;font-weight:500">{agent_id}</span>
                <span style="font-size:12px;color:var(--muted)">{model}</span>
                <span style="margin-left:auto;font-size:12px;color:var(--muted)">{tokens}" tok · "{dur}</span>
            </div>
            {(!task.is_empty()).then(|| view! {
                <div style="font-size:13px;color:var(--text-secondary);margin-top:4px">{task}</div>
            })}
            {error.map(|e| view! {
                <div style="font-size:12px;color:var(--error);margin-top:4px;background:var(--error-bg);padding:6px 10px;border-radius:var(--radius-sm)">{e}</div>
            })}
            {(!files.is_empty()).then(|| {
                let f = files.join(", ");
                view! { <div style="font-size:12px;color:var(--muted);margin-top:4px">"Files: "{f}</div> }
            })}
        </div>
    }
}

#[component]
pub fn ActivityCard(run: AgentRun) -> impl IntoView {
    let status = run.status.clone();
    let limit = crate::types::TASK_PREVIEW_CHARS + 20; // activity cards get more room
    let task = if run.task_description.len() > limit { format!("{}...", &run.task_description[..limit]) } else { run.task_description.clone() };
    let model = run.model_used.clone();
    let tokens = run.tokens_output;
    let dur = run.duration_human();

    view! {
        <div class="card" style="margin-bottom:8px;padding:12px 16px">
            <StatusBadge status=status />
            <span style="margin-left:8px;font-size:14px;font-weight:500">{task}</span>
            <div style="font-size:13px;color:var(--muted);margin-top:4px">
                {model}" · "{tokens}" tokens · "{dur}
            </div>
        </div>
    }
}
