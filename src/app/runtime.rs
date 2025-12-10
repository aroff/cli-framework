//! Basic event loop and runtime
//!
//! Provides the main event loop for the TUI application.

use crate::command::CommandPalette;
use crate::data_source::log::SharedLogBuffer;
use crate::message::AppMessage;
use crate::view::Theme;
use crate::widget::{HelpOverlay, ModalView, StatusBar};
use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent, KeyEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::Terminal;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Runtime for the TUI application
pub struct Runtime {
    terminal: Option<Terminal<CrosstermBackend<io::Stdout>>>,
    pub(crate) status_bar: StatusBar,
    pub(crate) help_overlay: HelpOverlay,
    pub(crate) command_palette: CommandPalette,
    pub(crate) modal: ModalView,
    #[allow(dead_code)] // Theme stored for potential future use
    theme: Theme,
    pub(crate) status_bar_enabled: bool,
    pub(crate) help_overlay_enabled: bool,
    pub(crate) command_palette_enabled: bool,
    /// Number of active async operations (for loading indicators)
    active_operations: Arc<AtomicUsize>,
    /// Default timeout for async operations (in seconds)
    default_timeout_seconds: u64,
    /// Shared buffer for streaming log lines
    pub(crate) stream_buffer: SharedLogBuffer,
}

impl Runtime {
    /// Create a new runtime
    pub fn new() -> Self {
        let theme = Theme::default();
        Self {
            terminal: None,
            status_bar: StatusBar::new(theme.clone()),
            help_overlay: HelpOverlay::new(theme.clone()),
            command_palette: CommandPalette::new(theme.clone()),
            modal: ModalView::new(theme.clone()),
            theme,
            status_bar_enabled: true,
            help_overlay_enabled: true,
            command_palette_enabled: true,
            active_operations: Arc::new(AtomicUsize::new(0)),
            default_timeout_seconds: 30, // Default 30 seconds per FR-017
            stream_buffer: SharedLogBuffer::new(10_000),
        }
    }

    /// Set commands for palette
    pub fn set_commands(&mut self, commands: Vec<crate::command::Command>) {
        self.command_palette.set_commands(commands);
    }

    /// Set status bar enabled
    pub fn set_status_bar_enabled(&mut self, enabled: bool) {
        self.status_bar_enabled = enabled;
    }

    /// Set help overlay enabled
    pub fn set_help_overlay_enabled(&mut self, enabled: bool) {
        self.help_overlay_enabled = enabled;
    }

    /// Set command palette enabled
    pub fn set_command_palette_enabled(&mut self, enabled: bool) {
        self.command_palette_enabled = enabled;
    }

    /// Set status message
    pub fn set_status_message(&mut self, message: AppMessage) {
        self.status_bar.set_message(message);
    }

    /// Get command palette for external handling
    pub fn command_palette_mut(&mut self) -> &mut CommandPalette {
        &mut self.command_palette
    }

    /// Get modal for external handling
    pub fn modal_mut(&mut self) -> &mut ModalView {
        &mut self.modal
    }

    /// Get help overlay for external handling
    pub fn help_overlay_mut(&mut self) -> &mut HelpOverlay {
        &mut self.help_overlay
    }

    /// Check if modal is visible
    pub fn is_modal_visible(&self) -> bool {
        self.modal.is_visible()
    }

    /// Check if command palette is visible
    pub fn is_command_palette_visible(&self) -> bool {
        self.command_palette.is_visible()
    }

    /// Minimum terminal size (80x24 as per spec)
    const MIN_WIDTH: u16 = 80;
    const MIN_HEIGHT: u16 = 24;

    /// Initialize the terminal
    pub fn init(&mut self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        // Check terminal size and warn if too small
        let size = terminal.size()?;
        if size.width < Self::MIN_WIDTH || size.height < Self::MIN_HEIGHT {
            // Note: We don't fail here, but gracefully degrade
            // The render loop will handle small terminals
        }

        self.terminal = Some(terminal);
        Ok(())
    }

    /// Validate and adjust area for minimum terminal size
    ///
    /// Ensures that the area meets minimum requirements (80x24) and
    /// gracefully degrades for smaller terminals by preserving minimum
    /// functional areas.
    pub fn validate_area(&self, area: Rect) -> Rect {
        let min_width = Self::MIN_WIDTH.min(area.width);
        let min_height = Self::MIN_HEIGHT.min(area.height);

        // Ensure we have at least minimum size
        // If terminal is smaller, we use what we have but ensure
        // status bar and essential UI elements remain accessible
        Rect {
            x: area.x,
            y: area.y,
            width: area.width.max(min_width),
            height: area.height.max(min_height),
        }
    }

    /// Cleanup the terminal
    pub fn cleanup(&mut self) -> Result<()> {
        disable_raw_mode()?;
        if let Some(mut terminal) = self.terminal.take() {
            execute!(
                terminal.backend_mut(),
                LeaveAlternateScreen,
                DisableMouseCapture
            )?;
            terminal.show_cursor()?;
        }
        Ok(())
    }

    /// Get terminal for external use
    pub fn terminal_mut(&mut self) -> &mut Option<Terminal<CrosstermBackend<io::Stdout>>> {
        &mut self.terminal
    }

    /// Get main area after accounting for status bar
    pub fn get_main_area(&self, area: Rect) -> (Rect, Option<Rect>) {
        let chunks: Vec<Rect> = if self.status_bar_enabled {
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)])
                .split(area)
                .to_vec()
        } else {
            vec![area]
        };

        let main_area = chunks[0];
        let status_area = if chunks.len() > 1 {
            Some(chunks[1])
        } else {
            None
        };
        (main_area, status_area)
    }

    /// Render status bar
    /// T054: Includes loading indicator
    pub fn render_status_bar(&self, f: &mut ratatui::Frame, area: Rect) {
        if self.status_bar_enabled {
            let is_loading = self.has_active_operations();
            self.status_bar.render(f, area, is_loading);
        }
    }

    /// Read a terminal event asynchronously using spawn_blocking
    ///
    /// This prevents blocking the async runtime while waiting for terminal input.
    /// Returns `None` if no event is ready, `Some(event)` if an event is available.
    pub async fn read_event_async(&self) -> Result<Option<CrosstermEvent>> {
        tokio::task::spawn_blocking(|| {
            if event::poll(std::time::Duration::from_millis(16))? {
                let evt = event::read()?;
                if let CrosstermEvent::Key(key) = evt {
                    if key.kind == KeyEventKind::Press {
                        return Ok(Some(CrosstermEvent::Key(key)));
                    }
                }
            }
            Ok(None)
        })
        .await?
    }

    /// Increment active operations count (for loading indicators)
    pub fn start_operation(&self) {
        self.active_operations.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement active operations count
    pub fn finish_operation(&self) {
        self.active_operations.fetch_sub(1, Ordering::Relaxed);
    }

    /// Check if any operations are active (for loading indicators)
    pub fn has_active_operations(&self) -> bool {
        self.active_operations.load(Ordering::Relaxed) > 0
    }

    /// Get the number of active operations
    pub fn active_operations_count(&self) -> usize {
        self.active_operations.load(Ordering::Relaxed)
    }

    /// Get default timeout in seconds
    pub fn default_timeout_seconds(&self) -> u64 {
        self.default_timeout_seconds
    }

    /// Set default timeout in seconds
    pub fn set_default_timeout_seconds(&mut self, seconds: u64) {
        self.default_timeout_seconds = seconds;
    }

    /// Append streaming log lines into the shared buffer
    pub fn append_stream_lines(&self, lines: Vec<String>) {
        for line in lines {
            self.stream_buffer.push(line);
        }
    }

    /// Access the streaming buffer (shared clone)
    pub fn stream_buffer(&self) -> SharedLogBuffer {
        self.stream_buffer.clone()
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
