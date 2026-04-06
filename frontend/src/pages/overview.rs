use leptos::prelude::*;
use crate::state::AppState;
use crate::api;
use crate::components::card::StatCard;
use crate::components::agent_card::ActivityCard;

#[component]
pub fn OverviewPage() -> impl IntoView {
    let state = expect_context::<AppState>();

    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(models) = api::list_models().await { state.models.set(models); }
        state.health_online.set(api::health().await.is_ok());
    });

    let status_text = Signal::derive(move || if state.health_online.get() { "Online".to_string() } else { "Offline".to_string() });
    let active = Signal::derive(move || state.active_count.get().to_string());
    let model_count = Signal::derive(move || state.models.get().len().to_string());

    view! {
        <h1>"Overview"</h1>
        <p class="subtitle">"System status and recent activity"</p>
        <div class="grid grid-3">
            <StatCard title="System" value=status_text label="wasmCloud 2.0" />
            <StatCard title="Active Agents" value=active label="currently running" />
            <StatCard title="Models" value=model_count label="available" />
        </div>
        <h2 style="font-size:15px;font-weight:600;margin:28px 0 12px">"Recent Activity"</h2>
        {move || {
            let activity = state.recent_activity.get();
            if activity.is_empty() {
                view! { <p class="empty">"No recent activity"</p> }.into_any()
            } else {
                activity.into_iter().map(|run| {
                    view! { <ActivityCard run=run /> }
                }).collect_view().into_any()
            }
        }}
    }
}
