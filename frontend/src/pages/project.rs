use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

#[component]
pub fn ProjectDetailPage() -> impl IntoView {
    let params = use_params_map();
    let name = move || params.get().get("name").unwrap_or_default();

    view! {
        <h1>{name}</h1>
        <p class="subtitle">"Project detail — kanban + run history"</p>
        <p>"TODO: kanban board, metrics, run history"</p>
    }
}
