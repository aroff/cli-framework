//! ModalView widget implementation
//!
//! Displays modal dialogs for detailed feedback/errors

use crate::message::AppMessage;
use crate::view::Theme;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

/// ModalView widget for displaying modal dialogs
pub struct ModalView {
    message: Option<AppMessage>,
    visible: bool,
    theme: Theme,
}

impl ModalView {
    /// Create a new modal view
    pub fn new(theme: Theme) -> Self {
        Self {
            message: None,
            visible: false,
            theme,
        }
    }

    /// Show a message in the modal
    pub fn show(&mut self, message: AppMessage) {
        self.message = Some(message);
        self.visible = true;
    }

    /// Hide the modal
    pub fn hide(&mut self) {
        self.visible = false;
        self.message = None;
    }

    /// Check if visible
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Render the modal
    pub fn render(&self, f: &mut Frame, area: Rect) {
        if !self.visible || self.message.is_none() {
            return;
        }

        let message = self.message.as_ref().unwrap();

        // Create a centered modal area
        let vertical = Layout::vertical([
            Constraint::Percentage(25),
            Constraint::Percentage(50),
            Constraint::Percentage(25),
        ])
        .split(area);

        let horizontal = Layout::horizontal([
            Constraint::Percentage(20),
            Constraint::Percentage(60),
            Constraint::Percentage(20),
        ])
        .split(vertical[1]);

        let modal_area = horizontal[1];

        // Determine style based on message kind
        let style = match message.kind {
            crate::message::AppMessageKind::Info => self.theme.secondary_style,
            crate::message::AppMessageKind::Warning => self.theme.error_style,
            crate::message::AppMessageKind::Error => self.theme.error_style,
        };

        // Build text content
        let mut lines = vec![Line::from(Span::styled(
            message.short.clone(),
            style.clone(),
        ))];

        if let Some(ref details) = message.details {
            lines.push(Line::from(""));
            for line in details.lines() {
                lines.push(Line::from(Span::styled(line, self.theme.secondary_style)));
            }
        }

        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(match message.kind {
                        crate::message::AppMessageKind::Info => "Information",
                        crate::message::AppMessageKind::Warning => "Warning",
                        crate::message::AppMessageKind::Error => "Error",
                    })
                    .style(self.theme.modal_style),
            )
            .wrap(Wrap { trim: true });

        f.render_widget(paragraph, modal_area);
    }
}
