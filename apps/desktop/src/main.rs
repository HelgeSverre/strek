//! Desktop application for the vector editor.
//!
//! Uses winit for windowing and wgpu for rendering.

mod layer_panel;
mod status_bar;
mod toolbar;

use std::sync::Arc;

use editor_core::input::{Cursor, InputEvent, Key, Modifiers, MouseButton};
use editor_core::Editor;
use glam::Vec2;
use render_wgpu::WgpuRenderer;
use ui::DrawList;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// Main application state.
struct App {
    window: Option<Arc<Window>>,
    renderer: Option<WgpuRenderer>,
    editor: Editor,
    needs_redraw: bool,
    mouse_position: Vec2,
    /// Current modifier key state
    modifiers: Modifiers,
    /// Layer panel configuration
    layer_panel_config: layer_panel::LayerPanelConfig,
    /// Whether to show the layer panel
    show_layer_panel: bool,
    /// Toolbar configuration
    toolbar_config: toolbar::ToolbarConfig,
    /// Status bar configuration
    status_bar_config: status_bar::StatusBarConfig,
}

impl App {
    fn new() -> Self {
        Self {
            window: None,
            renderer: None,
            editor: Editor::with_demo_content(),
            needs_redraw: true,
            mouse_position: Vec2::ZERO,
            modifiers: Modifiers::default(),
            layer_panel_config: layer_panel::LayerPanelConfig::default(),
            show_layer_panel: true,
            toolbar_config: toolbar::ToolbarConfig::default(),
            status_bar_config: status_bar::StatusBarConfig::default(),
        }
    }

    fn render(&mut self) {
        if let Some(renderer) = &mut self.renderer {
            if let Some(window) = &self.window {
                let size = window.inner_size();
                self.editor
                    .set_viewport_size(Vec2::new(size.width as f32, size.height as f32));
            }
            let display_list = self.editor.build_display_list();

            // Build UI draw list
            let mut ui_draw_list = DrawList::new();

            if let Some(window) = &self.window {
                let size = window.inner_size();
                let viewport_width = size.width as f32;
                let viewport_height = size.height as f32;

                // Build toolbar at top
                let toolbar_draw_list =
                    toolbar::build_toolbar(viewport_width, self.editor.tool, &self.toolbar_config);
                ui_draw_list.append(&toolbar_draw_list);

                // Build status bar at bottom
                let status_state = status_bar::StatusBarState {
                    tool: self.editor.tool,
                    selection_count: self.editor.selection().len(),
                    mouse_position: Some(self.editor.view().to_world(self.mouse_position)),
                    zoom: self.editor.view().zoom,
                };
                let status_draw_list = status_bar::build_status_bar(
                    viewport_width,
                    viewport_height,
                    &status_state,
                    &self.status_bar_config,
                );
                ui_draw_list.append(&status_draw_list);

                // Build layer panel on the right
                if self.show_layer_panel {
                    let layer_entries = self.editor.build_layer_tree();
                    let panel_draw_list = layer_panel::build_layer_panel(
                        &layer_entries,
                        viewport_height,
                        &self.layer_panel_config,
                    );

                    // Offset the panel to the right side of the window
                    let panel_x = viewport_width - self.layer_panel_config.width;
                    for cmd in panel_draw_list.commands {
                        ui_draw_list.push(offset_command(cmd, panel_x, 0.0));
                    }
                }
            }

            match renderer.render_with_ui(&display_list, &ui_draw_list) {
                Ok(_) => {}
                Err(wgpu::SurfaceError::Lost) => {
                    if let Some(window) = &self.window {
                        renderer.resize(window.inner_size());
                    }
                }
                Err(wgpu::SurfaceError::OutOfMemory) => {
                    log::error!("Out of GPU memory");
                }
                Err(e) => {
                    log::warn!("Surface error: {:?}", e);
                }
            }
            self.needs_redraw = false;
        }
    }

    fn set_cursor(&self, cursor: Cursor) {
        if let Some(window) = &self.window {
            window.set_cursor(cursor_to_winit(cursor));
        }
    }

    /// Handle a layer panel action.
    fn handle_layer_panel_action(&mut self, action: layer_panel::LayerPanelAction) {
        use layer_panel::LayerPanelAction;

        match action {
            LayerPanelAction::SelectLayer(id) => {
                let add_to_selection = self.modifiers.shift;
                self.editor.select_layer(id, add_to_selection);
                self.needs_redraw = true;
            }
            LayerPanelAction::ToggleExpand(id) => {
                self.editor.toggle_layer_expand(id);
                self.needs_redraw = true;
            }
            LayerPanelAction::ToggleVisibility(id) => {
                self.editor.toggle_visibility(id);
                self.needs_redraw = true;
            }
            LayerPanelAction::ToggleLock(id) => {
                self.editor.toggle_locked(id);
                self.needs_redraw = true;
            }
        }
    }

    /// Handle a toolbar action.
    fn handle_toolbar_action(&mut self, action: toolbar::ToolbarAction) {
        use toolbar::{ToolButton, ToolbarAction};

        match action {
            ToolbarAction::SelectTool(tool_button) => {
                let editor_action = match tool_button {
                    ToolButton::Select => Some(editor_core::EditorAction::ToolSelect),
                    ToolButton::Rectangle => Some(editor_core::EditorAction::ToolRectangle),
                    ToolButton::Ellipse => Some(editor_core::EditorAction::ToolEllipse),
                    ToolButton::Pen | ToolButton::Text => None,
                };
                if let Some(editor_action) = editor_action {
                    self.editor.execute_action(editor_action);
                    self.set_cursor(self.editor.cursor());
                }
                self.needs_redraw = true;
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window_attributes = Window::default_attributes()
            .with_title("Strek")
            .with_inner_size(LogicalSize::new(1024, 768));

        let window = Arc::new(
            event_loop
                .create_window(window_attributes)
                .expect("Failed to create window"),
        );

        // Create renderer
        let renderer = pollster::block_on(WgpuRenderer::new(window.clone()))
            .expect("Failed to create renderer");

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.needs_redraw = true;
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }

            WindowEvent::Resized(new_size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(new_size);
                    self.needs_redraw = true;
                }
            }

            WindowEvent::RedrawRequested => {
                self.render();
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_position = Vec2::new(position.x as f32, position.y as f32);

                let input = InputEvent::PointerMove {
                    position: self.mouse_position,
                    modifiers: self.modifiers,
                };

                let effects = self.editor.handle_event(input);

                if effects.redraw {
                    self.needs_redraw = true;
                }

                if let Some(cursor) = effects.cursor {
                    self.set_cursor(cursor);
                }
            }

            WindowEvent::MouseInput { state, button, .. } => {
                // Check if click is in UI regions (toolbar, layer panel, etc.)
                let mut handled_by_ui = false;

                if state == ElementState::Pressed && button == winit::event::MouseButton::Left {
                    if let Some(window) = &self.window {
                        let size = window.inner_size();
                        let viewport_width = size.width as f32;

                        // Check toolbar (top of window)
                        if self.mouse_position.y < self.toolbar_config.height {
                            if let Some(action) = toolbar::hit_test_toolbar(
                                self.mouse_position.x,
                                self.mouse_position.y,
                                &self.toolbar_config,
                            ) {
                                self.handle_toolbar_action(action);
                                handled_by_ui = true;
                            }
                        }

                        // Check layer panel (right side of window)
                        if !handled_by_ui && self.show_layer_panel {
                            let panel_x = viewport_width - self.layer_panel_config.width;

                            if self.mouse_position.x >= panel_x {
                                // Convert to local panel coordinates
                                let local_x = self.mouse_position.x - panel_x;
                                let local_y = self.mouse_position.y;

                                // Get layer entries and hit test
                                let entries = self.editor.build_layer_tree();
                                let result = layer_panel::hit_test_layer_panel(
                                    local_x,
                                    local_y,
                                    &entries,
                                    &self.layer_panel_config,
                                );

                                if let Some(action) = result.action {
                                    self.handle_layer_panel_action(action);
                                    handled_by_ui = true;
                                }
                            }
                        }
                    }
                }

                // If not handled by UI, pass to editor
                if !handled_by_ui {
                    let mouse_button = match button {
                        winit::event::MouseButton::Left => MouseButton::Left,
                        winit::event::MouseButton::Middle => MouseButton::Middle,
                        winit::event::MouseButton::Right => MouseButton::Right,
                        _ => MouseButton::Left,
                    };

                    let input = match state {
                        ElementState::Pressed => InputEvent::PointerDown {
                            position: self.mouse_position,
                            button: mouse_button,
                            modifiers: self.modifiers,
                        },
                        ElementState::Released => InputEvent::PointerUp {
                            position: self.mouse_position,
                            button: mouse_button,
                            modifiers: self.modifiers,
                        },
                    };

                    let effects = self.editor.handle_event(input);

                    if effects.redraw {
                        self.needs_redraw = true;
                    }

                    if let Some(cursor) = effects.cursor {
                        self.set_cursor(cursor);
                    }
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let scroll_delta = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => {
                        Vec2::new(x * 20.0, y * 20.0)
                    }
                    winit::event::MouseScrollDelta::PixelDelta(pos) => {
                        Vec2::new(pos.x as f32, pos.y as f32)
                    }
                };

                let input = InputEvent::Scroll {
                    position: self.mouse_position,
                    delta: scroll_delta,
                    modifiers: self.modifiers,
                };

                let effects = self.editor.handle_event(input);

                if effects.redraw {
                    self.needs_redraw = true;
                }
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    if let Some(key) = keycode_to_key(&event.physical_key) {
                        let input = InputEvent::KeyDown {
                            key,
                            modifiers: self.modifiers,
                        };

                        let effects = self.editor.handle_event(input);

                        if effects.redraw {
                            self.needs_redraw = true;
                        }
                    }
                }
            }

            WindowEvent::ModifiersChanged(new_modifiers) => {
                let state = new_modifiers.state();
                self.modifiers = Modifiers {
                    shift: state.shift_key(),
                    ctrl: state.control_key(),
                    alt: state.alt_key(),
                    meta: state.super_key(),
                };
                let effects = self.editor.handle_event(InputEvent::ModifiersChanged {
                    modifiers: self.modifiers,
                });
                if effects.redraw {
                    self.needs_redraw = true;
                }
            }

            _ => {}
        }

        // Request redraw if needed
        if self.needs_redraw {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.needs_redraw {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
}

/// Convert editor Cursor to winit CursorIcon.
fn cursor_to_winit(cursor: Cursor) -> winit::window::CursorIcon {
    match cursor {
        Cursor::Default => winit::window::CursorIcon::Default,
        Cursor::Pointer => winit::window::CursorIcon::Pointer,
        Cursor::Move => winit::window::CursorIcon::Move,
        Cursor::Crosshair => winit::window::CursorIcon::Crosshair,
        Cursor::Text => winit::window::CursorIcon::Text,
        Cursor::Grab => winit::window::CursorIcon::Grab,
        Cursor::Grabbing => winit::window::CursorIcon::Grabbing,
        Cursor::ResizeNS => winit::window::CursorIcon::NsResize,
        Cursor::ResizeEW => winit::window::CursorIcon::EwResize,
        Cursor::ResizeNESW => winit::window::CursorIcon::NeswResize,
        Cursor::ResizeNWSE => winit::window::CursorIcon::NwseResize,
    }
}

/// Convert winit PhysicalKey to editor Key.
fn keycode_to_key(key: &PhysicalKey) -> Option<Key> {
    match key {
        PhysicalKey::Code(code) => match code {
            KeyCode::KeyA => Some(Key::A),
            KeyCode::KeyB => Some(Key::B),
            KeyCode::KeyC => Some(Key::C),
            KeyCode::KeyD => Some(Key::D),
            KeyCode::KeyE => Some(Key::E),
            KeyCode::KeyF => Some(Key::F),
            KeyCode::KeyG => Some(Key::G),
            KeyCode::KeyH => Some(Key::H),
            KeyCode::KeyI => Some(Key::I),
            KeyCode::KeyJ => Some(Key::J),
            KeyCode::KeyK => Some(Key::K),
            KeyCode::KeyL => Some(Key::L),
            KeyCode::KeyM => Some(Key::M),
            KeyCode::KeyN => Some(Key::N),
            KeyCode::KeyO => Some(Key::O),
            KeyCode::KeyP => Some(Key::P),
            KeyCode::KeyQ => Some(Key::Q),
            KeyCode::KeyR => Some(Key::R),
            KeyCode::KeyS => Some(Key::S),
            KeyCode::KeyT => Some(Key::T),
            KeyCode::KeyU => Some(Key::U),
            KeyCode::KeyV => Some(Key::V),
            KeyCode::KeyW => Some(Key::W),
            KeyCode::KeyX => Some(Key::X),
            KeyCode::KeyY => Some(Key::Y),
            KeyCode::KeyZ => Some(Key::Z),
            KeyCode::Digit0 => Some(Key::Digit0),
            KeyCode::Digit1 => Some(Key::Digit1),
            KeyCode::Digit2 => Some(Key::Digit2),
            KeyCode::Digit3 => Some(Key::Digit3),
            KeyCode::Digit4 => Some(Key::Digit4),
            KeyCode::Digit5 => Some(Key::Digit5),
            KeyCode::Digit6 => Some(Key::Digit6),
            KeyCode::Digit7 => Some(Key::Digit7),
            KeyCode::Digit8 => Some(Key::Digit8),
            KeyCode::Digit9 => Some(Key::Digit9),
            KeyCode::Escape => Some(Key::Escape),
            KeyCode::Backspace => Some(Key::Backspace),
            KeyCode::Delete => Some(Key::Delete),
            KeyCode::Enter => Some(Key::Enter),
            KeyCode::Tab => Some(Key::Tab),
            KeyCode::Space => Some(Key::Space),
            KeyCode::ArrowUp => Some(Key::ArrowUp),
            KeyCode::ArrowDown => Some(Key::ArrowDown),
            KeyCode::ArrowLeft => Some(Key::ArrowLeft),
            KeyCode::ArrowRight => Some(Key::ArrowRight),
            KeyCode::Home => Some(Key::Home),
            KeyCode::End => Some(Key::End),
            KeyCode::PageUp => Some(Key::PageUp),
            KeyCode::PageDown => Some(Key::PageDown),
            KeyCode::F1 => Some(Key::F1),
            KeyCode::F2 => Some(Key::F2),
            KeyCode::F3 => Some(Key::F3),
            KeyCode::F4 => Some(Key::F4),
            KeyCode::F5 => Some(Key::F5),
            KeyCode::F6 => Some(Key::F6),
            KeyCode::F7 => Some(Key::F7),
            KeyCode::F8 => Some(Key::F8),
            KeyCode::F9 => Some(Key::F9),
            KeyCode::F10 => Some(Key::F10),
            KeyCode::F11 => Some(Key::F11),
            KeyCode::F12 => Some(Key::F12),
            KeyCode::BracketLeft => Some(Key::BracketLeft),
            KeyCode::BracketRight => Some(Key::BracketRight),
            _ => None,
        },
        PhysicalKey::Unidentified(_) => None,
    }
}

/// Offset a UI draw command by x and y.
fn offset_command(cmd: ui::DrawCommand, dx: f32, dy: f32) -> ui::DrawCommand {
    use ui::DrawCommand;

    match cmd {
        DrawCommand::FillRect {
            rect,
            color,
            corner_radius,
        } => DrawCommand::FillRect {
            rect: ui::Rect::new(rect.x + dx, rect.y + dy, rect.width, rect.height),
            color,
            corner_radius,
        },
        DrawCommand::StrokeRect {
            rect,
            color,
            width,
            corner_radius,
        } => DrawCommand::StrokeRect {
            rect: ui::Rect::new(rect.x + dx, rect.y + dy, rect.width, rect.height),
            color,
            width,
            corner_radius,
        },
        DrawCommand::Text {
            text,
            rect,
            style,
            clip,
        } => DrawCommand::Text {
            text,
            rect: ui::Rect::new(rect.x + dx, rect.y + dy, rect.width, rect.height),
            style,
            clip,
        },
        DrawCommand::Icon { kind, rect, color } => DrawCommand::Icon {
            kind,
            rect: ui::Rect::new(rect.x + dx, rect.y + dy, rect.width, rect.height),
            color,
        },
        DrawCommand::PushClip(rect) => DrawCommand::PushClip(ui::Rect::new(
            rect.x + dx,
            rect.y + dy,
            rect.width,
            rect.height,
        )),
        DrawCommand::PopClip => DrawCommand::PopClip,
    }
}

fn main() {
    env_logger::init();

    let event_loop = EventLoop::new().expect("Failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::new();
    event_loop.run_app(&mut app).expect("Event loop error");
}
