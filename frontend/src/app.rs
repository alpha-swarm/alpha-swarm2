use leptos::prelude::*;
use leptos_router::components::*;
use leptos_router::{StaticSegment, ParamSegment};

use crate::state::AppState;
use crate::components::sidebar::Sidebar;
use crate::pages::*;

#[component]
pub fn App() -> impl IntoView {
    let state = AppState::new();
    provide_context(state);

    let r_root = (StaticSegment(""),);
    let r_projects = (StaticSegment("projects"),);
    let r_project = (StaticSegment("project"), ParamSegment("name"));
    let r_agents = (StaticSegment("agents"),);
    let r_models = (StaticSegment("models"),);
    let r_resources = (StaticSegment("resources"),);
    let r_submit = (StaticSegment("submit"),);
    let r_plan = (StaticSegment("plan"), ParamSegment("id"));

    view! {
        <Router>
            <div class="app" style="display:flex;min-height:100vh">
                <Sidebar />
                <main class="main" style="flex:1;padding:24px 32px;min-width:0">
                    {move || state.error_message.get().map(|e| view! {
                        <div
                            style="position:fixed;top:16px;right:16px;z-index:100;background:var(--error-bg);color:var(--error);padding:12px 20px;border-radius:var(--radius);border:1px solid var(--error);font-size:13px;max-width:400px;cursor:pointer"
                            on:click=move |_| state.error_message.set(None)
                        >
                            {e}
                        </div>
                    })}
                    <Routes fallback=|| "404">
                        <Route path=r_root view=OverviewPage />
                        <Route path=r_projects view=ProjectsPage />
                        <Route path=r_project view=ProjectDetailPage />
                        <Route path=r_agents view=AgentsPage />
                        <Route path=r_models view=ModelsPage />
                        <Route path=r_resources view=ResourcesPage />
                        <Route path=r_submit view=SubmitPage />
                        <Route path=r_plan view=PlanReviewPage />
                    </Routes>
                </main>
            </div>
        </Router>
    }
}
