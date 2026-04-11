use ratatui::prelude::*;
use ratatui::widgets::*;
use crate::app::{App, Tab};

const HEADER_STYLE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
const ERROR_STYLE: Style = Style::new().fg(Color::Red);
const DIM_STYLE: Style = Style::new().fg(Color::DarkGray);
const HIGHLIGHT_STYLE: Style = Style::new().bg(Color::DarkGray).fg(Color::White);

pub fn render(frame: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ]).split(frame.area());

    render_header(frame, chunks[0], app);
    render_tabs(frame, chunks[1], app);
    render_body(frame, chunks[2], app);
    render_footer(frame, chunks[3], app);
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let dot = if app.connected { "●" } else { "○" };
    let dot_style = if app.connected { Style::new().fg(Color::Green) } else { Style::new().fg(Color::Red) };
    let title = Line::from(vec![
        Span::styled(" alpha-swarm ", HEADER_STYLE),
        Span::styled(dot, dot_style),
        Span::styled(format!("  events: {}", app.events.len()), DIM_STYLE),
    ]);
    let block = Block::bordered().title(title);
    frame.render_widget(block, area);
}

fn render_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let tabs = Tabs::new(vec!["Live", "Log"])
        .select(match app.tab { Tab::Live => 0, Tab::Log => 1 })
        .style(DIM_STYLE)
        .highlight_style(HEADER_STYLE);
    frame.render_widget(tabs, area);
}

fn render_body(frame: &mut Frame, area: Rect, app: &App) {
    match app.tab {
        Tab::Live => render_live(frame, area, app),
        Tab::Log => render_log(frame, area, app),
    }
}

fn render_live(frame: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app.events.iter().enumerate().map(|(i, e)| {
        let style = if i == app.selected { HIGHLIGHT_STYLE } else { Style::default() };
        let (kind, msg) = event_summary(e);
        Row::new(vec![
            Cell::from(kind).style(event_color(e)),
            Cell::from(msg),
        ]).style(style)
    }).collect();

    let table = Table::new(rows, [Constraint::Length(12), Constraint::Min(0)])
        .block(Block::bordered().title(" Events "))
        .highlight_style(HIGHLIGHT_STYLE);
    frame.render_widget(table, area);
}

fn render_log(frame: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app.log_lines.iter().enumerate().map(|(i, l)| {
        let style = if i == app.selected { HIGHLIGHT_STYLE } else if l.is_error { ERROR_STYLE } else { Style::default() };
        Row::new(vec![
            Cell::from(l.timestamp.as_str()).style(DIM_STYLE),
            Cell::from(l.kind.as_str()).style(if l.is_error { ERROR_STYLE } else { Style::new().fg(Color::Yellow) }),
            Cell::from(l.message.as_str()),
        ]).style(style)
    }).collect();

    let table = Table::new(rows, [Constraint::Length(9), Constraint::Length(7), Constraint::Min(0)])
        .block(Block::bordered().title(" Log "));
    frame.render_widget(table, area);
}

fn render_footer(frame: &mut Frame, area: Rect, _app: &App) {
    let help = Line::from(vec![
        Span::styled(" q", HEADER_STYLE), Span::raw(" quit  "),
        Span::styled("↑↓", HEADER_STYLE), Span::raw(" scroll  "),
        Span::styled("Tab", HEADER_STYLE), Span::raw(" switch  "),
        Span::styled("Enter", HEADER_STYLE), Span::raw(" expand "),
    ]);
    frame.render_widget(Paragraph::new(help).style(DIM_STYLE), area);
}

fn event_summary(e: &swarm_events::SwarmEvent) -> (String, String) {
    match e {
        swarm_events::SwarmEvent::AgentStarted { task, .. } => ("STARTED".into(), task[..task.len().min(80)].into()),
        swarm_events::SwarmEvent::AgentFinished { status, duration_ms, .. } => ("DONE".into(), format!("{status} ({duration_ms}ms)")),
        swarm_events::SwarmEvent::AgentFailed { error, .. } => ("FAIL".into(), error[..error.len().min(80)].into()),
        swarm_events::SwarmEvent::AgentProgress { step, max_steps, action, .. } => (format!("STEP {step}/{max_steps}"), action.clone()),
        swarm_events::SwarmEvent::ToolCallExecuted { tool, duration_ms, .. } => ("TOOL".into(), format!("{tool} ({duration_ms}ms)")),
        swarm_events::SwarmEvent::SwarmPlanned { task_count, goal, .. } => ("PLAN".into(), format!("{task_count} tasks: {}", &goal[..goal.len().min(60)])),
        swarm_events::SwarmEvent::SwarmCompleted { quality_passed, .. } => ("DONE".into(), format!("QG: {quality_passed}")),
        swarm_events::SwarmEvent::TaskSubmitted { goal, .. } => ("NEW".into(), goal[..goal.len().min(80)].into()),
        swarm_events::SwarmEvent::QualityChecked { check_name, passed, .. } => ("QG".into(), format!("{check_name}: {passed}")),
    }
}

fn event_color(e: &swarm_events::SwarmEvent) -> Style {
    match e {
        swarm_events::SwarmEvent::AgentStarted { .. } => Style::new().fg(Color::Blue),
        swarm_events::SwarmEvent::AgentFinished { .. } => Style::new().fg(Color::Green),
        swarm_events::SwarmEvent::AgentFailed { .. } => ERROR_STYLE,
        swarm_events::SwarmEvent::AgentProgress { .. } => Style::new().fg(Color::Yellow),
        swarm_events::SwarmEvent::ToolCallExecuted { is_error, .. } => if *is_error { ERROR_STYLE } else { Style::new().fg(Color::Cyan) },
        swarm_events::SwarmEvent::SwarmPlanned { .. } => Style::new().fg(Color::Magenta),
        swarm_events::SwarmEvent::SwarmCompleted { quality_passed, .. } => if *quality_passed { Style::new().fg(Color::Green) } else { ERROR_STYLE },
        swarm_events::SwarmEvent::TaskSubmitted { .. } => Style::new().fg(Color::White),
        swarm_events::SwarmEvent::QualityChecked { passed, .. } => if *passed { Style::new().fg(Color::Green) } else { ERROR_STYLE },
    }
}
