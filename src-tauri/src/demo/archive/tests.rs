//! Exercise archive trust boundaries and the real public offline import.

use std::cell::Cell;
use std::io::{Cursor, Write};
use std::sync::Arc;

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use super::*;

fn tiny_catalog() -> Manifest {
    let mut catalog = manifest().expect("catalogue");
    catalog.papers.truncate(1);
    catalog.papers[0].pdf_bytes = 5;
    catalog
}

fn layout_zip(catalog: &Manifest, change: Option<(&str, &str)>) -> Vec<u8> {
    let mut output = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let root = format!("graphite-demo-{}/", catalog.id);
    for name in METADATA_FILES {
        let path = if change.is_some_and(|(original, _)| original == name) {
            change.expect("change").1.to_owned()
        } else {
            format!("{root}{name}")
        };
        output.start_file(path, options).expect("start metadata");
        output.write_all(b"test").expect("metadata");
    }
    for paper in &catalog.papers {
        output
            .start_file(format!("{root}pdfs/{}.pdf", paper.id), options)
            .expect("start PDF");
        output.write_all(b"%PDF-").expect("PDF boundary bytes");
    }
    output.finish().expect("ZIP").into_inner()
}

#[test]
fn validates_published_pin_and_only_version_specific_zenodo_urls() {
    let info = archive_info().expect("configuration");
    assert_eq!(info.manifest_id, manifest().expect("catalogue").id);
    assert_eq!(info.sha256.len(), 64);
    assert_eq!(
        info.url.as_deref(),
        Some("https://zenodo.org/records/24282715/files/graphite-demo-seismology-ml-v2-r1.zip")
    );
    let url = |value: &str| reqwest::Url::parse(value).expect("URL");
    assert!(record_file_url(
        &url(&format!(
            "https://zenodo.org/records/123/files/{}?download=1",
            info.file_name
        )),
        &info.file_name,
    ));
    for value in [
        "http://zenodo.org/records/123/files/a.zip",
        "https://user@zenodo.org/records/123/files/a.zip",
        "https://zenodo.org.evil.invalid/records/123/files/a.zip",
        "https://zenodo.org/records/latest/files/a.zip",
        "https://zenodo.org/records/123/files/a.zip?token=secret",
        "https://zenodo.org/records/123/files/a.zip#fragment",
    ] {
        assert!(!record_file_url(&url(value), "a.zip"));
    }
}

#[test]
#[ignore = "explicit opt-in: downloads the published 412 MB Zenodo archive"]
fn published_zenodo_archive_downloads_and_imports() {
    assert_eq!(
        std::env::var("GRAPHITE_TEST_ZENODO_DOWNLOAD").as_deref(),
        Ok("1")
    );
    let directory = tempfile::tempdir().expect("temporary folder");
    let database =
        crate::database::ResearchDatabase::initialize(directory.path().join("test.sqlite3"))
            .expect("database");
    let coordinator = DemoCoordinator::new(
        database,
        directory.path().join("pdfs"),
        Arc::new(tokio::sync::Mutex::new(())),
    );
    coordinator.activate(true).expect("activate demo");
    let result = coordinator
        .acquire_archive(None, Arc::new(|_| {}))
        .expect("download and import published ZIP");
    assert_eq!(result.pdf_count, 60);
    for paper in manifest().expect("catalogue").papers {
        validate_file(&paper, &coordinator.root.join(format!("{}.pdf", paper.id)))
            .expect("verified original");
    }
}

#[test]
fn bounded_stream_rejects_truncation_oversize_read_failure_and_cancellation() {
    let bytes = b"verified archive bytes";
    let mut output = Vec::new();
    let progress = Cell::new(0);
    assert_eq!(
        stream_checked(
            bytes.as_slice(),
            &mut output,
            bytes.len() as u64,
            &|| Ok(()),
            &|count| progress.set(count)
        )
        .expect("valid stream"),
        digest_hex(&Sha256::digest(bytes)),
    );
    assert_eq!(output, bytes);
    assert_eq!(progress.get(), bytes.len() as u64);
    assert!(stream_checked(bytes.as_slice(), std::io::sink(), 2, &|| Ok(()), &|_| {}).is_err());
    assert!(stream_checked(bytes.as_slice(), std::io::sink(), 100, &|| Ok(()), &|_| {}).is_err());
    assert!(stream_checked(
        bytes.as_slice(),
        std::io::sink(),
        bytes.len() as u64,
        &|| Err(failure("cancelled")),
        &|_| {}
    )
    .is_err());
    struct Interrupted;
    impl Read for Interrupted {
        fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "interrupted",
            ))
        }
    }
    assert!(stream_checked(Interrupted, std::io::sink(), 100, &|| Ok(()), &|_| {}).is_err());
}

#[test]
fn archive_layout_rejects_traversal_unexpected_entries_and_wrong_sizes() {
    let catalog = tiny_catalog();
    let valid = layout_zip(&catalog, None);
    validate_layout(
        &mut ZipArchive::new(Cursor::new(valid)).expect("ZIP"),
        &catalog,
    )
    .expect("layout");
    for name in [
        "../escape",
        "/absolute",
        "graphite-demo-seismology-ml-v2/extra.txt",
        "graphite-demo-seismology-ml-v2/../README.txt",
        "C:\\outside.txt",
    ] {
        let bytes = layout_zip(&catalog, Some(("README.txt", name)));
        assert!(validate_layout(
            &mut ZipArchive::new(Cursor::new(bytes)).expect("ZIP"),
            &catalog
        )
        .is_err());
    }
    let mut wrong_size = catalog.clone();
    wrong_size.papers[0].pdf_bytes += 1;
    let bytes = layout_zip(&catalog, None);
    assert!(validate_layout(
        &mut ZipArchive::new(Cursor::new(bytes)).expect("ZIP"),
        &wrong_size
    )
    .is_err());
}

#[test]
fn archive_layout_rejects_symlink_and_encrypted_entries() {
    let catalog = tiny_catalog();
    let mut bytes = layout_zip(&catalog, None);
    let central = bytes
        .windows(4)
        .position(|value| value == b"PK\x01\x02")
        .expect("central directory");
    // Declare the first metadata entry as a Unix symbolic link.
    bytes[central + 5] = 3;
    bytes[central + 38..central + 42].copy_from_slice(&(0o120777_u32 << 16).to_le_bytes());
    assert!(validate_layout(
        &mut ZipArchive::new(Cursor::new(bytes)).expect("ZIP"),
        &catalog
    )
    .is_err());
    let mut bytes = layout_zip(&catalog, None);
    let central = bytes
        .windows(4)
        .position(|value| value == b"PK\x01\x02")
        .expect("central directory");
    bytes[6] |= 1;
    bytes[central + 8] |= 1;
    assert!(validate_layout(
        &mut ZipArchive::new(Cursor::new(bytes)).expect("ZIP"),
        &catalog
    )
    .is_err());
}

#[test]
fn downloaded_cache_hash_detects_corruption_and_cancellation() {
    let directory = tempfile::tempdir().expect("temporary folder");
    let database =
        crate::database::ResearchDatabase::initialize(directory.path().join("test.sqlite3"))
            .expect("database");
    let coordinator = DemoCoordinator::new(
        database,
        directory.path().join("pdfs"),
        Arc::new(tokio::sync::Mutex::new(())),
    );
    let path = directory.path().join("archive.zip");
    std::fs::write(&path, b"archive").expect("cached bytes");
    let mut info = archive_info().expect("configuration");
    info.bytes = 7;
    info.sha256 = digest_hex(&Sha256::digest(b"archive"));
    assert!(coordinator
        .valid_downloaded_archive(&path, &info)
        .expect("checksum"));
    std::fs::write(&path, b"changed").expect("changed bytes");
    assert!(!coordinator
        .valid_downloaded_archive(&path, &info)
        .expect("checksum mismatch"));
    coordinator.cancel();
    assert!(coordinator.valid_downloaded_archive(&path, &info).is_err());
}

#[test]
#[ignore = "requires GRAPHITE_TEST_DEMO_ARCHIVE pointing to the public 60-PDF ZIP"]
fn real_zip_import_resumes_preserves_source_and_reuses_cache_without_network() {
    let archive = std::path::PathBuf::from(
        std::env::var_os("GRAPHITE_TEST_DEMO_ARCHIVE").expect("public ZIP"),
    );
    let directory = tempfile::tempdir().expect("temporary folder");
    let database =
        crate::database::ResearchDatabase::initialize(directory.path().join("test.sqlite3"))
            .expect("database");
    let coordinator = DemoCoordinator::new(
        database.clone(),
        directory.path().join("pdfs"),
        Arc::new(tokio::sync::Mutex::new(())),
    );
    let original_bytes = std::fs::metadata(&archive).expect("ZIP metadata").len();
    let initial_hash = stream_checked(
        File::open(&archive).expect("source ZIP"),
        std::io::sink(),
        original_bytes,
        &|| Ok(()),
        &|_| {},
    )
    .expect("source checksum");
    coordinator.activate(true).expect("activate");
    let cancel = coordinator.clone();
    let interrupted = coordinator.acquire_archive(
        Some(&archive),
        Arc::new(move |event| {
            if event.phase == "Importing verified PDFs" && event.completed == 1 {
                cancel.cancel();
            }
        }),
    );
    assert!(interrupted.is_err());
    assert_eq!(coordinator.status().expect("partial status").pdf_count, 1);
    let complete = coordinator
        .acquire_archive(Some(&archive), Arc::new(|_| {}))
        .expect("resume ZIP");
    assert_eq!(complete.pdf_count, 60);
    for paper in manifest().expect("catalogue").papers {
        validate_file(&paper, &coordinator.root.join(format!("{}.pdf", paper.id)))
            .expect("verified original");
    }
    let cached = coordinator
        .acquire_archive(None, Arc::new(|_| {}))
        .expect("cached acquisition needs no network");
    assert_eq!(cached.pdf_count, 60);
    assert_eq!(
        initial_hash,
        stream_checked(
            File::open(&archive).expect("source ZIP"),
            std::io::sink(),
            original_bytes,
            &|| Ok(()),
            &|_| {}
        )
        .expect("source checksum")
    );
    assert!(
        database
            .load_workspace()
            .expect("workspace")
            .corpus
            .resolved_pdf_count
            >= 60
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("local download");
    let address = listener.local_addr().expect("address");
    let served_archive = archive.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("download connection");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        let mut request = [0_u8; 4096];
        let read = stream.read(&mut request).expect("request");
        assert!(read > 0);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {original_bytes}\r\nConnection: close\r\n\r\n"
        )
        .expect("headers");
        std::io::copy(
            &mut File::open(served_archive).expect("public ZIP"),
            &mut stream,
        )
        .expect("archive response");
    });
    let downloaded_database =
        crate::database::ResearchDatabase::initialize(directory.path().join("download.sqlite3"))
            .expect("download database");
    let downloaded = DemoCoordinator::new(
        downloaded_database,
        directory.path().join("download-pdfs"),
        Arc::new(tokio::sync::Mutex::new(())),
    );
    downloaded.activate(true).expect("activate downloaded demo");
    std::fs::create_dir_all(&downloaded.root).expect("download cache");
    let mut info = archive_info().expect("configuration");
    info.url = Some(format!("http://{address}/archive.zip"));
    info.bytes = original_bytes;
    info.sha256 = initial_hash;
    let received = downloaded.root.join(CACHED_ARCHIVE);
    let download_progress: ProgressCallback = Arc::new(|_| {});
    downloaded
        .download_archive(&received, &info, &download_progress)
        .expect("bounded verified HTTP download");
    server.join().expect("download server completed");
    assert_eq!(
        downloaded
            .acquire_archive(Some(&received), Arc::new(|_| {}))
            .expect("downloaded ZIP import")
            .pdf_count,
        60
    );
    for paper in manifest().expect("catalogue").papers {
        validate_file(&paper, &downloaded.root.join(format!("{}.pdf", paper.id)))
            .expect("same downloaded PDF");
    }
}

#[test]
fn download_checks_headers_digest_interruption_and_cancellation_before_publish() {
    use std::net::TcpListener;
    use std::thread;

    let directory = tempfile::tempdir().expect("temporary folder");
    let database =
        crate::database::ResearchDatabase::initialize(directory.path().join("download.sqlite3"))
            .expect("database");
    let coordinator = DemoCoordinator::new(
        database,
        directory.path().join("pdfs"),
        Arc::new(tokio::sync::Mutex::new(())),
    );
    std::fs::create_dir_all(&coordinator.root).expect("cache directory");
    let body = b"bounded download test";
    for (label, status, declared_size, sent_body, cancel, success) in [
        ("valid", "200 OK", body.len(), body.as_slice(), false, true),
        (
            "bad status",
            "503 Unavailable",
            body.len(),
            body.as_slice(),
            false,
            false,
        ),
        (
            "bad length",
            "200 OK",
            body.len() + 1,
            body.as_slice(),
            false,
            false,
        ),
        (
            "bad hash",
            "200 OK",
            body.len(),
            b"modified download test".as_slice(),
            false,
            false,
        ),
        (
            "interrupted",
            "200 OK",
            body.len(),
            &body[..3],
            false,
            false,
        ),
        (
            "cancelled",
            "200 OK",
            body.len(),
            body.as_slice(),
            true,
            false,
        ),
    ] {
        coordinator.cancelled.store(false, Ordering::Relaxed);
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback server");
        let address = listener.local_addr().expect("address");
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {declared_size}\r\nConnection: close\r\n\r\n"
        );
        let sent_body = sent_body.to_vec();
        let server = thread::spawn(move || {
            let (mut connection, _) = listener.accept().expect("connection");
            connection
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("timeout");
            let mut request = [0_u8; 4096];
            let read = connection.read(&mut request).expect("request");
            assert!(read > 0);
            assert!(String::from_utf8_lossy(&request[..read])
                .contains(concat!("user-agent: Graphite/", env!("CARGO_PKG_VERSION"))));
            let _ = connection.write_all(response.as_bytes());
            let _ = connection.write_all(&sent_body);
        });
        let mut info = archive_info().expect("configuration");
        // Only this private test path supplies a loopback server. Production
        // obtains its URL exclusively from the validated bundled descriptor.
        info.url = Some(format!("http://{address}/archive.zip"));
        info.bytes = body.len() as u64;
        info.sha256 = digest_hex(&Sha256::digest(body));
        let destination = coordinator.root.join(format!("{label}.zip"));
        let cancellation = coordinator.clone();
        let progress: ProgressCallback = Arc::new(move |event| {
            if cancel && event.bytes > 0 {
                cancellation.cancel();
            }
        });
        let result = coordinator.download_archive(&destination, &info, &progress);
        server.join().expect("server completed");
        assert_eq!(result.is_ok(), success, "{label}: {result:?}");
        assert_eq!(destination.exists(), success, "{label}");
        if success {
            assert_eq!(std::fs::read(&destination).expect("downloaded bytes"), body);
        }
    }
}

#[test]
fn cached_locator_registration_keeps_the_readers_revision_binding() {
    let directory = tempfile::tempdir().expect("temporary folder");
    let database =
        crate::database::ResearchDatabase::initialize(directory.path().join("locator.sqlite3"))
            .expect("database");
    let coordinator = DemoCoordinator::new(
        database.clone(),
        directory.path().join("pdfs"),
        Arc::new(tokio::sync::Mutex::new(())),
    );
    coordinator.activate(true).expect("demo");
    std::fs::create_dir_all(&coordinator.root).expect("cache");
    let paper = manifest().expect("catalogue").papers.remove(0);
    let path = coordinator.root.join(format!("{}.pdf", paper.id));
    // This test exercises locator registration, after the caller's checksum
    // validation; PDF validation is covered by the acquisition tests.
    std::fs::write(&path, b"locator boundary input").expect("path");
    let document_id = coordinator.document_id(&paper).expect("document");
    database
        .store_document_locator(
            &document_id,
            &path.canonicalize().expect("canonical path"),
            "reader-revision",
            &paper.sha256,
            paper.pdf_bytes,
            Some(1234),
        )
        .expect("reader locator");
    coordinator
        .register_cached_pdf(&paper, &path)
        .expect("register cached file");
    let (revision, modified): (String, i64) = database
        .connect()
        .expect("connection")
        .query_row(
            "SELECT revision_key,modified_ms FROM document_locators WHERE document_id=?1",
            [&document_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("locator");
    assert_eq!(revision, "reader-revision");
    assert_eq!(modified, 1234);
}
