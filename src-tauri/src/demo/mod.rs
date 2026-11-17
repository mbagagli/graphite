//! Prepare the public demo in its own application-owned source partition.

mod archive;
pub(crate) mod catalog;
pub use archive::{archive_info, DemoArchiveInfo};
#[cfg(test)]
mod migration_tests;
#[cfg(test)]
mod tests;
pub(crate) mod text;

#[cfg(test)]
use std::fs;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(test)]
use std::time::Duration;

use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::database::ResearchDatabase;
use crate::error::{AppError, AppResult};
use crate::model::WorkspacePayload;

use catalog::{manifest, Paper, LIBRARY_ID};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DemoProgress {
    pub phase: String,
    pub completed: usize,
    pub total: usize,
    pub bytes: u64,
    pub total_bytes: u64,
    pub paper_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DemoDocumentStatus {
    pub paper_id: String,
    pub document_id: String,
    pub pages: u32,
    pub available: bool,
    pub text_indexed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DemoStatus {
    pub pdf_count: usize,
    pub total_pdfs: usize,
    pub total_bytes: u64,
    pub documents: Vec<DemoDocumentStatus>,
    pub busy: bool,
    pub progress: Option<DemoProgress>,
}

type ProgressCallback = Arc<dyn Fn(DemoProgress) + Send + Sync>;

#[derive(Clone)]
pub struct DemoCoordinator {
    database: ResearchDatabase,
    root: PathBuf,
    guard: Arc<Mutex<()>>,
    progress: Arc<Mutex<Option<DemoProgress>>>,
    cancelled: Arc<AtomicBool>,
    source_guard: Arc<tokio::sync::Mutex<()>>,
}

impl DemoCoordinator {
    pub fn new(
        database: ResearchDatabase,
        root: PathBuf,
        source_guard: Arc<tokio::sync::Mutex<()>>,
    ) -> Self {
        Self {
            database,
            root,
            source_guard,
            guard: Arc::new(Mutex::new(())),
            progress: Arc::new(Mutex::new(None)),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn activate(&self, demo: bool) -> AppResult<WorkspacePayload> {
        let _guard = self.guard.try_lock().map_err(|_| {
            failure("Wait for demo preparation to finish before switching libraries.")
        })?;
        let _source_guard = self.source_guard.blocking_lock();
        if demo {
            catalog::register(&self.database)?;
        }
        catalog::activate(&self.database, demo)?;
        self.database.load_workspace()
    }

    pub fn status(&self) -> AppResult<DemoStatus> {
        let catalog = manifest()?;
        let connection = self.database.connect()?;
        let mut documents = Vec::new();
        for paper in &catalog.papers {
            let document_id: Option<String> = connection
                .query_row(
                    "SELECT d.id FROM documents d JOIN research_items i ON i.id=d.research_item_id
                 WHERE d.library_id=?1 AND i.zotero_key=?2",
                    params![LIBRARY_ID, paper.id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(document_id) = document_id {
                let available = self
                    .root
                    .join(format!("{}.pdf", paper.id))
                    .metadata()
                    .is_ok_and(|m| m.is_file() && m.len() == paper.pdf_bytes);
                let text_indexed: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM document_texts WHERE document_id=?1 AND is_active=1)", [&document_id], |r|r.get(0))?;
                documents.push(DemoDocumentStatus {
                    paper_id: paper.id.clone(),
                    document_id,
                    pages: paper.pdf_pages,
                    available,
                    text_indexed,
                });
            }
        }
        Ok(DemoStatus {
            pdf_count: documents.iter().filter(|d| d.available).count(),
            total_pdfs: catalog.papers.len(),
            total_bytes: catalog.papers.iter().map(|p| p.pdf_bytes).sum(),
            documents,
            busy: self.guard.try_lock().is_err(),
            progress: self
                .progress
                .lock()
                .map_err(|_| failure("Demo progress is unavailable."))?
                .clone(),
        })
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    fn check_cancelled(&self) -> AppResult<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(failure(
                "Demo preparation cancelled. Verified PDFs were kept.",
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn acquire(
        &self,
        folder: Option<&Path>,
        on_progress: ProgressCallback,
    ) -> AppResult<DemoStatus> {
        let _guard = self
            .guard
            .try_lock()
            .map_err(|_| failure("Demo preparation is already running."))?;
        let _source_guard = self.source_guard.blocking_lock();
        self.require_active()?;
        self.cancelled.store(false, Ordering::Relaxed);
        fs::create_dir_all(&self.root).map_err(io_failure)?;
        let catalog = manifest()?;
        let total_bytes = catalog.papers.iter().map(|p| p.pdf_bytes).sum();
        let folder = folder
            .map(|path| path.canonicalize().map_err(io_failure))
            .transpose()?;
        let pdf_folder = folder.as_ref().map(|path| {
            if path.join("pdfs").is_dir() {
                path.join("pdfs")
            } else {
                path.clone()
            }
        });
        if let (Some(selected), Some(pdfs)) = (&folder, &pdf_folder) {
            if !pdfs
                .canonicalize()
                .map_err(io_failure)?
                .starts_with(selected)
            {
                return Err(failure(
                    "The PDF folder must remain inside the selected demo folder.",
                ));
            }
        }
        let client = if folder.is_none() {
            Some(
                reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(120))
                    .redirect(reqwest::redirect::Policy::custom(|attempt| {
                        if attempt.previous().len() < 5 && catalog::publisher_url(attempt.url()) {
                            attempt.follow()
                        } else {
                            attempt.error("The publisher redirected outside approved public hosts.")
                        }
                    }))
                    .build()
                    .map_err(|_| failure("Could not initialize the publisher download client."))?,
            )
        } else {
            None
        };
        let mut completed_bytes = 0;
        let result = (|| {
            for (index, paper) in catalog.papers.iter().enumerate() {
                self.check_cancelled()?;
                let progress = |bytes| {
                    let event = DemoProgress {
                        phase: if folder.is_some() {
                            "importing"
                        } else {
                            "downloading"
                        }
                        .into(),
                        completed: index,
                        total: catalog.papers.len(),
                        bytes: completed_bytes + bytes,
                        total_bytes,
                        paper_id: Some(paper.id.clone()),
                    };
                    if let Ok(mut current) = self.progress.lock() {
                        *current = Some(event.clone());
                    }
                    on_progress(event);
                };
                let destination = self.root.join(format!("{}.pdf", paper.id));
                if destination.exists() && validate_file(paper, &destination).is_ok() {
                    progress(paper.pdf_bytes);
                } else {
                    let bytes = if let Some(folder) = &pdf_folder {
                        let path = folder
                            .join(format!("{}.pdf", paper.id))
                            .canonicalize()
                            .map_err(|_| {
                                failure(&format!(
                                    "Missing PDF for {} in the selected demo folder.",
                                    paper.id
                                ))
                            })?;
                        if !path.starts_with(folder.canonicalize().map_err(io_failure)?) {
                            return Err(failure(
                                "Demo PDFs must remain inside the selected folder.",
                            ));
                        }
                        bounded_read(
                            paper,
                            File::open(path).map_err(io_failure)?,
                            &progress,
                            &|| self.check_cancelled(),
                        )?
                    } else {
                        let response=client.as_ref().expect("download client").get(&paper.pdf_url).send()
                            .map_err(|_|failure(&format!("Download failed for {}. Retry preparation or import the offline archive.",paper.id)))?;
                        if !response.status().is_success() {
                            return Err(failure(&format!("Publisher could not provide {}. Retry later or use the offline archive.",paper.id)));
                        }
                        bounded_read(paper, response, &progress, &|| self.check_cancelled())?
                    };
                    self.store_verified_pdf(paper, &destination, &bytes)?;
                }
                self.register_cached_pdf(paper, &destination)?;
                completed_bytes += paper.pdf_bytes;
            }
            Ok(())
        })();
        let event = DemoProgress {
            phase: if self.cancelled.load(Ordering::Relaxed) {
                "cancelled"
            } else if result.is_ok() {
                "ready"
            } else {
                "failed"
            }
            .into(),
            completed: self.status()?.pdf_count,
            total: catalog.papers.len(),
            bytes: completed_bytes,
            total_bytes,
            paper_id: None,
        };
        *self
            .progress
            .lock()
            .map_err(|_| failure("Demo progress is unavailable."))? = Some(event.clone());
        on_progress(event);
        result?;
        drop(_guard);
        self.status()
    }

    fn store_verified_pdf(&self, paper: &Paper, destination: &Path, bytes: &[u8]) -> AppResult<()> {
        validate_bytes(paper, bytes)?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.root).map_err(io_failure)?;
        temporary.write_all(bytes).map_err(io_failure)?;
        temporary.as_file().sync_all().map_err(io_failure)?;
        temporary.persist(destination).map_err(|_| {
            failure("Could not save the verified demo PDF. Check available storage.")
        })?;
        Ok(())
    }

    fn register_cached_pdf(&self, paper: &Paper, destination: &Path) -> AppResult<()> {
        let document_id = self.document_id(paper)?;
        let canonical = destination.canonicalize().map_err(io_failure)?;
        // Reader resolution binds page text to its validated file revision.
        // Re-registering identical bytes must not replace that revision.
        let registered: bool = self.database.connect()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM document_locators locator
             JOIN document_revisions revision
               ON revision.document_id=locator.document_id
              AND revision.revision_key=locator.revision_key
             WHERE locator.document_id=?1 AND locator.canonical_path=?2
               AND locator.file_size=?3 AND revision.content_hash=?4
               AND revision.revision_kind='artifact-content')",
            params![
                document_id,
                canonical.to_string_lossy(),
                paper.pdf_bytes as i64,
                paper.sha256,
            ],
            |row| row.get(0),
        )?;
        if registered {
            return Ok(());
        }
        self.database.store_document_locator(
            &document_id,
            &canonical,
            &format!("demo:{}", paper.sha256),
            &paper.sha256,
            paper.pdf_bytes,
            None,
        )
    }

    pub fn store_pages(&self, document_id: &str, pages: Vec<String>) -> AppResult<()> {
        let _guard = self
            .guard
            .try_lock()
            .map_err(|_| failure("Wait for demo PDF preparation to finish."))?;
        let _source_guard = self.source_guard.blocking_lock();
        self.require_active()?;
        let paper = self.paper_for_document(document_id)?;
        let path = self.root.join(format!("{}.pdf", paper.id));
        validate_file(&paper, &path)?;
        text::store_pages(&self.database, document_id, &paper, pages)
    }

    fn require_active(&self) -> AppResult<()> {
        if !catalog::is_active(&self.database)? {
            return Err(failure("Open the real demo before preparing its PDFs."));
        }
        Ok(())
    }

    fn document_id(&self, paper: &Paper) -> AppResult<String> {
        self.database.connect()?.query_row("SELECT d.id FROM documents d JOIN research_items i ON i.id=d.research_item_id WHERE d.library_id=?1 AND i.zotero_key=?2",params![LIBRARY_ID,paper.id],|r|r.get(0)).map_err(Into::into)
    }

    fn paper_for_document(&self, document_id: &str) -> AppResult<Paper> {
        let id: String=self.database.connect()?.query_row("SELECT i.zotero_key FROM documents d JOIN research_items i ON i.id=d.research_item_id WHERE d.id=?1 AND d.library_id=?2",params![document_id,LIBRARY_ID],|r|r.get(0))?;
        manifest()?
            .papers
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| failure("Unknown demo document."))
    }
}

fn bounded_read(
    paper: &Paper,
    mut input: impl Read,
    progress: &impl Fn(u64),
    check_cancelled: &impl Fn() -> AppResult<()>,
) -> AppResult<Vec<u8>> {
    let mut bytes = Vec::with_capacity(paper.pdf_bytes as usize);
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        check_cancelled()?;
        let count = input
            .read(&mut buffer)
            .map_err(|_| failure(&format!("Could not read the PDF for {}.", paper.id)))?;
        if count == 0 {
            break;
        }
        if bytes.len() as u64 + count as u64 > paper.pdf_bytes {
            return Err(failure(&format!(
                "PDF size differs from the verified catalogue for {}.",
                paper.id
            )));
        }
        bytes.extend_from_slice(&buffer[..count]);
        progress(bytes.len() as u64);
    }
    Ok(bytes)
}

pub(crate) fn validate_bytes(paper: &Paper, bytes: &[u8]) -> AppResult<()> {
    let digest: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if bytes.len() as u64 != paper.pdf_bytes
        || !bytes.starts_with(b"%PDF-")
        || digest != paper.sha256
    {
        return Err(failure(&format!(
            "PDF integrity check failed for {}. Use the unchanged publisher PDF.",
            paper.id
        )));
    }
    Ok(())
}

fn validate_file(paper: &Paper, path: &Path) -> AppResult<()> {
    let metadata = path.metadata().map_err(io_failure)?;
    if !metadata.is_file() || metadata.len() != paper.pdf_bytes {
        return Err(failure(&format!("Invalid demo PDF for {}.", paper.id)));
    }
    let bytes = bounded_read(
        paper,
        File::open(path).map_err(io_failure)?,
        &|_| {},
        &|| Ok(()),
    )?;
    validate_bytes(paper, &bytes)
}

fn io_failure(_error: std::io::Error) -> AppError {
    failure("The demo file could not be accessed. Check available storage and folder access.")
}
fn failure(message: &str) -> AppError {
    AppError::ApplicationData(message.into())
}
