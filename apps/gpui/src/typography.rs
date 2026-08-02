//! Cross-platform font families used by the application chrome.

#[cfg(target_os = "macos")]
pub(crate) const MONOSPACE_FONT_FAMILY: &str = "Menlo";

#[cfg(target_os = "windows")]
pub(crate) const MONOSPACE_FONT_FAMILY: &str = "Consolas";

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) const MONOSPACE_FONT_FAMILY: &str = "DejaVu Sans Mono";

pub(crate) fn system_font_database() -> resvg::usvg::fontdb::Database {
    let mut database = resvg::usvg::fontdb::Database::new();
    database.load_system_fonts();

    #[cfg(target_os = "linux")]
    {
        database.set_serif_family("DejaVu Serif");
        database.set_sans_serif_family("DejaVu Sans");
        database.set_cursive_family("DejaVu Sans");
        database.set_fantasy_family("DejaVu Sans");
        database.set_monospace_family(MONOSPACE_FONT_FAMILY);
    }

    database
}
