use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use crate::api;
use crate::types::*;

/// Plan review as a conversation — user sees plan, types feedback, sees updated plan.
#[component]
pub fn PlanReviewPage() -> impl IntoView {
    let params = use_params_map();
    let run_id = move || params.read().get("id").unwrap_or_default();

    let plans = RwSignal::new(Vec::<GoalPlan>::new());
    let run = RwSignal::new(Option::<AgentRun>::None);
    let feedback_text = RwSignal::new(String::new());
    let submitting = RwSignal::new(false);
    let poll_count = RwSignal::new(0u32);

    // Initial load
    let rid = run_id();
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(p) = api::get_plans(&rid).await { plans.set(p); }
        if let Ok(d) = api::get_run_detail(&rid).await { run.set(Some(d)); }
    });

    // Auto-refresh while status is planning
    {
        let rid = run_id();
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                sleep_ms(5000).await;
                if let Ok(d) = api::get_run_detail(&rid).await {
                    let status = d.status.clone();
                    run.set(Some(d));
                    if status == "planned" || status == "approved" || status == "running" || status == "passed" || status == "failed" {
                        if let Ok(p) = api::get_plans(&rid).await { plans.set(p); }
                    }
                    if status != "planning" {
                        poll_count.set(poll_count.get() + 1);
                        if poll_count.get() > 60 { break; } // Stop after 5 min of non-planning
                    }
                }
            }
        });
    }

    let on_refine = {
        let rid = run_id();
        move |_| {
            let fb = feedback_text.get();
            if fb.is_empty() || submitting.get() { return; }
            submitting.set(true);
            let rid = rid.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = api::send_plan_feedback(&rid, &fb).await;
                feedback_text.set(String::new());
                submitting.set(false);
            });
        }
    };

    let on_approve = {
        let rid = run_id();
        move |_| {
            if submitting.get() { return; }
            submitting.set(true);
            let rid = rid.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = api::approve_plan(&rid).await;
                submitting.set(false);
            });
        }
    };

    view! {
        <div style="max-width:800px">
            <div style="display:flex;align-items:center;gap:12px;margin-bottom:16px">
                <a href="/projects" class="btn" style="padding:4px 8px;text-decoration:none">"back"</a>
                <h1 style="flex:1;font-size:20px">"Plan Review"</h1>
                {move || {
                    let status = run.get().map(|r| r.status.clone()).unwrap_or_default();
                    view! { <span class=format!("badge {status}")>{status.clone()}</span> }
                }}
            </div>

            // Goal description
            {move || run.get().map(|r| view! {
                <div class="card" style="margin-bottom:16px;padding:14px">
                    <div style="font-size:15px;font-weight:600">{r.task_description}</div>
                    {r.progress_message.map(|p| view! {
                        <div style="font-size:13px;color:var(--accent);font-style:italic;margin-top:4px">{p}</div>
                    })}
                </div>
            })}

            // Conversation: plan versions as chat messages
            <div style="display:flex;flex-direction:column;gap:12px;margin-bottom:16px">
                {move || {
                    let plan_list = plans.get();
                    if plan_list.is_empty() {
                        let status = run.get().map(|r| r.status.clone()).unwrap_or_default();
                        return if status == "planning" {
                            view! { <div class="card" style="padding:20px;text-align:center;color:var(--muted)"><div class="spinner"></div>"Generating plan..."</div> }.into_any()
                        } else {
                            view! { <div class="card" style="padding:20px;text-align:center;color:var(--muted)">"No plan yet"</div> }.into_any()
                        };
                    }

                    plan_list.into_iter().map(|plan| {
                        let is_user_feedback = plan.user_feedback.is_some();
                        let version = plan.version;
                        let model = plan.model_used.clone();
                        let reasoning = plan.reasoning.clone();
                        let tasks = plan.sub_tasks.clone();
                        let feedback = plan.user_feedback.clone();
                        let dur = format_duration(plan.duration_ms);
                        let status = plan.status.clone();

                        view! {
                            // User feedback message (if this version was triggered by feedback)
                            {feedback.map(|fb| view! {
                                <div style="display:flex;justify-content:flex-end">
                                    <div style="background:var(--accent-bg);color:var(--accent);padding:10px 14px;border-radius:12px 12px 2px 12px;max-width:70%;font-size:14px">
                                        {fb}
                                    </div>
                                </div>
                            })}

                            // Planner response
                            <div style="display:flex;justify-content:flex-start">
                                <div class="card" style="max-width:90%;padding:14px">
                                    <div style="display:flex;align-items:center;gap:8px;margin-bottom:8px;font-size:12px;color:var(--muted)">
                                        <span style="font-weight:600;color:var(--text)">"Plan v"{version}</span>
                                        <span>{model}</span>
                                        <span>{dur}</span>
                                        <span class=format!("badge {status}") style="font-size:10px">{status.clone()}</span>
                                    </div>

                                    {(!reasoning.is_empty()).then(|| view! {
                                        <div style="font-size:13px;color:var(--text-secondary);margin-bottom:10px">{reasoning}</div>
                                    })}

                                    // Sub-tasks table
                                    {(!tasks.is_empty()).then(|| {
                                        let tasks = tasks.clone();
                                        view! {
                                            <table style="width:100%;border-collapse:collapse;font-size:12px">
                                                <tr>
                                                    {["#", "Task", "Files", "Complexity"].into_iter().map(|h| view! {
                                                        <th style="text-align:left;padding:4px 6px;color:var(--muted);border-bottom:1px solid var(--border);font-weight:500">{h}</th>
                                                    }).collect_view()}
                                                </tr>
                                                {tasks.into_iter().map(|t| {
                                                    let files = t.files.join(", ");
                                                    view! {
                                                        <tr>
                                                            <td style="padding:4px 6px;border-bottom:1px solid var(--border);font-weight:500">{t.id}</td>
                                                            <td style="padding:4px 6px;border-bottom:1px solid var(--border)">{t.description}</td>
                                                            <td style="padding:4px 6px;border-bottom:1px solid var(--border);color:var(--muted);font-size:11px">{files}</td>
                                                            <td style="padding:4px 6px;border-bottom:1px solid var(--border)">{t.complexity}</td>
                                                        </tr>
                                                    }
                                                }).collect_view()}
                                            </table>
                                        }
                                    })}
                                </div>
                            </div>
                        }
                    }).collect_view().into_any()
                }}
            </div>

            // Input area — only when status is "planned"
            {move || {
                let status = run.get().map(|r| r.status.clone()).unwrap_or_default();
                match status.as_str() {
                    "planned" => view! {
                        <div style="border-top:1px solid var(--border);padding-top:16px">
                            <textarea
                                class="input"
                                style="min-height:60px;margin-bottom:8px"
                                prop:value=move || feedback_text.get()
                                on:input=move |ev| feedback_text.set(event_target_value(&ev))
                                placeholder="Type feedback to refine the plan, or approve to start..."
                            ></textarea>
                            <div style="display:flex;gap:8px">
                                <button
                                    class="btn"
                                    style="flex:1;justify-content:center;padding:10px"
                                    on:click=on_refine.clone()
                                    disabled=move || submitting.get() || feedback_text.get().is_empty()
                                >
                                    {move || if submitting.get() { "Sending..." } else { "Refine Plan" }}
                                </button>
                                <button
                                    class="btn btn-primary"
                                    style="flex:1;justify-content:center;padding:10px"
                                    on:click=on_approve.clone()
                                    disabled=move || submitting.get()
                                >
                                    "Approve and Run"
                                </button>
                            </div>
                        </div>
                    }.into_any(),
                    "planning" => view! {
                        <div style="text-align:center;padding:20px;color:var(--muted);font-style:italic">
                            "Planner is working..."
                        </div>
                    }.into_any(),
                    "approved" | "running" => {
                        let project = run.get().map(|r| r.project.clone()).unwrap_or_default();
                        view! {
                            <div style="text-align:center;padding:16px;color:var(--accent)">
                                "Plan approved — agents executing. "
                                <a href=format!("/project/{project}") style="text-decoration:underline">"View progress"</a>
                            </div>
                        }.into_any()
                    }
                    "passed" => view! {
                        <div style="text-align:center;padding:16px;color:var(--success)">"Task completed successfully."</div>
                    }.into_any(),
                    "failed" => {
                        let err = run.get().and_then(|r| r.error_message.clone()).unwrap_or_default();
                        view! {
                            <div style="text-align:center;padding:16px;color:var(--error)">"Task failed: "{err}</div>
                        }.into_any()
                    }
                    _ => view! { <div></div> }.into_any(),
                }
            }}
        </div>
    }
}

async fn sleep_ms(ms: u64) {
    wasm_bindgen_futures::JsFuture::from(
        js_sys::Promise::new(&mut |resolve, _| {
            let _ = web_sys::window().unwrap().set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32);
        })
    ).await.ok();
}
