//! View trait definition
//!
//! Defines the View trait that all views must implement.

use crate::app::context::AppContext;
use crate::message::AppMessage;
use async_trait::async_trait;
use crossterm::event::Event;
use ratatui::layout::Rect;
use ratatui::Frame;

/// Result of handling an event in a view
#[derive(Debug, Clone)]
pub enum ViewResult {
    /// Event was processed by view
    Handled,
    /// Event not relevant to this view
    Ignored,
    /// Request to switch to another view
    SwitchView(String),
    /// Request to show modal with message
    ShowModal(AppMessage),
    /// Request to exit application
    Exit,
}

/// Help item for display in help overlay
#[derive(Debug, Clone)]
pub struct HelpItem {
    /// The key or key sequence (e.g., "1", "t", "Ctrl+C")
    pub key: String,
    /// What the keybinding does
    pub description: String,
}

/// View trait that all views must implement
///
/// # Async Operations
///
/// The `handle_event` method is async, allowing views to trigger async operations
/// during user interactions without blocking the UI.
#[async_trait]
pub trait View: Send + Sync {
    /// Stable identifier for this view. Literal, compile-time string.
    fn id(&self) -> &'static str;

    /// Name shown in the status bar / tabs.
    fn title(&self) -> &'static str;

    /// Called every frame to draw this view.
    ///
    /// Note: Rendering remains synchronous for performance. This method is called
    /// from the async event loop but rendering itself is fast and non-blocking.
    fn render(&mut self, f: &mut Frame, area: Rect, ctx: &dyn AppContext);

    /// Handles view-specific events (arrows, enter, letters, etc.).
    ///
    /// This method is async, allowing views to perform async operations (e.g., data refresh,
    /// service calls) in response to user interactions without blocking the UI.
    async fn handle_event(&mut self, event: &Event, ctx: &mut dyn AppContext) -> ViewResult;

    /// Help items specific to this view (used by '?').
    fn help_items(&self) -> Vec<HelpItem>;

    /// Optional contextual information for header display (left side).
    /// Returns key-value pairs like [("Context", "prod"), ("Cluster", "k8s")].
    /// Default implementation returns None (no contextual info).
    fn header_info(&self) -> Option<Vec<(String, String)>> {
        None
    }

    /// Optional short help items for header display (right side).
    /// Should be concise (max 5 items) for header display.
    /// Default implementation returns None (no header help).
    fn header_help(&self) -> Option<Vec<HelpItem>> {
        None
    }
}
