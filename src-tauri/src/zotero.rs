use std::collections::HashMap;
use std::time::Duration;

use reqwest::header::HeaderMap;
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::model::{
    FullTextInventory, FullTextRecord, RawObject, RemoteLibrary, SourceProbe, VersionMap,
};

const ZOTERO_ROOT: &str = "http://127.0.0.1:23119/api";
const API_VERSION_HEADER: &str = "zotero-api-version";
const SERVER_ID_HEADER: &str = "zotero-server-id";
const SCHEMA_VERSION_HEADER: &str = "zotero-schema-version";
const LIBRARY_VERSION_HEADER: &str = "last-modified-version";
const BATCH_SIZE: usize = 50;

#[derive(Clone)]
pub struct ZoteroReadClient {
    client: reqwest::Client,
    root: String,
}

struct ReadResponse {
    headers: HeaderMap,
    value: Value,
}

impl ZoteroReadClient {
    pub fn new() -> AppResult<Self> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            client,
            root: ZOTERO_ROOT.to_owned(),
        })
    }

    #[cfg(test)]
    pub fn with_root(root: String) -> AppResult<Self> {
        let mut client = Self::new()?;
        client.root = root;
        Ok(client)
    }

    pub async fn probe(&self) -> AppResult<SourceProbe> {
        let headers = self.read_headers("/", &[], None).await?;
        let server_id =
            header_text(&headers, SERVER_ID_HEADER).ok_or(AppError::MissingServerIdentity)?;
        let api_version =
            header_text(&headers, API_VERSION_HEADER).unwrap_or_else(|| "3".to_owned());

        Ok(SourceProbe {
            server_id,
            api_version,
            schema_version: header_text(&headers, SCHEMA_VERSION_HEADER),
        })
    }

    pub async fn discover_libraries(&self, probe: &SourceProbe) -> AppResult<Vec<RemoteLibrary>> {
        let mut libraries = vec![RemoteLibrary {
            kind: "user".to_owned(),
            zotero_id: "0".to_owned(),
            name: "My Library".to_owned(),
        }];
        let response = self
            .read_json(
                "/users/0/groups",
                &[("format", "json")],
                Some(&probe.server_id),
            )
            .await?;
        self.verify_source(&response.headers, &probe.server_id)?;

        let groups = response.value.as_array().ok_or_else(|| {
            AppError::InvalidZoteroResponse("group discovery did not return an array".to_owned())
        })?;
        for group in groups {
            let data = group.get("data").unwrap_or(group);
            let id = data
                .get("id")
                .and_then(Value::as_i64)
                .map(|value| value.to_string())
                .or_else(|| data.get("id").and_then(Value::as_str).map(str::to_owned));
            if let Some(zotero_id) = id {
                let name = data
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Zotero Group")
                    .to_owned();
                libraries.push(RemoteLibrary {
                    kind: "group".to_owned(),
                    zotero_id,
                    name,
                });
            }
        }
        Ok(libraries)
    }

    pub async fn item_inventory(
        &self,
        library: &RemoteLibrary,
        server_id: &str,
    ) -> AppResult<VersionMap> {
        self.object_inventory(library, "items", server_id).await
    }

    pub async fn collection_inventory(
        &self,
        library: &RemoteLibrary,
        server_id: &str,
    ) -> AppResult<VersionMap> {
        self.object_inventory(library, "collections", server_id)
            .await
    }

    pub async fn fetch_items(
        &self,
        library: &RemoteLibrary,
        keys: &[String],
        server_id: &str,
        expected_version: i64,
    ) -> AppResult<Vec<RawObject>> {
        self.fetch_objects(
            library,
            "items",
            "itemKey",
            keys,
            server_id,
            expected_version,
        )
        .await
    }

    pub async fn fetch_collections(
        &self,
        library: &RemoteLibrary,
        keys: &[String],
        server_id: &str,
        expected_version: i64,
    ) -> AppResult<Vec<RawObject>> {
        self.fetch_objects(
            library,
            "collections",
            "collectionKey",
            keys,
            server_id,
            expected_version,
        )
        .await
    }

    pub async fn fulltext_inventory(
        &self,
        library: &RemoteLibrary,
        since: i64,
        server_id: &str,
    ) -> AppResult<FullTextInventory> {
        let prefix = library.api_prefix();
        let path = format!("{prefix}/fulltext");
        let since_text = since.to_string();
        let response = self
            .read_json(&path, &[("since", since_text.as_str())], Some(server_id))
            .await?;
        self.verify_source(&response.headers, server_id)?;
        let map: HashMap<String, i64> =
            serde_json::from_value(response.value).map_err(|error| {
                AppError::InvalidZoteroResponse(format!(
                    "invalid Zotero full-text version map: {error}"
                ))
            })?;
        let mut versions: Vec<_> = map.into_iter().collect();
        versions.sort_by(|left, right| left.0.cmp(&right.0));
        let header_version = header_text(&response.headers, LIBRARY_VERSION_HEADER)
            .map(|value| {
                value.parse::<i64>().map_err(|error| {
                    AppError::InvalidZoteroResponse(format!(
                        "invalid full-text Last-Modified-Version '{value}': {error}"
                    ))
                })
            })
            .transpose()?;
        Ok(FullTextInventory {
            library_version: header_version.or_else(|| {
                versions
                    .iter()
                    .map(|(_, version)| *version)
                    .max()
                    .or(Some(since))
            }),
            versions,
        })
    }

    pub async fn fulltext_record(
        &self,
        library: &RemoteLibrary,
        attachment_key: &str,
        expected_version: i64,
        server_id: &str,
    ) -> AppResult<FullTextRecord> {
        let prefix = library.api_prefix();
        let path = format!("{prefix}/items/{attachment_key}/fulltext");
        let response = self.read_json(&path, &[], Some(server_id)).await?;
        self.verify_source(&response.headers, server_id)?;
        let content = response
            .value
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AppError::InvalidZoteroResponse(format!(
                    "full-text response for attachment {attachment_key} omitted content"
                ))
            })?
            .to_owned();
        let version = header_text(&response.headers, LIBRARY_VERSION_HEADER)
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(expected_version);
        Ok(FullTextRecord {
            attachment_key: attachment_key.to_owned(),
            version,
            content,
            indexed_pages: unsigned_page_field(&response.value, "indexedPages"),
            total_pages: unsigned_page_field(&response.value, "totalPages"),
            indexed_chars: unsigned_field(&response.value, "indexedChars"),
            total_chars: unsigned_field(&response.value, "totalChars"),
        })
    }

    pub async fn file_view_url(
        &self,
        library: &RemoteLibrary,
        attachment_key: &str,
        server_id: &str,
    ) -> AppResult<String> {
        let prefix = library.api_prefix();
        let path = format!("{prefix}/items/{attachment_key}/file/view/url");
        let (headers, bytes) = self.send_get(&path, &[], Some(server_id)).await?;
        self.verify_source(&headers, server_id)?;
        String::from_utf8(bytes)
            .map(|value| value.trim().to_owned())
            .map_err(|error| {
                AppError::InvalidZoteroResponse(format!(
                    "attachment file URL was not UTF-8: {error}"
                ))
            })
    }

    async fn object_inventory(
        &self,
        library: &RemoteLibrary,
        object_type: &str,
        server_id: &str,
    ) -> AppResult<VersionMap> {
        let prefix = library.api_prefix();
        let path = format!("{prefix}/{object_type}");
        let mut query = vec![("format", "versions")];
        if object_type == "items" {
            query.push(("includeTrashed", "1"));
        }
        let response = self.read_json(&path, &query, Some(server_id)).await?;
        self.verify_source(&response.headers, server_id)?;
        let map: HashMap<String, i64> =
            serde_json::from_value(response.value).map_err(|error| {
                AppError::InvalidZoteroResponse(format!(
                    "invalid {object_type} version map: {error}"
                ))
            })?;
        let mut versions: Vec<_> = map.into_iter().collect();
        versions.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(VersionMap {
            versions,
            library_version: library_version(&response.headers)?,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn fetch_objects(
        &self,
        library: &RemoteLibrary,
        object_type: &str,
        key_parameter: &str,
        keys: &[String],
        server_id: &str,
        expected_version: i64,
    ) -> AppResult<Vec<RawObject>> {
        let mut objects = Vec::with_capacity(keys.len());
        for batch in keys.chunks(BATCH_SIZE) {
            let prefix = library.api_prefix();
            let path = format!("{prefix}/{object_type}");
            let joined_keys = batch.join(",");
            let mut query = vec![("format", "json"), (key_parameter, joined_keys.as_str())];
            if object_type == "items" {
                query.push(("includeTrashed", "1"));
            }
            let response = self.read_json(&path, &query, Some(server_id)).await?;
            self.verify_source(&response.headers, server_id)?;
            if library_version(&response.headers)? != expected_version {
                return Err(AppError::ConcurrentChange);
            }
            let values = response.value.as_array().ok_or_else(|| {
                AppError::InvalidZoteroResponse(format!(
                    "{object_type} fetch did not return an array"
                ))
            })?;
            for value in values {
                let key = value
                    .get("key")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        AppError::InvalidZoteroResponse(format!("{object_type} object has no key"))
                    })?
                    .to_owned();
                let version = value
                    .get("version")
                    .and_then(Value::as_i64)
                    .unwrap_or_default();
                objects.push(RawObject {
                    key,
                    version,
                    object_type: object_type.to_owned(),
                    json: value.clone(),
                });
            }
        }
        Ok(objects)
    }

    async fn read_json(
        &self,
        path: &str,
        query: &[(&str, &str)],
        expected_server_id: Option<&str>,
    ) -> AppResult<ReadResponse> {
        let (headers, bytes) = self.send_get(path, query, expected_server_id).await?;
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).map_err(|error| {
                AppError::InvalidZoteroResponse(format!("response was not valid JSON: {error}"))
            })?
        };
        Ok(ReadResponse { headers, value })
    }

    async fn read_headers(
        &self,
        path: &str,
        query: &[(&str, &str)],
        expected_server_id: Option<&str>,
    ) -> AppResult<HeaderMap> {
        let (headers, _) = self.send_get(path, query, expected_server_id).await?;
        Ok(headers)
    }

    async fn send_get(
        &self,
        path: &str,
        query: &[(&str, &str)],
        expected_server_id: Option<&str>,
    ) -> AppResult<(HeaderMap, Vec<u8>)> {
        let url = format!("{}{path}", self.root);
        let mut request = self
            .client
            .get(url)
            .header("Zotero-API-Version", "3")
            .query(query);
        if let Some(server_id) = expected_server_id {
            request = request.header("Zotero-Server-ID", server_id);
        }
        let response = request.send().await?;
        if response.status().is_redirection() {
            return Err(AppError::InvalidZoteroResponse(
                "Zotero Local API redirects are disabled.".to_owned(),
            ));
        }
        match response.status().as_u16() {
            403 => return Err(AppError::ZoteroDisabled),
            404 => return Err(AppError::ZoteroResourceUnavailable(path.to_owned())),
            412 => return Err(AppError::SourceChanged),
            _ => {}
        }
        let response = response.error_for_status()?;
        let headers = response.headers().clone();
        let bytes = response.bytes().await?.to_vec();
        Ok((headers, bytes))
    }

    fn verify_source(&self, headers: &HeaderMap, expected_server_id: &str) -> AppResult<()> {
        match header_text(headers, SERVER_ID_HEADER) {
            Some(actual) if actual == expected_server_id => Ok(()),
            Some(_) => Err(AppError::SourceChanged),
            None => Err(AppError::MissingServerIdentity),
        }
    }
}

fn unsigned_field(value: &Value, name: &str) -> Option<u64> {
    value.get(name).and_then(Value::as_u64)
}

fn unsigned_page_field(value: &Value, name: &str) -> Option<u32> {
    unsigned_field(value, name).and_then(|number| u32::try_from(number).ok())
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn library_version(headers: &HeaderMap) -> AppResult<i64> {
    let value = header_text(headers, LIBRARY_VERSION_HEADER).ok_or_else(|| {
        AppError::InvalidZoteroResponse("response omitted Last-Modified-Version".to_owned())
    })?;
    value.parse::<i64>().map_err(|error| {
        AppError::InvalidZoteroResponse(format!("invalid Last-Modified-Version '{value}': {error}"))
    })
}

#[cfg(test)]
mod tests {
    use reqwest::header::{HeaderMap, HeaderValue};

    use super::{header_text, library_version, ZoteroReadClient};

    #[tokio::test]
    async fn local_api_redirects_are_rejected_without_following() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request");
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .expect("timeout");
            let mut request = [0; 1024];
            let received = stream.read(&mut request).expect("request bytes");
            assert!(received > 0);
            stream.write_all(
                b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            ).expect("response");
        });
        let client = ZoteroReadClient::with_root(format!("http://{address}")).expect("client");
        let error = client.probe().await.expect_err("redirect rejected");
        server.join().expect("server");
        assert!(error.to_string().contains("redirects are disabled"));
    }

    #[test]
    fn parses_zotero_version_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("last-modified-version", HeaderValue::from_static("42"));
        headers.insert("zotero-server-id", HeaderValue::from_static("source-a"));

        assert_eq!(library_version(&headers).expect("version"), 42);
        assert_eq!(
            header_text(&headers, "zotero-server-id").as_deref(),
            Some("source-a")
        );
    }
}
