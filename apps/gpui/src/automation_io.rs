//! Blocking file and encoding work prepared by the automation controller.

use std::path::PathBuf;

use editor_core::ArtworkSnapshot;

use crate::{automation, document_io, export};

pub(crate) enum AutomationIoOperation {
    Open {
        path: PathBuf,
        revision: u64,
    },
    Save {
        path: PathBuf,
        document: Box<editor_core::Document>,
        revision: u64,
    },
    ExportToPath {
        path: PathBuf,
        format: export::ExportFormat,
        snapshot: ArtworkSnapshot,
    },
    Encode {
        format: automation::ArtifactFormat,
        export_format: export::ExportFormat,
        snapshot: ArtworkSnapshot,
        max_bytes: usize,
    },
}

pub(crate) enum AutomationIoSuccess {
    Opened {
        path: PathBuf,
        revision: u64,
        document: Box<document_io::OpenedDocument>,
    },
    Saved {
        path: PathBuf,
        revision: u64,
    },
    Exported {
        path: PathBuf,
    },
    Encoded {
        format: automation::ArtifactFormat,
        bytes: Vec<u8>,
    },
}

pub(crate) fn execute(operation: AutomationIoOperation) -> Result<AutomationIoSuccess, String> {
    match operation {
        AutomationIoOperation::Open { path, revision } => {
            let document =
                Box::new(document_io::read_document(&path).map_err(|error| error.to_string())?);
            Ok(AutomationIoSuccess::Opened {
                path,
                revision,
                document,
            })
        }
        AutomationIoOperation::Save {
            path,
            document,
            revision,
        } => {
            let json =
                document_io::serialize_document(&document).map_err(|error| error.to_string())?;
            document_io::write_document(&path, &json).map_err(|error| error.to_string())?;
            Ok(AutomationIoSuccess::Saved { path, revision })
        }
        AutomationIoOperation::ExportToPath {
            path,
            format,
            snapshot,
        } => {
            export::write_export(&path, format, &snapshot).map_err(|error| error.to_string())?;
            Ok(AutomationIoSuccess::Exported { path })
        }
        AutomationIoOperation::Encode {
            format,
            export_format,
            snapshot,
            max_bytes,
        } => {
            let bytes = export::encode_artwork(export_format, &snapshot)
                .map_err(|error| error.to_string())?;
            if bytes.len() > max_bytes {
                return Err(format!(
                    "encoded artifact is {} bytes; inline limit is {max_bytes} bytes, so export it to a path instead",
                    bytes.len()
                ));
            }
            Ok(AutomationIoSuccess::Encoded { format, bytes })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn save_and_open_operations_round_trip_outside_the_ui_controller() {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "strek-automation-io-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("drawing.strek.json");
        let document = editor_core::Document::new();

        let saved = execute(AutomationIoOperation::Save {
            path: path.clone(),
            document: Box::new(document),
            revision: 7,
        })
        .unwrap();
        assert!(matches!(
            saved,
            AutomationIoSuccess::Saved { revision: 7, .. }
        ));

        let opened = execute(AutomationIoOperation::Open { path, revision: 9 }).unwrap();
        let AutomationIoSuccess::Opened {
            revision, document, ..
        } = opened
        else {
            panic!("open operation returned the wrong success variant");
        };
        assert_eq!(revision, 9);
        assert!(matches!(*document, document_io::OpenedDocument::Native(_)));

        fs::remove_dir_all(directory).unwrap();
    }
}
