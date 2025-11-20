//! AppBuilder implementation
//!
//! Provides a builder pattern for constructing TUI applications.

use crate::app::context::AppContext;
use crate::app::module::Module;
use crate::app::runtime::Runtime;
use crate::command::{Command, CommandArgs, CommandRegistry, CommandPaletteResult};
use crate::keymap::{AppCommand, KeymapConfig, KeymapRegistry, KeymapResolver, ViewSlot};
use crate::message::AppMessage;
use crate::view::{View, ViewResult, ViewRegistry};
use anyhow::Result;
use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use std::collections::HashMap;

/// Builder for constructing TUI applications
pub struct AppBuilder {
    view_registry: ViewRegistry,
    command_registry: CommandRegistry,
    view_slots: HashMap<ViewSlot, String>,
    keymap_config: KeymapConfig,
    status_bar_enabled: bool,
    help_overlay_enabled: bool,
    command_palette_enabled: bool,
}

impl AppBuilder {
    /// Create a new AppBuilder with default configuration
    pub fn new() -> Self {
        Self {
            view_registry: ViewRegistry::new(),
            command_registry: CommandRegistry::new(),
            view_slots: HashMap::new(),
            keymap_config: KeymapConfig::new(),
            status_bar_enabled: true,
            help_overlay_enabled: true,
            command_palette_enabled: true,
        }
    }

    /// Register a view
    pub fn register_view<V: View + 'static>(mut self, view: V) -> Self {
        self.view_registry.register(Box::new(view));
        self
    }

    /// Map a view to a numeric key slot (1-9)
    pub fn map_view_slot(mut self, slot: ViewSlot, view_id: &'static str) -> Self {
        self.view_slots.insert(slot, view_id.to_string());
        self
    }

    /// Register a command
    pub fn register_command(mut self, command: Command) -> Self {
        self.command_registry.register(command);
        self
    }

    /// Register a module
    ///
    /// Modules allow grouping related views, commands, and keybindings together.
    /// This method calls the module's `register` method to add its components.
    pub fn register_module<M: Module>(mut self, module: M) -> Result<Self> {
        module.register(&mut self)?;
        Ok(self)
    }

    /// Configure keymap
    pub fn configure_keymap(mut self, keymap: KeymapConfig) -> Self {
        self.keymap_config = keymap;
        self
    }

    /// Enable or disable status bar
    pub fn with_status_bar(mut self, enabled: bool) -> Self {
        self.status_bar_enabled = enabled;
        self
    }

    /// Enable or disable help overlay
    pub fn with_help_overlay(mut self, enabled: bool) -> Self {
        self.help_overlay_enabled = enabled;
        self
    }

    /// Enable or disable command palette
    pub fn with_command_palette(mut self, enabled: bool) -> Self {
        self.command_palette_enabled = enabled;
        self
    }

    /// Build the application
    pub fn build<C: AppContext + 'static>(self, ctx: C) -> Result<App<C>> {
        // Build keymap registry and resolver
        let mut keymap_registry = KeymapRegistry::new();
        keymap_registry.load_config(self.keymap_config.clone());
        let keymap_resolver = KeymapResolver::new(keymap_registry);

        // Get all commands for palette
        let commands: Vec<Command> = self.command_registry.commands().cloned().collect();

        let mut runtime = Runtime::new();
        runtime.set_status_bar_enabled(self.status_bar_enabled);
        runtime.set_help_overlay_enabled(self.help_overlay_enabled);
        runtime.set_command_palette_enabled(self.command_palette_enabled);
        runtime.set_commands(commands);
        
        Ok(App {
            view_registry: self.view_registry,
            command_registry: self.command_registry,
            view_slots: self.view_slots,
            keymap_resolver,
            ctx,
            runtime,
            current_view_id: None,
        })
    }
}

impl Default for AppBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Built application
pub struct App<C: AppContext> {
    view_registry: ViewRegistry,
    command_registry: CommandRegistry,
    view_slots: HashMap<ViewSlot, String>,
    keymap_resolver: KeymapResolver,
    ctx: C,
    runtime: Runtime,
    current_view_id: Option<String>,
}

impl<C: AppContext> App<C> {
    /// Run the application
    pub fn run(&mut self) -> Result<()> {
        // Set initial view if any views are registered
        if let Some((first_id, _)) = self.view_registry.views().next() {
            self.current_view_id = Some(first_id.clone());
        }

        self.runtime.init()?;
        
        loop {
            // Render - use unsafe to work around borrow checker for v1
            // TODO: Refactor in v2 to use better patterns (Rc<RefCell>, etc.)
            let current_view_id = self.current_view_id.clone();
            let status_bar_enabled = self.runtime.status_bar_enabled;
            
            unsafe {
                let runtime_ptr = &mut self.runtime as *mut Runtime;
                let view_registry = &mut self.view_registry;
                let ctx = &self.ctx;
                
                if let Some(ref mut terminal) = (*runtime_ptr).terminal_mut() {
                    terminal.draw(|f| {
                        let mut area = f.size();
                        
                        // Validate and adjust area for minimum terminal size (80x24)
                        // Gracefully degrade for smaller terminals
                        area = (*runtime_ptr).validate_area(area);
                        
                        // Ensure minimum functional area
                        // For very small terminals, preserve status bar and minimum view area
                        let min_view_height = if area.height < 24 {
                            area.height.saturating_sub(if status_bar_enabled { 1 } else { 0 })
                        } else {
                            area.height.saturating_sub(if status_bar_enabled { 1 } else { 0 })
                        };
                        
                        // Calculate main area and status area
                        let chunks: Vec<Rect> = if status_bar_enabled {
                            Layout::vertical([
                                Constraint::Min(min_view_height.max(1)),
                                Constraint::Length(1),
                            ])
                            .split(area)
                            .to_vec()
                        } else {
                            vec![area]
                        };
                        let main_area = chunks[0];
                        let status_area = if chunks.len() > 1 { Some(chunks[1]) } else { None };
                        
                        // Render overlays (access widgets through raw pointer)
                        let modal_visible = (*runtime_ptr).modal.is_visible();
                        let palette_visible = (*runtime_ptr).command_palette.is_visible();
                        let help_visible = (*runtime_ptr).help_overlay.is_visible();
                        
                        if modal_visible {
                            (*runtime_ptr).modal.render(f, main_area);
                        } else if palette_visible {
                            (*runtime_ptr).command_palette.render(f, main_area);
                        } else if help_visible {
                            (*runtime_ptr).help_overlay.render(f, main_area);
                        } else {
                            // Render current view if no overlay is active
                            if let Some(ref view_id) = current_view_id {
                                if let Some(view) = view_registry.get_mut(view_id) {
                                    // Get header info and help once
                                    let header_info = view.header_info();
                                    let header_help = view.header_help();
                                    let has_header = header_info.is_some() || header_help.is_some();
                                    
                                    if has_header {
                                        // Calculate header height dynamically
                                        let header_height = {
                                            let info_lines = header_info.as_ref().map(|i| i.len() as u16).unwrap_or(0);
                                            let help_lines = header_help.as_ref().map(|h| h.len().min(5) as u16).unwrap_or(0);
                                            info_lines.max(help_lines).max(1) + 1
                                        };
                                        
                                        // Split main area into header and content
                                        let chunks = ratatui::layout::Layout::vertical([
                                            ratatui::layout::Constraint::Length(header_height),
                                            ratatui::layout::Constraint::Min(0),
                                        ])
                                        .split(main_area);
                                        
                                        let header_area = chunks[0];
                                        let content_area = chunks[1];
                                        
                                        // Build and render header
                                        let mut header = crate::widget::ViewHeader::new(
                                            view.title().to_string(),
                                            crate::view::Theme::default(),
                                        );
                                        
                                        if let Some(info) = header_info {
                                            header = header.with_info(info);
                                        }
                                        
                                        if let Some(help) = header_help {
                                            header = header.with_help(help);
                                        }
                                        
                                        header.render(f, header_area);
                                        
                                        // Render view content in remaining area
                                        view.render(f, content_area, ctx);
                                    } else {
                                        // No header, render view directly
                                        view.render(f, main_area, ctx);
                                    }
                                }
                            }
                        }
                        
                        // Render status bar
                        if let Some(status_rect) = status_area {
                            (*runtime_ptr).status_bar.render(f, status_rect);
                        }
                    })?;
                }
            }

            // Handle events
            if crossterm::event::poll(std::time::Duration::from_millis(16))? {
                if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                    if key.kind == KeyEventKind::Press {
                        if !self.handle_key(key.code) {
                            break;
                        }
                    }
                }
            }
        }

        self.runtime.cleanup()?;
        Ok(())
    }

    /// Handle key event
    fn handle_key(&mut self, key: KeyCode) -> bool {
        // Handle command palette
        if self.runtime.is_command_palette_visible() {
            match self.runtime.command_palette_mut().handle_key(key) {
                CommandPaletteResult::Continue => {}
                CommandPaletteResult::Cancel => {
                    self.runtime.command_palette_mut().hide();
                }
                CommandPaletteResult::Execute(command_id) => {
                    self.runtime.command_palette_mut().hide();
                    self.execute_command(&command_id, CommandArgs {
                        positional: vec![],
                        named: std::collections::HashMap::new(),
                    });
                }
            }
            return true;
        }

        // Handle modal
        if self.runtime.is_modal_visible() {
            match key {
                KeyCode::Esc | KeyCode::Enter => {
                    self.runtime.modal_mut().hide();
                }
                _ => {}
            }
            return true;
        }

        // Handle global keys
        match key {
            KeyCode::Char('q') => {
                return false; // Exit
            }
            KeyCode::Char('?') => {
                if self.runtime.help_overlay_enabled {
                    self.runtime.help_overlay_mut().toggle();
                }
            }
            KeyCode::Char(':') => {
                if self.runtime.command_palette_enabled {
                    self.runtime.command_palette_mut().show();
                }
            }
            KeyCode::Esc => {
                if self.runtime.help_overlay_mut().is_visible() {
                    self.runtime.help_overlay_mut().hide();
                }
            }
            _ => {}
        }

        // Handle numeric keys (1-9) for view switching
        if let Some(slot) = self.key_to_view_slot(key) {
            if let Some(view_id) = self.view_slots.get(&slot) {
                self.current_view_id = Some(view_id.clone());
                self.runtime.set_status_message(AppMessage::info(format!("Switched to view: {}", view_id)));
                return true;
            }
        }

        // Try keymap resolver
        if let Some(app_cmd) = self.keymap_resolver.resolve(
            key,
            self.current_view_id.as_deref(),
            self.runtime.is_modal_visible(),
        ) {
            match app_cmd {
                AppCommand::SwitchView(view_id) => {
                    self.current_view_id = Some(view_id);
                }
                AppCommand::RunCommand(cmd_id, named_args) => {
                    self.execute_command(&cmd_id, CommandArgs {
                        positional: vec![],
                        named: named_args,
                    });
                }
                AppCommand::InvokeAction(_) => {
                    // Actions will be implemented later
                }
            }
            return true;
        }

        // Pass to current view
        if let Some(ref view_id) = self.current_view_id {
            if let Some(view) = self.view_registry.get_mut(view_id) {
                let event = CrosstermEvent::Key(crossterm::event::KeyEvent {
                    code: key,
                    kind: KeyEventKind::Press,
                    modifiers: crossterm::event::KeyModifiers::empty(),
                    state: crossterm::event::KeyEventState::empty(),
                });
                match view.handle_event(&event, &mut self.ctx) {
                    ViewResult::Exit => return false,
                    ViewResult::SwitchView(new_view_id) => {
                        self.current_view_id = Some(new_view_id);
                    }
                    ViewResult::ShowModal(msg) => {
                        self.runtime.modal_mut().show(msg);
                    }
                    _ => {}
                }
            }
        }

        true
    }

    /// Execute a command
    fn execute_command(&mut self, command_id: &str, args: CommandArgs) {
        if let Some(command) = self.command_registry.get(command_id) {
            match (command.execute)(&mut self.ctx, args) {
                Ok(()) => {
                    self.runtime.set_status_message(AppMessage::info(format!("Command '{}' executed successfully", command_id)));
                }
                Err(e) => {
                    let error_msg = AppMessage::error(format!("Command '{}' failed", command_id))
                        .with_details(e.to_string());
                    self.runtime.set_status_message(error_msg.clone());
                    self.runtime.modal_mut().show(error_msg);
                }
            }
        }
    }

    /// Convert key code to view slot
    fn key_to_view_slot(&self, key: KeyCode) -> Option<ViewSlot> {
        match key {
            KeyCode::Char('1') => Some(ViewSlot::Slot1),
            KeyCode::Char('2') => Some(ViewSlot::Slot2),
            KeyCode::Char('3') => Some(ViewSlot::Slot3),
            KeyCode::Char('4') => Some(ViewSlot::Slot4),
            KeyCode::Char('5') => Some(ViewSlot::Slot5),
            KeyCode::Char('6') => Some(ViewSlot::Slot6),
            KeyCode::Char('7') => Some(ViewSlot::Slot7),
            KeyCode::Char('8') => Some(ViewSlot::Slot8),
            KeyCode::Char('9') => Some(ViewSlot::Slot9),
            _ => None,
        }
    }
}
