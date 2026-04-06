use leptos::prelude::*;

#[component]
pub fn StatCard(
    #[prop(into)] title: String,
    value: Signal<String>,
    #[prop(optional, into)] label: String,
) -> impl IntoView {
    view! {
        <div class="card">
            <h3>{title}</h3>
            <div class="value">{move || value.get()}</div>
            {(!label.is_empty()).then(|| view! { <div class="label">{label}</div> })}
        </div>
    }
}
