use leptos::prelude::*;
use crate::state::AppState;
use crate::api;

#[component]
pub fn ResourcesPage() -> impl IntoView {
    let state = expect_context::<AppState>();

    // Load and auto-refresh resources
    let load_resources = move || {
        let state = state;
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok(res) = api::get_resources().await {
                state.resources.set(res);
            }
        });
    };
    load_resources();

    // TODO: auto-refresh via set_interval when leptos-use is added

    view! {
        <h1>"Resources"</h1>
        <p class="subtitle">"Multi-machine resource monitoring — daemon defers tasks when thresholds exceeded"</p>
        <div>
            {move || {
                let hosts = state.resources.get();
                if hosts.is_empty() {
                    return view! { <p class="empty">"No resource data — is the daemon running?"</p> }.into_any();
                }
                hosts.into_iter().map(|h| {
                    let host_name = h.host.clone();
                    let host_type = h.host_type.clone();
                    let is_local = host_type == "local";
                    let is_ollama = host_type == "ollama";
                    let badge_class = format!("badge {}", if is_ollama { "agent" } else { "worker" });

                    view! {
                        <div class="card" style="margin-bottom:16px">
                            <div style="display:flex;align-items:center;gap:8px;margin-bottom:12px">
                                <span style="font-size:16px;font-weight:600">{host_name}</span>
                                <span class=badge_class style="font-size:11px">{host_type}</span>
                            </div>
                            {is_local.then(|| {
                                let cpu = h.cpu_percent;
                                let ram = h.ram_percent;
                                let disk = h.disk_percent;
                                view! {
                                    <div class="grid grid-3">
                                        <div>
                                            <div style="font-size:12px;color:var(--muted)">"CPU"</div>
                                            <div style="font-size:20px;font-weight:600">{format!("{cpu:.1}%")}</div>
                                            <ProgressBar pct=cpu />
                                        </div>
                                        <div>
                                            <div style="font-size:12px;color:var(--muted)">"RAM"</div>
                                            <div style="font-size:20px;font-weight:600">{format!("{ram:.1}%")}</div>
                                            <div style="font-size:12px;color:var(--muted)">{format!("{} / {} MB", h.ram_used_mb, h.ram_total_mb)}</div>
                                            <ProgressBar pct=ram />
                                        </div>
                                        <div>
                                            <div style="font-size:12px;color:var(--muted)">"Disk"</div>
                                            <div style="font-size:20px;font-weight:600">{format!("{disk:.1}%")}</div>
                                            <div style="font-size:12px;color:var(--muted)">{format!("{:.1} GB free", h.disk_free_gb)}</div>
                                            <ProgressBar pct=disk />
                                        </div>
                                    </div>
                                }
                            })}
                            {is_ollama.then(|| {
                                let vram_gb = h.ram_used_mb as f64 / 1024.0;
                                let models = h.ollama_models.clone();
                                let model_count = h.disk_total_gb as u32;
                                view! {
                                    <div style="margin-bottom:8px">
                                        <div style="font-size:12px;color:var(--muted)">"Loaded Models (VRAM)"</div>
                                        <div style="font-size:20px;font-weight:600">
                                            {if vram_gb > 0.0 { format!("{vram_gb:.1} GB") } else { "idle".into() }}
                                        </div>
                                    </div>
                                    {(!models.is_empty()).then(|| {
                                        let models = models.clone();
                                        view! {
                                            <div style="display:flex;flex-wrap:wrap;gap:6px">
                                                {models.into_iter().map(|m| {
                                                    let name = m.name.clone();
                                                    let size = format!("{:.1}GB", m.size_mb as f64 / 1024.0);
                                                    view! {
                                                        <span style="font-size:12px;padding:3px 10px;border-radius:12px;background:var(--accent-bg);color:var(--accent)">
                                                            {name}" ("{size}")"
                                                        </span>
                                                    }
                                                }).collect_view()}
                                            </div>
                                        }
                                    })}
                                    <div style="margin-top:8px;font-size:12px;color:var(--muted)">{model_count}" models available"</div>
                                }
                            })}
                        </div>
                    }
                }).collect_view().into_any()
            }}
        </div>
    }
}

#[component]
fn ProgressBar(pct: f64) -> impl IntoView {
    let color = if pct > 80.0 { "var(--error)" } else if pct > 50.0 { "var(--warning)" } else { "var(--success)" };
    let width = format!("{}%", pct.min(100.0));
    view! {
        <div style="margin-top:6px;height:6px;background:var(--border);border-radius:3px;overflow:hidden">
            <div style=format!("height:100%;background:{color};width:{width};border-radius:3px;transition:width 0.5s")></div>
        </div>
    }
}
