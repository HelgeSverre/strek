//! Cross-platform font families used by the application chrome.

#[cfg(target_os = "macos")]
pub(crate) const MONOSPACE_FONT_FAMILY: &str = "Menlo";

#[cfg(target_os = "windows")]
pub(crate) const MONOSPACE_FONT_FAMILY: &str = "Consolas";

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) const MONOSPACE_FONT_FAMILY: &str = "DejaVu Sans Mono";
