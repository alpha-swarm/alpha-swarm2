use leptos::prelude::*;
use leptos_router::components::A;

use crate::state::AppState;

#[component]
pub fn Sidebar() -> impl IntoView {
    let state = expect_context::<AppState>();
    let active_count = move || state.active_count.get();
    let is_online = move || state.health_online.get();

    view! {
        <nav class="sidebar">
            <div class="logo"><span>"alpha"</span>"-swarm"</div>
            <A href="/" attr:class="nav-item">
                <span class="nav-icon">"📊"</span>
                " Overview"
                <span class="health-dot" class:offline=move || !is_online()></span>
            </A>
            <A href="/projects" attr:class="nav-item">
                <span class="nav-icon">"📁"</span>
                " Projects"
            </A>
            <A href="/agents" attr:class="nav-item">
                <span class="nav-icon">"🤖"</span>
                " Agents"
                {move || {
                    let count = active_count();
                    (count > 0).then(|| view! { <span class="badge-count">{count}</span> })
                }}
            </A>
            <A href="/models" attr:class="nav-item">
                <span class="nav-icon">"⬡"</span>
                " Models"
            </A>
            <A href="/resources" attr:class="nav-item">
                <span class="nav-icon">"📦"</span>
                " Resources"
            </A>
            <A href="/submit" attr:class="nav-item">
                <span class="nav-icon">"+"</span>
                " Submit Task"
            </A>
        </nav>
    }
}
