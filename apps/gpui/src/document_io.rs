//! Native document persistence and recent-file bookkeeping.

use std::env;
use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use editor_core::{Document, DocumentLoadError, DocumentValidationError};

use crate::svg_import::{self, SvgImportError};

const DOCUMENT_EXTENSION: &str = "strek.json";
const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RECENT_FILES: usize = 8;
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Failure while loading or persisting an editor document.
#[derive(Debug)]
pub enum DocumentIoError {
    TooLarge {
        path: PathBuf,
        limit: u64,
    },
    Read {
        path: PathBuf,
        source: io::Error,
    },
    Decode {
        path: PathBuf,
        source: DocumentLoadError,
    },
    Import {
        path: PathBuf,
        source: SvgImportError,
    },
    Encode {
        source: serde_json::Error,
    },
    InvalidDocument {
        source: DocumentValidationError,
    },
    UnsafeImportDestination {
        source: PathBuf,
        destination: PathBuf,
    },
    Write {
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for DocumentIoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { path, limit } => write!(
                formatter,
                "document {} exceeds the {limit}-byte limit",
                path.display()
            ),
            Self::Read { path, source } => {
                write!(formatter, "could not read {}: {source}", path.display())
            }
            Self::Decode { path, source } => {
                write!(formatter, "could not open {}: {source}", path.display())
            }
            Self::Import { path, source } => {
                write!(formatter, "could not import {}: {source}", path.display())
            }
            Self::Encode { source } => write!(formatter, "could not serialize document: {source}"),
            Self::InvalidDocument { source } => {
                write!(formatter, "could not save invalid document: {source}")
            }
            Self::UnsafeImportDestination {
                source,
                destination,
            } => write!(
                formatter,
                "refusing to overwrite imported SVG {} through save destination {}; choose a different native document path",
                source.display(),
                destination.display()
            ),
            Self::Write { path, source } => {
                write!(formatter, "could not save {}: {source}", path.display())
            }
        }
    }
}

impl Error for DocumentIoError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::TooLarge { .. } => None,
            Self::Read { source, .. } | Self::Write { source, .. } => Some(source),
            Self::Decode { source, .. } => Some(source),
            Self::Import { source, .. } => Some(source),
            Self::Encode { source } => Some(source),
            Self::InvalidDocument { source } => Some(source),
            Self::UnsafeImportDestination { .. } => None,
        }
    }
}

/// A native document or a foreign document imported into the native model.
pub(crate) enum OpenedDocument {
    Native(Document),
    ImportedSvg(Document),
}

/// Read and validate a native document, or import a supported SVG.
pub fn read_document(path: &Path) -> Result<OpenedDocument, DocumentIoError> {
    let file = File::open(path).map_err(|source| DocumentIoError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let metadata = file.metadata().map_err(|source| DocumentIoError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    ensure_document_size(path, metadata.len(), MAX_DOCUMENT_BYTES)?;

    let mut contents = String::new();
    file.take(MAX_DOCUMENT_BYTES + 1)
        .read_to_string(&mut contents)
        .map_err(|source| DocumentIoError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    ensure_document_size(path, contents.len() as u64, MAX_DOCUMENT_BYTES)?;

    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
    {
        let document_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Imported SVG");
        svg_import::import_svg(&contents, document_name)
            .map(OpenedDocument::ImportedSvg)
            .map_err(|source| DocumentIoError::Import {
                path: path.to_path_buf(),
                source,
            })
    } else {
        Document::from_json(&contents)
            .map(OpenedDocument::Native)
            .map_err(|source| DocumentIoError::Decode {
                path: path.to_path_buf(),
                source,
            })
    }
}

/// Serialize a document before handing its snapshot to a background writer.
pub fn serialize_document(document: &Document) -> Result<String, DocumentIoError> {
    let saved = document
        .to_validated_saved()
        .map_err(|source| DocumentIoError::InvalidDocument { source })?;
    serde_json::to_string_pretty(&saved).map_err(|source| DocumentIoError::Encode { source })
}

/// Persist a serialized document snapshot using a same-directory temporary file.
pub fn write_document(path: &Path, json: &str) -> Result<(), DocumentIoError> {
    write_document_with_limit(path, json, MAX_DOCUMENT_BYTES)
}

fn write_document_with_limit(path: &Path, json: &str, limit: u64) -> Result<(), DocumentIoError> {
    ensure_document_size(path, json.len() as u64, limit)?;
    write_atomic(path, json.as_bytes()).map_err(|source| DocumentIoError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn ensure_document_size(path: &Path, bytes: u64, limit: u64) -> Result<(), DocumentIoError> {
    if bytes > limit {
        return Err(DocumentIoError::TooLarge {
            path: path.to_path_buf(),
            limit,
        });
    }
    Ok(())
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

/// Force an imported document's save destination to the native extension and
/// reject aliases that would overwrite the source SVG.
pub fn normalize_imported_document_path(
    path: PathBuf,
    source: &Path,
) -> Result<PathBuf, DocumentIoError> {
    let is_native_path = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.to_ascii_lowercase()
                .ends_with(&format!(".{DOCUMENT_EXTENSION}"))
        });
    let destination = if is_native_path {
        path
    } else {
        path.with_extension(DOCUMENT_EXTENSION)
    };

    if paths_refer_to_same_file(source, &destination) {
        return Err(DocumentIoError::UnsafeImportDestination {
            source: source.to_path_buf(),
            destination,
        });
    }

    Ok(destination)
}

fn paths_refer_to_same_file(first: &Path, second: &Path) -> bool {
    if first == second {
        return true;
    }
    if matches!(
        (first.canonicalize(), second.canonicalize()),
        (Ok(first), Ok(second)) if first == second
    ) {
        return true;
    }

    #[cfg(windows)]
    {
        return matches!(
            (windows_file_identity(first), windows_file_identity(second)),
            (Ok(first), Ok(second)) if first == second
        );
    }

    #[cfg(not(windows))]
    {
        let (Ok(first), Ok(second)) = (first.metadata(), second.metadata()) else {
            return false;
        };
        same_file_metadata(&first, &second)
    }
}

#[cfg(unix)]
fn same_file_metadata(first: &fs::Metadata, second: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    first.dev() == second.dev() && first.ino() == second.ino()
}

#[cfg(windows)]
fn windows_file_identity(path: &Path) -> io::Result<(u32, u64)> {
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let file = File::open(path)?;
    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: `file` owns a valid handle for the duration of the call, and
    // `information` points to writable storage for the requested structure.
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: a successful `GetFileInformationByHandle` call initialized the structure.
    let information = unsafe { information.assume_init() };
    let file_index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Ok((information.dwVolumeSerialNumber, file_index))
}

#[cfg(not(any(unix, windows)))]
fn same_file_metadata(_: &fs::Metadata, _: &fs::Metadata) -> bool {
    false
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

    /// Move a path to the front of the in-memory list.
    pub fn record(&mut self, path: &Path) {
        record_recent_path(&mut self.paths, path);
    }

    /// Remove a path that failed to open from the in-memory list.
    pub fn remove(&mut self, path: &Path) {
        self.paths.retain(|candidate| candidate != path);
    }

    /// Persist the current snapshot. Call from a background executor.
    pub fn persist(&self) {
        if let Err(error) = self.write() {
            log::warn!("could not persist recent files: {error}");
        }
    }

    fn write(&self) -> io::Result<()> {
        let Some(path) = recent_files_path() else {
            return Ok(());
        };
        let json = serde_json::to_vec_pretty(&self.paths).map_err(io::Error::other)?;
        write_atomic(&path, &json)
    }
}

fn record_recent_path(paths: &mut Vec<PathBuf>, path: &Path) {
    paths.retain(|candidate| !paths_refer_to_same_file(candidate, path));
    paths.insert(0, path.to_path_buf());
    paths.truncate(MAX_RECENT_FILES);
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
        {
            let mut file = create_temporary_file(&temporary_path)?;
            file.write_all(contents)?;
            file.sync_all()?;
        }
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
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_WRITE_THROUGH, REPLACEFILE_WRITE_THROUGH,
    };

    let destination_exists = destination.exists();
    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();

    if destination_exists {
        // SAFETY: Both paths are valid, nul-terminated UTF-16 buffers that stay
        // alive for the duration of the call. The optional pointers are null.
        let replaced = unsafe {
            ReplaceFileW(
                destination_wide.as_ptr(),
                source_wide.as_ptr(),
                ptr::null(),
                REPLACEFILE_WRITE_THROUGH,
                ptr::null(),
                ptr::null(),
            )
        };
        if replaced == 0 {
            return Err(io::Error::last_os_error());
        }
    } else {
        // SAFETY: Both paths are valid, nul-terminated UTF-16 buffers that stay
        // alive for the duration of the call.
        let moved = unsafe {
            MoveFileExW(
                source_wide.as_ptr(),
                destination_wide.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        };
        if moved == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
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
        let OpenedDocument::Native(restored) = read_document(&path).unwrap() else {
            panic!("native document was imported as SVG");
        };

        assert_eq!(restored.descendants(restored.root).count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn atomic_writer_replaces_existing_contents() {
        let directory = temporary_test_directory("atomic-replace");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("preferences.json");
        fs::write(&path, b"old contents").unwrap();

        write_atomic(&path, b"new contents").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"new contents");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn failed_atomic_replace_preserves_the_existing_windows_file() {
        let directory = temporary_test_directory("atomic-replace-failure");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("preferences.json");
        fs::write(&path, b"old contents").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();

        assert!(write_atomic(&path, b"new contents").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old contents");

        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&path, permissions).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn svg_files_are_imported_as_editable_documents() {
        let directory = temporary_test_directory("svg-import");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("drawing.SVG");
        fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10">
                <rect id="box" width="20" height="10" fill="#f00"/>
            </svg>"##,
        )
        .unwrap();

        let OpenedDocument::ImportedSvg(document) = read_document(&path).unwrap() else {
            panic!("SVG document was decoded as a native document");
        };
        assert!(document
            .descendants(document.root)
            .filter_map(|id| document.get(id))
            .any(|node| node.name == "box"));

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

    #[test]
    fn imported_save_paths_always_use_the_native_extension() {
        let source = Path::new("/tmp/logo.svg");

        assert_eq!(
            normalize_imported_document_path(PathBuf::from("/tmp/logo.svg"), source).unwrap(),
            PathBuf::from("/tmp/logo.strek.json")
        );
        assert_eq!(
            normalize_imported_document_path(PathBuf::from("/tmp/logo.STREK.JSON"), source)
                .unwrap(),
            PathBuf::from("/tmp/logo.STREK.JSON")
        );
    }

    #[test]
    fn imported_saves_reject_existing_aliases_to_the_source() {
        let directory = temporary_test_directory("svg-save-alias");
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.svg");
        let destination = directory.join("drawing.strek.json");
        fs::write(&source, "<svg/>").unwrap();
        fs::hard_link(&source, &destination).unwrap();

        assert!(matches!(
            normalize_imported_document_path(destination, &source),
            Err(DocumentIoError::UnsafeImportDestination { .. })
        ));

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn recent_files_preserve_the_opened_alias_while_deduplicating_identity() {
        let directory = temporary_test_directory("recent-svg-alias");
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("source-without-extension");
        let alias = directory.join("source.svg");
        fs::write(&target, "<svg/>").unwrap();
        fs::hard_link(&target, &alias).unwrap();
        let mut paths = vec![target];

        record_recent_path(&mut paths, &alias);

        assert_eq!(paths, vec![alias]);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn oversized_documents_are_rejected_before_decoding() {
        let directory = temporary_test_directory("oversized-document");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("oversized.strek.json");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_DOCUMENT_BYTES + 1).unwrap();

        assert!(matches!(
            read_document(&path),
            Err(DocumentIoError::TooLarge { limit, .. }) if limit == MAX_DOCUMENT_BYTES
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn documents_that_exceed_load_limits_are_not_saved() {
        let directory = temporary_test_directory("oversized-save");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("oversized.strek.json");

        assert!(matches!(
            write_document_with_limit(&path, "12345", 4),
            Err(DocumentIoError::TooLarge { limit: 4, .. })
        ));
        assert!(!path.exists());

        let mut document = Document::new();
        document
            .add_child(
                document.root,
                Node::text("Oversized", "x".repeat(4 * 1024 * 1024 + 1)),
            )
            .unwrap();
        assert!(matches!(
            serialize_document(&document),
            Err(DocumentIoError::InvalidDocument {
                source: DocumentValidationError::TextTooLong { .. }
            })
        ));
        fs::remove_dir_all(directory).unwrap();
    }
}
