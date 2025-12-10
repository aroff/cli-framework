//! Login screen implementation
//!
//! Provides optional built-in authentication mechanisms

use crate::view::Theme;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

/// Login screen widget
///
/// Applications can use this for built-in authentication, or implement
/// their own authentication via AppContext.
pub struct LoginScreen {
    username: String,
    password: String,
    theme: Theme,
}

impl LoginScreen {
    /// Create a new login screen
    pub fn new(theme: Theme) -> Self {
        Self {
            username: String::new(),
            password: String::new(),
            theme,
        }
    }

    /// Get username
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Get password
    pub fn password(&self) -> &str {
        &self.password
    }

    /// Set username
    pub fn set_username(&mut self, username: String) {
        self.username = username;
    }

    /// Set password
    pub fn set_password(&mut self, password: String) {
        self.password = password;
    }

    /// Render the login screen
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Login")
            .style(self.theme.modal_style);

        let paragraph =
            Paragraph::new("Login screen - implement authentication logic here").block(block);

        f.render_widget(paragraph, area);
    }
}
