use leptos::prelude::*;
use crate::state::AppState;

#[component]
pub fn ProjectsPage() -> impl IntoView {
    let state = expect_context::<AppState>();
    let projects = move || state.projects.get();

    view! {
        <h1>"Projects"</h1>
        <p class="subtitle">"Manage and monitor your projects"</p>
        <div class="grid grid-2">
            {move || projects().into_iter().map(|p| view! {
                <a href={format!("/project/{}", p.name)} class="card card-clickable">
                    <h3 style="text-transform:none;font-size:16px;font-weight:600">{&p.name}</h3>
                    <div style="font-size:13px;color:var(--accent);font-family:monospace">{&p.repo_url}</div>
                    <div style="font-size:13px;color:var(--muted)">{&p.description}</div>
                </a>
            }).collect_view()}
        </div>
    }
}
