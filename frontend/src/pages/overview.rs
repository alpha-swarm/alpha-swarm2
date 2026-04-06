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
    let clearing = RwSignal::new(false);

    let on_clear = move |_| {
        if clearing.get() { return; }
        let window = web_sys::window().unwrap();
        if !window.confirm_with_message("Clear ALL projects and agent runs?").unwrap_or(false) {
            return;
        }
        clearing.set(true);
        wasm_bindgen_futures::spawn_local(async move {
            match api::clear_all().await {
                Ok(_) => {
                    state.projects.set(vec![]);
                    state.recent_activity.set(vec![]);
                    state.live_agents.set(vec![]);
                    state.active_count.set(0);
                    web_sys::console::log_1(&"All data cleared".into());
                }
                Err(e) => {
                    web_sys::console::error_1(&format!("Clear failed: {e}").into());
                }
            }
            clearing.set(false);
        });
    };

    view! {
        <div style="display:flex;align-items:center;gap:12px;margin-bottom:4px">
            <h1 style="flex:1">"Overview"</h1>
            <button class="btn" style="color:var(--error);font-size:13px" on:click=on_clear>
                {move || if clearing.get() { "Clearing..." } else { "Clear All Data" }}
            </button>
        </div>
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
