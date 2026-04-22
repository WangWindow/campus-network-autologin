use std::{
    cmp::min,
    io::{self, Stdout},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::{
    autostart::{
        autostart_enabled, install_autostart as install_user_autostart,
        remove_autostart as remove_user_autostart,
    },
    config::AppConfig,
    network::{CampusEnvironment, detect_campus_environment},
    portal::{LoginStatus, PortalClient},
};

const FIELD_COUNT: usize = 8;
const BUTTON_COUNT: usize = 4;
const LABEL_WIDTH: usize = 24;

pub fn run_setup_tui(config: AppConfig) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let result = SetupApp::new(config).run(&mut terminal);
    let restore_result = restore_terminal(&mut terminal);
    restore_result?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("failed to enter alternate screen")?;
    Terminal::new(CrosstermBackend::new(stdout)).context("failed to create terminal")
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().context("failed to disable raw mode")?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .context("failed to leave alternate screen")?;
    terminal.show_cursor().context("failed to show cursor")
}

struct SetupApp {
    fields: [InputField; FIELD_COUNT],
    focus: usize,
    show_password: bool,
    status: StatusMessage,
    should_quit: bool,
    autostart_button_hitbox: Option<Rect>,
}

impl SetupApp {
    fn new(config: AppConfig) -> Self {
        Self {
            fields: [
                InputField::new("Username / student ID", config.auth.username, false),
                InputField::new("Password", config.auth.password, true),
                InputField::new("Portal URL", config.auth.portal_url, false),
                InputField::new("Probe URL", config.detect.probe_url, false),
                InputField::new(
                    "Online check interval",
                    config.daemon.online_check_interval_secs.to_string(),
                    false,
                ),
                InputField::new(
                    "Request timeout",
                    config.detect.request_timeout_secs.to_string(),
                    false,
                ),
                InputField::new(
                    "Campus IPv4 CIDRs",
                    config.campus.ipv4_cidrs.join(", "),
                    false,
                ),
                InputField::new(
                    "Campus gateways",
                    config.campus.gateway_hosts.join(", "),
                    false,
                ),
            ],
            focus: 0,
            show_password: false,
            status: StatusMessage::info(
                "Edit fields, then Save or Save & Test. CIDRs are optional; gateways are required.",
            ),
            should_quit: false,
            autostart_button_hitbox: None,
        }
    }

    fn run(mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| self.draw(frame))?;

            if !event::poll(Duration::from_millis(200)).context("failed to poll terminal events")? {
                continue;
            }

            match event::read().context("failed to read terminal event")? {
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Press {
                        self.handle_key(key)?;
                    }
                }
                Event::Mouse(mouse) => self.handle_mouse(mouse),
                _ => {}
            }
        }

        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        match key {
            KeyEvent {
                code: KeyCode::Esc, ..
            }
            | KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.should_quit = true;
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            }
            | KeyEvent {
                code: KeyCode::Down,
                ..
            } => self.next_focus(),
            KeyEvent {
                code: KeyCode::BackTab,
                ..
            }
            | KeyEvent {
                code: KeyCode::Up, ..
            } => self.previous_focus(),
            KeyEvent {
                code: KeyCode::F(2),
                ..
            } => {
                self.show_password = !self.show_password;
                self.status = StatusMessage::info(if self.show_password {
                    "Password is now visible."
                } else {
                    "Password is now hidden."
                });
            }
            KeyEvent {
                code: KeyCode::Char('s'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) => self.save(),
            KeyEvent {
                code: KeyCode::Char('t'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) => self.save_and_test(),
            KeyEvent {
                code: KeyCode::Char('a'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) => self.toggle_autostart_entry(),
            KeyEvent {
                code: KeyCode::Char(' '),
                ..
            } if self.focus == FIELD_COUNT + 2 => self.toggle_autostart_entry(),
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                if self.focus < FIELD_COUNT {
                    self.next_focus();
                } else {
                    self.activate_button();
                }
            }
            KeyEvent {
                code: KeyCode::Left,
                ..
            } => {
                if self.focus < FIELD_COUNT {
                    self.current_field_mut().move_left();
                } else {
                    self.move_button_focus_left();
                }
            }
            KeyEvent {
                code: KeyCode::Right,
                ..
            } => {
                if self.focus < FIELD_COUNT {
                    self.current_field_mut().move_right();
                } else {
                    self.move_button_focus_right();
                }
            }
            KeyEvent {
                code: KeyCode::Home,
                ..
            } if self.focus < FIELD_COUNT => self.current_field_mut().move_home(),
            KeyEvent {
                code: KeyCode::End, ..
            } if self.focus < FIELD_COUNT => self.current_field_mut().move_end(),
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } if self.focus < FIELD_COUNT => self.current_field_mut().backspace(),
            KeyEvent {
                code: KeyCode::Delete,
                ..
            } if self.focus < FIELD_COUNT => self.current_field_mut().delete(),
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers,
                ..
            } if self.focus < FIELD_COUNT && !modifiers.contains(KeyModifiers::CONTROL) => {
                self.current_field_mut().insert(ch);
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            if let Some(rect) = self.autostart_button_hitbox {
                let inside_x = mouse.column >= rect.x && mouse.column < rect.x + rect.width;
                let inside_y = mouse.row >= rect.y && mouse.row < rect.y + rect.height;
                if inside_x && inside_y {
                    self.focus = FIELD_COUNT + 2;
                    self.toggle_autostart_entry();
                }
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(3),
                Constraint::Min(13),
                Constraint::Length(5),
            ])
            .split(area);

        let title = Paragraph::new("campus-network-autologin")
            .style(Style::default().add_modifier(Modifier::BOLD));
        frame.render_widget(title, chunks[0]);

        let help = Paragraph::new(vec![
            Line::from("Tab/Shift+Tab: move    Enter: next/activate    Ctrl+S: save"),
            Line::from("Ctrl+T: save and test    F2: show/hide password    Esc: quit"),
        ])
        .wrap(Wrap { trim: true });
        frame.render_widget(help, chunks[1]);

        self.draw_form(frame, chunks[2]);
        self.draw_status(frame, chunks[3]);
    }

    fn draw_form(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .title("Setup")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        for (index, field) in self.fields.iter().enumerate() {
            let row = Rect {
                x: inner.x,
                y: inner.y + index as u16,
                width: inner.width,
                height: 1,
            };
            let selected = self.focus == index;
            let style = if selected {
                Style::default().bg(Color::Blue).fg(Color::Black)
            } else {
                Style::default()
            };
            let label = format!(
                "{} {:<width$} ",
                if selected { ">" } else { " " },
                field.label,
                width = LABEL_WIDTH
            );
            let value = field.display_value(self.show_password);
            let line = Line::from(vec![
                Span::styled(label, style.add_modifier(Modifier::BOLD)),
                Span::styled(value, style),
            ]);
            frame.render_widget(Paragraph::new(line), row);
        }

        let path_line = Rect {
            x: inner.x,
            y: inner.y + FIELD_COUNT as u16,
            width: inner.width,
            height: 3,
        };
        let config_path = match AppConfig::config_path() {
            Ok(path) => path.display().to_string(),
            Err(error) => format!("unavailable: {error}"),
        };
        let autostart_state = match autostart_enabled() {
            Ok(true) => "enabled",
            Ok(false) => "disabled",
            Err(_) => "unavailable",
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(format!("Config path: {config_path}")),
                Line::from(format!(
                    "Password visibility: {}",
                    if self.show_password {
                        "shown"
                    } else {
                        "hidden"
                    }
                )),
                Line::from(format!("Autostart: {autostart_state}")),
            ])
            .wrap(Wrap { trim: true }),
            path_line,
        );

        let button_row = Rect {
            x: inner.x,
            y: inner.y + FIELD_COUNT as u16 + 3,
            width: inner.width,
            height: 1,
        };
        let labels = self.button_labels();
        frame.render_widget(Paragraph::new(self.button_line(&labels)), button_row);
        self.autostart_button_hitbox = self.autostart_button_hitbox(&labels, button_row);

        if self.focus < FIELD_COUNT {
            let field = &self.fields[self.focus];
            let value_len = field.display_char_width(self.show_password) as u16;
            let visible_cursor = min(field.cursor as u16, value_len);
            let cursor_x = button_row
                .x
                .saturating_sub(0)
                .max(inner.x + (LABEL_WIDTH + 3) as u16 + visible_cursor);
            let cursor_y = inner.y + self.focus as u16;
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }

    fn draw_status(&self, frame: &mut Frame, area: Rect) {
        let (title, color) = match self.status.kind {
            StatusKind::Info => ("Status", Color::Cyan),
            StatusKind::Success => ("Success", Color::Green),
            StatusKind::Error => ("Error", Color::Red),
        };
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(color));
        frame.render_widget(
            Paragraph::new(self.status.message.as_str())
                .block(block)
                .wrap(Wrap { trim: true }),
            area,
        );
    }

    fn button_labels(&self) -> Vec<String> {
        let autostart_button = match autostart_enabled() {
            Ok(true) => "Autostart: ON",
            Ok(false) => "Autostart: OFF",
            Err(_) => "Autostart: N/A",
        };
        vec![
            "Save".to_owned(),
            "Save & Test".to_owned(),
            autostart_button.to_owned(),
            "Quit".to_owned(),
        ]
    }

    fn button_line(&self, buttons: &[String]) -> Line<'static> {
        let spans = buttons
            .iter()
            .enumerate()
            .flat_map(|(index, label)| {
                let selected = self.focus == FIELD_COUNT + index;
                let style = if selected {
                    Style::default()
                        .bg(Color::Green)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                [Span::styled(format!("[ {label} ]"), style), Span::raw("  ")]
            })
            .collect::<Vec<_>>();
        Line::from(spans)
    }

    fn autostart_button_hitbox(&self, buttons: &[String], row: Rect) -> Option<Rect> {
        let mut x = row.x;
        for (index, label) in buttons.iter().enumerate() {
            let button = format!("[ {label} ]");
            let width = button.chars().count() as u16;
            if index == 2 {
                return Some(Rect {
                    x,
                    y: row.y,
                    width,
                    height: 1,
                });
            }
            x = x.saturating_add(width + 2);
        }
        None
    }

    fn current_field_mut(&mut self) -> &mut InputField {
        &mut self.fields[self.focus]
    }

    fn next_focus(&mut self) {
        self.focus = (self.focus + 1) % (FIELD_COUNT + BUTTON_COUNT);
    }

    fn previous_focus(&mut self) {
        self.focus = if self.focus == 0 {
            FIELD_COUNT + BUTTON_COUNT - 1
        } else {
            self.focus - 1
        };
    }

    fn move_button_focus_left(&mut self) {
        if self.focus > FIELD_COUNT {
            self.focus -= 1;
        }
    }

    fn move_button_focus_right(&mut self) {
        if self.focus + 1 < FIELD_COUNT + BUTTON_COUNT {
            self.focus += 1;
        }
    }

    fn activate_button(&mut self) {
        match self.focus - FIELD_COUNT {
            0 => self.save(),
            1 => self.save_and_test(),
            2 => self.toggle_autostart_entry(),
            3 => {
                self.should_quit = true;
            }
            _ => {}
        }
    }

    fn save(&mut self) {
        match self.try_build_config().and_then(|config| {
            config.save()?;
            Ok(config)
        }) {
            Ok(_) => match AppConfig::config_path() {
                Ok(path) => {
                    self.status =
                        StatusMessage::success(format!("Saved config to {}", path.display()))
                }
                Err(error) => {
                    self.status = StatusMessage::success(format!(
                        "Saved config, but path lookup failed: {error}"
                    ))
                }
            },
            Err(error) => self.status = StatusMessage::error(error.to_string()),
        }
    }

    fn save_and_test(&mut self) {
        let result = self.try_build_config().and_then(|config| {
            config.save()?;
            match detect_campus_environment(&config)? {
                CampusEnvironment::OnCampus(_) => {}
                CampusEnvironment::OffCampus(reason) => {
                    return Err(anyhow!("campus network not detected: {reason}"));
                }
            }
            let client =
                PortalClient::new(Duration::from_secs(config.detect.request_timeout_secs))?;
            let outcome = client.login_and_verify(&config)?;
            Ok(outcome)
        });

        match result {
            Ok(outcome) => {
                self.status = match outcome.status {
                    LoginStatus::Success => {
                        StatusMessage::success(format!("Login successful: {}", outcome.detail))
                    }
                    LoginStatus::Failed => {
                        StatusMessage::error(format!("Login failed: {}", outcome.detail))
                    }
                };
            }
            Err(error) => self.status = StatusMessage::error(error.to_string()),
        }
    }

    fn toggle_autostart_entry(&mut self) {
        match autostart_enabled() {
            Ok(true) => match remove_user_autostart() {
                Ok(path) => {
                    self.status =
                        StatusMessage::success(format!("Autostart disabled: {}", path.display()));
                }
                Err(error) => self.status = StatusMessage::error(error.to_string()),
            },
            Ok(false) => match install_user_autostart() {
                Ok(path) => {
                    self.status =
                        StatusMessage::success(format!("Autostart enabled: {}", path.display()));
                }
                Err(error) => self.status = StatusMessage::error(error.to_string()),
            },
            Err(error) => self.status = StatusMessage::error(error.to_string()),
        }
    }

    fn try_build_config(&self) -> Result<AppConfig> {
        let online_check_interval_secs = self.fields[4]
            .value
            .trim()
            .parse::<u64>()
            .with_context(|| "online check interval must be a positive integer")?;
        let request_timeout_secs = self.fields[5]
            .value
            .trim()
            .parse::<u64>()
            .with_context(|| "request timeout must be a positive integer")?;
        let campus_ipv4_cidrs = parse_csv_list(&self.fields[6].value);
        let campus_gateway_hosts = parse_csv_list(&self.fields[7].value);

        let config = AppConfig {
            auth: crate::config::AuthConfig {
                username: self.fields[0].value.trim().to_owned(),
                password: self.fields[1].value.clone(),
                portal_url: self.fields[2].value.trim().to_owned(),
            },
            detect: crate::config::DetectConfig {
                probe_url: self.fields[3].value.trim().to_owned(),
                request_timeout_secs,
            },
            daemon: crate::config::DaemonConfig {
                online_check_interval_secs,
            },
            campus: crate::config::CampusConfig {
                ipv4_cidrs: campus_ipv4_cidrs,
                gateway_hosts: campus_gateway_hosts,
            },
        };

        config.validate().map_err(|error| anyhow!(error))?;
        Ok(config)
    }
}

fn parse_csv_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

struct InputField {
    label: &'static str,
    value: String,
    cursor: usize,
    secret: bool,
}

impl InputField {
    fn new(label: &'static str, value: String, secret: bool) -> Self {
        let cursor = value.chars().count();
        Self {
            label,
            value,
            cursor,
            secret,
        }
    }

    fn display_value(&self, show_secret: bool) -> String {
        if self.secret && !show_secret {
            "*".repeat(self.value.chars().count())
        } else {
            self.value.clone()
        }
    }

    fn display_char_width(&self, show_secret: bool) -> usize {
        if self.secret && !show_secret {
            self.value.chars().count()
        } else {
            self.value.chars().count()
        }
    }

    fn insert(&mut self, ch: char) {
        let idx = byte_index(&self.value, self.cursor);
        self.value.insert(idx, ch);
        self.cursor += 1;
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let end = byte_index(&self.value, self.cursor);
        let start = byte_index(&self.value, self.cursor - 1);
        self.value.replace_range(start..end, "");
        self.cursor -= 1;
    }

    fn delete(&mut self) {
        if self.cursor >= self.value.chars().count() {
            return;
        }
        let start = byte_index(&self.value, self.cursor);
        let end = byte_index(&self.value, self.cursor + 1);
        self.value.replace_range(start..end, "");
    }

    fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_right(&mut self) {
        self.cursor = min(self.cursor + 1, self.value.chars().count());
    }

    fn move_home(&mut self) {
        self.cursor = 0;
    }

    fn move_end(&mut self) {
        self.cursor = self.value.chars().count();
    }
}

fn byte_index(value: &str, char_index: usize) -> usize {
    if char_index == 0 {
        return 0;
    }
    value
        .char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or(value.len())
}

struct StatusMessage {
    kind: StatusKind,
    message: String,
}

impl StatusMessage {
    fn info(message: impl Into<String>) -> Self {
        Self {
            kind: StatusKind::Info,
            message: message.into(),
        }
    }

    fn success(message: impl Into<String>) -> Self {
        Self {
            kind: StatusKind::Success,
            message: message.into(),
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            kind: StatusKind::Error,
            message: message.into(),
        }
    }
}

enum StatusKind {
    Info,
    Success,
    Error,
}
