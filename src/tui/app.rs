use std::{cmp::min, io::Stdout, time::Duration};

use anyhow::{Context, Result, anyhow};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
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
    config::{AppConfig, AuthConfig, CampusConfig, DaemonConfig, DetectConfig},
    network::{CampusEnvironment, detect_campus_environment},
    portal::{LoginStatus, PortalClient},
};

use super::{
    input::InputField,
    status::{StatusKind, StatusMessage},
};

const FIELD_COUNT: usize = 8;
const BUTTON_COUNT: usize = 4;
const LABEL_WIDTH: usize = 24;
const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(200);

const FIELD_USERNAME: usize = 0;
const FIELD_PASSWORD: usize = 1;
const FIELD_PORTAL_URL: usize = 2;
const FIELD_PROBE_URL: usize = 3;
const FIELD_ONLINE_INTERVAL: usize = 4;
const FIELD_REQUEST_TIMEOUT: usize = 5;
const FIELD_CAMPUS_CIDRS: usize = 6;
const FIELD_GATEWAYS: usize = 7;

const BUTTON_SAVE: usize = 0;
const BUTTON_TEST: usize = 1;
const BUTTON_AUTOSTART: usize = 2;
const BUTTON_QUIT: usize = 3;

pub(super) struct SetupApp {
    fields: [InputField; FIELD_COUNT],
    focus: Focus,
    show_password: bool,
    show_shortcuts: bool,
    status: StatusMessage,
    should_quit: bool,
    ui_state: UiState,
    drag_state: Option<DragState>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Field(usize),
    Button(usize),
}

#[derive(Default)]
struct UiState {
    field_rows: Vec<Rect>,
    button_hitboxes: Vec<ButtonHitbox>,
    value_start_x: u16,
}

#[derive(Clone, Copy)]
struct ButtonHitbox {
    index: usize,
    rect: Rect,
}

#[derive(Clone, Copy)]
struct DragState {
    field_index: usize,
    anchor: usize,
}

impl UiState {
    fn reset(&mut self, value_start_x: u16) {
        self.field_rows.clear();
        self.button_hitboxes.clear();
        self.value_start_x = value_start_x;
    }

    fn field_at(&self, column: u16, row: u16) -> Option<usize> {
        self.field_rows
            .iter()
            .enumerate()
            .find(|(_, rect)| contains_point(**rect, column, row))
            .map(|(index, _)| index)
    }

    fn button_at(&self, column: u16, row: u16) -> Option<usize> {
        self.button_hitboxes
            .iter()
            .find(|hitbox| contains_point(hitbox.rect, column, row))
            .map(|hitbox| hitbox.index)
    }
}

fn contains_point(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

impl SetupApp {
    pub(super) fn new(config: AppConfig) -> Self {
        Self {
            fields: [
                InputField::new("Username", config.auth.username, false),
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
                InputField::new("Campus IPv4 CIDRs", config.campus.ipv4_cidrs.join(", "), false),
                InputField::new("Campus gateways", config.campus.gateway_hosts.join(", "), false),
            ],
            focus: Focus::Field(FIELD_USERNAME),
            show_password: false,
            show_shortcuts: false,
            status: StatusMessage::info(
                "Edit fields, then Save or Save & Test. Mouse wheel/click/drag is enabled. Press ? for shortcuts.",
            ),
            should_quit: false,
            ui_state: UiState::default(),
            drag_state: None,
        }
    }

    pub(super) fn run(mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| self.draw(frame))?;

            if !event::poll(EVENT_POLL_INTERVAL).context("failed to poll terminal events")? {
                continue;
            }

            match event::read().context("failed to read terminal event")? {
                Event::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(key)?,
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
                code: KeyCode::Char('?'),
                ..
            } => {
                self.show_shortcuts = !self.show_shortcuts;
                self.status = StatusMessage::info(if self.show_shortcuts {
                    "Shortcuts are now visible."
                } else {
                    "Shortcuts hidden. Press ? to show them."
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
            } if self.focus == Focus::Button(BUTTON_AUTOSTART) => self.toggle_autostart_entry(),
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => match self.focus {
                Focus::Field(_) => self.next_focus(),
                Focus::Button(index) => self.activate_button(index),
            },
            KeyEvent {
                code: KeyCode::Left,
                ..
            } => {
                if let Some(field) = self.current_field_mut() {
                    field.move_left();
                } else {
                    self.move_button_focus_left();
                }
            }
            KeyEvent {
                code: KeyCode::Right,
                ..
            } => {
                if let Some(field) = self.current_field_mut() {
                    field.move_right();
                } else {
                    self.move_button_focus_right();
                }
            }
            KeyEvent {
                code: KeyCode::Home,
                ..
            } => {
                if let Some(field) = self.current_field_mut() {
                    field.move_home();
                }
            }
            KeyEvent {
                code: KeyCode::End, ..
            } => {
                if let Some(field) = self.current_field_mut() {
                    field.move_end();
                }
            }
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                if let Some(field) = self.current_field_mut() {
                    field.backspace();
                }
            }
            KeyEvent {
                code: KeyCode::Delete,
                ..
            } => {
                if let Some(field) = self.current_field_mut() {
                    field.delete();
                }
            }
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers,
                ..
            } if self.current_field_mut().is_some()
                && !modifiers.contains(KeyModifiers::CONTROL)
                && !modifiers.contains(KeyModifiers::ALT) =>
            {
                if let Some(field) = self.current_field_mut() {
                    field.insert(ch);
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollDown => self.next_focus(),
            MouseEventKind::ScrollUp => self.previous_focus(),
            MouseEventKind::Down(MouseButton::Left) => self.handle_left_down(mouse),
            MouseEventKind::Drag(MouseButton::Left) => self.handle_left_drag(mouse),
            MouseEventKind::Up(MouseButton::Left) => self.drag_state = None,
            _ => {}
        }
    }

    fn handle_left_down(&mut self, mouse: MouseEvent) {
        if let Some(button_index) = self.ui_state.button_at(mouse.column, mouse.row) {
            self.set_focus(Focus::Button(button_index));
            self.activate_button(button_index);
            return;
        }

        if let Some(field_index) = self.ui_state.field_at(mouse.column, mouse.row) {
            self.set_focus(Focus::Field(field_index));
            let cursor = self.cursor_from_mouse_column(field_index, mouse.column);
            self.fields[field_index].set_cursor(cursor);
            self.drag_state = Some(DragState {
                field_index,
                anchor: cursor,
            });
            return;
        }

        self.drag_state = None;
    }

    fn handle_left_drag(&mut self, mouse: MouseEvent) {
        let Some(drag_state) = self.drag_state else {
            return;
        };
        if drag_state.field_index >= FIELD_COUNT {
            return;
        }

        self.set_focus(Focus::Field(drag_state.field_index));
        let cursor = self.cursor_from_mouse_column(drag_state.field_index, mouse.column);
        self.fields[drag_state.field_index].set_cursor_with_anchor(drag_state.anchor, cursor);
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(if self.show_shortcuts { 3 } else { 0 }),
                Constraint::Min(13),
                Constraint::Length(5),
            ])
            .split(area);

        self.draw_title(frame, chunks[0]);
        if self.show_shortcuts {
            self.draw_shortcuts(frame, chunks[1]);
        }
        self.draw_form(frame, chunks[2]);
        self.draw_status(frame, chunks[3]);
    }

    fn draw_title(&self, frame: &mut Frame, area: Rect) {
        let shortcut_hint = if self.show_shortcuts {
            "Press ? to hide shortcuts"
        } else {
            "Press ? to show shortcuts"
        };
        let title = Paragraph::new(Line::from(vec![
            Span::styled(
                "campus-network-autologin",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(shortcut_hint, Style::default().fg(Color::Gray)),
        ]));
        frame.render_widget(title, area);
    }

    fn draw_shortcuts(&self, frame: &mut Frame, area: Rect) {
        let help = Paragraph::new(vec![
            Line::from("Esc/Ctrl+C: quit"),
            Line::from("Tab/Shift+Tab: move     Enter: next/activate"),
            Line::from("Ctrl+S: save&test       Ctrl+T: save&test"),
            Line::from("Ctrl+A: autostart       F2: show password"),
        ])
        .wrap(Wrap { trim: true });
        frame.render_widget(help, area);
    }

    fn draw_form(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .title("Setup")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let value_start_x = inner.x + (LABEL_WIDTH + 3) as u16;
        self.ui_state.reset(value_start_x);

        for (index, field) in self.fields.iter().enumerate() {
            let row = Rect {
                x: inner.x,
                y: inner.y + index as u16,
                width: inner.width,
                height: 1,
            };
            self.ui_state.field_rows.push(row);

            let selected = self.focus == Focus::Field(index);
            let row_style = if selected {
                Style::default().bg(Color::Blue).fg(Color::Black)
            } else {
                Style::default()
            };
            let selection_style = if selected {
                Style::default()
                    .bg(Color::Yellow)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD)
            } else {
                row_style
            };

            let marker = if selected { ">" } else { " " };
            let label = format!("{marker} {:<width$} ", field.label(), width = LABEL_WIDTH);
            let mut spans = vec![Span::styled(label, row_style.add_modifier(Modifier::BOLD))];
            spans.extend(field.display_spans(self.show_password, row_style, selection_style));
            frame.render_widget(Paragraph::new(Line::from(spans)), row);
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
                Line::from(format!("Autostart state: {autostart_state}")),
                Line::from("Mouse: click fields/buttons, wheel switch focus, drag to select text"),
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
        let (button_line, hitboxes) = self.button_line_and_hitboxes(&labels, button_row);
        self.ui_state.button_hitboxes = hitboxes;
        frame.render_widget(Paragraph::new(button_line), button_row);

        if let Focus::Field(field_index) = self.focus {
            let field = &self.fields[field_index];
            let value_len = field.display_char_width(self.show_password) as u16;
            let visible_cursor = min(field.cursor() as u16, value_len);
            let max_x = inner.x + inner.width.saturating_sub(1);
            let cursor_x = min(self.ui_state.value_start_x.saturating_add(visible_cursor), max_x);
            let cursor_y = inner.y + field_index as u16;
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

    fn button_labels(&self) -> [String; BUTTON_COUNT] {
        let autostart_button = match autostart_enabled() {
            Ok(true) => "Autostart: ON".to_owned(),
            Ok(false) => "Autostart: OFF".to_owned(),
            Err(_) => "Autostart: N/A".to_owned(),
        };
        [
            "Save".to_owned(),
            "Save & Test".to_owned(),
            autostart_button,
            "Quit".to_owned(),
        ]
    }

    fn button_line_and_hitboxes(
        &self,
        labels: &[String; BUTTON_COUNT],
        row: Rect,
    ) -> (Line<'static>, Vec<ButtonHitbox>) {
        let mut spans = Vec::with_capacity(BUTTON_COUNT * 2);
        let mut hitboxes = Vec::with_capacity(BUTTON_COUNT);
        let mut x = row.x;

        for (index, label) in labels.iter().enumerate() {
            let text = format!("[ {label} ]");
            let width = text.chars().count() as u16;
            let selected = self.focus == Focus::Button(index);
            let style = if selected {
                Style::default()
                    .bg(Color::Green)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            spans.push(Span::styled(text, style));
            if index + 1 < BUTTON_COUNT {
                spans.push(Span::raw("  "));
            }
            hitboxes.push(ButtonHitbox {
                index,
                rect: Rect {
                    x,
                    y: row.y,
                    width,
                    height: 1,
                },
            });
            x = x.saturating_add(width + 2);
        }

        (Line::from(spans), hitboxes)
    }

    fn cursor_from_mouse_column(&self, field_index: usize, column: u16) -> usize {
        let value_start_x = self.ui_state.value_start_x;
        if column <= value_start_x {
            return 0;
        }
        let offset = column.saturating_sub(value_start_x) as usize;
        min(
            offset,
            self.fields[field_index].display_char_width(self.show_password),
        )
    }

    fn current_field_mut(&mut self) -> Option<&mut InputField> {
        let Focus::Field(index) = self.focus else {
            return None;
        };
        Some(&mut self.fields[index])
    }

    fn next_focus(&mut self) {
        let next = (self.focus_order() + 1) % (FIELD_COUNT + BUTTON_COUNT);
        self.set_focus_by_order(next);
    }

    fn previous_focus(&mut self) {
        let current = self.focus_order();
        let previous = if current == 0 {
            FIELD_COUNT + BUTTON_COUNT - 1
        } else {
            current - 1
        };
        self.set_focus_by_order(previous);
    }

    fn focus_order(&self) -> usize {
        match self.focus {
            Focus::Field(index) => index,
            Focus::Button(index) => FIELD_COUNT + index,
        }
    }

    fn set_focus_by_order(&mut self, focus_order: usize) {
        if focus_order < FIELD_COUNT {
            self.set_focus(Focus::Field(focus_order));
        } else {
            self.set_focus(Focus::Button(focus_order - FIELD_COUNT));
        }
    }

    fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
        self.drag_state = None;
        self.clear_unfocused_selection();
    }

    fn clear_unfocused_selection(&mut self) {
        match self.focus {
            Focus::Field(active_index) => {
                for (index, field) in self.fields.iter_mut().enumerate() {
                    if index != active_index {
                        field.clear_selection();
                    }
                }
            }
            Focus::Button(_) => {
                for field in &mut self.fields {
                    field.clear_selection();
                }
            }
        }
    }

    fn move_button_focus_left(&mut self) {
        let Focus::Button(index) = self.focus else {
            return;
        };
        if index > 0 {
            self.set_focus(Focus::Button(index - 1));
        }
    }

    fn move_button_focus_right(&mut self) {
        let Focus::Button(index) = self.focus else {
            return;
        };
        if index + 1 < BUTTON_COUNT {
            self.set_focus(Focus::Button(index + 1));
        }
    }

    fn activate_button(&mut self, button_index: usize) {
        match button_index {
            BUTTON_SAVE => self.save(),
            BUTTON_TEST => self.save_and_test(),
            BUTTON_AUTOSTART => self.toggle_autostart_entry(),
            BUTTON_QUIT => self.should_quit = true,
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
                    self.status = StatusMessage::success(format!("Saved config to {}", path.display()))
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
            let client = PortalClient::new(Duration::from_secs(config.detect.request_timeout_secs))?;
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
        let online_check_interval_secs = self.fields[FIELD_ONLINE_INTERVAL]
            .value()
            .trim()
            .parse::<u64>()
            .with_context(|| "online check interval must be a positive integer")?;
        let request_timeout_secs = self.fields[FIELD_REQUEST_TIMEOUT]
            .value()
            .trim()
            .parse::<u64>()
            .with_context(|| "request timeout must be a positive integer")?;
        let campus_ipv4_cidrs = parse_csv_list(self.fields[FIELD_CAMPUS_CIDRS].value());
        let campus_gateway_hosts = parse_csv_list(self.fields[FIELD_GATEWAYS].value());

        let config = AppConfig {
            auth: AuthConfig {
                username: self.fields[FIELD_USERNAME].value().trim().to_owned(),
                password: self.fields[FIELD_PASSWORD].value().to_owned(),
                portal_url: self.fields[FIELD_PORTAL_URL].value().trim().to_owned(),
            },
            detect: DetectConfig {
                probe_url: self.fields[FIELD_PROBE_URL].value().trim().to_owned(),
                request_timeout_secs,
            },
            daemon: DaemonConfig {
                online_check_interval_secs,
            },
            campus: CampusConfig {
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
