use std::io::{BufRead, BufReader, Read};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::blocking::{Client, Response};
use reqwest::redirect::Policy;
use reqwest::Url;
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

use super::managed::visible_without_reasoning;
use super::model::{ExternalProviderConfig, GenerationRequest, GenerationResult};
use super::provider::{ConservativeTokenEstimator, EvidenceTokenCounter, LocalModelProvider};

const MAX_EXTERNAL_OUTPUT_BYTES: usize = 512 * 1024;
const MAX_EXTERNAL_FRAME_BYTES: usize = 1024 * 1024;
const MAX_EXTERNAL_STREAM_BYTES: usize = 32 * 1024 * 1024;

pub struct OpenAiCompatibleLocalProvider {
    config: ExternalProviderConfig,
    client: Client,
    completion_url: Url,
    models_url: Url,
}

impl OpenAiCompatibleLocalProvider {
    pub fn new(config: ExternalProviderConfig) -> AppResult<Self> {
        validate_model_identifier(&config.model)?;
        if !(2_048..=131_072).contains(&config.context_tokens) {
            return Err(AppError::LocalModel(
                "External provider context must be between 2,048 and 131,072 tokens.".to_owned(),
            ));
        }
        let (base, host, addresses) = validate_loopback_endpoint(&config.endpoint)?;
        let mut builder = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(300));
        if host == "localhost" {
            builder = builder.resolve_to_addrs(&host, &addresses);
        }
        let client = builder.build().map_err(|error| {
            AppError::LocalModel(format!("External provider client failed: {error}"))
        })?;
        let completion_url = endpoint_url(&base, "chat/completions")?;
        let models_url = endpoint_url(&base, "models")?;
        Ok(Self {
            config,
            client,
            completion_url,
            models_url,
        })
    }

    pub fn health_check(&self) -> AppResult<String> {
        let mut request = self.client.get(self.models_url.clone());
        if let Some(key) = self.config.api_key.as_deref().filter(|key| !key.is_empty()) {
            request = request.bearer_auth(key);
        }
        let response = request.send().map_err(|error| {
            AppError::LocalModel(format!("The loopback provider is not reachable: {error}"))
        })?;
        if response.status().is_redirection() {
            return Err(AppError::LocalModel(
                "External provider redirects are disabled.".to_owned(),
            ));
        }
        if !response.status().is_success() {
            return Err(external_status_error(response));
        }
        Ok("Loopback provider responded without leaving this computer.".to_owned())
    }
}

impl EvidenceTokenCounter for OpenAiCompatibleLocalProvider {
    fn count_tokens(&self, text: &str) -> AppResult<u32> {
        ConservativeTokenEstimator.count_tokens(text)
    }

    fn token_accounting_method(&self) -> &'static str {
        ConservativeTokenEstimator.token_accounting_method()
    }
}

impl LocalModelProvider for OpenAiCompatibleLocalProvider {
    fn provider_id(&self) -> &'static str {
        "openai-compatible"
    }

    fn provider_label(&self) -> String {
        "External loopback provider".to_owned()
    }

    fn model_label(&self) -> String {
        self.config.model.clone()
    }

    fn context_capacity(&self) -> u32 {
        self.config.context_tokens
    }

    fn generate(
        &self,
        request: &GenerationRequest,
        cancellation: Arc<AtomicBool>,
        on_delta: &mut dyn FnMut(&str),
    ) -> AppResult<GenerationResult> {
        let started = Instant::now();
        let payload = json!({
            "model": self.config.model,
            "messages": [
                {"role": "system", "content": request.system_prompt},
                {"role": "user", "content": format!("{}\n/no_think", request.user_prompt)}
            ],
            "stream": true,
            "temperature": request.temperature,
            "max_tokens": request.max_tokens,
            "seed": 42
        });
        let mut builder = self.client.post(self.completion_url.clone()).json(&payload);
        if let Some(key) = self.config.api_key.as_deref().filter(|key| !key.is_empty()) {
            builder = builder.bearer_auth(key);
        }
        let response = builder.send().map_err(|error| {
            AppError::LocalModel(format!("Loopback generation request failed: {error}"))
        })?;
        if response.status().is_redirection() {
            return Err(AppError::LocalModel(
                "External provider redirects are disabled.".to_owned(),
            ));
        }
        if !response.status().is_success() {
            return Err(external_status_error(response));
        }
        let mut raw = String::new();
        let mut visible = String::new();
        let mut first_token_ms = None;
        let mut generated_tokens = 0_u32;
        let mut reader = BufReader::new(response);
        let mut received_bytes = 0;
        loop {
            if cancellation.load(Ordering::SeqCst) {
                return Err(AppError::Agent(
                    "Generation stopped by the user. The partial answer was not marked complete."
                        .to_owned(),
                ));
            }
            let Some(line) = read_stream_line(&mut reader, &mut received_bytes)? else {
                break;
            };
            let data = line
                .strip_prefix("data:")
                .map(str::trim)
                .unwrap_or(line.trim());
            if data == "[DONE]" {
                break;
            }
            if data.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(data).map_err(|error| {
                AppError::LocalModel(format!("Loopback provider returned invalid JSON: {error}"))
            })?;
            if let Some(tokens) = value
                .get("usage")
                .and_then(|usage| usage.get("completion_tokens"))
                .and_then(Value::as_u64)
            {
                generated_tokens = tokens.min(u32::MAX as u64) as u32;
            }
            let piece = value
                .pointer("/choices/0/delta/content")
                .or_else(|| value.pointer("/choices/0/message/content"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if piece.is_empty() {
                continue;
            }
            raw.push_str(piece);
            if raw.len() > MAX_EXTERNAL_OUTPUT_BYTES {
                return Err(AppError::LocalModel(
                    "The loopback provider exceeded Graphite's output byte limit.".to_owned(),
                ));
            }
            let next_visible = visible_without_reasoning(&raw);
            if next_visible.len() > visible.len() {
                let delta = next_visible.strip_prefix(&visible).ok_or_else(|| {
                    AppError::LocalModel(
                        "The loopback stream changed previously emitted text.".to_owned(),
                    )
                })?;
                if first_token_ms.is_none() && !delta.trim().is_empty() {
                    first_token_ms = Some(started.elapsed().as_millis() as u64);
                }
                on_delta(delta);
                visible = next_visible;
            }
        }
        if generated_tokens == 0 {
            generated_tokens = visible.split_whitespace().count() as u32;
        }
        Ok(GenerationResult {
            text: visible_without_reasoning(&raw).trim().to_owned(),
            generated_tokens,
            time_to_first_token_ms: first_token_ms,
            generation_ms: started.elapsed().as_millis() as u64,
        })
    }
}

fn read_stream_line(
    reader: &mut impl BufRead,
    received_bytes: &mut usize,
) -> AppResult<Option<String>> {
    let mut line = String::new();
    let bytes = reader
        .take((MAX_EXTERNAL_FRAME_BYTES + 1) as u64)
        .read_line(&mut line)
        .map_err(|error| {
            AppError::LocalModel(format!("Loopback response stream failed: {error}"))
        })?;
    *received_bytes += bytes;
    if bytes > MAX_EXTERNAL_FRAME_BYTES || *received_bytes > MAX_EXTERNAL_STREAM_BYTES {
        return Err(AppError::LocalModel(
            "The loopback provider exceeded Graphite's response byte limit.".to_owned(),
        ));
    }
    Ok((bytes != 0).then_some(line))
}

fn validate_loopback_endpoint(endpoint: &str) -> AppResult<(Url, String, Vec<SocketAddr>)> {
    let mut url = Url::parse(endpoint).map_err(|error| {
        AppError::LocalModel(format!(
            "The external provider endpoint is invalid: {error}"
        ))
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::LocalModel(
            "External providers must use an http or https loopback URL.".to_owned(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::LocalModel(
            "Credentials are not allowed inside the provider URL.".to_owned(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(AppError::LocalModel(
            "Provider URLs cannot contain a query or fragment.".to_owned(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| AppError::LocalModel("The external provider URL has no host.".to_owned()))?
        .to_owned();
    let resolver_host = host.trim_start_matches('[').trim_end_matches(']');
    if host != "localhost"
        && resolver_host
            .parse::<IpAddr>()
            .map_or(true, |address| !address.is_loopback())
    {
        return Err(AppError::LocalModel(
            "M5 accepts only 127.0.0.1, [::1], or localhost providers.".to_owned(),
        ));
    }
    let port = url.port_or_known_default().ok_or_else(|| {
        AppError::LocalModel("The external provider URL needs an explicit port.".to_owned())
    })?;
    if url.port().is_none() {
        return Err(AppError::LocalModel(
            "The external provider URL needs an explicit loopback port.".to_owned(),
        ));
    }
    let addresses = (resolver_host, port)
        .to_socket_addrs()
        .map_err(|error| AppError::LocalModel(format!("Loopback resolution failed: {error}")))?
        .collect::<Vec<_>>();
    if addresses.is_empty() || addresses.iter().any(|address| !address.ip().is_loopback()) {
        return Err(AppError::LocalModel(
            "The provider hostname did not resolve exclusively to loopback addresses.".to_owned(),
        ));
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok((url, host, addresses))
}

fn endpoint_url(base: &Url, resource: &str) -> AppResult<Url> {
    let mut normalized = base.clone();
    let path = normalized.path().trim_end_matches('/');
    let prefix = if path.ends_with("/v1") {
        format!("{path}/")
    } else if path.is_empty() || path == "/" {
        "/v1/".to_owned()
    } else {
        format!("{path}/v1/")
    };
    normalized.set_path(&prefix);
    normalized.join(resource).map_err(|error| {
        AppError::LocalModel(format!("Provider API URL construction failed: {error}"))
    })
}

fn validate_model_identifier(model: &str) -> AppResult<()> {
    let valid = !model.trim().is_empty()
        && model.len() <= 160
        && model
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._:/-".contains(character));
    if !valid {
        return Err(AppError::LocalModel(
            "The external model identifier contains unsupported characters.".to_owned(),
        ));
    }
    Ok(())
}

fn external_status_error(response: Response) -> AppError {
    let status = response.status();
    // Provider error bodies may echo credentials or private prompt text.
    AppError::LocalModel(format!(
        "The loopback provider returned HTTP {status}. Check the provider's \
         local configuration. Response details were withheld for privacy."
    ))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::Duration;

    use super::{
        endpoint_url, validate_loopback_endpoint, validate_model_identifier,
        OpenAiCompatibleLocalProvider,
    };
    use crate::agent::model::{ExternalProviderConfig, GenerationRequest};
    use crate::agent::provider::{EvidenceTokenCounter, LocalModelProvider};

    fn local_response_server(body: &str, status: &str) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
        let address = listener.local_addr().expect("listener address");
        let body = body.to_owned();
        let status = status.to_owned();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("local request");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("read timeout");
            let mut request = [0_u8; 16_384];
            let _ = stream.read(&mut request).expect("request bytes");
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).expect("response");
        });
        (format!("http://{address}"), handle)
    }

    fn provider(endpoint: String) -> OpenAiCompatibleLocalProvider {
        OpenAiCompatibleLocalProvider::new(ExternalProviderConfig {
            endpoint,
            model: "fixture-model".to_owned(),
            context_tokens: 8_192,
            api_key: None,
        })
        .expect("provider")
    }

    #[test]
    fn loopback_validation_rejects_remote_and_private_networks() {
        assert!(validate_loopback_endpoint("http://127.0.0.1:11434").is_ok());
        assert!(validate_loopback_endpoint("http://[::1]:1234/v1").is_ok());
        assert!(validate_loopback_endpoint("http://192.168.1.4:11434").is_err());
        assert!(validate_loopback_endpoint("https://example.com:443").is_err());
        assert!(validate_loopback_endpoint("http://127.0.0.1").is_err());
        assert!(validate_loopback_endpoint("file:///tmp/model").is_err());
    }

    #[test]
    fn endpoint_paths_are_normalized_to_openai_v1() {
        let (base, _, _) = validate_loopback_endpoint("http://127.0.0.1:11434").expect("valid");
        assert_eq!(
            endpoint_url(&base, "chat/completions")
                .expect("url")
                .as_str(),
            "http://127.0.0.1:11434/v1/chat/completions"
        );
    }

    #[test]
    fn model_identifiers_are_bounded_data_not_commands() {
        assert!(validate_model_identifier("qwen3:4b").is_ok());
        assert!(validate_model_identifier("qwen; rm -rf").is_err());
        assert!(validate_model_identifier("").is_err());
    }

    #[test]
    fn external_provider_labels_context_accounting_as_estimated() {
        let provider = provider("http://127.0.0.1:11434".to_owned());
        assert_eq!(
            provider.token_accounting_method(),
            "estimated-characters-3.0-v1"
        );
        assert_eq!(
            provider.count_tokens("1234567").expect("estimated tokens"),
            3
        );
    }

    #[test]
    fn loopback_provider_streams_openai_compatible_events() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Grounded \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"answer [S1].\"}}]}\n\n",
            "data: [DONE]\n\n",
            "data: trailing invalid data must not be read\n"
        );
        let (endpoint, server) = local_response_server(body, "200 OK");
        let provider = provider(endpoint);
        let request = GenerationRequest {
            system_prompt: "Ground answers".to_owned(),
            user_prompt: "Question".to_owned(),
            max_tokens: 32,
            context_tokens: 8_192,
            temperature: 0.0,
        };
        let mut streamed = String::new();
        let result = provider
            .generate(&request, Arc::new(AtomicBool::new(false)), &mut |delta| {
                streamed.push_str(delta)
            })
            .expect("generation");
        server.join().expect("server");
        assert_eq!(streamed, "Grounded answer [S1].");
        assert_eq!(result.text, streamed);
    }

    #[test]
    fn stream_limits_include_metadata_before_json_parsing() {
        use super::{read_stream_line, MAX_EXTERNAL_FRAME_BYTES, MAX_EXTERNAL_STREAM_BYTES};
        use std::io::Cursor;

        let metadata = format!(
            "data: {{\"private\":\"{}\"}}\n",
            "x".repeat(MAX_EXTERNAL_FRAME_BYTES)
        );
        let mut reader = Cursor::new(metadata.as_bytes());
        let mut received = 0;
        let error = read_stream_line(&mut reader, &mut received).expect_err("bounded frame");
        assert!(error.to_string().contains("response byte limit"));
        assert!(!error.to_string().contains("private"));
        assert_eq!(reader.position(), (MAX_EXTERNAL_FRAME_BYTES + 1) as u64);

        let mut reader = Cursor::new(b"data: {}\n\n");
        let mut received = MAX_EXTERNAL_STREAM_BYTES - 9;
        assert!(read_stream_line(&mut reader, &mut received)
            .expect("last frame")
            .is_some());
        assert!(read_stream_line(&mut reader, &mut received).is_err());
    }

    #[test]
    fn provider_error_bodies_do_not_escape_into_app_errors() {
        let private_body = "Bearer fixture-secret; private research context";
        for generation in [false, true] {
            let (endpoint, server) = local_response_server(private_body, "401 Unauthorized");
            let local = provider(endpoint);
            let error = if generation {
                local
                    .generate(
                        &GenerationRequest {
                            system_prompt: "Ground answers".to_owned(),
                            user_prompt: "Question".to_owned(),
                            max_tokens: 32,
                            context_tokens: 8_192,
                            temperature: 0.0,
                        },
                        Arc::new(AtomicBool::new(false)),
                        &mut |_| {},
                    )
                    .expect_err("generation failure")
                    .to_string()
            } else {
                local
                    .health_check()
                    .expect_err("health failure")
                    .to_string()
            };
            server.join().expect("server");
            assert!(error.contains("401"));
            assert!(error.contains("withheld for privacy"));
            assert!(!error.contains("fixture-secret"));
            assert!(!error.contains("private research context"));
        }
    }

    #[test]
    fn loopback_provider_rejects_redirects_and_malformed_streams() {
        let (redirect_endpoint, redirect_server) =
            local_response_server("", "302 Found\r\nLocation: https://example.com");
        let redirect = provider(redirect_endpoint).health_check();
        redirect_server.join().expect("redirect server");
        assert!(redirect.is_err());

        let (malformed_endpoint, malformed_server) =
            local_response_server("data: not-json\n\n", "200 OK");
        let request = GenerationRequest {
            system_prompt: "Ground answers".to_owned(),
            user_prompt: "Question".to_owned(),
            max_tokens: 32,
            context_tokens: 8_192,
            temperature: 0.0,
        };
        let malformed = provider(malformed_endpoint).generate(
            &request,
            Arc::new(AtomicBool::new(false)),
            &mut |_| {},
        );
        malformed_server.join().expect("malformed server");
        assert!(malformed.is_err());
    }
}
