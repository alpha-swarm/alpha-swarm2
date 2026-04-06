use leptos::prelude::*;
use crate::state::AppState;
use crate::api;
use crate::types::ModelRole;
use crate::components::badge::TierBadge;

#[component]
pub fn ModelsPage() -> impl IntoView {
    let state = expect_context::<AppState>();

    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(models) = api::list_models().await { state.models.set(models); }
        if let Ok(roles) = api::list_model_roles().await { state.model_roles.set(roles); }
    });

    view! {
        <h1>"Models"</h1>
        <p class="subtitle">"Available inference models across backends"</p>
        <div class="grid grid-3">
            {move || {
                let models = state.models.get();
                let roles = state.model_roles.get();
                if models.is_empty() {
                    return view! { <p class="empty">"No models available. Is Ollama running?"</p> }.into_any();
                }
                models.into_iter().map(|m| {
                    let name = m.display_name().to_string();
                    let params = m.display_params();
                    let family = m.display_family();
                    let role = roles.iter().find(|r| name.contains(&r.name) || r.name.contains(&name)).cloned();
                    view! { <ModelCard name=name params=params family=family role=role /> }
                }).collect_view().into_any()
            }}
        </div>
    }
}

#[component]
fn ModelCard(
    #[prop(into)] name: String,
    #[prop(into)] params: String,
    #[prop(into)] family: String,
    role: Option<ModelRole>,
) -> impl IntoView {
    let tier = role.as_ref().map(|r| r.tier.clone()).unwrap_or_default();
    let role_desc = role.as_ref().map(|r| r.role.clone()).unwrap_or_default();
    let fuel = role.as_ref().map(|r| r.fuel.clone()).unwrap_or_default();
    let good_for = role.as_ref().map(|r| r.good_for.clone()).unwrap_or_default();

    view! {
        <div class="card">
            <div style="display:flex;justify-content:space-between;align-items:start">
                <div style="font-size:14px;font-weight:600;color:var(--accent)">{name}</div>
                <TierBadge tier=tier />
            </div>
            <div style="font-size:13px;font-weight:500;color:var(--text-secondary);margin-top:6px">{role_desc}</div>
            <div style="font-size:13px;color:var(--muted);margin-top:4px">{params}" · "{family}</div>
            {(!fuel.is_empty()).then(|| view! {
                <div style="margin-top:6px;font-size:12px;color:var(--muted)">"Fuel: "{fuel}</div>
            })}
            {(!good_for.is_empty()).then(|| view! {
                <div style="margin-top:8px;display:flex;flex-wrap:wrap;gap:4px">
                    {good_for.into_iter().map(|tag| view! {
                        <span style="font-size:11px;padding:2px 8px;border-radius:12px;background:var(--accent-bg);color:var(--accent)">{tag}</span>
                    }).collect_view()}
                </div>
            })}
        </div>
    }
}
