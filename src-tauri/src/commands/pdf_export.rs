//! Export edited PDF copies through a user-selected native save dialog.

use std::io::Write;
use std::path::{Path, PathBuf};

use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::AppState;

const MAX_EXPORT_BYTES: usize = 128 * 1024 * 1024;

#[tauri::command]
pub(crate) async fn export_pdf_copy(
    app: AppHandle,
    state: State<'_, AppState>,
    request: Request<'_>,
) -> Result<bool, String> {
    let bytes = match request.body() {
        InvokeBody::Raw(bytes) => bytes.clone(),
        _ => return Err("PDF export requires PDF bytes.".to_owned()),
    };
    validate_pdf_bytes(&bytes)?;
    let name = request
        .headers()
        .get("x-graphite-file-name")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("Graphite-edited.pdf");
    let name = export_name(name);
    let originals = state
        .database
        .source_document_paths()
        .map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let selected = app
            .dialog()
            .file()
            .set_title("Export edited PDF copy")
            .set_file_name(name)
            .add_filter("PDF document", &["pdf"])
            .blocking_save_file();
        let Some(selected) = selected else {
            return Ok(false);
        };
        let path = selected
            .as_path()
            .ok_or_else(|| "Choose a local destination for the PDF copy.".to_owned())?;
        write_copy(path, &bytes, &originals)?;
        Ok(true)
    })
    .await
    .map_err(|error| format!("The PDF export task failed: {error}"))?
}

fn validate_pdf_bytes(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_EXPORT_BYTES || !bytes.starts_with(b"%PDF-") {
        return Err("Choose a valid PDF of at most 128 MB to export.".to_owned());
    }
    let ending = &bytes[bytes.len().saturating_sub(1024)..];
    if !ending.windows(5).any(|part| part == b"%%EOF") {
        return Err("The PDF is incomplete and cannot be exported.".to_owned());
    }
    Ok(())
}

fn export_name(name: &str) -> String {
    let stem = name.trim_end_matches(".pdf");
    let cleaned: String = stem
        .chars()
        .take(120)
        .map(|character| {
            if character.is_ascii_alphanumeric() || " -_().".contains(character) {
                character
            } else {
                '_'
            }
        })
        .collect();
    format!("{}.pdf", cleaned.trim_matches([' ', '.']))
}

fn write_copy(path: &Path, bytes: &[u8], originals: &[PathBuf]) -> Result<(), String> {
    if let Ok(canonical) = path.canonicalize() {
        if originals.iter().any(|original| original == &canonical) {
            return Err("Choose a different filename. Graphite preserves source PDFs.".to_owned());
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| "Choose a folder for the exported PDF.".to_owned())?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("The destination folder could not be used: {error}"))?;
    temporary
        .write_all(bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|error| format!("The PDF copy could not be written: {error}"))?;
    // Atomic replacement also avoids writing through existing hard links.
    temporary
        .persist(path)
        .map_err(|error| format!("The PDF copy could not be saved: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_pdf_payloads_and_safe_suggested_names() {
        assert!(validate_pdf_bytes(b"%PDF-1.7\n%%EOF").is_ok());
        assert!(validate_pdf_bytes(b"not a PDF").is_err());
        assert!(validate_pdf_bytes(b"%PDF-1.7\nincomplete").is_err());
        assert_eq!(export_name("paper/../copy.pdf"), "paper_.._copy.pdf");
    }

    #[test]
    fn preserves_originals_and_atomically_exports_a_copy() {
        let directory = tempfile::tempdir().expect("directory");
        let source = directory.path().join("source.pdf");
        std::fs::write(&source, b"original").expect("source");
        let originals = vec![source.canonicalize().expect("canonical")];
        assert!(write_copy(&source, b"edited", &originals).is_err());
        let destination = directory.path().join("copy.pdf");
        std::fs::hard_link(&source, &destination).expect("hard link");
        write_copy(&destination, b"edited", &originals).expect("export");
        assert_eq!(std::fs::read(&source).expect("source"), b"original");
        assert_eq!(std::fs::read(&destination).expect("copy"), b"edited");
    }
}
