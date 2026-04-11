use ratatui::prelude::*;
use ratatui::widgets::*;
use crate::app::{App, Tab, InputMode, Goal, PhaseTiming};

const CYAN: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
const RED: Style = Style::new().fg(Color::Red);
const DIM: Style = Style::new().fg(Color::DarkGray);
const HL: Style = Style::new().bg(Color::DarkGray).fg(Color::White);

pub fn render(frame: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(3), Constraint::Length(1),
        Constraint::Min(0), Constraint::Length(3), Constraint::Length(1),
    ]).split(frame.area());

    render_header(frame, chunks[0], app);
    render_tabs(frame, chunks[1], app);
    match app.tab { Tab::Goals => render_goals(frame, chunks[2], app), Tab::Log => render_log(frame, chunks[2], app) }
    render_input(frame, chunks[3], app);
    render_footer(frame, chunks[4], app);
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let dot = if app.connected { Span::styled("●", Style::new().fg(Color::Green)) } else { Span::styled("○", Style::new().fg(Color::Red)) };
    let stats = goal_stats(&app.goals);
    let title = Line::from(vec![
        Span::styled(" alpha-swarm ", CYAN), dot,
        Span::styled(format!("  {}", app.project), DIM),
        Span::raw("  "), Span::styled(stats, DIM),
    ]);
    frame.render_widget(Block::bordered().title(title), area);
}

fn goal_stats(goals: &[Goal]) -> String {
    let r = goals.iter().filter(|g| g.status == "running").count();
    let p = goals.iter().filter(|g| g.status == "passed").count();
    let f = goals.iter().filter(|g| g.status == "failed").count();
    format!("run:{r} pass:{p} fail:{f}")
}

fn render_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let tabs = Tabs::new(vec!["Goals", "Log"])
        .select(match app.tab { Tab::Goals => 0, Tab::Log => 1 })
        .style(DIM).highlight_style(CYAN);
    frame.render_widget(tabs, area);
}

fn status_color(s: &str) -> Style {
    match s {
        "running" | "planning" => Style::new().fg(Color::Yellow),
        "passed" => Style::new().fg(Color::Green),
        "failed" => RED,
        "planned" => Style::new().fg(Color::Blue),
        _ => DIM,
    }
}

fn render_goals(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, goal) in app.goals.iter().enumerate() {
        let sel = i == app.selected;
        let arrow = if app.is_expanded(&goal.id) { "▼" } else { "▶" };
        let dur = format_dur(goal.duration_ms);
        let desc: String = goal.task_description.chars().take(60).collect();
        let base = if sel { HL } else { Style::default() };
        lines.push(Line::from(vec![
            Span::styled(format!(" {arrow} "), DIM),
            Span::styled(format!("[{}]", goal.status), status_color(&goal.status)),
            Span::styled(format!(" {desc}"), base),
            Span::styled(format!("  {dur}"), DIM),
        ]));
        if app.is_expanded(&goal.id) { render_detail(&mut lines, goal); }
    }
    let w = Paragraph::new(lines).block(Block::bordered().title(" Goals ")).wrap(Wrap { trim: false });
    frame.render_widget(w, area);
}

fn render_detail(lines: &mut Vec<Line<'static>>, g: &Goal) {
    if let Some(ref pt) = g.phase_timings { lines.push(waterfall(pt)); }
    if let Some(ref m) = g.progress_message { lines.push(Line::from(Span::styled(format!("    {m}"), Style::new().fg(Color::Yellow)))); }
    if let Some(ref e) = g.error_message { lines.push(Line::from(Span::styled(format!("    ERR: {e}"), RED))); }
    render_tools(lines, g);
    render_files(lines, g);
}

fn render_tools(lines: &mut Vec<Line<'static>>, g: &Goal) {
    let Some(ref tools) = g.tool_calls else { return };
    for tc in tools {
        let (icon, color) = if tc.is_error { ("✗", Color::Red) } else { ("✓", Color::Green) };
        let p: String = tc.params_preview.chars().take(40).collect();
        let tool = tc.tool.clone();
        lines.push(Line::from(vec![
            Span::styled(format!("    {icon} "), Style::new().fg(color)),
            Span::styled(tool, Style::new().fg(Color::Cyan)),
            Span::styled(format!(" {p}"), DIM),
            Span::styled(format!("  {:.1}s", tc.duration_ms as f64 / 1000.0), DIM),
        ]));
    }
}

fn render_files(lines: &mut Vec<Line<'static>>, g: &Goal) {
    if g.files_modified.is_empty() { return; }
    let f: String = g.files_modified.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
    lines.push(Line::from(Span::styled(format!("    files: {f}"), DIM)));
}

fn waterfall(pt: &PhaseTiming) -> Line<'static> {
    let total = pt.embedding_ms + pt.rag_ms + pt.planning_ms + pt.agent_execution_ms + pt.quality_gate_ms;
    if total == 0 { return Line::from(Span::styled("    (no timing)", DIM)); }
    let phases = [("emb", pt.embedding_ms, Color::Magenta), ("rag", pt.rag_ms, Color::Blue),
        ("plan", pt.planning_ms, Color::Yellow), ("agent", pt.agent_execution_ms, Color::Green), ("qg", pt.quality_gate_ms, Color::Red)];
    let mut spans = vec![Span::raw("    ")];
    for (name, ms, color) in phases {
        if ms == 0 { continue; }
        let bar_len = ((ms as f64 / total as f64) * 20.0) as usize;
        let bar: String = "█".repeat(bar_len.max(1));
        spans.push(Span::styled(format!("{bar} "), Style::new().fg(color)));
        spans.push(Span::styled(format!("{name}:{:.1}s ", ms as f64 / 1000.0), DIM));
    }
    Line::from(spans)
}

fn format_dur(ms: u64) -> String {
    if ms == 0 { return "...".into(); }
    if ms < 60_000 { return format!("{:.0}s", ms as f64 / 1000.0); }
    format!("{:.1}m", ms as f64 / 60_000.0)
}

fn render_log(frame: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app.log_lines.iter().enumerate().map(|(i, l)| {
        let s = if i == app.selected { HL } else if l.is_error { RED } else { Style::default() };
        Row::new(vec![
            Cell::from(l.timestamp.as_str()).style(DIM),
            Cell::from(l.kind.as_str()).style(if l.is_error { RED } else { Style::new().fg(Color::Yellow) }),
            Cell::from(l.message.as_str()),
        ]).style(s)
    }).collect();
    let t = Table::new(rows, [Constraint::Length(9), Constraint::Length(6), Constraint::Min(0)])
        .block(Block::bordered().title(" Log "));
    frame.render_widget(t, area);
}

fn render_input(frame: &mut Frame, area: Rect, app: &App) {
    let (title, style) = match app.input_mode {
        InputMode::Normal => (" 'i' to type goal ", DIM),
        InputMode::Input => (" Goal (Enter=submit, Esc=cancel) ", CYAN),
    };
    let w = Paragraph::new(app.input_buf.as_str()).style(style).block(Block::bordered().title(title));
    frame.render_widget(w, area);
    if app.input_mode == InputMode::Input {
        frame.set_cursor_position((area.x + app.input_buf.len() as u16 + 1, area.y + 1));
    }
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let k = match app.input_mode {
        InputMode::Normal => " q quit  ↑↓ scroll  Tab switch  Enter expand  i input  a approve  d delete ",
        InputMode::Input => " Enter submit  Esc cancel ",
    };
    frame.render_widget(Paragraph::new(k).style(DIM), area);
}
