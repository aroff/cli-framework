//! HelpOverlay widget implementation
//!
//! Displays help information overlay

use crate::view::{HelpItem, Theme};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

/// HelpOverlay widget for displaying help information
pub struct HelpOverlay {
    items: Vec<HelpItem>,
    theme: Theme,
    visible: bool,
}

impl HelpOverlay {
    /// Create a new help overlay
    pub fn new(theme: Theme) -> Self {
        Self {
            items: Vec::new(),
            theme,
            visible: false,
        }
    }

    /// Set help items
    pub fn set_items(&mut self, items: Vec<HelpItem>) {
        self.items = items;
    }

    /// Show the help overlay
    pub fn show(&mut self) {
        self.visible = true;
    }

    /// Hide the help overlay
    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Toggle visibility
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Check if visible
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Render the help overlay
    pub fn render(&self, f: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }

        // Create a centered modal area
        let vertical = Layout::vertical([
            Constraint::Percentage(20),
            Constraint::Percentage(60),
            Constraint::Percentage(20),
        ])
        .split(area);

        let horizontal = Layout::horizontal([
            Constraint::Percentage(20),
            Constraint::Percentage(60),
            Constraint::Percentage(20),
        ])
        .split(vertical[1]);

        let modal_area = horizontal[1];

        let items: Vec<ListItem> = self
            .items
            .iter()
            .map(|item| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:20}", item.key),
                        self.theme.primary_style,
                    ),
                    Span::raw(" "),
                    Span::styled(item.description.clone(), self.theme.secondary_style),
                ]))
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Help")
                    .style(self.theme.modal_style),
            );

        f.render_widget(list, modal_area);
    }
}
