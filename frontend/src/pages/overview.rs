use leptos::prelude::*;
use crate::state::AppState;

#[component]
pub fn OverviewPage() -> impl IntoView {
    let state = expect_context::<AppState>();

    view! {
        <h1>"Overview"</h1>
        <p class="subtitle">"System status and recent activity"</p>
        <div class="grid grid-3">
            <div class="card">
                <h3>"System"</h3>
                <div class="value">{move || if state.health_online.get() { "Online" } else { "Offline" }}</div>
            </div>
            <div class="card">
                <h3>"Active Agents"</h3>
                <div class="value">{move || state.active_count.get()}</div>
            </div>
            <div class="card">
                <h3>"Models"</h3>
                <div class="value">{move || state.models.get().len()}</div>
            </div>
        </div>
    }
}
