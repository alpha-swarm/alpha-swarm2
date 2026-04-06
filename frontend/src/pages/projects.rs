use leptos::prelude::*;
use crate::state::AppState;
use crate::api;
use crate::types::Project;

#[component]
pub fn ProjectsPage() -> impl IntoView {
    let state = expect_context::<AppState>();
    let show_form = RwSignal::new(false);

    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(projects) = api::list_projects().await {
            state.projects.set(projects);
        }
    });

    view! {
        <div style="display:flex;align-items:center;gap:12px;margin-bottom:4px">
            <h1 style="flex:1">"Projects"</h1>
            <button
                class="btn btn-primary"
                style="font-size:13px"
                on:click=move |_| show_form.set(!show_form.get())
            >
                {move || if show_form.get() { "Cancel" } else { "+ New Project" }}
            </button>
        </div>
        <p class="subtitle">"Manage and monitor your projects"</p>

        {move || show_form.get().then(|| view! { <CreateProjectForm on_created=move || show_form.set(false) /> })}

        <div class="grid grid-2">
            {move || state.projects.get().into_iter().map(|p| {
                let href = format!("/project/{}", p.name);
                let name = p.name.clone();
                let repo = p.repo_url.clone();
                let desc = p.description.clone();
                view! {
                    <a href=href class="card card-clickable">
                        <h3 style="text-transform:none;font-size:16px;font-weight:600">{name}</h3>
                        <div style="font-size:13px;color:var(--accent);font-family:monospace">{repo}</div>
                        <div style="font-size:13px;color:var(--muted)">{desc}</div>
                    </a>
                }
            }).collect_view()}
        </div>
    }
}

#[component]
fn CreateProjectForm(on_created: impl Fn() + Clone + 'static) -> impl IntoView {
    let state = expect_context::<AppState>();
    let name = RwSignal::new(String::new());
    let repo_url = RwSignal::new(String::new());
    let branch = RwSignal::new("main".to_string());
    let description = RwSignal::new(String::new());
    let submitting = RwSignal::new(false);
    let error_msg = RwSignal::new(Option::<String>::None);

    let on_submit = {
        let on_created = on_created.clone();
        move |_| {
            let n = name.get();
            let r = repo_url.get();
            if n.is_empty() || r.is_empty() {
                error_msg.set(Some("Name and repo URL are required".into()));
                return;
            }
            error_msg.set(None);
            submitting.set(true);

            let project = Project {
                id: None,
                name: n,
                repo_url: r,
                branch: branch.get(),
                description: description.get(),
                status: "active".into(),
                created_at: None,
            };

            let on_created = on_created.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match api::create_project(&project).await {
                    Ok(_) => {
                        if let Ok(projects) = api::list_projects().await {
                            state.projects.set(projects);
                        }
                        on_created();
                    }
                    Err(e) => {
                        error_msg.set(Some(e));
                    }
                }
                submitting.set(false);
            });
        }
    };

    view! {
        <div class="card" style="margin-bottom:20px;max-width:600px">
            <h3 style="font-size:15px;font-weight:600;margin-bottom:12px">"New Project"</h3>
            <FormField label="Name" value=name placeholder="my-project" />
            <FormField label="Repository URL" value=repo_url placeholder="https://github.com/user/repo.git" />
            <FormField label="Branch" value=branch placeholder="main" />
            <FormField label="Description" value=description placeholder="What this project does" />

            {move || error_msg.get().map(|e| view! {
                <div style="color:var(--error);font-size:13px;margin-bottom:8px">{e}</div>
            })}

            <button
                class="btn btn-primary"
                style="width:100%;justify-content:center;padding:10px;margin-top:4px"
                on:click=on_submit
                disabled=move || submitting.get()
            >
                {move || if submitting.get() { "Creating..." } else { "Create Project" }}
            </button>
        </div>
    }
}

#[component]
fn FormField(
    #[prop(into)] label: String,
    value: RwSignal<String>,
    #[prop(into)] placeholder: String,
) -> impl IntoView {
    view! {
        <div style="margin-bottom:12px">
            <label style="display:block;font-size:13px;font-weight:500;color:var(--text-secondary);margin-bottom:4px">{label}</label>
            <input
                class="input"
                prop:value=move || value.get()
                on:input=move |ev| value.set(event_target_value(&ev))
                placeholder=placeholder
            />
        </div>
    }
}
