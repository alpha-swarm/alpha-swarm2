use leptos::prelude::*;
use crate::state::AppState;
use crate::api;

#[component]
pub fn OverviewPage() -> impl IntoView {
    let state = expect_context::<AppState>();

    // Load initial data
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(models) = api::list_models().await {
            state.models.set(models);
        }
        state.health_online.set(api::health().await.is_ok());
    });

    view! {
        <h1>"Overview"</h1>
        <p class="subtitle">"System status and recent activity"</p>
        <div class="grid grid-3">
            <div class="card">
                <h3>"System"</h3>
                <div class="value">{move || if state.health_online.get() { "Online" } else { "Offline" }}</div>
                <div class="label">"wasmCloud 2.0"</div>
            </div>
            <div class="card">
                <h3>"Active Agents"</h3>
                <div class="value">{move || state.active_count.get()}</div>
                <div class="label">"currently running"</div>
            </div>
            <div class="card">
                <h3>"Models"</h3>
                <div class="value">{move || state.models.get().len()}</div>
                <div class="label">"available"</div>
            </div>
        </div>
        <h2 style="font-size:15px;font-weight:600;margin:28px 0 12px">"Recent Activity"</h2>
        <div>
            {move || {
                let activity = state.recent_activity.get();
                if activity.is_empty() {
                    view! { <p class="empty">"No recent activity"</p> }.into_any()
                } else {
                    activity.into_iter().map(|run| {
                        let status = run.status.clone();
                        let task = run.task_description.clone();
                        let model = run.model_used.clone();
                        let dur = format!("{:.1}s", run.duration_secs());
                        let tokens = run.tokens_output;
                        view! {
                            <div class="card" style="margin-bottom:8px;padding:12px 16px">
                                <span class={format!("badge {status}")}>{status.clone()}</span>
                                <span style="margin-left:8px;font-size:14px;font-weight:500">{task}</span>
                                <div style="font-size:13px;color:var(--muted);margin-top:4px">
                                    {model}" · "{tokens}" tokens · "{dur}
                                </div>
                            </div>
                        }
                    }).collect_view().into_any()
                }
            }}
        </div>
    }
}
