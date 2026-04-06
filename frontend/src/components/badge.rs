use leptos::prelude::*;

#[component]
pub fn StatusBadge(#[prop(into)] status: String) -> impl IntoView {
    let class = format!("badge {status}");
    view! { <span class=class>{status}</span> }
}

#[component]
pub fn TierBadge(#[prop(into)] tier: String) -> impl IntoView {
    let class = format!("badge {tier}");
    view! { <span class=class style="font-size:11px">{tier}</span> }
}
