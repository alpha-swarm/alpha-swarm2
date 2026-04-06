use leptos::prelude::*;
use leptos_router::components::*;
use leptos_router::path;

use crate::state::AppState;
use crate::components::sidebar::Sidebar;
use crate::pages::*;

#[component]
pub fn App() -> impl IntoView {
    let state = AppState::new();
    provide_context(state);

    // TODO: Initialize SSE connection here

    view! {
        <Router>
            <div class="app" style="display:flex;min-height:100vh">
                <Sidebar />
                <main class="main" style="flex:1;padding:24px 32px;min-width:0">
                    <Routes fallback=|| view! { <p>"Page not found"</p> }>
                        <Route path=path!("/") view=OverviewPage />
                        <Route path=path!("/projects") view=ProjectsPage />
                        <Route path=path!("/project/:name") view=ProjectDetailPage />
                        <Route path=path!("/agents") view=AgentsPage />
                        <Route path=path!("/models") view=ModelsPage />
                        <Route path=path!("/resources") view=ResourcesPage />
                        <Route path=path!("/submit") view=SubmitPage />
                        <Route path=path!("/plan/:id") view=PlanReviewPage />
                    </Routes>
                </main>
            </div>
        </Router>
    }
}
