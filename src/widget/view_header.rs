//! ViewHeader widget implementation
//!
//! Displays view title, contextual information, and keybindings in a header layout

use crate::view::{HelpItem, Theme};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// ViewHeader widget for displaying view information
pub struct ViewHeader {
    title: String,
    info: Option<Vec<(String, String)>>,
    help: Option<Vec<HelpItem>>,
    theme: Theme,
}

impl ViewHeader {
    /// Create a new ViewHeader
    pub fn new(title: String, theme: Theme) -> Self {
        Self {
            title,
            info: None,
            help: None,
            theme,
        }
    }

    /// Set contextual information (left side)
    pub fn with_info(mut self, info: Vec<(String, String)>) -> Self {
        self.info = Some(info);
        self
    }

    /// Set header help items (right side, max 5)
    pub fn with_help(mut self, help: Vec<HelpItem>) -> Self {
        // Limit to 5 items max
        let limited: Vec<HelpItem> = help.into_iter().take(5).collect();
        self.help = if limited.is_empty() { None } else { Some(limited) };
        self
    }

    /// Calculate the height needed for the header
    pub fn height(&self) -> u16 {
        let info_height = self.info.as_ref().map(|i| i.len() as u16).unwrap_or(0);
        let help_height = self.help.as_ref().map(|h| h.len() as u16).unwrap_or(0);
        // Height is max of info lines and help lines, plus 1 for spacing
        info_height.max(help_height).max(1) + 1
    }

    /// Render the view header
    pub fn render(&self, f: &mut Frame, area: Rect) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        // Split area into three sections: left (info), center (title), right (help)
        let chunks = Layout::horizontal([
            Constraint::Percentage(35), // Left: contextual info
            Constraint::Percentage(30), // Center: title
            Constraint::Percentage(35), // Right: help
        ])
        .split(area);

        let left_area = chunks[0];
        let center_area = chunks[1];
        let right_area = chunks[2];

        // Render left side: contextual information
        if let Some(ref info) = self.info {
            let mut lines = Vec::new();
            for (key, value) in info {
                let line = Line::from(vec![
                    Span::styled(
                        format!("{}: ", key),
                        self.theme.primary_style,
                    ),
                    Span::styled(
                        value.clone(),
                        self.theme.secondary_style,
                    ),
                ]);
                lines.push(line);
            }
            let paragraph = Paragraph::new(lines);
            f.render_widget(paragraph, left_area);
        }

        // Render center: title (centered)
        let title_line = Line::from(vec![Span::styled(
            self.title.clone(),
            self.theme.primary_style.add_modifier(Modifier::BOLD),
        )]);
        let title_paragraph = Paragraph::new(title_line)
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(title_paragraph, center_area);

        // Render right side: help items
        if let Some(ref help) = self.help {
            let mut lines = Vec::new();
            for item in help {
                let line = Line::from(vec![
                    Span::styled(
                        format!("<{}> ", item.key),
                        self.theme.primary_style,
                    ),
                    Span::styled(
                        item.description.clone(),
                        self.theme.secondary_style,
                    ),
                ]);
                lines.push(line);
            }
            let paragraph = Paragraph::new(lines)
                .alignment(ratatui::layout::Alignment::Right);
            f.render_widget(paragraph, right_area);
        }
    }
}

