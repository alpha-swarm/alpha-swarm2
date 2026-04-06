use leptos::prelude::*;
use crate::types::AgentRun;
use super::badge::StatusBadge;

#[component]
pub fn RunDetailPanel(
    run: AgentRun,
    on_close: impl Fn() + 'static,
) -> impl IntoView {
    let status = run.status.clone();
    let task = run.task_description.clone();
    let model = run.model_used.clone();
    let agent = run.agent_id.clone();
    let tokens_in = run.tokens_input;
    let tokens_out = run.tokens_output;
    let dur = format!("{:.1}s", run.duration_secs());
    let error = run.error_message.clone();
    let diff = run.diff.clone().unwrap_or_default();
    let has_pr = diff.starts_with("PR: ");
    let pr_url = if has_pr { diff[4..].to_string() } else { String::new() };
    let files = run.files_modified.clone();

    view! {
        <div style="width:380px;flex-shrink:0">
            <div class="card" style="position:sticky;top:20px">
                <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:12px">
                    <span style="font-size:15px;font-weight:600">"Run Detail"</span>
                    <button class="btn" style="padding:2px 8px" on:click=move |_| on_close()>"×"</button>
                </div>
                <StatusBadge status=status />
                <div style="font-size:14px;font-weight:500;margin:8px 0;line-height:1.4">{task}</div>
                <DetailRow label="Model" value=model />
                <DetailRow label="Agent" value=agent />
                <DetailRow label="Tokens" value=format!("{tokens_in} in / {tokens_out} out") />
                <DetailRow label="Duration" value=dur />
                {(!files.is_empty()).then(|| {
                    let f = files.join(", ");
                    view! { <DetailRow label="Files" value=f /> }
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
}

#[component]
fn DetailRow(#[prop(into)] label: String, #[prop(into)] value: String) -> impl IntoView {
    view! {
        <div style="display:flex;justify-content:space-between;padding:4px 0;font-size:13px">
            <span style="color:var(--muted)">{label}</span>
            <span style="font-weight:500;text-align:right;max-width:220px;word-break:break-all">{value}</span>
        </div>
    }
}
