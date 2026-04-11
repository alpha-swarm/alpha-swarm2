mod app;
mod ui;
mod events;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io::{self, stdout};

const NATS_URL: &str = "nats://127.0.0.1:4223";
const TICK_MS: u64 = 100;

#[tokio::main]
async fn main() -> Result<()> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut app = app::App::new();

    // Start NATS subscription in background
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(events::subscribe_nats(NATS_URL.to_string(), tx));

    loop {
        terminal.draw(|frame| ui::render(frame, &app))?;

        // Drain NATS events
        while let Ok(evt) = rx.try_recv() {
            app.push_event(evt);
        }

        // Handle keyboard
        if event::poll(std::time::Duration::from_millis(TICK_MS))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press { continue; }
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Up | KeyCode::Char('k') => app.scroll_up(),
                    KeyCode::Down | KeyCode::Char('j') => app.scroll_down(),
                    KeyCode::Enter => app.toggle_expand(),
                    KeyCode::Tab => app.next_tab(),
                    _ => {}
                }
            }
        }

        app.tick();
    }

    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    Ok(())
}
