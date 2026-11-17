//! Acquire the pinned demo archive without trusting archive paths or metadata.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::{CompressionMethod, ZipArchive};

use super::catalog::{manifest, Manifest};
use super::{failure, io_failure, validate_bytes, validate_file};
use super::{DemoCoordinator, DemoProgress, DemoStatus, ProgressCallback};
use crate::error::AppResult;

const METADATA_LIMIT: u64 = 1024 * 1024;
const ARCHIVE_OVERHEAD_LIMIT: u64 = 8 * 1024 * 1024;
const METADATA_FILES: [&str; 5] = [
    "manifest.json",
    "RIGHTS.json",
    "ATTRIBUTION.txt",
    "README.txt",
    "SHA256SUMS",
];
const CACHED_ARCHIVE: &str = "verified-demo-library.zip";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DemoArchiveInfo {
    pub manifest_id: String,
    pub file_name: String,
    pub url: Option<String>,
    pub sha256: String,
    pub bytes: u64,
}

/// Return the bundled release identity, including an unpublished URL state.
pub fn archive_info() -> AppResult<DemoArchiveInfo> {
    let info: DemoArchiveInfo =
        serde_json::from_str(include_str!("../../../demo/distribution.json"))?;
    let catalog = manifest()?;
    let expected_name = format!("graphite-demo-{}-r1.zip", catalog.id);
    if info.manifest_id != catalog.id
        || info.file_name != expected_name
        || info.sha256.len() != 64
        || !info.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        || info.bytes == 0
        || info.bytes > archive_size_limit(&catalog)
        || info.url.as_ref().is_some_and(|value| {
            reqwest::Url::parse(value)
                .map(|url| !record_file_url(&url, &info.file_name))
                .unwrap_or(true)
        })
    {
        return Err(failure(
            "The bundled demo download configuration is invalid.",
        ));
    }
    Ok(info)
}

fn archive_size_limit(catalog: &Manifest) -> u64 {
    catalog
        .papers
        .iter()
        .map(|paper| paper.pdf_bytes)
        .sum::<u64>()
        + ARCHIVE_OVERHEAD_LIMIT
}

fn public_zenodo_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("zenodo.org")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
}

fn record_file_url(url: &reqwest::Url, file_name: &str) -> bool {
    let components = url.path().split('/').collect::<Vec<_>>();
    public_zenodo_url(url)
        && components.len() == 5
        && components[1] == "records"
        && !components[2].is_empty()
        && components[2].bytes().all(|byte| byte.is_ascii_digit())
        && components[3] == "files"
        && components[4] == file_name
        && matches!(url.query(), None | Some("download=1"))
}

impl DemoCoordinator {
    /// Import a local ZIP, or acquire the exact published ZIP on demand.
    pub fn acquire_archive(
        &self,
        selected: Option<&Path>,
        on_progress: ProgressCallback,
    ) -> AppResult<DemoStatus> {
        let guard = self
            .guard
            .try_lock()
            .map_err(|_| failure("Demo preparation is already running."))?;
        let _source_guard = self.source_guard.blocking_lock();
        self.require_active()?;
        self.cancelled.store(false, Ordering::Relaxed);
        fs::create_dir_all(&self.root).map_err(io_failure)?;
        let catalog = manifest()?;
        let result = self.acquire_archive_inner(selected, &catalog, &on_progress);
        let status = self.status()?;
        self.report_archive_progress(
            DemoProgress {
                phase: if self.cancelled.load(Ordering::Relaxed) {
                    "cancelled"
                } else if result.is_ok() {
                    "ready"
                } else {
                    "failed"
                }
                .into(),
                completed: status.pdf_count,
                total: status.total_pdfs,
                bytes: 0,
                total_bytes: 0,
                paper_id: None,
            },
            &on_progress,
        );
        result?;
        drop(guard);
        self.status()
    }

    fn acquire_archive_inner(
        &self,
        selected: Option<&Path>,
        catalog: &Manifest,
        on_progress: &ProgressCallback,
    ) -> AppResult<()> {
        // A completed cache must remain usable before publication or offline.
        if selected.is_none() {
            let mut valid = true;
            for (index, paper) in catalog.papers.iter().enumerate() {
                self.check_cancelled()?;
                let path = self.root.join(format!("{}.pdf", paper.id));
                if validate_file(paper, &path).is_err() {
                    valid = false;
                    break;
                }
                self.register_cached_pdf(paper, &path)?;
                self.report_archive_progress(
                    DemoProgress {
                        phase: "Checking saved PDFs".into(),
                        completed: index + 1,
                        total: catalog.papers.len(),
                        bytes: 0,
                        total_bytes: 0,
                        paper_id: Some(paper.id.clone()),
                    },
                    on_progress,
                );
            }
            if valid {
                return Ok(());
            }
        }
        let cached = self.root.join(CACHED_ARCHIVE);
        let path = if let Some(path) = selected {
            path.to_path_buf()
        } else {
            let info = archive_info()?;
            if !self.valid_downloaded_archive(&cached, &info)? {
                self.download_archive(&cached, &info, on_progress)?;
            }
            cached.clone()
        };
        let file = File::open(&path).map_err(io_failure)?;
        let size = file.metadata().map_err(io_failure)?.len();
        if size > archive_size_limit(catalog) {
            return Err(failure(
                "The selected ZIP exceeds the demo archive size limit.",
            ));
        }
        let mut archive = ZipArchive::new(file)
            .map_err(|_| failure("The demo ZIP is damaged or unsupported. Download it again."))?;
        validate_layout(&mut archive, catalog)?;
        validate_metadata(&mut archive, catalog)?;
        for (index, paper) in catalog.papers.iter().enumerate() {
            self.check_cancelled()?;
            let destination = self.root.join(format!("{}.pdf", paper.id));
            let name = format!("graphite-demo-{}/pdfs/{}.pdf", catalog.id, paper.id);
            let entry = archive.by_name(&name).map_err(zip_failure)?;
            let bytes = super::bounded_read(paper, entry, &|_| {}, &|| self.check_cancelled())?;
            validate_bytes(paper, &bytes)?;
            if validate_file(paper, &destination).is_err() {
                self.store_verified_pdf(paper, &destination, &bytes)?;
            }
            self.register_cached_pdf(paper, &destination)?;
            self.report_archive_progress(
                DemoProgress {
                    phase: "Importing verified PDFs".into(),
                    completed: index + 1,
                    total: catalog.papers.len(),
                    bytes: 0,
                    total_bytes: 0,
                    paper_id: Some(paper.id.clone()),
                },
                on_progress,
            );
        }
        // Keep a verified download only while it is needed for resumption.
        if selected.is_none() && cached.exists() {
            fs::remove_file(cached).map_err(io_failure)?;
        }
        Ok(())
    }

    fn valid_downloaded_archive(&self, path: &Path, info: &DemoArchiveInfo) -> AppResult<bool> {
        let Ok(file) = File::open(path) else {
            return Ok(false);
        };
        if file.metadata().map_err(io_failure)?.len() != info.bytes {
            return Ok(false);
        }
        let digest = stream_checked(
            file,
            std::io::sink(),
            info.bytes,
            &|| self.check_cancelled(),
            &|_| {},
        )?;
        Ok(digest == info.sha256)
    }

    fn download_archive(
        &self,
        destination: &Path,
        info: &DemoArchiveInfo,
        on_progress: &ProgressCallback,
    ) -> AppResult<()> {
        let url = info.url.as_ref().ok_or_else(|| {
            failure(
                "The demo archive has not been published yet. Choose Already downloaded? to use the provided ZIP.",
            )
        })?;
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("Graphite/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(20))
            // The blocking client applies this timeout to each body read.
            // It bounds stalled reads without timing the whole large ZIP.
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() < 3 && public_zenodo_url(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("The demo archive redirected outside its approved host.")
                }
            }))
            .build()
            .map_err(|_| failure("Could not start the demo archive download."))?;
        let response = client.get(url).send().map_err(|_| {
            failure("The demo download was interrupted. Check your connection and retry.")
        })?;
        if !response.status().is_success() {
            return Err(failure(&format!(
                "Zenodo could not provide the demo archive (HTTP {}). Retry later or use a downloaded ZIP.",
                response.status().as_u16()
            )));
        }
        if response
            .content_length()
            .is_some_and(|bytes| bytes != info.bytes)
        {
            return Err(failure(
                "The demo archive size does not match this Graphite version.",
            ));
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&self.root).map_err(io_failure)?;
        let digest = stream_checked(
            response,
            temporary.as_file_mut(),
            info.bytes,
            &|| self.check_cancelled(),
            &|bytes| {
                self.report_archive_progress(
                    DemoProgress {
                        phase: "downloading-archive".into(),
                        completed: 0,
                        total: 0,
                        bytes,
                        total_bytes: info.bytes,
                        paper_id: None,
                    },
                    on_progress,
                );
            },
        )?;
        if digest != info.sha256 {
            return Err(failure(
                "The downloaded demo ZIP failed its integrity check. Retry the download.",
            ));
        }
        temporary.as_file().sync_all().map_err(io_failure)?;
        temporary.persist(destination).map_err(|_| {
            failure("Could not save the verified archive. Check available storage.")
        })?;
        Ok(())
    }

    fn report_archive_progress(&self, event: DemoProgress, on_progress: &ProgressCallback) {
        if let Ok(mut progress) = self.progress.lock() {
            *progress = Some(event.clone());
        }
        on_progress(event);
    }
}

/// Stream bounded bytes and report progress without retaining the ZIP in RAM.
fn stream_checked(
    mut input: impl Read,
    mut output: impl Write,
    expected_bytes: u64,
    check_cancelled: &impl Fn() -> AppResult<()>,
    on_progress: &impl Fn(u64),
) -> AppResult<String> {
    let mut hasher = Sha256::new();
    let mut count = 0;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        check_cancelled()?;
        let read = input.read(&mut buffer).map_err(|_| {
            failure("The demo archive could not be read completely. Retry the download or select the ZIP again.")
        })?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > expected_bytes {
            return Err(failure("The demo archive exceeds its expected size."));
        }
        output.write_all(&buffer[..read]).map_err(io_failure)?;
        hasher.update(&buffer[..read]);
        on_progress(count);
    }
    if count != expected_bytes {
        return Err(failure(
            "The demo archive is incomplete. Download it again.",
        ));
    }
    Ok(digest_hex(&hasher.finalize()))
}

fn validate_layout<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    catalog: &Manifest,
) -> AppResult<()> {
    if archive.has_duplicate_names() || archive.len() != catalog.papers.len() + METADATA_FILES.len()
    {
        return Err(failure(
            "The ZIP does not contain exactly this demo's PDFs and supporting files.",
        ));
    }
    let root = format!("graphite-demo-{}/", catalog.id);
    let mut expected = BTreeMap::new();
    for paper in &catalog.papers {
        expected.insert(format!("{root}pdfs/{}.pdf", paper.id), paper.pdf_bytes);
    }
    for name in METADATA_FILES {
        expected.insert(format!("{root}{name}"), METADATA_LIMIT);
    }
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(zip_failure)?;
        let name =
            std::str::from_utf8(entry.name_raw()).map_err(|_| failure("Invalid ZIP filename."))?;
        let maximum = expected
            .get(name)
            .ok_or_else(|| failure("The ZIP contains an unexpected or unsafe path."))?;
        let pdf = name.ends_with(".pdf");
        if entry.enclosed_name().is_none()
            || entry.is_dir()
            || entry.is_symlink()
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000)
            || entry.encrypted()
            || entry.compression() != CompressionMethod::Stored
            || entry.size() > *maximum
            || (pdf && entry.size() != *maximum)
        {
            return Err(failure("The ZIP contains unsupported, unsafe or incorrectly sized content. Use the original demo ZIP."));
        }
    }
    Ok(())
}

fn validate_metadata<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    catalog: &Manifest,
) -> AppResult<()> {
    let root = format!("graphite-demo-{}/", catalog.id);
    let mut metadata = BTreeMap::new();
    for name in METADATA_FILES {
        let entry = archive
            .by_name(&format!("{root}{name}"))
            .map_err(zip_failure)?;
        let mut bytes = Vec::new();
        entry
            .take(METADATA_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| failure("The demo ZIP metadata failed its integrity check."))?;
        if bytes.len() as u64 > METADATA_LIMIT {
            return Err(failure("The demo ZIP metadata is too large."));
        }
        metadata.insert(name, bytes);
    }
    let expected_manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../../demo/manifest.json"))?;
    let expected_rights: serde_json::Value =
        serde_json::from_str(include_str!("../../../demo/rights-review.json"))?;
    let actual_manifest: serde_json::Value = serde_json::from_slice(&metadata["manifest.json"])?;
    let actual_rights: serde_json::Value = serde_json::from_slice(&metadata["RIGHTS.json"])?;
    if actual_manifest != expected_manifest || actual_rights != expected_rights {
        return Err(failure("The ZIP belongs to a different or modified demo collection. Use this version's archive."));
    }
    let mut expected_checksums = BTreeMap::new();
    for paper in &catalog.papers {
        expected_checksums.insert(format!("pdfs/{}.pdf", paper.id), paper.sha256.clone());
    }
    for (name, bytes) in &metadata {
        if *name != "SHA256SUMS" {
            expected_checksums.insert((*name).to_owned(), digest_hex(&Sha256::digest(bytes)));
        }
    }
    let checksums = std::str::from_utf8(&metadata["SHA256SUMS"])
        .map_err(|_| failure("The demo checksum list is invalid."))?;
    let mut actual_checksums = BTreeMap::new();
    for line in checksums.lines() {
        let (checksum, path) = line
            .split_once("  ")
            .ok_or_else(|| failure("The demo checksum list is invalid."))?;
        if actual_checksums
            .insert(path.to_owned(), checksum.to_owned())
            .is_some()
        {
            return Err(failure("The demo checksum list contains duplicate paths."));
        }
    }
    if actual_checksums != expected_checksums {
        return Err(failure(
            "The ZIP checksum list does not match the verified demo files.",
        ));
    }
    Ok(())
}

fn digest_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn zip_failure(_error: zip::result::ZipError) -> crate::error::AppError {
    failure("The demo ZIP is damaged, encrypted or unsupported. Use the original downloaded ZIP.")
}

#[cfg(test)]
mod tests;
