//! CommandPalette widget for displaying and executing commands

use crate::command::Command;
use crate::view::Theme;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;
use std::collections::HashMap;

/// CommandPalette widget for displaying and executing commands
pub struct CommandPalette {
    commands: Vec<Command>,
    input: String,
    selected_index: usize,
    visible: bool,
    theme: Theme,
}

impl CommandPalette {
    /// Create a new command palette
    pub fn new(theme: Theme) -> Self {
        Self {
            commands: Vec::new(),
            input: String::new(),
            selected_index: 0,
            visible: false,
            theme,
        }
    }

    /// Set available commands
    pub fn set_commands(&mut self, commands: Vec<Command>) {
        self.commands = commands;
        self.selected_index = 0;
    }

    /// Show the command palette
    pub fn show(&mut self) {
        self.visible = true;
        self.input.clear();
        self.selected_index = 0;
    }

    /// Hide the command palette
    pub fn hide(&mut self) {
        self.visible = false;
        self.input.clear();
    }

    /// Check if visible
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Get current input
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Handle key event
    pub fn handle_key(&mut self, key: KeyCode) -> CommandPaletteResult {
        match key {
            KeyCode::Esc => {
                self.hide();
                return CommandPaletteResult::Cancel;
            }
            KeyCode::Enter => {
                if let Some(command) = self.get_selected_command() {
                    let command_id = command.id.to_string();
                    self.hide();
                    return CommandPaletteResult::Execute(command_id);
                }
            }
            KeyCode::Up => {
                if !self.filtered_commands().is_empty() {
                    self.selected_index = if self.selected_index == 0 {
                        self.filtered_commands().len() - 1
                    } else {
                        self.selected_index - 1
                    };
                }
            }
            KeyCode::Down => {
                let filtered = self.filtered_commands();
                if !filtered.is_empty() {
                    self.selected_index = (self.selected_index + 1) % filtered.len();
                }
            }
            KeyCode::Char(c) => {
                if c == ':' && self.input.is_empty() {
                    // Already in command mode
                } else {
                    self.input.push(c);
                    self.selected_index = 0;
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.selected_index = 0;
            }
            _ => {}
        }
        CommandPaletteResult::Continue
    }

    /// Get filtered commands based on input
    fn filtered_commands(&self) -> Vec<&Command> {
        if self.input.is_empty() {
            return self.commands.iter().collect();
        }
        self.commands
            .iter()
            .filter(|cmd| {
                cmd.id.contains(&self.input)
                    || cmd.summary.to_lowercase().contains(&self.input.to_lowercase())
            })
            .collect()
    }

    /// Get selected command
    fn get_selected_command(&self) -> Option<&Command> {
        let filtered = self.filtered_commands();
        filtered.get(self.selected_index).copied()
    }

    /// Render the command palette
    pub fn render(&self, f: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }

        // Create a centered modal area
        let vertical = Layout::vertical([
            Constraint::Percentage(30),
            Constraint::Percentage(40),
            Constraint::Percentage(30),
        ])
        .split(area);

        let horizontal = Layout::horizontal([
            Constraint::Percentage(20),
            Constraint::Percentage(60),
            Constraint::Percentage(20),
        ])
        .split(vertical[1]);

        let modal_area = horizontal[1];

        // Split modal into input and command list
        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(5),
        ])
        .split(modal_area);

        // Render input field
        let input_text = format!(":{}", self.input);
        let input_paragraph = Paragraph::new(input_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Command")
                    .style(self.theme.modal_style),
            );
        f.render_widget(input_paragraph, chunks[0]);

        // Render command list
        let filtered = self.filtered_commands();
        let items: Vec<ListItem> = filtered
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                let is_selected = i == self.selected_index;
                let style = if is_selected {
                    self.theme.primary_style.add_modifier(Modifier::REVERSED)
                } else {
                    self.theme.secondary_style
                };

                let mut spans = vec![
                    Span::styled(format!("{:20}", cmd.id), style.clone()),
                    Span::raw(" "),
                ];

                if let Some(syntax) = cmd.syntax {
                    spans.push(Span::styled(
                        format!("{} ", syntax),
                        Style::default().fg(ratatui::style::Color::Gray),
                    ));
                }

                spans.push(Span::styled(cmd.summary, style));

                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Commands")
                    .style(self.theme.modal_style),
            );

        f.render_widget(list, chunks[1]);
    }
}

/// Result of command palette interaction
#[derive(Debug, Clone)]
pub enum CommandPaletteResult {
    /// Continue showing palette
    Continue,
    /// Cancel (user pressed Esc)
    Cancel,
    /// Execute command with given ID
    Execute(String),
}
