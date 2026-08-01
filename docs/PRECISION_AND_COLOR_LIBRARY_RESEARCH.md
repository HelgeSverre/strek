# Precision Controls and Color Library Research

Research date: 2026-08-01

## Objective

Define a bounded implementation for Strek's grids, rulers, guides, snapping
controls, and reusable colors. The comparison focuses on workflows that matter
for logo and icon editing, not on reproducing complete illustration or design-
token systems.

Questions considered:

1. Which settings belong to the document, and which belong to the current
   workspace?
2. Which grid, guide, and snap controls provide most of the practical value?
3. How should competing snap targets be resolved?
4. What vocabulary keeps saved values distinct from linked variables?
5. How should colors be added, found, grouped, sorted, edited, and removed?
6. Which established features are scope traps for Strek?

Primary product documentation and public source/schema material were preferred.
Confidence is high unless a row says otherwise; exact snap tie-breaking is often
undocumented and is therefore an informed product decision rather than copied
behavior.

## Product comparison

| Product | Precision model | Reusable-color model | Transferable lesson |
| --- | --- | --- | --- |
| Figma | Frame layout guides, canvas/frame ruler guides, compact global snap preferences | Linked styles and variables in searchable collections/groups | Put saved colors in every picker, but do not inherit variable modes and aliases |
| Sketch | Canvas/frame square and layout grids, rulers, Smart Guides, separate pixel/vector snapping | Linked Color Variables, quick creation from a color well, searchable picker | Quick add and keyboard search are more important than a large manager |
| Illustrator | Document grid, global/artboard rulers, ruler and vector guides, granular Smart Guides | Document swatches, groups, linked global colors, process/spot types | Familiar ruler gestures and preserve-appearance deletion; avoid print-era complexity |
| Affinity Vector Studio | Automatic/fixed grids, many grid types, exact guide controls, detailed snapping categories | Document/application palettes and optional linked global colors | Independent visibility/snap controls and screen-space tolerance |
| Penpot | Page ruler guides, board grids, independently controlled workspace snapping | File-library color assets plus a separate token system | Keep simple saved colors separate from semantic tokens |
| Inkscape | Document grids/guides, zoom-adaptive rendering, very granular snap targets | Linked document swatches and static external palettes | Suppress visually dense grid lines; avoid duplicating its full snap matrix |

## Precision controls

### Convergent patterns

The strongest cross-product patterns are:

- Guide coordinates and grid definitions are authored document data.
- Visibility, snapping categories, and screen tolerance are interaction
  preferences and should not make a document dirty.
- Grid spacing remains fixed in document coordinates while rendering adapts to
  zoom so the canvas does not become an opaque mesh.
- Rulers create horizontal/vertical guides through direct dragging.
- Guides can be moved, entered numerically, hidden, locked, cleared, deleted
  with Delete, and removed by returning them to a ruler.
- Snapping categories are independently controllable in the more approachable
  tools. Systems that make grid and object snapping mutually exclusive are less
  predictable and should not be copied.
- Snap tolerance is a screen-pixel distance, independent of zoom.
- Holding Control to suppress snapping temporarily is established in Figma and
  fits Strek's existing modifier-driven interactions.

### Useful differences

- Figma, Sketch, and Penpot emphasize row/column layout grids on frames or
  boards. Those are valuable for screen layout but not essential to Strek's
  initial logo/icon workflow.
- Affinity and Inkscape expose rectangular, angular, isometric/axonometric, and
  multiple grids. The flexibility carries a sizable configuration and rendering
  cost.
- Illustrator and Affinity support custom ruler origins. Penpot does not
  establish that behavior, and Inkscape's strongest use case is configurable
  grid origin rather than a universal ruler-origin workflow.
- Illustrator can convert arbitrary artwork into guides; Inkscape supports
  rotated guides. Both are specialist extensions after ordinary guides work.
- Inkscape exposes path intersections, tangents, perpendiculars, equal spacing,
  clipping geometry, and many other snap targets. Strek's existing bounds,
  centers, corners, and baselines cover the first useful object category.

### Recommended boundary

Version one should provide:

- one square, document-wide grid with spacing and a major-line interval;
- top and left pixel rulers with a fixed document origin at `(0, 0)`;
- document-wide horizontal and vertical guides;
- exact guide coordinates, duplicate, delete, hide, lock-all, and clear-all;
- independent master, Objects, Guides, and Grid snap controls;
- the existing 8-screen-pixel default tolerance, user-adjustable in a compact
  range;
- nearest X/Y corrections chosen independently, with deterministic tie order
  `guide > object > grid`;
- Control as a temporary snap bypass;
- zoom-adaptive grid rendering and visible feedback for the winning target.

Defer per-frame grids/guides, rows/columns, custom origins, multiple/angular
grids, rotated guides, artwork-as-guide, snap presets, pixel fitting, and the
full path-geometry target matrix.

## Reusable colors

### Two distinct concepts

The surveyed products often provide both of these concepts, even when their UI
does not make the boundary obvious:

1. A saved value that is copied when applied. Existing artwork has no continuing
   dependency on it.
2. A linked variable/global color. Editing the definition updates usages, and
   applying a literal value detaches a usage.

The requested behavior is the first concept: removing a saved entry must not
remove or modify that color in existing artwork. Implementing implicit links
would add reference tracking, bulk update semantics, detachment, aliases,
themes, and migration decisions that are not needed for an ad-hoc palette.

### Vocabulary

Use the following terms consistently:

| Term | Meaning |
| --- | --- |
| Document Color Library | The complete document-local feature and data set |
| Color Library | The movable manager panel title |
| Saved Colors | The picker section containing reusable values |
| Saved Color | One optional-name, solid RGBA value |
| Color Group | An ordered, collapsible organizational container |
| Swatch | The small visual color sample, not the primary data-model name |
| Recent Colors | Automatically remembered picker history; separate from saved colors |
| Color Variable | Reserved for a future linked definition that can update artwork |

Avoid using “token” for unlinked values. “Palette” can describe a visual
arrangement informally, but should not compete with Color Library as a second
feature name.

### Convergent workflows

- Figma, Sketch, Affinity, and Penpot provide a direct add action near the
  current picker color.
- Search by saved name is available from color-selection surfaces, not only in
  a separate asset manager.
- Groups/folders are common once the set grows beyond a small row of swatches.
- Editing and deletion are manager operations; application remains fast in the
  shared picker.
- Illustrator's deletion behavior preserves appearance when linked swatches are
  removed. The unlinked Strek model achieves this more simply because artwork
  already owns a literal RGBA value.
- Manual order remains valuable even where alphabetical sorting is the default.
  Numeric name prefixes, as suggested for Sketch, are a workaround Strek should
  replace with a real order field.

### Sorting definitions

HSL “lightness” is not perceptually uniform. CSS Color 4 describes OKLCH in
terms of perceptual lightness, chroma, and hue, making it a better basis for
organizing visually related colors. “Brightness” should remain a separate mode
based on the WCAG relative-luminance calculation for sRGB.

Recommended modes:

- **Manual:** stored group order, then stored color order.
- **Name:** case-insensitive name, with unnamed colors after named colors.
- **Hue & Shades:** chromatic colors by OKLCH hue, then OKLCH lightness;
  achromatic colors form a neutral section.
- **Lightness:** OKLCH `L`.
- **Chroma:** OKLCH `C`; the UI can explain this as color intensity rather than
  calling it HSL saturation.
- **Brightness:** sRGB relative luminance.

Automatic sorting is a presentation mode and must retain the manual order. A
drag while auto-sorted should first preserve the displayed order as Manual,
then apply the requested move.

### Recommended boundary

Version one should provide:

- document-local, copy-by-value solid RGBA Saved Colors;
- optional names with uppercase HEX fallback labels;
- one-click Save Color in every shared picker context;
- exact-RGBA duplicate detection, with explicit duplication available in the
  manager;
- typeahead by name, group, HEX, or RGB value wherever a color can be selected;
- manually ordered, collapsible groups and an Ungrouped section;
- a 1-based Order field in Manual mode plus drag reordering;
- inline name and color editing, duplicate, move, and remove;
- per-group Manual, Name, Hue & Shades, Lightness, Chroma, and Brightness sorts;
- a modeless, movable, resizable in-app Color Library panel whose geometry and
  collapsed state are workspace preferences.

Editing or deleting a Saved Color affects only the library entry. Deleting a
group moves its colors to Ungrouped by default; deleting the group and its
colors is an explicit confirmed action.

Defer linked Color Variables, modes/themes, aliases, app-wide/team libraries,
cloud synchronization, gradients/patterns, print color systems, palette file
interchange, image extraction, harmony generation, and automatic recoloring.

## Sources

### Figma

- [Layout guides](https://help.figma.com/hc/en-us/articles/360040450513-Create-layout-guides-with-grids-columns-and-rows)
- [Ruler guides](https://help.figma.com/hc/en-us/articles/360040449713-Add-guides-to-the-canvas-or-frames)
- [Snapping preferences](https://help.figma.com/hc/en-us/articles/360039956914-Adjust-alignment-rotation-and-position)
- [Color picker](https://help.figma.com/hc/en-us/articles/360041003774-Apply-paints-with-the-color-picker)
- [Variables, collections, and modes](https://help.figma.com/hc/en-us/articles/14506821864087-Overview-of-variables-collections-and-modes)

### Sketch

- [Canvas, grids, and rulers](https://www.sketch.com/docs/interface-and-settings/the-mac-app-interface/the-canvas/)
- [Color Variables](https://www.sketch.com/docs/symbols-and-styles/color-variables/)
- [Color panel](https://www.sketch.com/docs/symbols-and-styles/styling/the-color-panel/)
- [Selection Colors](https://www.sketch.com/docs/symbols-and-styles/selection-colors/)

### Adobe Illustrator

- [Document grid](https://helpx.adobe.com/illustrator/desktop/measure-and-align/grids-and-guides/align-graphic-objects-with-grids.html)
- [Rulers](https://helpx.adobe.com/illustrator/desktop/measure-and-align/grids-and-guides/use-rulers.html)
- [Ruler guides](https://helpx.adobe.com/illustrator/desktop/measure-and-align/grids-and-guides/align-graphic-objects-with-guides.html)
- [Smart Guide options](https://helpx.adobe.com/illustrator/desktop/measure-and-align/grids-and-guides/smart-guides-options.html)
- [Swatches](https://helpx.adobe.com/illustrator/desktop/manage-colors/use-swatches/about-swatches.html)
- [Color groups](https://helpx.adobe.com/illustrator/desktop/manage-colors/use-swatches/group-swatches.html)

### Affinity

- [Unified Affinity launch](https://www.canva.com/newsroom/news/all-new-affinity/)
- [Grids](https://www.affinity.studio/help/design-aids-grids/)
- [Rulers](https://www.affinity.studio/help/design-aids-rulers/)
- [Guides](https://www.affinity.studio/help/design-aids-guides/)
- [Snapping](https://www.affinity.studio/help/design-aids-snapping/)
- [Swatches panel](https://www.affinity.studio/help/panels-swatches-panel/)
- [Global colors](https://www.affinity.studio/help/clr-global-clr/)

### Penpot

- [Workspace basics](https://help.penpot.app/user-guide/designing/workspace-basics/)
- [Color and stroke](https://help.penpot.app/user-guide/designing/color-stroke/)
- [Assets](https://help.penpot.app/user-guide/design-systems/assets/)
- [File format](https://help.penpot.app/technical-guide/developer/data-model/penpot-file-format/)

### Inkscape

- [Keyboard and mouse reference](https://inkscape.org/doc/keys.html)
- [Inkscape 1.4 release notes](https://inkscape.org/doc/release_notes/1.4/Inkscape_1.4.html)
- [Inkscape 1.2 release notes](https://wiki.inkscape.org/wiki/Release_notes/1.2)
- [Grid implementation](https://gitlab.com/inkscape/inkscape/-/raw/master/src/display/control/canvas-item-grid.cpp)

### Color sorting

- [CSS Color Module Level 4: OKLCH](https://www.w3.org/TR/css-color-4/#ok-lab)
- [WCAG relative luminance](https://www.w3.org/WAI/WCAG21/Understanding/relative-luminance.html)
