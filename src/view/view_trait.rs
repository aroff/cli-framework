//! View trait definition
//!
//! Defines the View trait that all views must implement.

use crate::app::context::AppContext;
use crate::message::AppMessage;
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
    /// The key or key sequence (e.g., "F1", "t", "Ctrl+C")
    pub key: String,
    /// What the keybinding does
    pub description: String,
}

/// View trait that all views must implement
pub trait View {
    /// Stable identifier for this view. Literal, compile-time string.
    fn id(&self) -> &'static str;

    /// Name shown in the status bar / tabs.
    fn title(&self) -> &'static str;

    /// Called every frame to draw this view.
    fn render(&mut self, f: &mut Frame, area: Rect, ctx: &dyn AppContext);

    /// Handles view-specific events (arrows, enter, letters, etc.).
    fn handle_event(&mut self, event: &Event, ctx: &mut dyn AppContext) -> ViewResult;

    /// Help items specific to this view (used by '?').
    fn help_items(&self) -> Vec<HelpItem>;
}

