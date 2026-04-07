use leptos::prelude::*;
use crate::types::{AgentRun, format_duration, format_relative_time, DIFF_PREVIEW_CHARS, ZOMBIE_ACTIVE_MS, ZOMBIE_WARNING_MS};
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
    let dur = run.duration_human();
    let error = run.error_message.clone();
    let diff = run.diff.clone().unwrap_or_default();
    let has_pr = diff.starts_with("PR: ");
    let pr_url = if has_pr { diff[4..].to_string() } else { String::new() };
    let files = run.files_modified.clone();
    let prompt = run.prompt_sent.clone();
    let response = run.response_text.clone();
    let attempts = run.attempts.clone();
    let tool_calls_list = run.tool_calls.clone();
    let started = run.started_at.clone();
    let last_active = run.last_activity_at.clone();
    let is_running = run.status == "running";

    view! {
        <div style="width:380px;flex-shrink:0">
            <div class="card" style="position:sticky;top:20px;max-height:90vh;overflow-y:auto">
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
                {started.map(|s| {
                    let rel = format_relative_time(&s);
                    view! { <DetailRow label="Started" value=rel /> }
                })}
                {last_active.map(|la| {
                    let rel = format_relative_time(&la);
                    let style = if is_running { "display:flex;justify-content:space-between;padding:4px 0;font-size:13px" } else { "display:flex;justify-content:space-between;padding:4px 0;font-size:13px" };
                    view! {
                        <div style=style>
                            <span style="color:var(--muted)">"Last Active"</span>
                            <LastActiveBadge time=rel is_running=is_running />
                        </div>
                    }
                })}
                {(!files.is_empty()).then(|| {
                    let f = files.join(", ");
                    view! { <DetailRow label="Files" value=f /> }
                })}
                {error.map(|e| view! {
                    <div style="margin-top:8px;background:var(--error-bg);color:var(--error);padding:8px 12px;border-radius:var(--radius-sm);font-size:13px">{e}</div>
                })}

                // Attempts timeline
                {(!attempts.is_empty()).then(|| {
                    let att = attempts.clone();
                    view! {
                        <details style="margin-top:8px" open>
                            <summary style="cursor:pointer;font-size:12px;color:var(--accent);font-weight:500">"Attempts ("{att.len()}")"</summary>
                            <div style="margin-top:6px">
                                {att.into_iter().map(|a| {
                                    let badge = match a.quality_passed {
                                        Some(true) => "badge passed",
                                        Some(false) => "badge failed",
                                        None => "badge running",
                                    };
                                    let dur = format_duration(a.duration_ms);
                                    let err = a.error.clone();
                                    view! {
                                        <div style="padding:6px 0;border-bottom:1px solid var(--border);font-size:12px">
                                            <div style="display:flex;align-items:center;gap:6px">
                                                <span style="font-weight:500">"#"{a.attempt}</span>
                                                <span class=badge style="font-size:10px;padding:1px 6px">
                                                    {match a.quality_passed { Some(true) => "pass", Some(false) => "fail", None => "—" }}
                                                </span>
                                                <span style="color:var(--muted)">{a.model}</span>
                                                <span style="margin-left:auto;color:var(--muted)">{dur}</span>
                                            </div>
                                            <div style="color:var(--muted);margin-top:2px">
                                                {a.tokens_input}" in / "{a.tokens_output}" out"
                                            </div>
                                            {err.map(|e| view! {
                                                <div style="color:var(--error);margin-top:2px">{e}</div>
                                            })}
                                        </div>
                                    }
                                }).collect_view()}
                            </div>
                        </details>
                    }
                })}

                // Tool calls section
                {(!tool_calls_list.is_empty()).then(|| {
                    let calls = tool_calls_list.clone();
                    view! {
                        <details style="margin-top:8px" open>
                            <summary style="cursor:pointer;font-size:12px;color:var(--accent);font-weight:500">"Tool Calls ("{calls.len()}")"</summary>
                            <div style="margin-top:6px">
                                {calls.into_iter().map(|tc| {
                                    let icon = if tc.is_error { "ERR" } else { "OK" };
                                    let dur = format_duration(tc.duration_ms);
                                    view! {
                                        <div style="padding:4px 0;border-bottom:1px solid var(--border);font-size:12px">
                                            <div style="display:flex;align-items:center;gap:6px">
                                                <span style="font-weight:600;color:var(--accent)">{tc.tool}</span>
                                                <span class=if tc.is_error { "badge failed" } else { "badge passed" } style="font-size:10px;padding:1px 6px">{icon}</span>
                                                <span style="margin-left:auto;color:var(--muted)">{dur}</span>
                                            </div>
                                            {(!tc.params_preview.is_empty()).then(|| view! {
                                                <div style="font-size:11px;color:var(--muted);margin-top:2px;font-family:monospace">{tc.params_preview}</div>
                                            })}
                                            {(!tc.result_preview.is_empty()).then(|| view! {
                                                <div style="font-size:11px;color:var(--text-secondary);margin-top:2px;max-height:60px;overflow:hidden">{tc.result_preview}</div>
                                            })}
                                        </div>
                                    }
                                }).collect_view()}
                            </div>
                        </details>
                    }
                })}

                // Prompt section
                {prompt.map(|p| {
                    let preview = if p.len() > DIFF_PREVIEW_CHARS { format!("{}...", &p[..DIFF_PREVIEW_CHARS]) } else { p };
                    view! {
                        <details style="margin-top:8px">
                            <summary style="cursor:pointer;font-size:12px;color:var(--accent);font-weight:500">"Show Prompt"</summary>
                            <pre style="background:var(--bg-secondary);border:1px solid var(--border);border-radius:var(--radius-sm);padding:10px;font-size:11px;overflow-x:auto;max-height:300px;color:var(--text-secondary);margin-top:4px;white-space:pre-wrap">{preview}</pre>
                        </details>
                    }
                })}

                // Response section
                {response.map(|r| {
                    let preview = if r.len() > DIFF_PREVIEW_CHARS { format!("{}...", &r[..DIFF_PREVIEW_CHARS]) } else { r };
                    view! {
                        <details style="margin-top:8px">
                            <summary style="cursor:pointer;font-size:12px;color:var(--accent);font-weight:500">"Show Response"</summary>
                            <pre style="background:var(--bg-secondary);border:1px solid var(--border);border-radius:var(--radius-sm);padding:10px;font-size:11px;overflow-x:auto;max-height:300px;color:var(--text-secondary);margin-top:4px;white-space:pre-wrap">{preview}</pre>
                        </details>
                    }
                })}

                {has_pr.then(|| view! {
                    <a href=pr_url target="_blank" class="btn btn-primary" style="margin-top:8px;text-decoration:none;font-size:13px">"View PR →"</a>
                })}
                {(!has_pr && !diff.is_empty()).then(|| {
                    let d = if diff.len() > DIFF_PREVIEW_CHARS { format!("{}...", &diff[..DIFF_PREVIEW_CHARS]) } else { diff.clone() };
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
fn LastActiveBadge(#[prop(into)] time: String, is_running: bool) -> impl IntoView {
    let active_threshold_m = ZOMBIE_ACTIVE_MS / 60_000;
    let warning_threshold_m = ZOMBIE_WARNING_MS / 60_000;
    let color = if !is_running {
        "var(--muted)"
    } else if time.contains("just now") || time.ends_with("m ago") {
        let mins: u64 = time.trim_end_matches("m ago").parse().unwrap_or(0);
        if mins <= active_threshold_m || time == "just now" { "var(--success)" }
        else if mins <= warning_threshold_m { "var(--warning)" }
        else { "var(--error)" }
    } else {
        "var(--error)" // hours/days ago = zombie
    };

    view! {
        <span style=format!("font-weight:500;color:{color}")>{time}</span>
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
