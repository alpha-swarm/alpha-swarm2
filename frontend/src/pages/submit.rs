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

    // Load projects for dropdown
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

        wasm_bindgen_futures::spawn_local(async move {
            let _ = api::submit_task(&api::SubmitTask {
                task,
                project: project_clone.clone(),
                files: vec![],
            }).await;
            task_text.set(String::new());
            submitting.set(false);
            let nav = use_navigate();
            nav(&format!("/project/{project_clone}"), Default::default());
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
            <div style="padding:12px 0;font-size:13px;color:var(--muted)">
                "The orchestrator will use the most capable model to plan, then dispatch smaller specialized agents for each sub-task automatically."
            </div>
            <button
                class="btn btn-primary"
                style="width:100%;justify-content:center;padding:12px"
                on:click=on_submit
                disabled=move || submitting.get()
            >
                {move || if submitting.get() { "Submitting..." } else { "Submit Goal" }}
            </button>
        </div>
    }
}
