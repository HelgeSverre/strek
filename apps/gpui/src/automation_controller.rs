//! GPUI-thread automation dispatch and state projection.

use super::*;

impl Strek {
    pub(super) fn start_automation(
        &mut self,
        mut requests: tokio::sync::mpsc::UnboundedReceiver<automation::PendingRequest>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |editor, cx| {
            while let Some(pending) = requests.recv().await {
                let Some((request, responder)) = pending.begin() else {
                    continue;
                };
                let Ok(dispatch) = editor.update_in(cx, |editor, window, cx| {
                    editor.dispatch_automation(request, window, cx)
                }) else {
                    break;
                };
                let response = match dispatch {
                    AutomationDispatch::Immediate(response) => *response,
                    AutomationDispatch::Background(task) => {
                        let result = task.await;
                        let Ok(response) = editor.update_in(cx, |editor, window, cx| {
                            editor.complete_automation_io(result, window, cx)
                        }) else {
                            break;
                        };
                        response
                    }
                };
                responder.respond(response);
            }
        })
        .detach();
    }

    fn dispatch_automation(
        &mut self,
        request: automation::AutomationRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AutomationDispatch {
        use automation::AutomationRequest;
        use automation_io::AutomationIoOperation;

        let operation = match request {
            AutomationRequest::OpenDocument {
                path,
                discard_changes,
            } => {
                if self.file_operation != FileOperation::Idle {
                    return self
                        .automation_error("wait for the current file operation to finish", window);
                }
                self.settle_for_document_io();
                cx.notify();
                if self.document_is_dirty() && !discard_changes {
                    return self.automation_error(
                        "the document has unsaved changes; set discard_changes to true to replace it",
                        window,
                    );
                }
                let path = match automation_absolute_path(&path) {
                    Ok(path) => path,
                    Err(error) => return self.automation_error(error, window),
                };
                self.file_operation = FileOperation::Opening;
                AutomationIoOperation::Open {
                    path,
                    revision: self.editor.current_revision(),
                }
            }
            AutomationRequest::SaveDocument { path } => {
                if self.file_operation != FileOperation::Idle {
                    return self
                        .automation_error("wait for the current file operation to finish", window);
                }
                self.settle_for_document_io();
                let path = match automation_absolute_path(&path) {
                    Ok(path) => path,
                    Err(error) => return self.automation_error(error, window),
                };
                let path = if let Some(source) = self.document_origin.import_source_path() {
                    document_io::normalize_imported_document_path(path, source)
                } else {
                    Ok(document_io::normalize_document_path(path))
                };
                let path = match path {
                    Ok(path) => path,
                    Err(error) => return self.automation_error(error.to_string(), window),
                };
                self.file_operation = FileOperation::Saving;
                AutomationIoOperation::Save {
                    path,
                    document: Box::new(self.editor.document().clone()),
                    revision: self.editor.current_revision(),
                }
            }
            AutomationRequest::Export { format, path } => {
                if self.file_operation != FileOperation::Idle {
                    return self
                        .automation_error("wait for the current file operation to finish", window);
                }
                self.settle_for_document_io();
                let Some(snapshot) = self.editor.artwork_snapshot() else {
                    return self.automation_error("add visible artwork before exporting", window);
                };
                let export_format = automation_export_format(format);
                self.file_operation = FileOperation::Exporting;
                if let Some(path) = path {
                    let path = match automation_absolute_path(&path) {
                        Ok(path) => export::normalize_path(path, export_format),
                        Err(error) => {
                            self.file_operation = FileOperation::Idle;
                            return self.automation_error(error, window);
                        }
                    };
                    AutomationIoOperation::ExportToPath {
                        path,
                        format: export_format,
                        snapshot,
                    }
                } else {
                    AutomationIoOperation::Encode {
                        format,
                        export_format,
                        snapshot,
                        max_bytes: MAX_INLINE_AUTOMATION_ARTIFACT_BYTES,
                    }
                }
            }
            request => {
                return AutomationDispatch::Immediate(Box::new(
                    self.handle_automation(request, window, cx),
                ));
            }
        };

        self.update_window_title(window);
        cx.notify();
        let task = cx
            .background_executor()
            .spawn(async move { automation_io::execute(operation) });
        AutomationDispatch::Background(task)
    }

    fn complete_automation_io(
        &mut self,
        result: Result<automation_io::AutomationIoSuccess, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> automation::AutomationResponse {
        use automation_io::AutomationIoSuccess;

        self.file_operation = FileOperation::Idle;
        if let Ok(AutomationIoSuccess::Encoded { format, bytes }) = result.as_ref() {
            let state = self.automation_state(window);
            let mut response = automation::AutomationResponse::success(state, "encoded artwork");
            response.artifact = Some(automation::AutomationArtifact {
                format: automation_artifact_name(*format).to_owned(),
                media_type: automation_artifact_media_type(*format).to_owned(),
                base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            });
            self.update_window_title(window);
            cx.notify();
            return response;
        }
        let result = result.and_then(|success| match success {
            AutomationIoSuccess::Opened {
                path,
                revision,
                document,
            } => {
                if self.editor.current_revision() != revision {
                    return Err(
                        "the document changed while opening; the loaded document was not applied"
                            .to_owned(),
                    );
                }
                match *document {
                    document_io::OpenedDocument::Native(document) => {
                        self.replace_document(document, path.clone(), cx);
                        Ok(format!("opened {}", path.display()))
                    }
                    document_io::OpenedDocument::ImportedSvg(document) => {
                        self.replace_imported_document(document, path.clone(), cx);
                        Ok(format!("imported {}", path.display()))
                    }
                }
            }
            AutomationIoSuccess::Saved { path, revision } => {
                self.editor.mark_revision_saved(revision);
                self.document_origin = DocumentOrigin::Native(path.clone());
                self.record_recent_file(&path, cx);
                Ok(format!("saved {}", path.display()))
            }
            AutomationIoSuccess::Exported { path } => Ok(format!("exported {}", path.display())),
            AutomationIoSuccess::Encoded { .. } => {
                unreachable!("encoded results return before applying file metadata")
            }
        });

        self.refresh_color_library_panel(cx);
        self.update_window_title(window);
        cx.notify();
        let state = self.automation_state(window);
        match result {
            Ok(message) => automation::AutomationResponse::success(state, message),
            Err(message) => automation::AutomationResponse::error(state, message),
        }
    }

    fn automation_error(&self, message: impl Into<String>, window: &Window) -> AutomationDispatch {
        AutomationDispatch::Immediate(Box::new(automation::AutomationResponse::error(
            self.automation_state(window),
            message,
        )))
    }

    fn handle_automation(
        &mut self,
        request: automation::AutomationRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> automation::AutomationResponse {
        use automation::{AutomationRequest, PointerPhase, UiTarget};

        let result = match request {
            AutomationRequest::State => Ok("state read".to_owned()),
            AutomationRequest::Document => {
                let state = self.automation_state(window);
                return match self.automation_document() {
                    Ok(document) => {
                        let mut response =
                            automation::AutomationResponse::success(state, "document read");
                        response.document = Some(document);
                        response
                    }
                    Err(error) => automation::AutomationResponse::error(state, error),
                };
            }
            AutomationRequest::NewDocument { discard_changes } => {
                if self.file_operation != FileOperation::Idle {
                    Err("wait for the current file operation to finish".to_owned())
                } else {
                    self.settle_for_document_io();
                    cx.notify();
                    if self.document_is_dirty() && !discard_changes {
                        Err(
                            "the document has unsaved changes; set discard_changes to true to replace it"
                                .to_owned(),
                        )
                    } else {
                        self.reset_document();
                        Ok("created a new document".to_owned())
                    }
                }
            }
            AutomationRequest::OpenDocument {
                path: _,
                discard_changes: _,
            }
            | AutomationRequest::SaveDocument { path: _ }
            | AutomationRequest::Export { format: _, path: _ } => {
                unreachable!("file automation is dispatched to the background executor")
            }
            AutomationRequest::Action { id } => {
                let Some(spec) = commands::command(&id) else {
                    return automation::AutomationResponse::error(
                        self.automation_state(window),
                        format!("unknown command `{id}`"),
                    );
                };
                let commands::CommandTarget::Editor(action) = spec.target else {
                    return automation::AutomationResponse::error(
                        self.automation_state(window),
                        format!("`{id}` is not an editor action; use the dedicated UI tools"),
                    );
                };
                if !self.command_is_enabled(spec.target) {
                    Err(format!("command `{id}` is disabled in the current state"))
                } else {
                    let palette_closed = self.command_palette.take().is_some();
                    let menu_closed = self.dismiss_menus();
                    self.execute_automation_editor_action(action, window, cx);
                    if palette_closed || menu_closed {
                        cx.notify();
                    }
                    Ok(format!("performed `{id}`"))
                }
            }
            AutomationRequest::Select { ids, mode } => {
                match self.automation_selection(&ids, mode) {
                    Ok(()) => {
                        cx.notify();
                        Ok(format!("updated selection using {mode:?} mode"))
                    }
                    Err(error) => Err(error),
                }
            }
            AutomationRequest::SetColor { target, color } => {
                self.numeric_property_input = None;
                if self.editor.selection().is_empty() {
                    Err("select at least one layer before setting a color".to_owned())
                } else if matches!(target, automation::ColorTarget::FrameBackground)
                    && self.editor.selected_frame_data().is_none()
                {
                    Err("select a frame before setting its background color".to_owned())
                } else {
                    let paint = color
                        .as_deref()
                        .map(|color| {
                            color_picker::parse_hex_paint(color).ok_or_else(|| {
                                "color must be a 3, 4, 6, or 8 digit hexadecimal value".to_owned()
                            })
                        })
                        .transpose();
                    match paint {
                        Ok(paint) => {
                            match target {
                                automation::ColorTarget::Fill => {
                                    self.editor.set_selected_fill(paint);
                                }
                                automation::ColorTarget::Stroke => {
                                    self.editor.set_selected_stroke(paint);
                                }
                                automation::ColorTarget::FrameBackground => {
                                    self.editor.set_selected_frame_background(paint);
                                }
                            }
                            cx.notify();
                            Ok(format!("set selection {target:?} color"))
                        }
                        Err(error) => Err(error),
                    }
                }
            }
            AutomationRequest::SetNumericProperty { target, value } => {
                self.numeric_property_input = None;
                let target = automation_numeric_property(target);
                if !value.is_finite() {
                    Err("numeric property value must be finite".to_owned())
                } else if self.editor.selection().is_empty() {
                    Err("select at least one layer before setting a property".to_owned())
                } else if self.editor.set_numeric_property(target, value)
                    || self
                        .editor
                        .numeric_property_value(target)
                        .is_some_and(|current| (current - value).abs() <= f32::EPSILON)
                {
                    cx.notify();
                    Ok(format!("set selection {target:?} to {value}"))
                } else {
                    Err(format!(
                        "{target:?} is not available for the current selection"
                    ))
                }
            }
            AutomationRequest::SetPrecision { settings } => {
                let current_grid = self.editor.document().grid;
                let grid = editor_core::GridSettings::new(
                    settings.grid_spacing.unwrap_or(current_grid.spacing),
                    settings
                        .grid_major_every
                        .unwrap_or(current_grid.major_every),
                );
                match grid {
                    Err(error) => Err(error.to_string()),
                    Ok(grid) => {
                        let snap = editor_core::SnapSettings {
                            enabled: settings
                                .snapping
                                .unwrap_or(self.workspace_preferences.snapping_enabled),
                            objects: settings
                                .snap_objects
                                .unwrap_or(self.workspace_preferences.snap_to_objects),
                            guides: settings
                                .snap_guides
                                .unwrap_or(self.workspace_preferences.snap_to_guides),
                            grid: settings
                                .snap_grid
                                .unwrap_or(self.workspace_preferences.snap_to_grid),
                            tolerance: settings
                                .tolerance
                                .unwrap_or(self.workspace_preferences.snap_tolerance),
                        };
                        if let Err(error) = self.editor.set_snap_settings(snap) {
                            return automation::AutomationResponse::error(
                                self.automation_state(window),
                                error.to_string(),
                            );
                        }
                        if let Some(value) = settings.rulers {
                            self.workspace_preferences.show_rulers = value;
                        }
                        if let Some(value) = settings.grid_visible {
                            self.workspace_preferences.show_grid = value;
                        }
                        if let Some(value) = settings.guides_visible {
                            self.workspace_preferences.show_guides = value;
                        }
                        if let Some(value) = settings.guides_locked {
                            self.workspace_preferences.guides_locked = value;
                        }
                        if let Some(value) = settings.snapping {
                            self.workspace_preferences.snapping_enabled = value;
                        }
                        if let Some(value) = settings.snap_objects {
                            self.workspace_preferences.snap_to_objects = value;
                        }
                        if let Some(value) = settings.snap_guides {
                            self.workspace_preferences.snap_to_guides = value;
                        }
                        if let Some(value) = settings.snap_grid {
                            self.workspace_preferences.snap_to_grid = value;
                        }
                        if let Some(value) = settings.tolerance {
                            self.workspace_preferences.snap_tolerance = value;
                        }
                        match self.editor.set_grid_settings(grid) {
                            Ok(_) => {
                                self.schedule_workspace_preferences_persist(cx);
                                cx.notify();
                                Ok("updated precision settings".to_owned())
                            }
                            Err(error) => Err(error.to_string()),
                        }
                    }
                }
            }
            AutomationRequest::Guide {
                action,
                id,
                axis,
                position,
            } => {
                use automation::GuideAction;
                match action {
                    GuideAction::Add => match (axis, position) {
                        (Some(axis), Some(position)) => self
                            .editor
                            .add_guide(automation_guide_axis(axis), position)
                            .map(|id| format!("added guide {}", id.get()))
                            .map_err(|error| error.to_string()),
                        _ => Err("adding a guide requires axis and position".to_owned()),
                    },
                    GuideAction::Move => {
                        match (id.and_then(editor_core::GuideId::from_opaque), position) {
                            (Some(id), Some(position)) => self
                                .editor
                                .move_guide(id, position)
                                .map(|_| format!("moved guide {}", id.get()))
                                .map_err(|error| error.to_string()),
                            _ => Err("moving a guide requires a valid id and position".to_owned()),
                        }
                    }
                    GuideAction::Remove => match id.and_then(editor_core::GuideId::from_opaque) {
                        Some(id) => self
                            .editor
                            .remove_guide(id)
                            .map(|()| format!("removed guide {}", id.get()))
                            .map_err(|error| error.to_string()),
                        None => Err("removing a guide requires a valid id".to_owned()),
                    },
                    GuideAction::Clear => {
                        self.editor.clear_guides();
                        Ok("cleared guides".to_owned())
                    }
                }
            }
            AutomationRequest::ColorGroup { action, id, name } => {
                use automation::ColorGroupAction;
                match action {
                    ColorGroupAction::Add => name
                        .as_deref()
                        .ok_or_else(|| "adding a color group requires a name".to_owned())
                        .and_then(|name| {
                            self.editor
                                .add_color_group(name)
                                .map(|id| format!("added color group {}", id.get()))
                                .map_err(|error| error.to_string())
                        }),
                    ColorGroupAction::Rename => automation_color_group_id(id).and_then(|id| {
                        let name = name
                            .as_deref()
                            .ok_or_else(|| "renaming a color group requires a name".to_owned())?;
                        self.editor
                            .rename_color_group(id, name)
                            .map(|_| format!("renamed color group {}", id.get()))
                            .map_err(|error| error.to_string())
                    }),
                    ColorGroupAction::Remove => automation_color_group_id(id).and_then(|id| {
                        self.editor
                            .remove_color_group(id, false)
                            .map(|()| format!("removed color group {}", id.get()))
                            .map_err(|error| error.to_string())
                    }),
                }
            }
            AutomationRequest::SavedColor {
                action,
                id,
                group_id,
                name,
                color,
                target,
            } => {
                use automation::SavedColorAction;
                match action {
                    SavedColorAction::Add => {
                        automation_rgba_color(color.as_deref()).and_then(|rgba| {
                            let group = group_id
                                .map(|id| automation_color_group_id(Some(id)))
                                .transpose()?;
                            self.editor
                                .add_saved_color(group, name.as_deref(), rgba)
                                .map(|id| format!("added saved color {}", id.get()))
                                .map_err(|error| error.to_string())
                        })
                    }
                    SavedColorAction::Update => automation_saved_color_id(id).and_then(|id| {
                        let current = self
                            .editor
                            .document()
                            .color_library
                            .color(id)
                            .cloned()
                            .ok_or_else(|| "saved color does not exist".to_owned())?;
                        let rgba = color
                            .as_deref()
                            .map(|value| automation_rgba_color(Some(value)))
                            .transpose()?
                            .unwrap_or(current.rgba);
                        self.editor
                            .update_saved_color(
                                id,
                                name.as_deref().or(current.name.as_deref()),
                                rgba,
                            )
                            .map(|_| format!("updated saved color {}", id.get()))
                            .map_err(|error| error.to_string())
                    }),
                    SavedColorAction::Remove => automation_saved_color_id(id).and_then(|id| {
                        self.editor
                            .remove_saved_color(id)
                            .map(|()| format!("removed saved color {}", id.get()))
                            .map_err(|error| error.to_string())
                    }),
                    SavedColorAction::Apply => automation_saved_color_id(id).and_then(|id| {
                        if self.editor.selection().is_empty() {
                            return Err("select at least one layer before applying a saved color"
                                .to_owned());
                        }
                        let rgba = self
                            .editor
                            .document()
                            .color_library
                            .color(id)
                            .ok_or_else(|| "saved color does not exist".to_owned())?
                            .rgba
                            .components();
                        let paint = Some(editor_core::Paint::Solid(rgba));
                        let target = target.unwrap_or(automation::ColorTarget::Fill);
                        if matches!(target, automation::ColorTarget::FrameBackground)
                            && self.editor.selected_frame_data().is_none()
                        {
                            return Err("select a frame before applying a frame background color"
                                .to_owned());
                        }
                        match target {
                            automation::ColorTarget::Fill => self.editor.set_selected_fill(paint),
                            automation::ColorTarget::Stroke => {
                                self.editor.set_selected_stroke(paint)
                            }
                            automation::ColorTarget::FrameBackground => {
                                self.editor.set_selected_frame_background(paint)
                            }
                        };
                        Ok(format!("applied saved color {}", id.get()))
                    }),
                }
            }
            AutomationRequest::SetLayerProperties {
                ids,
                name,
                visible,
                locked,
            } => match self.set_automation_layer_properties(&ids, name, visible, locked) {
                Ok(()) => {
                    cx.notify();
                    Ok("updated layer properties".to_owned())
                }
                Err(error) => Err(error),
            },
            AutomationRequest::Pointer {
                phase,
                x,
                y,
                button,
                modifiers,
            } => {
                if self.file_operation == FileOperation::Opening {
                    Err(
                        "wait for the document to finish opening before sending canvas input"
                            .to_owned(),
                    )
                } else if self.numeric_property_scrub.is_some() {
                    Err(
                        "finish or cancel the numeric property scrub before sending canvas input"
                            .to_owned(),
                    )
                } else if self.command_palette.is_some()
                    || self.open_menu.is_some()
                    || self.layer_context_menu.is_some()
                {
                    Err("dismiss open menus and overlays before sending canvas input".to_owned())
                } else if !x.is_finite() || !y.is_finite() {
                    Err("pointer coordinates must be finite".to_owned())
                } else if matches!(phase, PointerPhase::Down)
                    && self.canvas_input_bounds.is_none_or(|bounds| {
                        x < 0.0 || y < 0.0 || x >= bounds.size.width.0 || y >= bounds.size.height.0
                    })
                {
                    Err("pointer down must be inside the canvas bounds from get_state".to_owned())
                } else {
                    if matches!(phase, PointerPhase::Down) {
                        self.property_color_input = None;
                        self.zoom_input = None;
                    }
                    let event = match phase {
                        PointerPhase::Down => editor_core::InputEvent::PointerDown {
                            position: glam::Vec2::new(x, y),
                            button: automation_mouse_button(button),
                            modifiers: automation_modifiers(modifiers),
                        },
                        PointerPhase::Move => editor_core::InputEvent::PointerMove {
                            position: glam::Vec2::new(x, y),
                            modifiers: automation_modifiers(modifiers),
                        },
                        PointerPhase::Up => editor_core::InputEvent::PointerUp {
                            position: glam::Vec2::new(x, y),
                            button: automation_mouse_button(button),
                            modifiers: automation_modifiers(modifiers),
                        },
                    };
                    let effects = self.editor.handle_event(event);
                    if let Some(cursor) = effects.cursor {
                        self.current_cursor = convert_cursor(cursor);
                    }
                    if effects.redraw {
                        cx.notify();
                    }
                    Ok(format!("sent pointer {phase:?}"))
                }
            }
            AutomationRequest::Text { text } => {
                if self.file_operation == FileOperation::Opening {
                    Err("wait for the document to finish opening before sending text".to_owned())
                } else if self.command_palette.is_some()
                    || self.open_menu.is_some()
                    || self.layer_context_menu.is_some()
                    || self.property_color_input.is_some()
                    || self.zoom_input.is_some()
                {
                    Err(
                        "dismiss open menus, overlays, and inline inputs before sending text"
                            .to_owned(),
                    )
                } else if self.editor.text_input_snapshot().is_none() {
                    Err("Strek is not editing text".to_owned())
                } else if self.editor.replace_text(None, &text) {
                    cx.notify();
                    Ok("inserted text".to_owned())
                } else {
                    Err("text did not change".to_owned())
                }
            }
            AutomationRequest::SetUi { target, visible } => {
                match target {
                    UiTarget::FillColorPicker => {
                        return self.automation_color_picker_response(
                            properties_panel::ColorTarget::Fill,
                            visible,
                            window,
                            cx,
                        );
                    }
                    UiTarget::StrokeColorPicker => {
                        return self.automation_color_picker_response(
                            properties_panel::ColorTarget::Stroke,
                            visible,
                            window,
                            cx,
                        );
                    }
                    UiTarget::FrameBackgroundColorPicker => {
                        return self.automation_color_picker_response(
                            properties_panel::ColorTarget::FrameBackground,
                            visible,
                            window,
                            cx,
                        );
                    }
                    UiTarget::MainMenu
                    | UiTarget::CommandPalette
                    | UiTarget::LayersPanel
                    | UiTarget::DesignPanel
                    | UiTarget::ColorLibrary
                    | UiTarget::PrecisionControls => {}
                }

                let scrub_cancelled = self.cancel_numeric_property_scrub();
                match target {
                    UiTarget::MainMenu => {
                        self.dismiss_menus();
                        if visible {
                            if self.editor.cancel_pointer_interaction() {
                                self.current_cursor = convert_cursor(self.editor.cursor());
                            }
                            self.property_color_input = None;
                            self.zoom_input = None;
                            self.finish_layer_rename(true, cx);
                            self.command_palette = None;
                            self.open_menu = Some(toolbar::MenuKind::Main);
                        }
                        cx.notify();
                    }
                    UiTarget::CommandPalette => {
                        self.set_command_palette_visible(
                            visible,
                            FocusPolicy::Preserve,
                            window,
                            cx,
                        );
                    }
                    UiTarget::LayersPanel => {
                        if self.show_layers_panel != visible {
                            self.toggle_layer_panel(&ToggleLayerPanel, window, cx);
                        }
                    }
                    UiTarget::DesignPanel => {
                        if self.show_design_panel != visible {
                            self.toggle_design_panel(&ToggleDesignPanel, window, cx);
                        }
                    }
                    UiTarget::ColorLibrary => {
                        if visible {
                            self.open_color_library(FocusPolicy::Preserve, window, cx);
                        } else {
                            self.color_library_open = false;
                            self.color_library_panel = None;
                            cx.notify();
                        }
                    }
                    UiTarget::PrecisionControls => {
                        self.precision_menu_open = visible;
                        cx.notify();
                    }
                    UiTarget::FillColorPicker
                    | UiTarget::StrokeColorPicker
                    | UiTarget::FrameBackgroundColorPicker => unreachable!(),
                }
                if scrub_cancelled {
                    cx.notify();
                }
                Ok(format!("set {target:?} visibility to {visible}"))
            }
            AutomationRequest::Activate => {
                cx.activate(true);
                window.activate_window();
                self.focus_handle.focus(window);
                Ok("activated Strek".to_owned())
            }
        };

        self.refresh_color_library_panel(cx);
        window.refresh();
        let state = self.automation_state(window);
        match result {
            Ok(message) => automation::AutomationResponse::success(state, message),
            Err(message) => automation::AutomationResponse::error(state, message),
        }
    }

    fn automation_color_picker_response(
        &mut self,
        target: properties_panel::ColorTarget,
        visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> automation::AutomationResponse {
        let matches_request = self
            .property_color_input
            .as_ref()
            .is_some_and(|input| input.matches(ColorInputScope::Selection, target));
        let result = if visible {
            if matches_request {
                Ok(())
            } else if color_input_paint(&self.editor, ColorInputScope::Selection, target).is_none()
            {
                Err("select at least one object before opening its color picker".to_owned())
            } else {
                let scrub_cancelled = self.cancel_numeric_property_scrub();
                let opened = self.start_property_color_input(
                    ColorInputScope::Selection,
                    target,
                    FocusPolicy::Preserve,
                    window,
                    cx,
                );
                if scrub_cancelled {
                    cx.notify();
                }
                opened
                    .then_some(())
                    .ok_or_else(|| "the selection color picker could not be opened".to_owned())
            }
        } else {
            let scrub_cancelled = self.cancel_numeric_property_scrub();
            if matches_request {
                self.dismiss_color_picker(cx);
            }
            if scrub_cancelled {
                cx.notify();
            }
            Ok(())
        };

        window.refresh();
        let state = self.automation_state(window);
        let message = format!("set selection {target:?} color picker visibility to {visible}");
        match result {
            Ok(()) => automation::AutomationResponse::success(state, message),
            Err(error) => automation::AutomationResponse::error(state, error),
        }
    }

    fn execute_automation_editor_action(
        &mut self,
        action: EditorAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            EditorAction::Undo => self.undo(&Undo, window, cx),
            EditorAction::Redo => self.redo(&Redo, window, cx),
            EditorAction::SelectAll => self.select_all(&SelectAll, window, cx),
            EditorAction::Delete => self.delete(&Delete, window, cx),
            action => self.execute_editor_action(action, cx),
        }
    }

    fn automation_state(&self, window: &Window) -> automation::AutomationState {
        let bounds = window.bounds();
        let view = self.editor.view();
        let (selected_layers, selected_layers_truncated) =
            bounded_automation_layer_names(self.editor.selection().iter().filter_map(|id| {
                self.editor
                    .document()
                    .get(id)
                    .map(|node| node.name.as_str())
            }));
        let actions = commands::COMMANDS
            .iter()
            .filter_map(|spec| {
                let commands::CommandTarget::Editor(_) = spec.target else {
                    return None;
                };
                Some(automation::AutomationAction {
                    id: spec.id.to_owned(),
                    label: spec.label.to_owned(),
                    enabled: self.command_is_enabled(spec.target),
                })
            })
            .collect();

        automation::AutomationState {
            process_id: std::process::id(),
            document: self.document_name(),
            dirty: self.document_is_dirty(),
            tool: automation_tool_name(self.editor.tool).to_owned(),
            interaction: automation_interaction_name(self.editor.interaction_kind()).to_owned(),
            selection_count: self.editor.selection().len(),
            selected_layers,
            selected_layers_truncated,
            zoom: view.zoom,
            pan: automation::AutomationPoint {
                x: view.pan.x,
                y: view.pan.y,
            },
            window: automation_bounds(bounds),
            canvas: self.canvas_input_bounds.map(automation_bounds),
            layers_panel_visible: self.show_layers_panel,
            design_panel_visible: self.show_design_panel,
            main_menu_open: self.open_menu == Some(toolbar::MenuKind::Main),
            command_palette_open: self.command_palette.is_some(),
            color_picker_open: self.property_color_input.is_some(),
            color_library_open: self.color_library_open,
            precision_controls_open: self.precision_menu_open,
            rulers_visible: self.workspace_preferences.show_rulers,
            grid_visible: self.workspace_preferences.show_grid,
            guides_visible: self.workspace_preferences.show_guides,
            numeric_property_scrub_active: self.numeric_property_scrub.is_some(),
            actions,
        }
    }

    fn automation_document(&mut self) -> Result<automation::AutomationDocument, String> {
        let root = self.editor.document().root;
        let ids = std::iter::once(root)
            .chain(self.editor.document().descendants(root))
            .take(MAX_AUTOMATION_DOCUMENT_LAYERS + 1)
            .collect::<Vec<_>>();
        if ids.len() > MAX_AUTOMATION_DOCUMENT_LAYERS {
            return Err(format!(
                "document contains more than {MAX_AUTOMATION_DOCUMENT_LAYERS} automation-visible layers"
            ));
        }

        let mut layers = Vec::with_capacity(ids.len());
        for id in ids {
            let Some((parent, children, name, kind, visible, locked, opacity, fill, stroke)) =
                self.editor.document().get(id).and_then(|node| {
                    (!node.deleted).then(|| {
                        (
                            node.parent,
                            node.children.clone(),
                            node.name.clone(),
                            automation_node_kind(&node.kind).to_owned(),
                            node.visible,
                            node.locked,
                            node.style.opacity,
                            node.style.fill.as_ref().map(color_picker::format_paint),
                            node.style
                                .stroke
                                .as_ref()
                                .map(|stroke| automation::AutomationStroke {
                                    color: color_picker::format_paint(&stroke.paint),
                                    width: stroke.width,
                                }),
                        )
                    })
                })
            else {
                continue;
            };
            let world = self
                .editor
                .layer_world_transform(id)
                .unwrap_or(glam::Affine2::IDENTITY);
            let world_bounds =
                self.editor
                    .layer_world_bounds(id)
                    .map(|bounds| automation::AutomationBounds {
                        x: bounds.min.x,
                        y: bounds.min.y,
                        width: bounds.width(),
                        height: bounds.height(),
                    });
            layers.push(automation::AutomationLayer {
                id: automation_node_id(id),
                parent_id: parent.map(automation_node_id),
                child_ids: children.into_iter().map(automation_node_id).collect(),
                name,
                kind,
                visible,
                locked,
                selected: self.editor.selection().contains(id),
                opacity,
                fill,
                stroke,
                world_bounds,
                world_transform: [
                    world.matrix2.x_axis.x,
                    world.matrix2.x_axis.y,
                    world.matrix2.y_axis.x,
                    world.matrix2.y_axis.y,
                    world.translation.x,
                    world.translation.y,
                ],
            });
        }

        let document = automation::AutomationDocument {
            root_id: automation_node_id(root),
            layers,
            grid: automation::AutomationGrid {
                spacing: self.editor.document().grid.spacing,
                major_every: self.editor.document().grid.major_every,
            },
            guides: self
                .editor
                .document()
                .guides
                .iter()
                .map(|guide| automation::AutomationGuide {
                    id: guide.id.get(),
                    axis: match guide.axis {
                        editor_core::GuideAxis::Horizontal => automation::GuideAxis::Horizontal,
                        editor_core::GuideAxis::Vertical => automation::GuideAxis::Vertical,
                    },
                    position: guide.position,
                })
                .collect(),
            color_groups: self
                .editor
                .document()
                .color_library
                .groups
                .iter()
                .map(|group| automation::AutomationColorGroup {
                    id: group.id.get(),
                    name: group.name.clone(),
                    manual_order: group.manual_order,
                    sort: automation_color_sort_name(group.sort_mode).to_owned(),
                    descending: group.sort_direction == editor_core::SortDirection::Descending,
                })
                .collect(),
            saved_colors: self
                .editor
                .document()
                .color_library
                .colors
                .iter()
                .map(|color| automation::AutomationSavedColor {
                    id: color.id.get(),
                    group_id: color.group_id.map(editor_core::ColorGroupId::get),
                    name: color.name.clone(),
                    color: color.rgba.hex_label(),
                    manual_order: color.manual_order,
                })
                .collect(),
        };
        let encoded_size = serde_json::to_vec(&document)
            .map_err(|error| format!("could not encode document inspection: {error}"))?
            .len();
        if encoded_size > MAX_AUTOMATION_DOCUMENT_BYTES {
            return Err(format!(
                "document inspection is {encoded_size} bytes; limit is {MAX_AUTOMATION_DOCUMENT_BYTES} bytes"
            ));
        }
        Ok(document)
    }

    fn automation_selection(
        &mut self,
        ids: &[String],
        mode: automation::SelectionMode,
    ) -> Result<(), String> {
        self.numeric_property_input = None;
        let ids = ids
            .iter()
            .map(|id| parse_automation_node_id(self.editor.document(), id))
            .collect::<Result<Vec<_>, _>>()?;
        if ids.contains(&self.editor.document().root) {
            return Err("the document root cannot be selected".to_owned());
        }
        if !matches!(mode, automation::SelectionMode::Remove) {
            if let Some(id) = ids
                .iter()
                .find(|id| !self.editor.document().is_effectively_editable(**id))
            {
                return Err(format!(
                    "layer `{}` cannot be selected because it or an ancestor is hidden or locked",
                    automation_node_id(*id)
                ));
            }
        }
        let mut selection = match mode {
            automation::SelectionMode::Replace => Vec::new(),
            automation::SelectionMode::Add
            | automation::SelectionMode::Remove
            | automation::SelectionMode::Toggle => self.editor.selection().to_vec(),
        };
        match mode {
            automation::SelectionMode::Replace | automation::SelectionMode::Add => {
                selection.extend(ids);
            }
            automation::SelectionMode::Remove => {
                selection.retain(|selected| !ids.contains(selected));
            }
            automation::SelectionMode::Toggle => {
                for id in ids {
                    if selection.contains(&id) {
                        selection.retain(|selected| *selected != id);
                    } else {
                        selection.push(id);
                    }
                }
            }
        }
        self.editor.set_layer_selection(selection);
        Ok(())
    }

    fn set_automation_layer_properties(
        &mut self,
        ids: &[String],
        name: Option<String>,
        visible: Option<bool>,
        locked: Option<bool>,
    ) -> Result<(), String> {
        self.numeric_property_input = None;
        if ids.is_empty() {
            return Err("provide at least one layer ID".to_owned());
        }
        if name.is_some() && ids.len() != 1 {
            return Err("name can only be set for one layer at a time".to_owned());
        }
        if name.is_none() && visible.is_none() && locked.is_none() {
            return Err("provide name, visible, or locked".to_owned());
        }
        let ids = ids
            .iter()
            .map(|id| parse_automation_node_id(self.editor.document(), id))
            .collect::<Result<Vec<_>, _>>()?;
        if ids.contains(&self.editor.document().root) {
            return Err("the document root properties cannot be changed".to_owned());
        }
        if let (Some(id), Some(name)) = (ids.first(), name) {
            if name.trim().is_empty() {
                return Err("layer name cannot be empty".to_owned());
            }
            self.editor.rename_layer(*id, &name);
        }
        for id in ids {
            if let Some(visible) = visible {
                self.editor.set_layer_visible(id, visible);
            }
            if let Some(locked) = locked {
                self.editor.set_layer_locked(id, locked);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use super::*;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn test_editor(cx: &mut gpui::TestAppContext) -> (Entity<Strek>, &mut gpui::VisualTestContext) {
        cx.add_window_view(|_, cx| Strek::new(commands::Keymap::default(), cx))
    }

    #[gpui::test]
    fn background_save_sends_one_response_and_restores_idle(cx: &mut gpui::TestAppContext) {
        let (editor, cx) = test_editor(cx);
        let (request_sender, request_receiver) = tokio::sync::mpsc::unbounded_channel();
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.start_automation(request_receiver, window, cx)
            });
        });

        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "strek-automation-controller-{}-{sequence}.strek.json",
            std::process::id()
        ));
        let (request, response_receiver) =
            automation::pending_request_for_test(automation::AutomationRequest::SaveDocument {
                path: path.display().to_string(),
            });
        request_sender.send(request).unwrap();
        cx.run_until_parked();

        let response = response_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert!(response.ok, "{:?}", response.message);
        assert!(response_receiver.try_recv().is_err());
        editor.update(cx, |editor, _| {
            assert_eq!(editor.file_operation, FileOperation::Idle);
            editor.recent_files_persist_task.take();
        });
        fs::remove_file(path).unwrap();
    }

    #[gpui::test]
    fn save_completion_does_not_clean_edits_made_after_its_snapshot(cx: &mut gpui::TestAppContext) {
        let (editor, cx) = test_editor(cx);
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                let saved_revision = editor.editor.current_revision();
                editor
                    .editor
                    .add_guide(editor_core::GuideAxis::Horizontal, 24.0)
                    .unwrap();
                editor.file_operation = FileOperation::Saving;
                let path = std::env::temp_dir().join("strek-save-completion-test.strek.json");

                let response = editor.complete_automation_io(
                    Ok(automation_io::AutomationIoSuccess::Saved {
                        path,
                        revision: saved_revision,
                    }),
                    window,
                    cx,
                );

                assert!(response.ok);
                assert!(editor.document_is_dirty());
                assert_eq!(editor.file_operation, FileOperation::Idle);
                editor.recent_files_persist_task.take();
            });
        });
    }

    #[gpui::test]
    fn open_completion_rejects_a_document_changed_while_loading(cx: &mut gpui::TestAppContext) {
        let (editor, cx) = test_editor(cx);
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                let opening_revision = editor.editor.current_revision();
                let initial_guide_count = editor.editor.document().guides.len();
                editor
                    .editor
                    .add_guide(editor_core::GuideAxis::Vertical, 12.0)
                    .unwrap();
                editor.file_operation = FileOperation::Opening;
                let response = editor.complete_automation_io(
                    Ok(automation_io::AutomationIoSuccess::Opened {
                        path: std::env::temp_dir().join("strek-open-completion-test.strek.json"),
                        revision: opening_revision,
                        document: Box::new(document_io::OpenedDocument::Native(
                            editor_core::Document::new(),
                        )),
                    }),
                    window,
                    cx,
                );

                assert!(!response.ok);
                assert_eq!(
                    editor.editor.document().guides.len(),
                    initial_guide_count + 1
                );
                assert_eq!(editor.file_operation, FileOperation::Idle);
            });
        });
    }

    #[gpui::test]
    fn focus_preserving_ui_automation_keeps_the_window_inactive(cx: &mut gpui::TestAppContext) {
        let (editor, cx) = test_editor(cx);
        cx.deactivate_window();
        cx.update(|window, cx| {
            assert!(!window.is_window_active());
            let dispatch = editor.update(cx, |editor, cx| {
                editor.dispatch_automation(
                    automation::AutomationRequest::SetUi {
                        target: automation::UiTarget::ColorLibrary,
                        visible: true,
                    },
                    window,
                    cx,
                )
            });
            assert!(matches!(dispatch, AutomationDispatch::Immediate(response) if response.ok));
            assert!(!window.is_window_active());
        });
    }

    #[gpui::test]
    fn background_open_settles_transient_edits_before_the_dirty_check(
        cx: &mut gpui::TestAppContext,
    ) {
        let (editor, cx) = test_editor(cx);
        cx.update(|window, cx| {
            let initial_descendant_count = editor.update(cx, |editor, _| {
                editor
                    .editor
                    .document()
                    .descendants(editor.editor.document().root)
                    .count()
            });
            let dispatch = editor.update(cx, |editor, cx| {
                assert!(editor.editor.execute_action(EditorAction::ToolPen));
                for position in [glam::Vec2::ZERO, glam::Vec2::new(20.0, 0.0)] {
                    editor
                        .editor
                        .handle_event(editor_core::InputEvent::PointerDown {
                            position,
                            button: editor_core::MouseButton::Left,
                            modifiers: editor_core::Modifiers::default(),
                        });
                    editor
                        .editor
                        .handle_event(editor_core::InputEvent::PointerUp {
                            position,
                            button: editor_core::MouseButton::Left,
                            modifiers: editor_core::Modifiers::default(),
                        });
                }
                assert!(!editor.document_is_dirty());

                editor.dispatch_automation(
                    automation::AutomationRequest::OpenDocument {
                        path: std::env::temp_dir()
                            .join("strek-transient-open-test.strek.json")
                            .display()
                            .to_string(),
                        discard_changes: false,
                    },
                    window,
                    cx,
                )
            });

            assert!(matches!(dispatch, AutomationDispatch::Immediate(response) if !response.ok));
            editor.update(cx, |editor, _| {
                assert!(editor.document_is_dirty());
                assert_eq!(editor.file_operation, FileOperation::Idle);
                assert_eq!(
                    editor
                        .editor
                        .document()
                        .descendants(editor.editor.document().root)
                        .count(),
                    initial_descendant_count + 1
                );
            });
        });
    }
}
