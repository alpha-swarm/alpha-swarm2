mod app;
mod db;
mod events;
mod ui;

use anyhow::Result;
use app::InputMode;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io::stdout;

const NATS_URL: &str = "nats://127.0.0.1:4223";
const TICK_MS: u64 = 100;
const POLL_INTERVAL: u64 = 30;

#[tokio::main]
async fn main() -> Result<()> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut app = app::App::new();

    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(events::subscribe_nats(NATS_URL.to_string(), tx));

    app.set_goals(db::fetch_goals(&app.project).await);

    loop {
        terminal.draw(|f| ui::render(f, &app))?;
        while let Ok(evt) = rx.try_recv() { app.push_event(evt); }

        if app.tick_count % POLL_INTERVAL == 0 {
            app.set_goals(db::fetch_goals(&app.project).await);
        }

        if !event::poll(std::time::Duration::from_millis(TICK_MS))? { app.tick(); continue; }
        let Event::Key(key) = event::read()? else { app.tick(); continue; };
        if key.kind != KeyEventKind::Press { app.tick(); continue; }

        match app.input_mode {
            InputMode::Normal => handle_normal(&mut app, key.code).await,
            InputMode::Input => handle_input(&mut app, key.code).await,
        }
        app.tick();
    }
}

fn quit() -> ! {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen);
    std::process::exit(0);
}

async fn handle_normal(app: &mut app::App, key: KeyCode) {
    match key {
        KeyCode::Char('q') => quit(),
        KeyCode::Up | KeyCode::Char('k') => app.scroll_up(),
        KeyCode::Down | KeyCode::Char('j') => app.scroll_down(),
        KeyCode::Enter => app.toggle_expand(),
        KeyCode::Tab => app.next_tab(),
        KeyCode::Char('i') => app.input_mode = InputMode::Input,
        KeyCode::Char('a') => approve_selected(app).await,
        KeyCode::Char('d') => delete_selected(app).await,
        _ => {}
    }
}

async fn approve_selected(app: &mut app::App) {
    let Some(g) = app.goals.get(app.selected) else { return };
    if g.status == "planned" { db::approve_goal(&g.id).await; }
}

async fn delete_selected(app: &mut app::App) {
    let Some(g) = app.goals.get(app.selected) else { return };
    let id = g.id.clone();
    db::delete_goal(&id).await;
    app.goals.retain(|g| g.id != id);
}

async fn handle_input(app: &mut app::App, key: KeyCode) {
    match key {
        KeyCode::Enter => {
            if !app.input_buf.is_empty() {
                db::submit_goal(&app.project, &app.input_buf).await;
                app.input_buf.clear();
            }
            app.input_mode = InputMode::Normal;
        }
        KeyCode::Esc => { app.input_buf.clear(); app.input_mode = InputMode::Normal; }
        KeyCode::Backspace => { app.input_buf.pop(); }
        KeyCode::Char(c) => app.input_buf.push(c),
        _ => {}
    }
}
