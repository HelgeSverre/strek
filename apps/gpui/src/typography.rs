//! Cross-platform font families used by the application chrome.

use std::sync::{Arc, LazyLock};

#[cfg(target_os = "macos")]
pub(crate) const MONOSPACE_FONT_FAMILY: &str = "Menlo";

#[cfg(target_os = "windows")]
pub(crate) const MONOSPACE_FONT_FAMILY: &str = "Consolas";

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) const MONOSPACE_FONT_FAMILY: &str = "DejaVu Sans Mono";

static SYSTEM_FONT_DATABASE: LazyLock<Arc<resvg::usvg::fontdb::Database>> =
    LazyLock::new(|| Arc::new(build_system_font_database()));

static EMPTY_FONT_DATABASE: LazyLock<Arc<resvg::usvg::fontdb::Database>> =
    LazyLock::new(|| Arc::new(resvg::usvg::fontdb::Database::new()));

fn build_system_font_database() -> resvg::usvg::fontdb::Database {
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

pub(crate) fn system_font_database() -> Arc<resvg::usvg::fontdb::Database> {
    Arc::clone(&SYSTEM_FONT_DATABASE)
}

pub(crate) fn empty_font_database() -> Arc<resvg::usvg::fontdb::Database> {
    Arc::clone(&EMPTY_FONT_DATABASE)
}
