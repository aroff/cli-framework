//! StatusBar widget implementation
//!
//! Displays status information at the bottom of the screen

use crate::message::{AppMessage, AppMessageKind};
use crate::view::Theme;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

/// StatusBar widget for displaying status information
pub struct StatusBar {
    message: Option<AppMessage>,
    theme: Theme,
}

impl StatusBar {
    /// Create a new status bar
    pub fn new(theme: Theme) -> Self {
        Self {
            message: None,
            theme,
        }
    }

    /// Set the current message
    pub fn set_message(&mut self, message: AppMessage) {
        self.message = Some(message);
    }

    /// Clear the current message
    pub fn clear(&mut self) {
        self.message = None;
    }

    /// Render the status bar
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let text = if let Some(ref msg) = self.message {
            let style = match msg.kind {
                AppMessageKind::Info => self.theme.secondary_style,
                AppMessageKind::Warning => self.theme.error_style,
                AppMessageKind::Error => self.theme.error_style,
            };
            Line::from(Span::styled(msg.short.clone(), style))
        } else {
            Line::from(Span::raw(""))
        };

        let paragraph = Paragraph::new(text)
            .block(Block::default().style(self.theme.status_bar_style));

        f.render_widget(paragraph, area);
    }
}
