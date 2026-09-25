//! Font family choices for the searchable font picker.

use crate::command_palette::{PaletteEntry, PaletteTarget};
use crate::typography::{FontResolution, FontResolutionStatus};

/// Generic families offered first, as stored document values and labels.
const GENERIC_FAMILIES: [(&str, &str); 3] = [
    ("sans-serif", "System Sans-Serif"),
    ("serif", "System Serif"),
    ("monospace", "System Monospace"),
];

/// Build picker rows: generic families, the current family when it is not
/// installed, then every installed family. While the query is empty the
/// current family ranks first, then the generic families in order.
///
/// Every row previews the face it will draw with.
pub(crate) fn entries(
    current_family: &str,
    installed: &[String],
    resolve: impl Fn(&str) -> FontResolution,
    is_bundled: impl Fn(&str) -> bool,
) -> Vec<PaletteEntry> {
    let current_family = current_family.trim();
    let current = resolve(current_family);
    let mut entries = Vec::with_capacity(installed.len() + GENERIC_FAMILIES.len() + 1);

    for (index, (family, label)) in GENERIC_FAMILIES.into_iter().enumerate() {
        let resolution = resolve(family);
        entries.push(entry(
            family,
            label,
            "Generic",
            resolution
                .family
                .as_deref()
                .map(|face| format!("Draws as {face}"))
                .unwrap_or_else(|| "No installed font".to_owned()),
            resolution.family,
            family.eq_ignore_ascii_case(current_family)
                || (family == "sans-serif" && current_family.eq_ignore_ascii_case("system-ui")),
            Some(index + 1),
        ));
    }

    if current.status == FontResolutionStatus::Missing && !current_family.is_empty() {
        entries.push(entry(
            current_family,
            current_family,
            "Missing",
            current
                .family
                .as_deref()
                .map(|face| format!("Not installed · draws as {face}"))
                .unwrap_or_else(|| "Not installed".to_owned()),
            current.family.clone(),
            true,
            None,
        ));
    }

    for family in installed {
        entries.push(entry(
            family,
            family,
            if is_bundled(family) {
                "Bundled with Strek"
            } else {
                "Installed"
            },
            String::new(),
            Some(family.clone()),
            current.status == FontResolutionStatus::Installed && current_family == family.as_str(),
            None,
        ));
    }
    entries
}

fn entry(
    family: &str,
    label: &str,
    category: &'static str,
    description: String,
    preview_font: Option<String>,
    is_current: bool,
    rank: Option<usize>,
) -> PaletteEntry {
    PaletteEntry {
        target: PaletteTarget::FontFamily(family.to_owned().into()),
        label: label.to_owned().into(),
        description: description.into(),
        category: category.into(),
        shortcut: is_current.then(|| "Current".to_owned()),
        enabled: true,
        recent_rank: if is_current { Some(0) } else { rank },
        preview_font: preview_font.map(Into::into),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(family: &str) -> FontResolution {
        let (family, status) = match family.trim() {
            "sans-serif" | "system-ui" => ("Sans Face", FontResolutionStatus::Generic),
            "serif" => ("Serif Face", FontResolutionStatus::Generic),
            "monospace" => ("Mono Face", FontResolutionStatus::Generic),
            "Inter" => ("Inter", FontResolutionStatus::Installed),
            "Sans Face" => ("Sans Face", FontResolutionStatus::Installed),
            _ => ("Serif Face", FontResolutionStatus::Missing),
        };
        FontResolution {
            family: Some(family.to_owned()),
            status,
        }
    }

    fn installed() -> Vec<String> {
        vec!["Inter".into(), "Sans Face".into(), "Serif Face".into()]
    }

    fn targets(entries: &[PaletteEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match &entry.target {
                PaletteTarget::FontFamily(family) => family.to_string(),
                PaletteTarget::Command(_) => panic!("font picker produced a command"),
            })
            .collect()
    }

    fn current(entries: &[PaletteEntry]) -> Vec<String> {
        entries
            .iter()
            .filter(|entry| entry.recent_rank == Some(0))
            .map(|entry| entry.label.to_string())
            .collect()
    }

    #[test]
    fn lists_generics_then_installed_families_with_previews() {
        let entries = entries("Inter", &installed(), resolve, |family| family == "Inter");
        assert_eq!(
            targets(&entries),
            [
                "sans-serif",
                "serif",
                "monospace",
                "Inter",
                "Sans Face",
                "Serif Face"
            ]
        );
        assert_eq!(current(&entries), ["Inter"]);
        assert_eq!(entries[3].category.as_ref(), "Bundled with Strek");
        assert_eq!(entries[4].category.as_ref(), "Installed");
        // The palette lists ranked rows before the alphabetical rest, so the
        // generic rows stay on top after the current family.
        let ranks = entries
            .iter()
            .map(|entry| entry.recent_rank)
            .collect::<Vec<_>>();
        assert_eq!(ranks, [Some(1), Some(2), Some(3), Some(0), None, None]);
        assert_eq!(entries[0].description.as_ref(), "Draws as Sans Face");
        assert_eq!(
            entries[0].preview_font.as_ref().map(AsRef::as_ref),
            Some("Sans Face")
        );
        assert_eq!(
            entries[3].preview_font.as_ref().map(AsRef::as_ref),
            Some("Inter")
        );
    }

    #[test]
    fn generic_current_family_marks_only_the_generic_row() {
        // "Sans Face" is both installed and the sans-serif face; a document
        // storing the generic must not mark the concrete family as current.
        assert_eq!(
            current(&entries("sans-serif", &installed(), resolve, |_| false)),
            ["System Sans-Serif"]
        );
        assert_eq!(
            current(&entries(" system-ui ", &installed(), resolve, |_| false)),
            ["System Sans-Serif"]
        );
        assert_eq!(
            current(&entries("Sans Face", &installed(), resolve, |_| false)),
            ["Sans Face"]
        );
    }

    #[test]
    fn missing_current_family_is_listed_with_its_fallback() {
        let entries = entries("Brand Display", &installed(), resolve, |_| false);
        let missing = entries
            .iter()
            .find(|entry| entry.category.as_ref() == "Missing")
            .expect("missing family row");
        assert_eq!(missing.label.as_ref(), "Brand Display");
        assert_eq!(
            missing.description.as_ref(),
            "Not installed · draws as Serif Face"
        );
        assert_eq!(
            missing.preview_font.as_ref().map(AsRef::as_ref),
            Some("Serif Face")
        );
        assert_eq!(current(&entries), ["Brand Display"]);
        assert_eq!(entries.len(), installed().len() + 4);
    }
}
