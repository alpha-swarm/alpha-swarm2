use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use crate::api;
use crate::types::*;

#[component]
pub fn PlanReviewPage() -> impl IntoView {
    let params = use_params_map();
    let run_id = move || params.read().get("id").unwrap_or_default();

    let plans = RwSignal::new(Vec::<GoalPlan>::new());
    let run = RwSignal::new(Option::<AgentRun>::None);
    let feedback_text = RwSignal::new(String::new());
    let submitting = RwSignal::new(false);

    let rid = run_id();
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(p) = api::get_plans(&rid).await { plans.set(p); }
        if let Ok(d) = api::get_run_detail(&rid).await { run.set(Some(d)); }
    });

    let on_refine = {
        let rid = run_id();
        move |_| {
            let fb = feedback_text.get();
            if fb.is_empty() { return; }
            submitting.set(true);
            let rid = rid.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = api::send_plan_feedback(&rid, &fb).await;
                feedback_text.set(String::new());
                // Reload plans after a moment
                tokio_sleep_ms(2000).await;
                if let Ok(p) = api::get_plans(&rid).await { plans.set(p); }
                if let Ok(d) = api::get_run_detail(&rid).await { run.set(Some(d)); }
                submitting.set(false);
            });
        }
    };

    let on_approve = {
        let rid = run_id();
        move |_| {
            submitting.set(true);
            let rid = rid.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = api::approve_plan(&rid).await;
                if let Ok(d) = api::get_run_detail(&rid).await { run.set(Some(d)); }
                submitting.set(false);
            });
        }
    };

    view! {
        <div style="display:flex;align-items:center;gap:12px;margin-bottom:4px">
            <a href="/projects" class="btn" style="padding:4px 8px;text-decoration:none">"←"</a>
            <h1 style="flex:1">"Plan Review"</h1>
        </div>

        // Run info
        {move || run.get().map(|r| {
            let status = r.status.clone();
            let goal = r.task_description.clone();
            let progress = r.progress_message.clone();
            view! {
                <div class="card" style="margin-bottom:16px">
                    <div style="display:flex;align-items:center;gap:8px;margin-bottom:8px">
                        <span class=format!("badge {status}")>{status.clone()}</span>
                        {progress.map(|p| view! { <span style="font-size:13px;color:var(--accent);font-style:italic">{p}</span> })}
                    </div>
                    <div style="font-size:15px;font-weight:600">{goal}</div>
                </div>
            }
        })}

        // Plan versions
        {move || {
            let plan_list = plans.get();
            if plan_list.is_empty() {
                return view! { <p class="empty">"No plan generated yet — waiting for the orchestrator..."</p> }.into_any();
            }

            plan_list.into_iter().rev().map(|plan| {
                let version = plan.version;
                let model = plan.model_used.clone();
                let reasoning = plan.reasoning.clone();
                let tasks = plan.sub_tasks.clone();
                let feedback = plan.user_feedback.clone();
                let dur = format_duration(plan.duration_ms);
                let files_count = plan.context_files.len();
                let status = plan.status.clone();

                view! {
                    <div class="card" style="margin-bottom:12px">
                        <div style="display:flex;align-items:center;gap:8px;margin-bottom:8px">
                            <span style="font-size:14px;font-weight:600">"Plan v"{version}</span>
                            <span class=format!("badge {status}")>{status.clone()}</span>
                            <span style="font-size:12px;color:var(--muted)">{model}" · "{dur}" · "{files_count}" files analyzed"</span>
                        </div>

                        {(!reasoning.is_empty()).then(|| view! {
                            <div style="font-size:13px;color:var(--text-secondary);margin-bottom:8px;font-style:italic">{reasoning}</div>
                        })}

                        {feedback.map(|fb| view! {
                            <div style="font-size:12px;color:var(--accent);margin-bottom:8px;padding:6px 10px;background:var(--accent-bg);border-radius:var(--radius-sm)">
                                "Feedback: "{fb}
                            </div>
                        })}

                        // Sub-tasks table
                        <table style="width:100%;border-collapse:collapse;font-size:13px">
                            <tr>
                                {["#", "Description", "Files", "Complexity"].into_iter().map(|h| view! {
                                    <th style="text-align:left;padding:6px 8px;color:var(--muted);border-bottom:1px solid var(--border);font-weight:500">{h}</th>
                                }).collect_view()}
                            </tr>
                            {tasks.into_iter().map(|t| {
                                let files = t.files.join(", ");
                                view! {
                                    <tr>
                                        <td style="padding:6px 8px;border-bottom:1px solid var(--border);font-weight:500">{t.id}</td>
                                        <td style="padding:6px 8px;border-bottom:1px solid var(--border)">{t.description}</td>
                                        <td style="padding:6px 8px;border-bottom:1px solid var(--border);font-size:12px;color:var(--muted)">{files}</td>
                                        <td style="padding:6px 8px;border-bottom:1px solid var(--border)">{t.complexity}</td>
                                    </tr>
                                }
                            }).collect_view()}
                        </table>
                    </div>
                }
            }).collect_view().into_any()
        }}

        // Actions (only if status is 'planned')
        {move || {
            let status = run.get().map(|r| r.status.clone()).unwrap_or_default();
            (status == "planned").then(|| view! {
                <div class="card" style="max-width:600px">
                    <div style="margin-bottom:12px">
                        <label style="display:block;font-size:13px;font-weight:500;color:var(--text-secondary);margin-bottom:4px">"Feedback (optional)"</label>
                        <textarea
                            class="input"
                            style="min-height:60px"
                            prop:value=move || feedback_text.get()
                            on:input=move |ev| feedback_text.set(event_target_value(&ev))
                            placeholder="Remove the frontend tasks, focus on backend only..."
                        ></textarea>
                    </div>
                    <div style="display:flex;gap:8px">
                        <button
                            class="btn"
                            style="flex:1;justify-content:center;padding:10px"
                            on:click=on_refine.clone()
                            disabled=move || submitting.get() || feedback_text.get().is_empty()
                        >
                            {move || if submitting.get() { "Refining..." } else { "Refine Plan" }}
                        </button>
                        <button
                            class="btn btn-primary"
                            style="flex:1;justify-content:center;padding:10px"
                            on:click=on_approve.clone()
                            disabled=move || submitting.get()
                        >
                            {move || if submitting.get() { "Approving..." } else { "Approve & Run" }}
                        </button>
                    </div>
                </div>
            })
        }}

        // If approved/running, show status
        {move || {
            let status = run.get().map(|r| r.status.clone()).unwrap_or_default();
            (status == "approved" || status == "running").then(|| {
                let project = run.get().map(|r| r.project.clone()).unwrap_or_default();
                view! {
                    <div style="margin-top:16px;font-size:14px;color:var(--accent)">
                        "Plan approved — agents are executing. "
                        <a href=format!("/project/{project}") style="text-decoration:underline">"View progress →"</a>
                    </div>
                }
            })
        }}
    }
}

async fn tokio_sleep_ms(ms: u64) {
    wasm_bindgen_futures::JsFuture::from(
        js_sys::Promise::new(&mut |resolve, _| {
            let _ = web_sys::window().unwrap().set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32);
        })
    ).await.ok();
}
