use leptos::prelude::*;
use crate::state::AppState;
use crate::api;
use crate::components::badge::TierBadge;
use crate::components::progress_bar::ProgressBar;

#[component]
pub fn ResourcesPage() -> impl IntoView {
    let state = expect_context::<AppState>();

    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(res) = api::get_resources().await { state.resources.set(res); }
    });

    view! {
        <h1>"Resources"</h1>
        <p class="subtitle">"Multi-machine resource monitoring"</p>
        {move || {
            let hosts = state.resources.get();
            if hosts.is_empty() {
                return view! { <p class="empty">"No resource data — is the daemon running?"</p> }.into_any();
            }
            hosts.into_iter().map(|h| view! { <HostCard host=h /> }).collect_view().into_any()
        }}
    }
}

#[component]
fn HostCard(host: crate::types::ResourceSnapshot) -> impl IntoView {
    let is_local = host.host_type == "local";
    let is_ollama = host.host_type == "ollama";
    let host_name = host.host.clone();
    let _host_type = host.host_type.clone();

    view! {
        <div class="card" style="margin-bottom:16px">
            <div style="display:flex;align-items:center;gap:8px;margin-bottom:12px">
                <span style="font-size:16px;font-weight:600">{host_name}</span>
                <TierBadge tier=if is_ollama { "agent".to_string() } else { "worker".to_string() } />
            </div>
            {is_local.then(|| view! { <LocalMetrics host=host.clone() /> })}
            {is_ollama.then(|| view! { <OllamaMetrics host=host.clone() /> })}
        </div>
    }
}

#[component]
fn LocalMetrics(host: crate::types::ResourceSnapshot) -> impl IntoView {
    view! {
        <div class="grid grid-3">
            <MetricGauge label="CPU" value=format!("{:.1}%", host.cpu_percent) pct=host.cpu_percent />
            <MetricGauge label="RAM" value=format!("{:.1}%", host.ram_percent) detail=format!("{} / {} MB", host.ram_used_mb, host.ram_total_mb) pct=host.ram_percent />
            <MetricGauge label="Disk" value=format!("{:.1}%", host.disk_percent) detail=format!("{:.1} GB free", host.disk_free_gb) pct=host.disk_percent />
        </div>
    }
}

#[component]
fn MetricGauge(
    #[prop(into)] label: String,
    #[prop(into)] value: String,
    #[prop(optional, into)] detail: String,
    pct: f64,
) -> impl IntoView {
    view! {
        <div>
            <div style="font-size:12px;color:var(--muted)">{label}</div>
            <div style="font-size:20px;font-weight:600">{value}</div>
            {(!detail.is_empty()).then(|| view! { <div style="font-size:12px;color:var(--muted)">{detail}</div> })}
            <ProgressBar pct=pct />
        </div>
    }
}

#[component]
fn OllamaMetrics(host: crate::types::ResourceSnapshot) -> impl IntoView {
    let vram_gb = host.ram_used_mb as f64 / 1024.0;
    let models = host.ollama_models.clone();
    let model_count = host.disk_total_gb as u32;

    view! {
        <div>
            <div style="font-size:12px;color:var(--muted)">"Loaded Models (VRAM)"</div>
            <div style="font-size:20px;font-weight:600">{if vram_gb > 0.0 { format!("{vram_gb:.1} GB") } else { "idle".into() }}</div>
        </div>
        {(!models.is_empty()).then(|| view! {
            <div style="display:flex;flex-wrap:wrap;gap:6px;margin-top:8px">
                {models.into_iter().map(|m| {
                    let name = m.name.clone();
                    let size = format!("{:.1}GB", m.size_mb as f64 / 1024.0);
                    view! {
                        <span style="font-size:12px;padding:3px 10px;border-radius:12px;background:var(--accent-bg);color:var(--accent)">{name}" ("{size}")"</span>
                    }
                }).collect_view()}
            </div>
        })}
        <div style="margin-top:8px;font-size:12px;color:var(--muted)">{model_count}" models available"</div>
    }
}
