use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};
use tauri::http::{header, Method, Request, Response, StatusCode};
use tauri::UriSchemeResponder;

use crate::database::{DocumentResolutionContext, ResearchDatabase};
use crate::error::{AppError, AppResult};
use crate::model::DocumentOpenResult;
use crate::zotero::ZoteroReadClient;

const ALLOWED_ORIGINS: &[&str] = &[
    "http://127.0.0.1:1420",
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
];
const MAX_DOCUMENT_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone)]
pub struct DocumentCoordinator {
    database: ResearchDatabase,
    client: ZoteroReadClient,
}

impl DocumentCoordinator {
    pub fn new(database: ResearchDatabase) -> AppResult<Self> {
        Ok(Self {
            database,
            client: ZoteroReadClient::new()?,
        })
    }

    #[cfg(test)]
    pub fn with_client(database: ResearchDatabase, client: ZoteroReadClient) -> Self {
        Self { database, client }
    }

    pub async fn resolve(&self, document_id: &str) -> AppResult<DocumentOpenResult> {
        let context = self.database.document_resolution_context(document_id)?;
        if context.media_type != "application/pdf" {
            return Ok(unavailable_result(
                &context,
                "unsupported",
                "Only PDF attachments can be opened in the M2 reader",
            ));
        }
        if let Some(path) = &context.canonical_path {
            if let Ok(validated) = validate_pdf(path) {
                if context.provider_kind == "real-demo" {
                    let paper = crate::demo::catalog::manifest()?
                        .papers
                        .into_iter()
                        .find(|paper| format!("pdf:{}", paper.id) == context.attachment_key)
                        .ok_or_else(|| AppError::DocumentUnavailable("Unknown demo PDF.".into()))?;
                    if validated.content_hash != paper.sha256
                        || validated.file_size != paper.pdf_bytes
                    {
                        return Ok(unavailable_result(
                            &context,
                            "unavailable",
                            "The demo PDF changed. Import an unchanged verified PDF.",
                        ));
                    }
                }
                self.persist_validated(&context, &validated)?;
                return Ok(open_result(&context, &validated));
            }
        }
        if context.provider_kind == "real-demo" {
            return Ok(unavailable_result(
                &context,
                "unavailable",
                "Download the demo PDFs or import the offline demo folder first.",
            ));
        }
        let file_url = match self
            .client
            .file_view_url(&context.remote, &context.attachment_key, &context.server_id)
            .await
        {
            Ok(value) => value,
            Err(error) => {
                return Ok(unavailable_result(
                    &context,
                    "unavailable",
                    &format!("Zotero could not resolve this attachment: {error}"),
                ));
            }
        };
        let url = reqwest::Url::parse(&file_url).map_err(|error| {
            AppError::DocumentUnavailable(format!(
                "Zotero returned an invalid attachment URL: {error}"
            ))
        })?;
        if url.scheme() != "file" {
            return Err(AppError::DocumentUnavailable(
                "Zotero returned a non-file attachment URL".to_owned(),
            ));
        }
        let path = url.to_file_path().map_err(|_| {
            AppError::DocumentUnavailable(
                "Zotero returned a file URL that is invalid on this platform".to_owned(),
            )
        })?;
        let validated = validate_pdf(&path)?;
        self.persist_validated(&context, &validated)?;
        Ok(open_result(&context, &validated))
    }

    fn persist_validated(
        &self,
        context: &DocumentResolutionContext,
        validated: &ValidatedDocument,
    ) -> AppResult<()> {
        self.database.store_document_locator(
            &context.document_id,
            &validated.path,
            &validated.revision_key,
            &validated.content_hash,
            validated.file_size,
            validated.modified_ms,
        )
    }
}

struct ValidatedDocument {
    path: PathBuf,
    revision_key: String,
    content_hash: String,
    file_size: u64,
    modified_ms: Option<i64>,
}

fn validate_pdf(path: &Path) -> AppResult<ValidatedDocument> {
    let canonical = path.canonicalize().map_err(|error| {
        AppError::DocumentUnavailable(format!("attachment file is unavailable: {error}"))
    })?;
    let metadata = canonical.metadata().map_err(|error| {
        AppError::DocumentUnavailable(format!("attachment metadata is unavailable: {error}"))
    })?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(AppError::DocumentUnavailable(
            "attachment does not resolve to a non-empty regular file".to_owned(),
        ));
    }
    if metadata.len() > MAX_DOCUMENT_BYTES {
        return Err(AppError::DocumentUnavailable(format!(
            "attachment exceeds the M2 reader limit of {} MiB",
            MAX_DOCUMENT_BYTES / 1024 / 1024
        )));
    }
    let mut header_bytes = [0_u8; 5];
    File::open(&canonical)
        .and_then(|mut file| file.read_exact(&mut header_bytes))
        .map_err(|error| {
            AppError::DocumentUnavailable(format!("attachment header is unreadable: {error}"))
        })?;
    if &header_bytes != b"%PDF-" {
        return Err(AppError::DocumentUnavailable(
            "attachment does not have a valid PDF header".to_owned(),
        ));
    }
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64);
    let content_hash = file_content_hash(&canonical)?;
    let revision_key = revision_key(metadata.len(), modified_ms, &content_hash);
    Ok(ValidatedDocument {
        path: canonical,
        revision_key,
        content_hash,
        file_size: metadata.len(),
        modified_ms,
    })
}

fn file_content_hash(path: &Path) -> AppResult<String> {
    let file = File::open(path).map_err(|error| {
        AppError::DocumentUnavailable(format!("attachment content is unreadable: {error}"))
    })?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|error| {
            AppError::DocumentUnavailable(format!(
                "attachment content could not be hashed: {error}"
            ))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn revision_key(file_size: u64, modified_ms: Option<i64>, content_hash: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(file_size.to_le_bytes());
    hasher.update(modified_ms.unwrap_or_default().to_le_bytes());
    hasher.update(content_hash.as_bytes());
    let bytes = hasher.finalize();
    bytes[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn open_result(
    context: &DocumentResolutionContext,
    validated: &ValidatedDocument,
) -> DocumentOpenResult {
    DocumentOpenResult {
        document_id: context.document_id.clone(),
        status: "ready".to_owned(),
        media_type: context.media_type.clone(),
        content_length: Some(validated.file_size),
        revision_key: Some(validated.revision_key.clone()),
        message: None,
    }
}

fn unavailable_result(
    context: &DocumentResolutionContext,
    status: &str,
    message: &str,
) -> DocumentOpenResult {
    DocumentOpenResult {
        document_id: context.document_id.clone(),
        status: status.to_owned(),
        media_type: context.media_type.clone(),
        content_length: None,
        revision_key: context.revision_key.clone(),
        message: Some(message.to_owned()),
    }
}

pub fn respond_to_document_request(
    database: ResearchDatabase,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    std::thread::spawn(move || {
        let response = build_document_response(&database, &request).unwrap_or_else(|error| {
            Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(error.to_string().into_bytes())
                .expect("valid document error response")
        });
        responder.respond(response);
    });
}

fn build_document_response(
    database: &ResearchDatabase,
    request: &Request<Vec<u8>>,
) -> AppResult<Response<Vec<u8>>> {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    if let Some(origin) = origin {
        if !ALLOWED_ORIGINS.contains(&origin) {
            return Ok(Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Vec::new())
                .expect("valid forbidden response"));
        }
    }
    if request.method() == Method::OPTIONS {
        return Ok(base_response(StatusCode::NO_CONTENT, origin, Vec::new()));
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return Ok(base_response(
            StatusCode::METHOD_NOT_ALLOWED,
            None,
            Vec::new(),
        ));
    }
    let document_id = decode_document_id(request.uri().path())?;
    let stored_path = database.document_path(&document_id)?;
    let validated = validate_pdf(&stored_path)?;
    let expected_query = format!("revision={}", validated.revision_key);
    if request.uri().query() != Some(expected_query.as_str()) {
        return Err(AppError::DocumentUnavailable(
            "The PDF changed or its revision is missing. Reopen the paper to load the current file."
                .to_owned(),
        ));
    }
    if validated.path
        != stored_path.canonicalize().map_err(|error| {
            AppError::DocumentUnavailable(format!("document path changed: {error}"))
        })?
    {
        return Err(AppError::DocumentUnavailable(
            "document path no longer resolves to its validated target".to_owned(),
        ));
    }
    let full_length = validated.file_size;
    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(|value| parse_range(value, full_length))
        .transpose()?;
    let (status, start, end) = range
        .map(|(start, end)| (StatusCode::PARTIAL_CONTENT, start, end))
        .unwrap_or((StatusCode::OK, 0, full_length.saturating_sub(1)));
    let body = if request.method() == Method::HEAD {
        Vec::new()
    } else {
        let byte_count = end.saturating_sub(start) + 1;
        let mut file = File::open(&validated.path).map_err(|error| {
            AppError::DocumentUnavailable(format!("document became unreadable: {error}"))
        })?;
        file.seek(SeekFrom::Start(start)).map_err(|error| {
            AppError::DocumentUnavailable(format!("document range seek failed: {error}"))
        })?;
        let mut bytes = vec![0_u8; byte_count as usize];
        file.read_exact(&mut bytes).map_err(|error| {
            AppError::DocumentUnavailable(format!("document range read failed: {error}"))
        })?;
        bytes
    };
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/pdf")
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "private, no-store")
        .header(header::CONTENT_LENGTH, end.saturating_sub(start) + 1);
    if status == StatusCode::PARTIAL_CONTENT {
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{full_length}"),
        );
    }
    if let Some(origin) = origin {
        builder = builder.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    }
    Ok(builder.body(body).expect("valid document response"))
}

fn base_response(status: StatusCode, origin: Option<&str>, body: Vec<u8>) -> Response<Vec<u8>> {
    let mut builder = Response::builder()
        .status(status)
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, HEAD, OPTIONS")
        .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "Range")
        .header(header::ACCEPT_RANGES, "bytes");
    if let Some(origin) = origin {
        builder = builder.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    }
    builder.body(body).expect("valid base response")
}

fn decode_document_id(path: &str) -> AppResult<String> {
    let encoded = path.trim_start_matches('/');
    let mut output = Vec::with_capacity(encoded.len());
    let bytes = encoded.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = hex_value(bytes[index + 1]);
            let low = hex_value(bytes[index + 2]);
            if let (Some(high), Some(low)) = (high, low) {
                output.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    let document_id = String::from_utf8(output).map_err(|_| {
        AppError::DocumentUnavailable("document identifier is not valid UTF-8".to_owned())
    })?;
    if document_id.is_empty()
        || !document_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ":_-".contains(character))
    {
        return Err(AppError::DocumentUnavailable(
            "document identifier contains unsupported characters".to_owned(),
        ));
    }
    Ok(document_id)
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn parse_range(value: &str, full_length: u64) -> AppResult<(u64, u64)> {
    let bytes = value
        .strip_prefix("bytes=")
        .ok_or_else(|| AppError::DocumentUnavailable("document range must use bytes".to_owned()))?;
    if bytes.contains(',') || full_length == 0 {
        return Err(AppError::DocumentUnavailable(
            "multiple or empty document ranges are unsupported".to_owned(),
        ));
    }
    let (start_text, end_text) = bytes
        .split_once('-')
        .ok_or_else(|| AppError::DocumentUnavailable("document range is malformed".to_owned()))?;
    let (start, end) = if start_text.is_empty() {
        let suffix = end_text.parse::<u64>().map_err(|_| {
            AppError::DocumentUnavailable("document suffix range is malformed".to_owned())
        })?;
        let count = suffix.min(full_length);
        (full_length - count, full_length - 1)
    } else {
        let start = start_text.parse::<u64>().map_err(|_| {
            AppError::DocumentUnavailable("document range start is malformed".to_owned())
        })?;
        let end = if end_text.is_empty() {
            full_length - 1
        } else {
            end_text.parse::<u64>().map_err(|_| {
                AppError::DocumentUnavailable("document range end is malformed".to_owned())
            })?
        };
        (start, end.min(full_length - 1))
    };
    if start > end || start >= full_length {
        return Err(AppError::DocumentUnavailable(
            "document range is outside the file".to_owned(),
        ));
    }
    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use crate::database::ResearchDatabase;
    use crate::model::{LibraryDelta, RawObject, RemoteLibrary, SourceProbe};
    use crate::zotero::ZoteroReadClient;

    use super::{
        decode_document_id, parse_range, validate_pdf, DocumentCoordinator, MAX_DOCUMENT_BYTES,
    };

    #[test]
    fn decodes_only_opaque_document_identifiers() {
        assert_eq!(
            decode_document_id("/document%3Aitem_123").expect("document identifier"),
            "document:item_123"
        );
        assert!(decode_document_id("/../../etc/passwd").is_err());
    }

    #[test]
    fn revision_identity_includes_bytes_when_file_metadata_is_unchanged() {
        assert_ne!(
            super::revision_key(100, Some(123), "original-hash"),
            super::revision_key(100, Some(123), "replacement-hash"),
        );
    }

    #[test]
    fn parses_bounded_single_ranges() {
        assert_eq!(parse_range("bytes=0-99", 1000).expect("range"), (0, 99));
        assert_eq!(parse_range("bytes=-20", 1000).expect("suffix"), (980, 999));
        assert!(parse_range("bytes=1000-1001", 1000).is_err());
        assert!(parse_range("bytes=0-1,4-5", 1000).is_err());
    }

    #[test]
    fn rejects_malformed_and_oversized_pdf_files() {
        let malformed_path = std::env::temp_dir().join(format!(
            "graphite-malformed-{}-{}.pdf",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        File::create(&malformed_path)
            .and_then(|mut file| file.write_all(b"not a pdf"))
            .expect("malformed fixture");
        assert!(validate_pdf(&malformed_path).is_err());
        fs::remove_file(&malformed_path).expect("remove malformed fixture");

        let large_path = std::env::temp_dir().join(format!(
            "graphite-large-{}-{}.pdf",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let mut large_file = File::create(&large_path).expect("large fixture");
        large_file.write_all(b"%PDF-").expect("header");
        large_file
            .set_len(MAX_DOCUMENT_BYTES + 1)
            .expect("sparse fixture size");
        assert!(validate_pdf(&large_path).is_err());
        fs::remove_file(&large_path).expect("remove large fixture");
    }

    #[tokio::test]
    async fn resolves_a_zotero_file_url_and_reopens_it_from_the_cached_locator() {
        let fixture_path = std::env::temp_dir().join(format!(
            "graphite-valid-{}-{}.pdf",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        File::create(&fixture_path)
            .and_then(|mut file| file.write_all(b"%PDF-1.7\n%%EOF\n"))
            .expect("PDF fixture");
        let file_url = reqwest::Url::from_file_path(&fixture_path)
            .expect("file URL")
            .to_string();
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request");
            let mut bytes = [0_u8; 8_192];
            let length = stream.read(&mut bytes).expect("read");
            let request = String::from_utf8_lossy(&bytes[..length]);
            assert!(request.starts_with("GET "));
            assert!(request.contains("/items/PDF1/file/view/url"));
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\
                 Zotero-Server-ID: document-source\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{}",
                file_url.len(),
                file_url,
            );
            stream.write_all(response.as_bytes()).expect("response");
        });
        let (database, database_path, document_id) = prepared_document_database();
        let client = ZoteroReadClient::with_root(format!("http://{address}/api")).expect("client");
        let coordinator = DocumentCoordinator::with_client(database.clone(), client);
        let resolved = coordinator.resolve(&document_id).await.expect("resolve");
        assert_eq!(resolved.status, "ready");
        server.join().expect("server");

        let offline_client = ZoteroReadClient::with_root("http://127.0.0.1:9/api".to_owned())
            .expect("offline client");
        let offline_coordinator =
            DocumentCoordinator::with_client(database.clone(), offline_client);
        let offline = offline_coordinator
            .resolve(&document_id)
            .await
            .expect("cached resolve");
        assert_eq!(offline.status, "ready");

        let request = |revision: &str| {
            tauri::http::Request::builder()
                .uri(format!("/{document_id}?revision={revision}"))
                .header(tauri::http::header::RANGE, "bytes=0-7")
                .body(Vec::new())
                .expect("PDF range request")
        };
        let original_request = request(resolved.revision_key.as_deref().expect("revision"));
        let missing_revision = tauri::http::Request::builder()
            .uri(format!("/{document_id}"))
            .body(Vec::new())
            .expect("request without revision");
        assert!(super::build_document_response(&database, &missing_revision).is_err());
        assert_eq!(
            super::build_document_response(&database, &original_request)
                .expect("original PDF bytes")
                .body(),
            b"%PDF-1.7",
        );
        fs::write(&fixture_path, b"%PDF-1.6\n%%EOF\n").expect("replace same-size PDF");
        assert!(super::build_document_response(&database, &original_request).is_err());
        let reopened = offline_coordinator
            .resolve(&document_id)
            .await
            .expect("reopen changed PDF");
        let updated_request = request(reopened.revision_key.as_deref().expect("updated revision"));
        assert_eq!(
            super::build_document_response(&database, &updated_request)
                .expect("updated PDF bytes")
                .body(),
            b"%PDF-1.6",
        );
        assert!(super::build_document_response(&database, &original_request).is_err());
        fs::remove_file(fixture_path).expect("remove PDF fixture");
        fs::remove_file(database_path).expect("remove database");
    }

    fn prepared_document_database() -> (ResearchDatabase, std::path::PathBuf, String) {
        let path = std::env::temp_dir().join(format!(
            "graphite-document-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let database = ResearchDatabase::initialize(path.clone()).expect("database");
        let source = database
            .register_source(&SourceProbe {
                server_id: "document-source".to_owned(),
                api_version: "3".to_owned(),
                schema_version: None,
            })
            .expect("source");
        let library = database
            .register_libraries(
                &source.id,
                &[RemoteLibrary {
                    kind: "user".to_owned(),
                    zotero_id: "0".to_owned(),
                    name: "My Library".to_owned(),
                }],
            )
            .expect("library")
            .remove(0);
        let item = RawObject {
            key: "PAPER1".to_owned(),
            version: 1,
            object_type: "items".to_owned(),
            json: serde_json::json!({
                "key": "PAPER1", "version": 1,
                "data": {
                    "key": "PAPER1", "version": 1,
                    "itemType": "journalArticle", "title": "Paper",
                    "creators": [], "tags": [], "collections": []
                }
            }),
        };
        let attachment = RawObject {
            key: "PDF1".to_owned(),
            version: 1,
            object_type: "items".to_owned(),
            json: serde_json::json!({
                "key": "PDF1", "version": 1,
                "data": {
                    "key": "PDF1", "version": 1,
                    "itemType": "attachment", "parentItem": "PAPER1",
                    "title": "PDF", "contentType": "application/pdf",
                    "filename": "paper.pdf", "linkMode": "imported_file"
                }
            }),
        };
        database
            .apply_delta(
                &source.id,
                &library.id,
                &LibraryDelta {
                    target_version: 1,
                    objects: vec![item, attachment],
                    deleted_items: Vec::new(),
                    deleted_collections: Vec::new(),
                },
            )
            .expect("metadata");
        let workspace = database.load_workspace().expect("workspace");
        let document_id = workspace.snapshot.items[0].attachments[0]
            .document_id
            .clone();
        (database, path, document_id)
    }
}
