use leptos::prelude::*;
use crate::state::AppState;
use crate::api;

#[component]
pub fn ProjectsPage() -> impl IntoView {
    let state = expect_context::<AppState>();

    // Load projects on mount
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(projects) = api::list_projects().await {
            state.projects.set(projects);
        }
    });

    view! {
        <h1>"Projects"</h1>
        <p class="subtitle">"Manage and monitor your projects"</p>
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
