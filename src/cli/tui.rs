use crate::agent::llm::LlmConfig;
use crate::agent::chat::AgentChat;
use crate::core::mcp::McpClient;
use crate::core::task::Task;
use crate::utils::shell;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use std::io;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, PartialEq)]
enum InputMode {
    Normal,
    Insert,
}

struct App {
    input: String,
    input_mode: InputMode,
    output_lines: Vec<(String, Color)>,
    scroll_offset: usize,
    agent_busy: bool,
    mcp_clients: Vec<Arc<McpClient>>,
    http_client: reqwest::Client,
}

impl App {
    fn new(mcp_clients: Vec<Arc<McpClient>>, http_client: reqwest::Client) -> Self {
        let mut lines: Vec<(String, Color)> = vec![
            ("Command Central — Agent Terminal".to_string(), Color::Yellow),
            ("Type a message to ask the AI agent, or use !<command> to run shell.".to_string(), Color::Gray),
            ("Press Ctrl+C to cancel an in-progress agent call.".to_string(), Color::Gray),
            (String::new(), Color::Reset),
        ];
        let config = LlmConfig::from_env();
        if !config.is_configured() {
            lines.push(("[!] LLM not configured — set LLM_API_KEY in .env for AI agent".to_string(), Color::Red));
            lines.push(("[!] Start with ! to run shell commands directly".to_string(), Color::Red));
            lines.push((String::new(), Color::Reset));
        }
        Self {
            input: String::new(),
            input_mode: InputMode::Insert,
            output_lines: lines,
            scroll_offset: 0,
            agent_busy: false,
            mcp_clients,
            http_client,
        }
    }

    fn add_output(&mut self, line: String, color: Color) {
        self.output_lines.push((line, color));
        if self.output_lines.len() > 1000 {
            self.output_lines.drain(0..500);
        }
    }
}

pub async fn run_tui(tx: mpsc::Sender<Task>) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let http_client = reqwest::Client::builder()
        .user_agent("command_central/0.2.0")
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()?;

    let mut app = App::new(vec![], http_client);

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<String>();

    loop {
        terminal.draw(|f| ui(f, &app))?;

        while let Ok(msg) = event_rx.try_recv() {
            if msg == "__CLEAR_BUSY__" {
                app.agent_busy = false;
            } else {
                app.add_output(msg, Color::White);
            }
        }

        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
                    {
                        app.add_output("^C — cancelled.".to_string(), Color::Yellow);
                        app.agent_busy = false;
                        continue;
                    }

                    match key.code {
                        KeyCode::Char('q') if app.input_mode == InputMode::Normal && !app.agent_busy => {
                            break;
                        }
                        KeyCode::Char('i') if app.input_mode == InputMode::Normal => {
                            app.input_mode = InputMode::Insert;
                        }
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Enter if app.input_mode == InputMode::Insert && !app.agent_busy => {
                            let input = app.input.clone();
                            app.input.clear();
                            if !input.trim().is_empty() {
                                app.add_output(format!("> {}", input.trim()), Color::Cyan);
                                app.agent_busy = true;
                                let event_tx_clone = event_tx.clone();
                                let tx_clone = tx.clone();
                                let input_owned = input.trim().to_string();
                                let http = app.http_client.clone();
                                tokio::spawn(async move {
                                    let result = process_input(&input_owned, &tx_clone, &http).await;
                                    let _ = event_tx_clone.send(result);
                                    let _ = event_tx_clone.send(String::new());
                                    let _ = event_tx_clone.send("__CLEAR_BUSY__".to_string());
                                });
                            }
                        }
                        KeyCode::Char(c) if app.input_mode == InputMode::Insert && !app.agent_busy => {
                            app.input.push(c);
                        }
                        KeyCode::Backspace if app.input_mode == InputMode::Insert && !app.agent_busy => {
                            app.input.pop();
                        }
                        KeyCode::PageUp | KeyCode::Up => {
                            if app.scroll_offset + 1 < app.output_lines.len().saturating_sub(10) {
                                app.scroll_offset += 1;
                            }
                        }
                        KeyCode::PageDown | KeyCode::Down => {
                            app.scroll_offset = app.scroll_offset.saturating_sub(1);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(f.size());

    let visible_start = app.scroll_offset;
    let max_visible = chunks[0].height.saturating_sub(2) as usize;

    let items: Vec<ListItem> = app
        .output_lines
        .iter()
        .rev()
        .skip(visible_start)
        .take(max_visible)
        .map(|(line, color)| {
            ListItem::new(line.as_str()).style(Style::default().fg(*color))
        })
        .collect();

    let title = if app.agent_busy {
        " Command Central [BUSY] "
    } else {
        " Command Central "
    };

    let messages = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .style(Style::default().fg(Color::Cyan));
    f.render_widget(messages, chunks[0]);

    let input_style = if app.agent_busy {
        Style::default().fg(Color::DarkGray)
    } else {
        match app.input_mode {
            InputMode::Normal => Style::default().fg(Color::Gray),
            InputMode::Insert => Style::default().fg(Color::Green),
        }
    };

    let title = if app.agent_busy {
        " Waiting for agent... (Ctrl+C to cancel) "
    } else {
        match app.input_mode {
            InputMode::Normal => " Press i to type, q to quit ",
            InputMode::Insert => " Input ",
        }
    };

    let input = Paragraph::new(app.input.as_str())
        .block(Block::default().borders(Borders::ALL).title(title).style(
            if app.agent_busy {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            },
        ))
        .style(input_style);
    f.render_widget(input, chunks[1]);

    if app.input_mode == InputMode::Insert && !app.agent_busy {
        let cursor_x = chunks[1].x + app.input.len() as u16 + 1;
        let cursor_y = chunks[1].y + 1;
        f.set_cursor(cursor_x, cursor_y);
    }
}

async fn process_input(input: &str, _tx: &mpsc::Sender<Task>, http_client: &reqwest::Client) -> String {
    if input.starts_with('!') {
        let cmd = &input[1..];
        match shell::run_shell(cmd) {
            Ok((stdout, stderr, status)) => {
                let mut out = String::new();
                if !stdout.is_empty() {
                    out.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&format!("STDERR: {stderr}"));
                }
                if out.is_empty() {
                    out.push_str(&format!("Exit: {:?}", status.code()));
                }
                out
            }
            Err(e) => format!("Error: {e}"),
        }
    } else {
        let config = LlmConfig::from_env();
        if !config.is_configured() {
            return "LLM not configured. Use !<command> to run shell, or set LLM_API_KEY in .env".to_string();
        }
        let mcp_clients: Vec<Arc<McpClient>> = vec![];
        let mut chat = AgentChat::new(
            config,
            mcp_clients,
            http_client.clone(),
            None,
        );
        chat.send_message(input).await
    }
}
