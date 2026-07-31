//! Native document persistence and recent-file bookkeeping.

use std::env;
use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use editor_core::{Document, DocumentLoadError};

const DOCUMENT_EXTENSION: &str = "strek.json";
const MAX_RECENT_FILES: usize = 8;
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Failure while loading or persisting an editor document.
#[derive(Debug)]
pub enum DocumentIoError {
    Read {
        path: PathBuf,
        source: io::Error,
    },
    Decode {
        path: PathBuf,
        source: DocumentLoadError,
    },
    Encode {
        source: serde_json::Error,
    },
    Write {
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for DocumentIoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(formatter, "could not read {}: {source}", path.display())
            }
            Self::Decode { path, source } => {
                write!(formatter, "could not open {}: {source}", path.display())
            }
            Self::Encode { source } => write!(formatter, "could not serialize document: {source}"),
            Self::Write { path, source } => {
                write!(formatter, "could not save {}: {source}", path.display())
            }
        }
    }
}

impl Error for DocumentIoError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { source, .. } | Self::Write { source, .. } => Some(source),
            Self::Decode { source, .. } => Some(source),
            Self::Encode { source } => Some(source),
        }
    }
}

/// Read and validate a document from disk.
pub fn read_document(path: &Path) -> Result<Document, DocumentIoError> {
    let json = fs::read_to_string(path).map_err(|source| DocumentIoError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    Document::from_json(&json).map_err(|source| DocumentIoError::Decode {
        path: path.to_path_buf(),
        source,
    })
}

/// Serialize a document before handing its snapshot to a background writer.
pub fn serialize_document(document: &Document) -> Result<String, DocumentIoError> {
    document
        .to_json()
        .map_err(|source| DocumentIoError::Encode { source })
}

/// Persist a serialized document snapshot using a same-directory temporary file.
pub fn write_document(path: &Path, json: &str) -> Result<(), DocumentIoError> {
    write_atomic(path, json.as_bytes()).map_err(|source| DocumentIoError::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// Give extensionless save destinations the editor's native extension.
pub fn normalize_document_path(path: PathBuf) -> PathBuf {
    if path.extension().is_some() {
        return path;
    }

    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return path;
    };
    path.with_file_name(format!("{file_name}.{DOCUMENT_EXTENSION}"))
}

/// Most-recently-used document paths.
#[derive(Debug, Clone, Default)]
pub struct RecentFiles {
    paths: Vec<PathBuf>,
}

impl RecentFiles {
    /// Load recent files, ignoring a missing or malformed preferences file.
    pub fn load() -> Self {
        let Some(path) = recent_files_path() else {
            return Self::default();
        };
        let Ok(json) = fs::read_to_string(path) else {
            return Self::default();
        };
        let Ok(paths) = serde_json::from_str::<Vec<PathBuf>>(&json) else {
            return Self::default();
        };

        Self {
            paths: paths
                .into_iter()
                .filter(|path| path.is_file())
                .take(MAX_RECENT_FILES)
                .collect(),
        }
    }

    /// Recent paths in most-recent-first order.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Move a path to the front and persist the updated list.
    pub fn record(&mut self, path: &Path) {
        let normalized = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.paths.retain(|candidate| candidate != &normalized);
        self.paths.insert(0, normalized);
        self.paths.truncate(MAX_RECENT_FILES);
        if let Err(error) = self.persist() {
            log::warn!("could not persist recent files: {error}");
        }
    }

    /// Remove a path that failed to open.
    pub fn remove(&mut self, path: &Path) {
        self.paths.retain(|candidate| candidate != path);
        if let Err(error) = self.persist() {
            log::warn!("could not persist recent files: {error}");
        }
    }

    fn persist(&self) -> io::Result<()> {
        let Some(path) = recent_files_path() else {
            return Ok(());
        };
        let json = serde_json::to_vec_pretty(&self.paths).map_err(io::Error::other)?;
        write_atomic(&path, &json)
    }
}

fn recent_files_path() -> Option<PathBuf> {
    app_config_directory().map(|root| root.join("recent-files.json"))
}

/// Per-user directory for editor preferences and recent state.
pub(crate) fn app_config_directory() -> Option<PathBuf> {
    config_root().map(|root| root.join("strek"))
}

#[cfg(target_os = "macos")]
fn config_root() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library").join("Application Support"))
}

#[cfg(target_os = "windows")]
fn config_root() -> Option<PathBuf> {
    env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn config_root() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".config"))
        })
}

pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "save destination has no parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "save destination has no valid file name",
            )
        })?;
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary_path = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        sequence
    ));

    let result = (|| {
        let mut file = create_temporary_file(&temporary_path)?;
        file.write_all(contents)?;
        file.sync_all()?;
        replace_file(&temporary_path, path)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn create_temporary_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(not(target_os = "windows"))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(target_os = "windows")]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{Node, PathData};

    fn temporary_test_directory(name: &str) -> PathBuf {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        env::temp_dir().join(format!("strek-{name}-{}-{sequence}", std::process::id()))
    }

    #[test]
    fn document_round_trip_uses_atomic_writer() {
        let directory = temporary_test_directory("document-io");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("drawing.strek.json");
        let mut document = Document::new();
        document
            .add_child(
                document.root,
                Node::shape("Rectangle", PathData::rect(0.0, 0.0, 10.0, 20.0)),
            )
            .unwrap();

        let json = serialize_document(&document).unwrap();
        write_document(&path, &json).unwrap();
        let restored = read_document(&path).unwrap();

        assert_eq!(restored.descendants(restored.root).count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn extensionless_paths_receive_native_extension() {
        assert_eq!(
            normalize_document_path(PathBuf::from("/tmp/drawing")),
            PathBuf::from("/tmp/drawing.strek.json")
        );
        assert_eq!(
            normalize_document_path(PathBuf::from("/tmp/drawing.json")),
            PathBuf::from("/tmp/drawing.json")
        );
    }
}
