use leptos::prelude::*;

#[component]
pub fn ProgressBar(pct: f64) -> impl IntoView {
    let color = if pct > 80.0 { "var(--error)" } else if pct > 50.0 { "var(--warning)" } else { "var(--success)" };
    let width = format!("{}%", pct.min(100.0));
    view! {
        <div style="margin-top:6px;height:6px;background:var(--border);border-radius:3px;overflow:hidden">
            <div style=format!("height:100%;background:{color};width:{width};border-radius:3px;transition:width 0.5s")></div>
        </div>
    }
}
