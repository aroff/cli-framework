//! Empty state and loading indicators
//!
//! Provides standard empty state messages and loading indicators for consistent UX

use crate::view::Theme;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

/// Empty state widget for displaying when there's no data
pub struct EmptyState {
    message: String,
    details: Option<String>,
    theme: Theme,
}

impl EmptyState {
    /// Create a new empty state with a message
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            details: None,
            theme: Theme::default(),
        }
    }

    /// Create a new empty state with theme
    pub fn with_theme(message: impl Into<String>, theme: Theme) -> Self {
        Self {
            message: message.into(),
            details: None,
            theme,
        }
    }

    /// Add detailed text
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    /// Render the empty state
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let mut lines = vec![Line::from(Span::styled(
            self.message.clone(),
            self.theme.secondary_style,
        ))];

        if let Some(ref details) = self.details {
            lines.push(Line::from(""));
            for line in details.lines() {
                lines.push(Line::from(Span::styled(line, self.theme.secondary_style)));
            }
        }

        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("No Data")
                    .style(self.theme.secondary_style),
            )
            .alignment(Alignment::Center);

        f.render_widget(paragraph, area);
    }
}

/// Loading indicator widget
pub struct LoadingIndicator {
    message: String,
    theme: Theme,
    spinner_chars: &'static [char],
    spinner_index: usize,
}

impl LoadingIndicator {
    /// Create a new loading indicator
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            theme: Theme::default(),
            spinner_chars: &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'],
            spinner_index: 0,
        }
    }

    /// Create with theme
    pub fn with_theme(message: impl Into<String>, theme: Theme) -> Self {
        Self {
            message: message.into(),
            theme,
            spinner_chars: &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'],
            spinner_index: 0,
        }
    }

    /// Advance the spinner animation
    pub fn tick(&mut self) {
        self.spinner_index = (self.spinner_index + 1) % self.spinner_chars.len();
    }

    /// Render the loading indicator
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let spinner = self.spinner_chars[self.spinner_index];
        let text = format!("{} {}", spinner, self.message);

        let paragraph = Paragraph::new(Line::from(Span::styled(text, self.theme.secondary_style)))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Loading")
                    .style(self.theme.secondary_style),
            )
            .alignment(Alignment::Center);

        f.render_widget(paragraph, area);
    }
}

impl Default for EmptyState {
    fn default() -> Self {
        Self::new("No data available")
    }
}
