//! Ratatui-based multi-pane UI for the ACCURE client.

use std::net::SocketAddr;
use std::time::Duration;

use accure_core::messages::{ClientCommand, ServerEvent, Snapshot};
use accure_core::op::Right;
use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};
use ratatui::Terminal;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use accure_client::{parse_command, ParseError};

const HELP_TEXT: &str =
    "insert <pos> <ch> | delete <pos> | allow <site> <a|r|w> | deny <site> <a|r|w> | snapshot | quit";

struct AppState {
    server_addr: SocketAddr,
    snapshot: Option<Snapshot>,
    trace: Vec<String>,
    errors: Vec<String>,
    input: String,
    history: Vec<String>,
    history_idx: Option<usize>,
}

impl AppState {
    fn new(server_addr: SocketAddr) -> Self {
        Self {
            server_addr,
            snapshot: None,
            trace: Vec::new(),
            errors: Vec::new(),
            input: String::new(),
            history: Vec::new(),
            history_idx: None,
        }
    }

    fn push_trace(&mut self, s: String) {
        self.trace.push(s);
        if self.trace.len() > 500 {
            let drop = self.trace.len() - 500;
            self.trace.drain(0..drop);
        }
    }

    fn push_error(&mut self, s: String) {
        self.errors.push(s);
        if self.errors.len() > 16 {
            let drop = self.errors.len() - 16;
            self.errors.drain(0..drop);
        }
    }
}

pub async fn run(
    server_addr: SocketAddr,
    mut events: mpsc::Receiver<ServerEvent>,
    commands: mpsc::Sender<ClientCommand>,
) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;
    let mut key_events = EventStream::new();
    let mut app = AppState::new(server_addr);
    let mut tick = tokio::time::interval(Duration::from_millis(500));

    let res: Result<()> = loop {
        if let Err(e) = term.draw(|f| draw(f, &app)) {
            break Err(e.into());
        }

        tokio::select! {
            biased;
            maybe_ev = events.recv() => {
                match maybe_ev {
                    Some(ev) => apply_server_event(&mut app, ev),
                    None => app.push_error("server connection closed".into()),
                }
            }
            maybe_key = key_events.next() => {
                match maybe_key {
                    Some(Ok(Event::Key(key))) => {
                        if handle_key(&mut app, key, &commands).await {
                            break Ok(());
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => app.push_error(format!("input error: {e}")),
                    None => break Ok(()),
                }
            }
            _ = tick.tick() => {}
        }
    };

    disable_raw_mode().ok();
    crossterm::execute!(term.backend_mut(), LeaveAlternateScreen).ok();
    term.show_cursor().ok();
    res
}

fn apply_server_event(app: &mut AppState, ev: ServerEvent) {
    match ev {
        ServerEvent::State(s) => app.snapshot = Some(s),
        ServerEvent::Trace(t) => app.push_trace(t),
        ServerEvent::Error(e) => app.push_error(e),
    }
}

async fn handle_key(
    app: &mut AppState,
    key: KeyEvent,
    commands: &mpsc::Sender<ClientCommand>,
) -> bool {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return false;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        return true;
    }
    match key.code {
        KeyCode::Esc => return true,
        KeyCode::Char(c) => {
            app.input.push(c);
            app.history_idx = None;
        }
        KeyCode::Backspace => {
            app.input.pop();
            app.history_idx = None;
        }
        KeyCode::Up => {
            if app.history.is_empty() {
                return false;
            }
            let next = match app.history_idx {
                None => app.history.len() - 1,
                Some(0) => 0,
                Some(i) => i - 1,
            };
            app.history_idx = Some(next);
            app.input = app.history[next].clone();
        }
        KeyCode::Down => {
            if let Some(i) = app.history_idx {
                if i + 1 < app.history.len() {
                    app.history_idx = Some(i + 1);
                    app.input = app.history[i + 1].clone();
                } else {
                    app.history_idx = None;
                    app.input.clear();
                }
            }
        }
        KeyCode::Enter => {
            let line = std::mem::take(&mut app.input);
            app.history_idx = None;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return false;
            }
            if matches!(trimmed, "quit" | "q" | "exit") {
                return true;
            }
            if trimmed == "help" || trimmed == "?" {
                app.push_trace(format!("help: {HELP_TEXT}"));
                return false;
            }
            if trimmed == "clear" {
                app.trace.clear();
                app.errors.clear();
                return false;
            }
            app.history.push(trimmed.to_string());
            if app.history.len() > 100 {
                let drop = app.history.len() - 100;
                app.history.drain(0..drop);
            }
            match parse_command(trimmed) {
                Ok(cmd) => {
                    if commands.send(cmd).await.is_err() {
                        app.push_error("server channel closed".into());
                    } else {
                        let _ = commands.send(ClientCommand::Snapshot).await;
                    }
                }
                Err(ParseError::Empty) => {}
                Err(e) => app.push_error(e.to_string()),
            }
        }
        _ => {}
    }
    false
}

fn draw(f: &mut ratatui::Frame<'_>, app: &AppState) {
    let area = f.area();
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(cols[0]);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(8)])
        .split(cols[1]);

    draw_document(f, left[0], app);
    draw_trace(f, left[1], app);
    draw_input(f, left[2], app);
    draw_policy(f, right[0], app);
    draw_status(f, right[1], app);
}

fn draw_document(f: &mut ratatui::Frame<'_>, area: Rect, app: &AppState) {
    let body = app
        .snapshot
        .as_ref()
        .map(|s| s.document.clone())
        .unwrap_or_else(|| "(waiting for snapshot…)".into());
    let title = match &app.snapshot {
        Some(s) => format!("Document @ {} ({} ops)", s.site, s.log_len),
        None => "Document".into(),
    };
    let p = Paragraph::new(body)
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn draw_trace(f: &mut ratatui::Frame<'_>, area: Rect, app: &AppState) {
    let visible_rows = area.height.saturating_sub(2) as usize;
    let lines: Vec<Line> = app
        .trace
        .iter()
        .rev()
        .take(visible_rows)
        .rev()
        .map(|l| Line::from(l.clone()))
        .collect();
    let p = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Protocol Log ({})", app.trace.len())),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn draw_input(f: &mut ratatui::Frame<'_>, area: Rect, app: &AppState) {
    let prompt = format!("> {}", app.input);
    let p = Paragraph::new(prompt).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Input — {HELP_TEXT}"))
            .title_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(p, area);
    let cursor_x = area.x + 3 + app.input.chars().count() as u16;
    let cursor_y = area.y + 1;
    if cursor_x < area.x + area.width.saturating_sub(1) {
        f.set_cursor_position((cursor_x, cursor_y));
    }
}

fn draw_policy(f: &mut ratatui::Frame<'_>, area: Rect, app: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Policy (✓ = allowed)");
    if let Some(snap) = &app.snapshot {
        let mut by_site: std::collections::BTreeMap<&str, [bool; 3]> = Default::default();
        for (s, r, ok) in &snap.policy {
            let entry = by_site.entry(s.as_str()).or_insert([false; 3]);
            let i = match r {
                Right::Admin => 0,
                Right::Read => 1,
                Right::Write => 2,
            };
            entry[i] = *ok;
        }
        let header = Row::new(vec![
            Cell::from("Site").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("A").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("R").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("W").style(Style::default().add_modifier(Modifier::BOLD)),
        ]);
        let rows: Vec<Row> = by_site
            .iter()
            .map(|(site, rights)| {
                let me = *site == snap.site;
                let style = if me {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                Row::new(vec![
                    Cell::from(Span::styled(site.to_string(), style)),
                    cell(rights[0]),
                    cell(rights[1]),
                    cell(rights[2]),
                ])
            })
            .collect();
        let widths = [
            Constraint::Length(14),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
        ];
        let table = Table::new(rows, widths).header(header).block(block);
        f.render_widget(table, area);
    } else {
        f.render_widget(Paragraph::new("(no snapshot yet)").block(block), area);
    }
}

fn cell(allowed: bool) -> Cell<'static> {
    if allowed {
        Cell::from(Span::styled("✓", Style::default().fg(Color::Green)))
    } else {
        Cell::from(Span::styled("·", Style::default().fg(Color::DarkGray)))
    }
}

fn draw_status(f: &mut ratatui::Frame<'_>, area: Rect, app: &AppState) {
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(format!("server: {}", app.server_addr)));
    if let Some(s) = &app.snapshot {
        lines.push(Line::from(format!("site: {}", s.site)));
        let peers = if s.peers.is_empty() {
            "(none)".into()
        } else {
            s.peers.join(", ")
        };
        lines.push(Line::from(format!("peers: {peers}")));
    }
    for e in app.errors.iter().rev().take(4) {
        lines.push(Line::from(Span::styled(
            format!("⚠ {e}"),
            Style::default().fg(Color::Red),
        )));
    }
    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Status"))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}
