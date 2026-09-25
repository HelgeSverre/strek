//! Cross-platform font families used by the application chrome and document text.
//!
//! Document text reaches three font engines: GPUI shaping for native canvas
//! text, and `usvg` for rotated canvas text, raster export, and outlined SVG
//! export. [`resolve_document_font_family`] is the single resolver for all of
//! them. It follows the same CSS family-list rules as `usvg` over the shared
//! [`system_font_database`], so every path draws a document family with the
//! same installed face, including when the requested family is missing.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock, Mutex};

use resvg::usvg::fontdb;

#[cfg(target_os = "macos")]
pub(crate) const MONOSPACE_FONT_FAMILY: &str = "Menlo";

#[cfg(target_os = "windows")]
pub(crate) const MONOSPACE_FONT_FAMILY: &str = "Consolas";

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) const MONOSPACE_FONT_FAMILY: &str = "DejaVu Sans Mono";

/// Installed families tried, in order, for each CSS generic family.
///
/// The first installed candidate becomes the concrete face for the generic in
/// both GPUI and `usvg`. Sans-serif follows the browser default on each
/// platform so exported live-text SVG looks the same in a browser.
#[cfg(target_os = "macos")]
const SANS_SERIF_CANDIDATES: &[&str] = &["Helvetica", "Helvetica Neue", "Arial"];
#[cfg(target_os = "macos")]
const SERIF_CANDIDATES: &[&str] = &["Times New Roman", "Times", "Georgia"];
#[cfg(target_os = "macos")]
const MONOSPACE_CANDIDATES: &[&str] = &[MONOSPACE_FONT_FAMILY, "Monaco", "Courier New"];

#[cfg(target_os = "windows")]
const SANS_SERIF_CANDIDATES: &[&str] = &["Arial", "Segoe UI", "Tahoma"];
#[cfg(target_os = "windows")]
const SERIF_CANDIDATES: &[&str] = &["Times New Roman", "Georgia", "Cambria"];
#[cfg(target_os = "windows")]
const MONOSPACE_CANDIDATES: &[&str] = &[MONOSPACE_FONT_FAMILY, "Courier New", "Lucida Console"];

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const SANS_SERIF_CANDIDATES: &[&str] = &[
    "DejaVu Sans",
    "Noto Sans",
    "Liberation Sans",
    "Cantarell",
    "Ubuntu",
    "FreeSans",
];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const SERIF_CANDIDATES: &[&str] = &[
    "DejaVu Serif",
    "Noto Serif",
    "Liberation Serif",
    "FreeSerif",
];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MONOSPACE_CANDIDATES: &[&str] = &[
    MONOSPACE_FONT_FAMILY,
    "Noto Sans Mono",
    "Liberation Mono",
    "Ubuntu Mono",
    "FreeMono",
];

/// Family name of the bundled interface font.
pub(crate) const UI_FONT_FAMILY: &str = "Inter";

/// Inter 4.1 (SIL Open Font License 1.1, see `assets/fonts/inter/OFL.txt`),
/// in the weights the interface uses. Embedding it gives every platform the
/// same interface font instead of GPUI's per-platform defaults, which on
/// Linux is the typewriter face "FreeMono".
const BUNDLED_UI_FONTS: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/inter/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/inter/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/inter/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/inter/Inter-Bold.ttf"),
];

/// Longest document family string the resolver will parse or cache.
///
/// Longer values resolve as missing without allocation in the cache.
const MAX_RESOLVED_FAMILY_BYTES: usize = 1024;
/// Resolution cache entries kept before the cache is cleared and rebuilt.
const MAX_RESOLUTION_CACHE_ENTRIES: usize = 256;

static SYSTEM_FONT_DATABASE: LazyLock<(Arc<fontdb::Database>, bool)> = LazyLock::new(|| {
    let (database, bundled_ui_font) = build_system_font_database();
    (Arc::new(database), bundled_ui_font)
});

static EMPTY_FONT_DATABASE: LazyLock<Arc<fontdb::Database>> =
    LazyLock::new(|| Arc::new(fontdb::Database::new()));

static SYSTEM_FONT_CATALOG: LazyLock<FontCatalog> =
    LazyLock::new(|| FontCatalog::new(&SYSTEM_FONT_DATABASE.0));

static RESOLUTION_CACHE: LazyLock<Mutex<HashMap<String, FontResolution>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Build the shared database; the flag records whether bundled Inter was loaded.
fn build_system_font_database() -> (fontdb::Database, bool) {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    let bundled_ui_font = load_bundled_fonts_if_missing(&mut database);
    configure_generic_families(&mut database);
    (database, bundled_ui_font)
}

/// Make the bundled interface font available to document text and export
/// when the system does not already provide it.
///
/// An installed copy wins so each engine sees exactly one "Inter" family.
/// Returns whether the bundled fonts were loaded.
fn load_bundled_fonts_if_missing(database: &mut fontdb::Database) -> bool {
    let installed = database.faces().any(|face| {
        face.families
            .iter()
            .any(|(family, _)| family == UI_FONT_FAMILY)
    });
    if !installed {
        for font in BUNDLED_UI_FONTS {
            database.load_font_data(font.to_vec());
        }
    }
    !installed
}

/// Bundled font files GPUI must register, following the same rule as
/// [`load_bundled_fonts_if_missing`] against GPUI's installed family names.
pub(crate) fn bundled_fonts_for_gpui(
    installed_families: &[String],
) -> Vec<std::borrow::Cow<'static, [u8]>> {
    if installed_families
        .iter()
        .any(|family| family == UI_FONT_FAMILY)
    {
        return Vec::new();
    }
    BUNDLED_UI_FONTS
        .iter()
        .map(|font| std::borrow::Cow::Borrowed(*font))
        .collect()
}

/// Point each CSS generic at the first installed platform candidate.
///
/// When no candidate is installed, sans-serif, serif, and monospace fall back
/// to each other, then to Inter, then to the alphabetically first installed
/// family, and
/// uninstalled cursive/fantasy defaults use the sans-serif face, so a generic
/// never names a family that the database cannot load.
fn configure_generic_families(database: &mut fontdb::Database) {
    let installed = installed_family_names(database);
    let pick = |candidates: &[&str]| {
        candidates
            .iter()
            .find(|candidate| installed.contains_key(**candidate))
            .map(|candidate| (*candidate).to_owned())
    };
    // Prefer the bundled interface font over an arbitrary installed family.
    let any_installed = installed
        .get_key_value(UI_FONT_FAMILY)
        .or_else(|| installed.iter().next())
        .map(|(family, _)| family.clone());
    let sans = pick(SANS_SERIF_CANDIDATES);
    let serif = pick(SERIF_CANDIDATES);
    let monospace = pick(MONOSPACE_CANDIDATES);

    let sans = sans
        .clone()
        .or_else(|| serif.clone())
        .or_else(|| any_installed.clone());
    let serif = serif.or_else(|| sans.clone());
    let monospace = monospace.or_else(|| sans.clone());
    if let Some(sans) = sans {
        // Keep fontdb's cursive/fantasy defaults when installed; otherwise
        // they would name a family that cannot load.
        if !installed.contains_key(database.family_name(&fontdb::Family::Cursive)) {
            database.set_cursive_family(sans.clone());
        }
        if !installed.contains_key(database.family_name(&fontdb::Family::Fantasy)) {
            database.set_fantasy_family(sans.clone());
        }
        database.set_sans_serif_family(sans);
    }
    if let Some(serif) = serif {
        database.set_serif_family(serif);
    }
    if let Some(monospace) = monospace {
        database.set_monospace_family(monospace);
    }
}

/// Every family name a face answers to, keyed by name, with the display name
/// (the face's primary family) as the value.
fn installed_family_names(database: &fontdb::Database) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    for face in database.faces() {
        let Some((primary, _)) = face.families.first() else {
            continue;
        };
        for (family, _) in &face.families {
            names
                .entry(family.clone())
                .or_insert_with(|| primary.clone());
        }
    }
    names
}

pub(crate) fn system_font_database() -> Arc<fontdb::Database> {
    Arc::clone(&SYSTEM_FONT_DATABASE.0)
}

pub(crate) fn empty_font_database() -> Arc<fontdb::Database> {
    Arc::clone(&EMPTY_FONT_DATABASE)
}

/// Installed font families offered by the font picker, sorted for display.
///
/// Names come from the same database that export and rotated text use, so a
/// picked family is always one every text path can load.
pub(crate) fn installed_font_families() -> &'static [String] {
    &SYSTEM_FONT_CATALOG.display_families
}

/// Whether a family comes from a font embedded in Strek rather than the system.
pub(crate) fn is_bundled_font_family(family: &str) -> bool {
    family == UI_FONT_FAMILY && SYSTEM_FONT_DATABASE.1
}

/// Interface font family, bundled so it is available on every platform.
pub(crate) fn ui_font_family() -> &'static str {
    UI_FONT_FAMILY
}

/// Installed monospace family for numeric fields and codes in the chrome.
pub(crate) fn ui_monospace_font_family() -> &'static str {
    static UI_MONOSPACE_FONT: LazyLock<String> = LazyLock::new(|| {
        SYSTEM_FONT_CATALOG
            .generic(GenericFamily::Monospace)
            .cloned()
            .unwrap_or_else(|| MONOSPACE_FONT_FAMILY.to_owned())
    });
    &UI_MONOSPACE_FONT
}

/// Resolve a stored document family to the installed family every text path
/// draws, using the system font database.
pub(crate) fn resolve_document_font_family(family: &str) -> FontResolution {
    if family.len() > MAX_RESOLVED_FAMILY_BYTES {
        return SYSTEM_FONT_CATALOG.resolve(family);
    }
    let Ok(mut cache) = RESOLUTION_CACHE.lock() else {
        return SYSTEM_FONT_CATALOG.resolve(family);
    };
    if let Some(resolution) = cache.get(family) {
        return resolution.clone();
    }
    let resolution = SYSTEM_FONT_CATALOG.resolve(family);
    if cache.len() >= MAX_RESOLUTION_CACHE_ENTRIES {
        cache.clear();
    }
    cache.insert(family.to_owned(), resolution.clone());
    resolution
}

/// How a document family string maps onto installed fonts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FontResolution {
    /// Installed family used for drawing, or `None` when no font is installed.
    pub family: Option<String>,
    pub status: FontResolutionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FontResolutionStatus {
    /// A CSS generic family (or `system-ui`) mapped to an installed face.
    Generic,
    /// A named family that is installed.
    Installed,
    /// No listed family is installed; drawing uses the serif fallback that
    /// `usvg` and browsers apply to unmatched families.
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GenericFamily {
    Serif,
    SansSerif,
    Cursive,
    Fantasy,
    Monospace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FamilyName {
    Generic(GenericFamily),
    Named(String),
}

struct FontCatalog {
    /// Every name a face answers to, mapped to that face's primary family.
    names: BTreeMap<String, String>,
    display_families: Vec<String>,
    serif: Option<String>,
    sans_serif: Option<String>,
    cursive: Option<String>,
    fantasy: Option<String>,
    monospace: Option<String>,
}

impl FontCatalog {
    fn new(database: &fontdb::Database) -> Self {
        let names = installed_family_names(database);
        let mut display_families = names
            .values()
            .filter(|family| !family.starts_with('.') && !family.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>();
        display_families.sort_by(|left, right| {
            left.to_lowercase()
                .cmp(&right.to_lowercase())
                .then_with(|| left.cmp(right))
        });
        display_families.dedup();
        let generic = |family: fontdb::Family<'_>| {
            let name = database.family_name(&family);
            names.contains_key(name).then(|| name.to_owned())
        };
        Self {
            serif: generic(fontdb::Family::Serif),
            sans_serif: generic(fontdb::Family::SansSerif),
            cursive: generic(fontdb::Family::Cursive),
            fantasy: generic(fontdb::Family::Fantasy),
            monospace: generic(fontdb::Family::Monospace),
            display_families,
            names,
        }
    }

    fn generic(&self, generic: GenericFamily) -> Option<&String> {
        match generic {
            GenericFamily::Serif => self.serif.as_ref(),
            GenericFamily::SansSerif => self.sans_serif.as_ref(),
            GenericFamily::Cursive => self.cursive.as_ref(),
            GenericFamily::Fantasy => self.fantasy.as_ref(),
            GenericFamily::Monospace => self.monospace.as_ref(),
        }
    }

    /// Mirror `usvg`'s font selection: take the first listed family that is
    /// installed, then fall back to the serif generic.
    fn resolve(&self, family: &str) -> FontResolution {
        let families = if family.len() > MAX_RESOLVED_FAMILY_BYTES {
            Vec::new()
        } else {
            parse_family_list(family)
        };
        for family in families {
            match family {
                FamilyName::Generic(generic) => {
                    if let Some(name) = self.generic(generic) {
                        return FontResolution {
                            family: Some(name.clone()),
                            status: FontResolutionStatus::Generic,
                        };
                    }
                }
                FamilyName::Named(name) => {
                    if self.names.contains_key(&name) {
                        return FontResolution {
                            family: Some(name),
                            status: FontResolutionStatus::Installed,
                        };
                    }
                }
            }
        }
        FontResolution {
            family: self.serif.clone(),
            status: FontResolutionStatus::Missing,
        }
    }
}

/// Parse a CSS `font-family` list the way SVG renderers read the attribute.
///
/// Unquoted entries collapse internal whitespace and match generic keywords
/// case-insensitively; quoted entries are always literal names. `system-ui`
/// is treated as sans-serif because the SVG exporter emits it with a
/// sans-serif fallback.
fn parse_family_list(value: &str) -> Vec<FamilyName> {
    let mut families = Vec::new();
    let mut characters = value.chars().peekable();
    loop {
        while characters.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(&first) = characters.peek() else {
            break;
        };
        if first == '"' || first == '\'' {
            characters.next();
            let mut name = String::new();
            let mut closed = false;
            while let Some(character) = characters.next() {
                if character == first {
                    closed = true;
                    break;
                }
                if character == '\\' {
                    if let Some(escaped) = characters.next() {
                        name.push(escaped);
                    }
                    continue;
                }
                name.push(character);
            }
            // CSS drops the whole declaration when an entry is malformed.
            while characters.next_if(|c| c.is_whitespace()).is_some() {}
            if !closed || !matches!(characters.next(), None | Some(',')) {
                return Vec::new();
            }
            if !name.is_empty() {
                families.push(FamilyName::Named(name));
            }
            continue;
        }

        let mut raw = String::new();
        for character in characters.by_ref() {
            if character == ',' {
                break;
            }
            raw.push(character);
        }
        let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() {
            continue;
        }
        let generic = match name.to_ascii_lowercase().as_str() {
            "serif" => Some(GenericFamily::Serif),
            "sans-serif" | "system-ui" => Some(GenericFamily::SansSerif),
            "cursive" => Some(GenericFamily::Cursive),
            "fantasy" => Some(GenericFamily::Fantasy),
            "monospace" => Some(GenericFamily::Monospace),
            _ => None,
        };
        families.push(match generic {
            Some(generic) => FamilyName::Generic(generic),
            None => FamilyName::Named(name),
        });
    }
    families
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database_with(families: &[&str]) -> fontdb::Database {
        let mut database = fontdb::Database::new();
        for family in families {
            database.push_face_info(fontdb::FaceInfo {
                id: fontdb::ID::dummy(),
                source: fontdb::Source::Binary(Arc::new(Vec::<u8>::new())),
                index: 0,
                families: vec![((*family).to_owned(), fontdb::Language::English_UnitedStates)],
                post_script_name: family.replace(' ', ""),
                style: fontdb::Style::Normal,
                weight: fontdb::Weight::NORMAL,
                stretch: fontdb::Stretch::Normal,
                monospaced: false,
            });
        }
        database
    }

    fn catalog_of(families: &[&str]) -> FontCatalog {
        let mut database = database_with(families);
        configure_generic_families(&mut database);
        FontCatalog::new(&database)
    }

    #[test]
    fn family_list_follows_css_quoting_and_generic_keywords() {
        assert_eq!(
            parse_family_list(r#"  Helvetica   Neue , "Sans-Serif", SANS-SERIF,'A\'B' "#),
            vec![
                FamilyName::Named("Helvetica Neue".into()),
                FamilyName::Named("Sans-Serif".into()),
                FamilyName::Generic(GenericFamily::SansSerif),
                FamilyName::Named("A'B".into()),
            ]
        );
        assert_eq!(
            parse_family_list("system-ui"),
            vec![FamilyName::Generic(GenericFamily::SansSerif)]
        );
        assert!(parse_family_list("'unterminated").is_empty());
        assert!(parse_family_list("'Name' trailing").is_empty());
        assert!(parse_family_list(" , ,").is_empty());
    }

    #[test]
    fn generics_use_first_installed_platform_candidate() {
        let sans = SANS_SERIF_CANDIDATES.last().unwrap();
        let serif = SERIF_CANDIDATES.last().unwrap();
        let catalog = catalog_of(&[sans, serif, "Zzz Display"]);

        let resolved = catalog.resolve("sans-serif");
        assert_eq!(resolved.family.as_deref(), Some(*sans));
        assert_eq!(resolved.status, FontResolutionStatus::Generic);
        assert_eq!(catalog.resolve("serif").family.as_deref(), Some(*serif));
        // No monospace candidate is installed, so monospace borrows sans-serif
        // instead of naming an uninstalled family.
        assert_eq!(catalog.resolve("monospace").family.as_deref(), Some(*sans));
    }

    #[test]
    fn generics_never_name_uninstalled_families() {
        let catalog = catalog_of(&["Only Custom Face"]);
        for generic in ["sans-serif", "serif", "monospace", "cursive", "system-ui"] {
            assert_eq!(
                catalog.resolve(generic).family.as_deref(),
                Some("Only Custom Face"),
                "{generic}"
            );
        }
        assert_eq!(
            catalog.resolve("anything").family.as_deref(),
            Some("Only Custom Face")
        );

        // With no platform candidate installed, the bundled Inter wins over
        // an alphabetically earlier family.
        let catalog = catalog_of(&["Aardvark Display", UI_FONT_FAMILY]);
        assert_eq!(
            catalog.resolve("sans-serif").family.as_deref(),
            Some(UI_FONT_FAMILY)
        );

        let empty = catalog_of(&[]);
        assert_eq!(empty.resolve("sans-serif").family, None);
        assert_eq!(
            empty.resolve("sans-serif").status,
            FontResolutionStatus::Missing
        );
    }

    #[test]
    fn named_families_resolve_exactly_and_missing_ones_use_serif_like_usvg() {
        let sans = SANS_SERIF_CANDIDATES[0];
        let serif = SERIF_CANDIDATES[0];
        let catalog = catalog_of(&[sans, serif, "Inter"]);

        let installed = catalog.resolve("  Inter ");
        assert_eq!(installed.family.as_deref(), Some("Inter"));
        assert_eq!(installed.status, FontResolutionStatus::Installed);

        // usvg's family match is case-sensitive, so the resolver must be too.
        let wrong_case = catalog.resolve("inter");
        assert_eq!(wrong_case.family.as_deref(), Some(serif));
        assert_eq!(wrong_case.status, FontResolutionStatus::Missing);

        let fallback_list = catalog.resolve("Not Installed, Inter, serif");
        assert_eq!(fallback_list.family.as_deref(), Some("Inter"));
        assert_eq!(fallback_list.status, FontResolutionStatus::Installed);

        let generic_fallback = catalog.resolve("Not Installed, sans-serif");
        assert_eq!(generic_fallback.family.as_deref(), Some(sans));
        assert_eq!(generic_fallback.status, FontResolutionStatus::Generic);

        let oversized = "x".repeat(MAX_RESOLVED_FAMILY_BYTES + 1);
        assert_eq!(
            catalog.resolve(&oversized).status,
            FontResolutionStatus::Missing
        );
    }

    #[test]
    fn bundled_ui_font_provides_the_interface_weights() {
        let mut database = fontdb::Database::new();
        load_bundled_fonts_if_missing(&mut database);
        let mut weights = database
            .faces()
            .filter(|face| {
                face.families
                    .iter()
                    .any(|(family, _)| family == UI_FONT_FAMILY)
            })
            .map(|face| face.weight.0)
            .collect::<Vec<_>>();
        weights.sort_unstable();
        assert_eq!(weights, [400, 500, 600, 700]);

        // A request for 650 (used by the interface) must resolve to a real
        // bundled face rather than a fallback family.
        let face = database
            .query(&fontdb::Query {
                families: &[fontdb::Family::Name(UI_FONT_FAMILY)],
                weight: fontdb::Weight(650),
                ..Default::default()
            })
            .and_then(|id| database.face(id))
            .expect("bundled Inter resolves");
        assert_eq!(face.weight.0, 700);
    }

    #[test]
    fn installed_inter_is_not_duplicated() {
        let mut database = database_with(&[UI_FONT_FAMILY]);
        assert!(!load_bundled_fonts_if_missing(&mut database));
        assert_eq!(database.len(), 1);

        let mut database = database_with(&["DejaVu Sans"]);
        assert!(load_bundled_fonts_if_missing(&mut database));
        assert_eq!(database.len(), 5);
        assert!(bundled_fonts_for_gpui(&[UI_FONT_FAMILY.to_owned()]).is_empty());
        assert_eq!(bundled_fonts_for_gpui(&["DejaVu Sans".to_owned()]).len(), 4);
    }

    #[test]
    fn interface_and_document_text_can_use_inter_on_this_machine() {
        assert_eq!(ui_font_family(), "Inter");
        assert!(installed_font_families()
            .iter()
            .any(|family| family == "Inter"));
        let resolution = resolve_document_font_family("Inter");
        assert_eq!(resolution.status, FontResolutionStatus::Installed);
        // Only report "bundled" when the system copy is absent.
        let mut system = fontdb::Database::new();
        system.load_system_fonts();
        let system_inter = system
            .faces()
            .any(|face| face.families.iter().any(|(family, _)| family == "Inter"));
        assert_eq!(is_bundled_font_family("Inter"), !system_inter);
        assert!(!is_bundled_font_family("DejaVu Sans"));
        assert!(SYSTEM_FONT_CATALOG
            .names
            .contains_key(ui_monospace_font_family()));
    }

    #[test]
    fn picker_lists_primary_names_sorted_without_hidden_faces() {
        let catalog = catalog_of(&["beta", "Alpha", ".Hidden UI", "Gamma", "Alpha"]);
        assert_eq!(catalog.display_families, vec!["Alpha", "beta", "Gamma"]);
    }

    fn render_text(family_attribute: &str) -> Vec<u8> {
        let svg = format!(
            concat!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="240" height="48">"#,
                r#"<text x="4" y="36" font-size="32" font-family="{}">Rag Wig 01</text></svg>"#
            ),
            family_attribute
        );
        crate::canvas::render_svg_pixmap(svg.as_bytes(), 240, 48)
            .expect("text SVG renders")
            .take()
    }

    /// Differential check through the real export renderer: drawing the
    /// stored family and drawing the family the canvas resolves it to must
    /// produce identical pixels, including for a family that is not installed.
    #[test]
    fn resolved_family_renders_identically_to_usvg_export_selection() {
        let mut requests = vec![
            "sans-serif".to_owned(),
            "serif".to_owned(),
            "monospace".to_owned(),
            "system-ui".to_owned(),
            "Strek Missing Family 7f3a".to_owned(),
        ];
        requests.extend(installed_font_families().iter().take(3).cloned());
        let mut compared = 0;
        for request in requests {
            let resolution = resolve_document_font_family(&request);
            let Some(resolved) = resolution.family else {
                // Nothing to draw with on a machine without fonts.
                assert!(installed_font_families().is_empty());
                continue;
            };
            let exported_request = if request == "system-ui" {
                "system-ui, sans-serif".to_owned()
            } else {
                request.clone()
            };
            let stored = render_text(&exported_request);
            assert!(
                stored.chunks_exact(4).any(|pixel| pixel[3] > 0),
                "{request} drew no glyphs"
            );
            assert!(
                stored == render_text(&format!("'{resolved}'")),
                "{request} resolved to {resolved}, which usvg does not select"
            );
            compared += 1;
        }
        assert!(installed_font_families().is_empty() || compared >= 5);

        // A plausible wrong resolver (sans fallback for missing names, as GPUI
        // does) must be distinguishable when serif and sans differ.
        let serif = resolve_document_font_family("serif").family;
        let sans = resolve_document_font_family("sans-serif").family;
        if let (Some(serif), Some(sans)) = (serif, sans) {
            if serif != sans {
                assert!(
                    render_text("Strek Missing Family 7f3a") != render_text(&format!("'{sans}'"))
                );
            }
        }
    }
}
