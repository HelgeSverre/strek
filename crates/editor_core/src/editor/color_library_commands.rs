//! Undoable document Color Library commands.

use super::*;

impl Editor {
    // === Document Color Library ===

    /// Add a named color group at the end of the document library.
    pub fn add_color_group(&mut self, name: &str) -> Result<ColorGroupId, ColorLibraryError> {
        if self.document.color_library.groups.len() >= MAX_COLOR_GROUPS {
            return Err(ColorLibraryError::TooManyGroups);
        }
        let name = normalize_required_name(name)?;
        let id = self.document.allocate_color_group_id()?;
        self.edit_color_library("Add Color Group", move |library| {
            library.groups.push(ColorGroup {
                id,
                name,
                manual_order: library.groups.len() as u32,
                sort_mode: ColorSortMode::Manual,
                sort_direction: SortDirection::Ascending,
            });
            Ok(())
        })?;
        Ok(id)
    }

    /// Rename a color group after trimming surrounding whitespace.
    pub fn rename_color_group(
        &mut self,
        id: ColorGroupId,
        name: &str,
    ) -> Result<bool, ColorLibraryError> {
        let name = normalize_required_name(name)?;
        self.edit_color_library("Rename Color Group", move |library| {
            let group = library
                .groups
                .iter_mut()
                .find(|group| group.id == id)
                .ok_or(ColorLibraryError::GroupNotFound(id))?;
            if group.name == name {
                return Ok(false);
            }
            group.name = name;
            Ok(true)
        })
    }

    /// Move a group to a zero-based manual position.
    pub fn reorder_color_group(
        &mut self,
        id: ColorGroupId,
        index: usize,
    ) -> Result<bool, ColorLibraryError> {
        self.edit_color_library("Reorder Color Group", move |library| {
            let current = library
                .groups
                .iter()
                .position(|group| group.id == id)
                .ok_or(ColorLibraryError::GroupNotFound(id))?;
            let target = index.min(library.groups.len().saturating_sub(1));
            if current == target {
                return Ok(false);
            }
            let group = library.groups.remove(current);
            library.groups.insert(target, group);
            normalize_group_orders(library);
            Ok(true)
        })
    }

    /// Change the computed ordering of colors in a group without altering manual order.
    pub fn set_color_group_sort(
        &mut self,
        id: ColorGroupId,
        mode: ColorSortMode,
        direction: SortDirection,
    ) -> Result<bool, ColorLibraryError> {
        self.edit_color_library("Sort Color Group", move |library| {
            let group = library
                .groups
                .iter_mut()
                .find(|group| group.id == id)
                .ok_or(ColorLibraryError::GroupNotFound(id))?;
            if group.sort_mode == mode && group.sort_direction == direction {
                return Ok(false);
            }
            group.sort_mode = mode;
            group.sort_direction = direction;
            Ok(true)
        })
    }

    /// Remove a group, either moving its colors to Ungrouped or deleting them.
    pub fn remove_color_group(
        &mut self,
        id: ColorGroupId,
        delete_colors: bool,
    ) -> Result<(), ColorLibraryError> {
        self.edit_color_library("Remove Color Group", move |library| {
            let index = library
                .groups
                .iter()
                .position(|group| group.id == id)
                .ok_or(ColorLibraryError::GroupNotFound(id))?;
            library.groups.remove(index);
            normalize_group_orders(library);
            if delete_colors {
                library.colors.retain(|color| color.group_id != Some(id));
            } else {
                for color in library
                    .colors
                    .iter_mut()
                    .filter(|color| color.group_id == Some(id))
                {
                    color.group_id = None;
                }
            }
            normalize_all_color_orders(library);
            Ok(())
        })
    }

    /// Add an unlinked saved color and return its stable ID.
    pub fn add_saved_color(
        &mut self,
        group_id: Option<ColorGroupId>,
        name: Option<&str>,
        rgba: RgbaColor,
    ) -> Result<SavedColorId, ColorLibraryError> {
        if self.document.color_library.colors.len() >= MAX_SAVED_COLORS {
            return Err(ColorLibraryError::TooManyColors);
        }
        ensure_group_exists(&self.document.color_library, group_id)?;
        rgba.validate()?;
        let name = normalize_optional_name(name)?;
        let id = self.document.allocate_saved_color_id()?;
        self.edit_color_library("Add Saved Color", move |library| {
            let manual_order = library
                .colors
                .iter()
                .filter(|color| color.group_id == group_id)
                .count() as u32;
            library.colors.push(SavedColor {
                id,
                group_id,
                name,
                rgba,
                manual_order,
            });
            Ok(())
        })?;
        Ok(id)
    }

    /// Update a saved color's optional name and RGBA value.
    pub fn update_saved_color(
        &mut self,
        id: SavedColorId,
        name: Option<&str>,
        rgba: RgbaColor,
    ) -> Result<bool, ColorLibraryError> {
        rgba.validate()?;
        let name = normalize_optional_name(name)?;
        self.edit_color_library("Edit Saved Color", move |library| {
            let color = library
                .colors
                .iter_mut()
                .find(|color| color.id == id)
                .ok_or(ColorLibraryError::ColorNotFound(id))?;
            if color.name == name && color.rgba == rgba {
                return Ok(false);
            }
            color.name = name;
            color.rgba = rgba;
            Ok(true)
        })
    }

    /// Duplicate a saved color directly after it in manual order.
    pub fn duplicate_saved_color(
        &mut self,
        id: SavedColorId,
    ) -> Result<SavedColorId, ColorLibraryError> {
        if self.document.color_library.colors.len() >= MAX_SAVED_COLORS {
            return Err(ColorLibraryError::TooManyColors);
        }
        let duplicate_id = self.document.allocate_saved_color_id()?;
        self.edit_color_library("Duplicate Saved Color", move |library| {
            let source = library
                .color(id)
                .cloned()
                .ok_or(ColorLibraryError::ColorNotFound(id))?;
            materialize_manual_order(library, source.group_id)?;
            let target = library
                .color(id)
                .ok_or(ColorLibraryError::ColorNotFound(id))?
                .manual_order as usize
                + 1;
            library.colors.push(SavedColor {
                id: duplicate_id,
                manual_order: u32::MAX,
                ..source
            });
            place_color(library, duplicate_id, source.group_id, target)?;
            Ok(())
        })?;
        Ok(duplicate_id)
    }

    /// Move a saved color into a group and zero-based manual position.
    pub fn move_saved_color(
        &mut self,
        id: SavedColorId,
        group_id: Option<ColorGroupId>,
        index: usize,
    ) -> Result<bool, ColorLibraryError> {
        self.edit_color_library("Move Saved Color", move |library| {
            ensure_group_exists(library, group_id)?;
            let before = library
                .color(id)
                .cloned()
                .ok_or(ColorLibraryError::ColorNotFound(id))?;
            let target_len = library
                .colors
                .iter()
                .filter(|color| color.group_id == group_id)
                .count();
            let target =
                index.min(target_len.saturating_sub(usize::from(before.group_id == group_id)));
            let target_is_manual = group_id
                .and_then(|group_id| library.group(group_id))
                .is_none_or(|group| group.sort_mode == ColorSortMode::Manual);
            if before.group_id == group_id
                && target_is_manual
                && before.manual_order as usize == target
            {
                return Ok(false);
            }
            materialize_manual_order(library, group_id)?;
            place_color(library, id, group_id, index)?;
            Ok(true)
        })
    }

    /// Remove a saved-color entry without touching matching artwork.
    pub fn remove_saved_color(&mut self, id: SavedColorId) -> Result<(), ColorLibraryError> {
        self.edit_color_library("Remove Saved Color", move |library| {
            let index = library
                .colors
                .iter()
                .position(|color| color.id == id)
                .ok_or(ColorLibraryError::ColorNotFound(id))?;
            library.colors.remove(index);
            normalize_all_color_orders(library);
            Ok(())
        })
    }

    fn edit_color_library<T>(
        &mut self,
        description: &str,
        edit: impl FnOnce(&mut ColorLibrary) -> Result<T, ColorLibraryError>,
    ) -> Result<T, ColorLibraryError> {
        let before = self.document.color_library.clone();
        let mut after = before.clone();
        let value = edit(&mut after)?;
        after.validate()?;
        if before != after {
            let command =
                Command::new(description).with_patch(Patch::SetColorLibrary { before, after });
            command.apply(&mut self.document);
            self.history.push(command);
            self.needs_redraw = true;
        }
        Ok(value)
    }
}
