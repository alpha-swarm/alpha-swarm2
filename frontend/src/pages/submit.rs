use leptos::prelude::*;
use leptos_router::hooks::use_navigate;
use crate::state::AppState;
use crate::api;

#[component]
pub fn SubmitPage() -> impl IntoView {
    let state = expect_context::<AppState>();
    let task_text = RwSignal::new(String::new());
    let selected_project = RwSignal::new(String::new());
    let submitting = RwSignal::new(false);
    let plan_first = RwSignal::new(true);
    let error_msg = RwSignal::new(Option::<String>::None);
    let nav = use_navigate();

    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(projects) = api::list_projects().await {
            if let Some(first) = projects.first() {
                selected_project.set(first.name.clone());
            }
            state.projects.set(projects);
        }
    });

    let on_submit = move |_| {
        let task = task_text.get();
        let project = selected_project.get();
        if task.is_empty() || project.is_empty() { return; }

        submitting.set(true);
        let project_clone = project.clone();
        let use_plan = plan_first.get();

        let nav = nav.clone();
        error_msg.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            let submit_task = api::SubmitTask {
                task,
                project: project_clone.clone(),
                files: vec![],
            };
            let result = if use_plan {
                api::submit_plan(&submit_task).await
            } else {
                api::submit_task(&submit_task).await
            };
            match result {
                Ok(resp) => {
                    task_text.set(String::new());
                    // For plan-first: navigate to plan review page if we got a run_id
                    let run_id = resp.get("run_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if use_plan && !run_id.is_empty() {
                        nav(&format!("/plan/{run_id}"), Default::default());
                    } else {
                        nav(&format!("/project/{project_clone}"), Default::default());
                    }
                }
                Err(e) => {
                    error_msg.set(Some(format!("Submit failed: {e}")));
                }
            }
            submitting.set(false);
        });
    };

    view! {
        <h1>"Submit Task"</h1>
        <p class="subtitle">"Describe what you want done — the system picks the best model and creates sub-agents as needed"</p>
        <div class="card" style="max-width:600px">
            <div style="margin-bottom:16px">
                <label style="display:block;font-size:13px;font-weight:500;color:var(--text-secondary);margin-bottom:6px">"What do you want to do?"</label>
                <textarea
                    class="input"
                    style="min-height:100px"
                    prop:value=move || task_text.get()
                    on:input=move |ev| task_text.set(event_target_value(&ev))
                    placeholder="Add error handling to all public functions in the auth module"
                ></textarea>
            </div>
            <div style="margin-bottom:16px">
                <label style="display:block;font-size:13px;font-weight:500;color:var(--text-secondary);margin-bottom:6px">"Project"</label>
                <select
                    class="input"
                    prop:value=move || selected_project.get()
                    on:change=move |ev| selected_project.set(event_target_value(&ev))
                >
                    {move || state.projects.get().into_iter().map(|p| {
                        let name = p.name.clone();
                        let display = format!("{}{}", p.name, if p.repo_url.is_empty() { String::new() } else { format!(" — {}", p.repo_url.replace("https://github.com/", "").replace(".git", "")) });
                        view! { <option value=name>{display}</option> }
                    }).collect_view()}
                </select>
            </div>

            // Plan first toggle
            <div style="margin-bottom:16px;display:flex;align-items:center;gap:8px">
                <input
                    type="checkbox"
                    prop:checked=move || plan_first.get()
                    on:change=move |ev| plan_first.set(event_target_checked(&ev))
                    style="width:16px;height:16px"
                />
                <label style="font-size:13px;font-weight:500;color:var(--text-secondary)">
                    "Plan first"
                    <span style="font-weight:400;color:var(--muted)">" — review the plan before agents start executing"</span>
                </label>
            </div>

            {move || error_msg.get().map(|e| view! {
                <div style="color:var(--error);background:var(--error-bg);padding:10px 14px;border-radius:var(--radius-sm);font-size:13px;margin-bottom:8px">{e}</div>
            })}

            <div style="padding:12px 0;font-size:13px;color:var(--muted)">
                {move || if plan_first.get() {
                    "The orchestrator will generate a plan for your review. You can refine it with feedback before approving."
                } else {
                    "The orchestrator will plan and execute immediately. Agents will start working right away."
                }}
            </div>
            <button
                class="btn btn-primary"
                style="width:100%;justify-content:center;padding:12px"
                on:click=on_submit
                disabled=move || submitting.get()
            >
                {move || if submitting.get() { "Submitting..." } else if plan_first.get() { "Submit for Planning" } else { "Submit & Execute" }}
            </button>
        </div>
    }
}
