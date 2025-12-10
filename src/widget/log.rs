//! LogView widget implementation
//!
//! Displays streaming log lines with scrolling, follow mode, and keyword filtering

use crate::view::Theme;
use ratatui::layout::Rect;
// Modifier and Style not currently used in this file
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Scrollbar, ScrollbarState};
use ratatui::Frame;
use std::collections::VecDeque;

/// LogView widget for displaying streaming log lines
pub struct LogView {
    /// Internal buffer of log lines
    lines: VecDeque<String>,
    /// Maximum number of lines to keep in buffer
    max_lines: usize,
    /// Current scroll position (0 = top, higher = scrolled down)
    scroll_position: usize,
    /// Follow mode: automatically scroll to bottom when new lines arrive
    follow_mode: bool,
    /// Filter string (case-insensitive substring match)
    filter: Option<String>,
    /// Filtered lines (cached for performance)
    filtered_lines: Vec<String>,
    /// Scrollbar state
    scrollbar_state: ScrollbarState,
    /// Theme for styling
    theme: Theme,
}

impl LogView {
    /// Create a new LogView
    pub fn new(theme: Theme) -> Self {
        Self {
            lines: VecDeque::new(),
            max_lines: 10000, // Default: keep last 10k lines
            scroll_position: 0,
            follow_mode: true, // Default: follow mode enabled
            filter: None,
            filtered_lines: Vec::new(),
            scrollbar_state: ScrollbarState::new(0),
            theme,
        }
    }

    /// Create a new LogView with custom max lines
    pub fn with_max_lines(theme: Theme, max_lines: usize) -> Self {
        Self {
            max_lines,
            ..Self::new(theme)
        }
    }

    /// Add a log line to the buffer
    pub fn add_line(&mut self, line: String) {
        // Add line to buffer
        self.lines.push_back(line);

        // Trim buffer if it exceeds max_lines
        while self.lines.len() > self.max_lines {
            self.lines.pop_front();
        }

        // Update filtered lines if filter is active
        if self.filter.is_some() {
            self.update_filtered_lines();
        }

        // Auto-scroll to bottom if follow mode is enabled
        if self.follow_mode {
            self.scroll_to_bottom();
        }
    }

    /// Add multiple log lines
    pub fn add_lines(&mut self, lines: Vec<String>) {
        for line in lines {
            self.add_line(line);
        }
    }

    /// Ingest streaming lines pushed from background tasks
    pub fn ingest_stream_lines<I>(&mut self, lines: I)
    where
        I: IntoIterator<Item = String>,
    {
        for line in lines {
            self.add_line(line);
        }
    }

    /// Set the filter string (case-insensitive substring match)
    pub fn set_filter(&mut self, filter: Option<String>) {
        self.filter = filter;
        self.update_filtered_lines();

        // If follow mode is on, scroll to bottom after filtering
        if self.follow_mode {
            self.scroll_to_bottom();
        } else {
            // Reset scroll position when filter changes
            self.scroll_position = 0;
        }
    }

    /// Get current filter
    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }

    /// Toggle follow mode
    pub fn toggle_follow_mode(&mut self) {
        self.follow_mode = !self.follow_mode;
        if self.follow_mode {
            self.scroll_to_bottom();
        }
    }

    /// Set follow mode
    pub fn set_follow_mode(&mut self, enabled: bool) {
        self.follow_mode = enabled;
        if enabled {
            self.scroll_to_bottom();
        }
    }

    /// Check if follow mode is enabled
    pub fn is_follow_mode(&self) -> bool {
        self.follow_mode
    }

    /// Scroll up by one line
    pub fn scroll_up(&mut self) {
        let visible_lines = self.get_visible_lines_count();
        if self.scroll_position > 0 {
            self.scroll_position -= 1;
            self.follow_mode = false; // Disable follow mode when user scrolls
        }
        self.update_scrollbar_state(visible_lines);
    }

    /// Scroll down by one line
    pub fn scroll_down(&mut self) {
        let visible_lines = self.get_visible_lines_count();
        let max_scroll = self.get_max_scroll_position(visible_lines);
        if self.scroll_position < max_scroll {
            self.scroll_position += 1;
        }
        self.update_scrollbar_state(visible_lines);
    }

    /// Scroll to top
    pub fn scroll_to_top(&mut self) {
        self.scroll_position = 0;
        self.follow_mode = false;
        let visible_lines = self.get_visible_lines_count();
        self.update_scrollbar_state(visible_lines);
    }

    /// Scroll to bottom
    pub fn scroll_to_bottom(&mut self) {
        let visible_lines = self.get_visible_lines_count();
        let max_scroll = self.get_max_scroll_position(visible_lines);
        self.scroll_position = max_scroll;
        self.update_scrollbar_state(visible_lines);
    }

    /// Page up (scroll up by visible height)
    pub fn page_up(&mut self) {
        let visible_lines = self.get_visible_lines_count();
        if self.scroll_position >= visible_lines {
            self.scroll_position -= visible_lines;
        } else {
            self.scroll_position = 0;
        }
        self.follow_mode = false;
        self.update_scrollbar_state(visible_lines);
    }

    /// Page down (scroll down by visible height)
    pub fn page_down(&mut self) {
        let visible_lines = self.get_visible_lines_count();
        let max_scroll = self.get_max_scroll_position(visible_lines);
        let new_position = (self.scroll_position + visible_lines).min(max_scroll);
        self.scroll_position = new_position;
        if self.scroll_position >= max_scroll {
            self.follow_mode = true; // Enable follow mode if we reach bottom
        }
        self.update_scrollbar_state(visible_lines);
    }

    /// Update filtered lines based on current filter
    fn update_filtered_lines(&mut self) {
        if let Some(ref filter) = self.filter {
            let filter_lower = filter.to_lowercase();
            self.filtered_lines = self
                .lines
                .iter()
                .filter(|line| line.to_lowercase().contains(&filter_lower))
                .cloned()
                .collect();
        } else {
            self.filtered_lines.clear();
        }
    }

    /// Get visible lines count (approximate based on area height)
    fn get_visible_lines_count(&self) -> usize {
        // This will be set during render based on actual area
        // For now, return a reasonable default
        20
    }

    /// Get maximum scroll position
    fn get_max_scroll_position(&self, visible_lines: usize) -> usize {
        let total_lines = if self.filter.is_some() {
            self.filtered_lines.len()
        } else {
            self.lines.len()
        };

        if total_lines <= visible_lines {
            0
        } else {
            total_lines - visible_lines
        }
    }

    /// Update scrollbar state
    fn update_scrollbar_state(&mut self, visible_lines: usize) {
        let total_lines = if self.filter.is_some() {
            self.filtered_lines.len()
        } else {
            self.lines.len()
        };

        self.scrollbar_state = ScrollbarState::new(total_lines)
            .position(self.scroll_position)
            .viewport_content_length(visible_lines);
    }

    /// Clear all log lines
    pub fn clear(&mut self) {
        self.lines.clear();
        self.filtered_lines.clear();
        self.scroll_position = 0;
        let visible_lines = self.get_visible_lines_count();
        self.update_scrollbar_state(visible_lines);
    }

    /// Get number of lines in buffer
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Render the log view
    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        // Update visible lines count based on actual area
        let visible_lines = (area.height.saturating_sub(2)) as usize; // Account for borders

        // Update filtered lines if needed
        if self.filter.is_some() && self.filtered_lines.is_empty() && !self.lines.is_empty() {
            self.update_filtered_lines();
        }

        // Get lines to display
        let display_lines: Vec<String> = if self.filter.is_some() {
            self.filtered_lines.clone()
        } else {
            self.lines.iter().cloned().collect()
        };

        // Calculate visible range
        let total_lines = display_lines.len();
        let max_scroll = self.get_max_scroll_position(visible_lines);

        // Ensure scroll position is valid
        if self.scroll_position > max_scroll {
            self.scroll_position = max_scroll;
        }

        // Get visible slice
        let start = self.scroll_position;
        let end = (start + visible_lines).min(total_lines);
        let visible_slice = if start < total_lines {
            &display_lines[start..end]
        } else {
            &[]
        };

        // Build list items
        let items: Vec<ListItem> = visible_slice
            .iter()
            .map(|line| {
                // Truncate line if too long for display
                let max_width = area.width.saturating_sub(4) as usize; // Account for borders and scrollbar
                let display_line = if line.len() > max_width {
                    &line[..max_width]
                } else {
                    line.as_str()
                };

                ListItem::new(Line::from(Span::raw(display_line)))
            })
            .collect();

        // Create block with title showing filter and follow mode status
        let mut title = "Logs".to_string();
        if let Some(ref filter) = self.filter {
            title.push_str(&format!(" [filter: {}]", filter));
        }
        if self.follow_mode {
            title.push_str(" [follow]");
        }

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .style(self.theme.secondary_style),
        );

        // Render list
        f.render_widget(list, area);

        // Render scrollbar if needed
        if total_lines > visible_lines {
            self.update_scrollbar_state(visible_lines);
            let scrollbar = Scrollbar::default()
                .orientation(ratatui::widgets::ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"));
            f.render_stateful_widget(scrollbar, area, &mut self.scrollbar_state);
        }
    }
}

impl Default for LogView {
    fn default() -> Self {
        Self::new(Theme::default())
    }
}
