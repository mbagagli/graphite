use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use graphite_pdf_benchmark_common::fixtures::generate_synthetic_corpus;
use graphite_pdf_benchmark_common::{
    content_hash, read_json, score_extraction, validate_aggregate_privacy, validate_anchor_set,
    validate_extraction, validate_manifest, validate_operational_privacy, validate_scoring_anchors,
    write_json_atomic, AggregateResult, AnchorSet, AssetLock, BenchmarkExtractionEnvelopeV1,
    BenchmarkProfile, CandidateDescriptor, CorpusDocument, CorpusManifest, ExtractionOutcome,
    ExtractionRequest, OperationalPhaseMeasurement, OperationalReport, OperationalResultSet,
    ProtocolRequest, ProtocolResponse, TimeoutClass, TimeoutPolicy, TimeoutStage,
    ZoteroTextSidecar, BENCHMARK_SCHEMA_VERSION,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("pdf-benchmark: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    let Some(command) = arguments.first().map(String::as_str) else {
        return Err(usage());
    };
    match command {
        "fixtures" => {
            let root = required_path(&arguments, 1, "fixtures root")?;
            let manifest = generate_synthetic_corpus(&root)?;
            println!(
                "generated {} checksum-locked synthetic documents under {}",
                manifest.documents.len(),
                root.display()
            );
            Ok(())
        }
        "prepare" => prepare_assets(&arguments[1..]),
        "validate-assets" => {
            let path = required_path(&arguments, 1, "asset lock")?;
            let lock: AssetLock = read_json(&path)?;
            validate_asset_lock(&lock)?;
            println!("validated {}", path.display());
            Ok(())
        }
        "verify-prepared" => verify_prepared_assets(&arguments[1..]),
        "verify-protocol" => verify_protocol_lock(&arguments[1..]),
        "validate" => {
            let manifest_path = required_path(&arguments, 1, "manifest")?;
            validate_corpus(&manifest_path)?;
            println!("validated {}", manifest_path.display());
            Ok(())
        }
        "capture-zotero" => capture_zotero_sidecar(&arguments[1..]),
        "run" => run_benchmark(&arguments[1..]),
        "measure" => measure_benchmark(&arguments[1..]),
        "score" => score_run(&arguments[1..]),
        "compare" => compare_runs(&arguments[1..]),
        "privacy" => {
            let path = required_path(&arguments, 1, "aggregate or operational result")?;
            let value: serde_json::Value = read_json(&path)?;
            if let Ok(aggregate) = serde_json::from_value::<AggregateResult>(value.clone()) {
                validate_aggregate_privacy(&aggregate).map_err(|errors| errors.join("\n"))?;
            } else if let Ok(operational) = serde_json::from_value::<OperationalResultSet>(value) {
                validate_operational_privacy(&operational).map_err(|errors| errors.join("\n"))?;
            } else {
                return Err(
                    "result is neither a scoring aggregate nor operational metrics".to_owned(),
                );
            }
            println!("result contains no benchmark payload or absolute path");
            Ok(())
        }
        _ => Err(usage()),
    }
}

fn usage() -> String {
    [
        "usage:",
        "  graphite-pdf-benchmark-runner fixtures <corpus-root>",
        "  graphite-pdf-benchmark-runner prepare <asset-lock> <candidate> <asset-root>",
        "  graphite-pdf-benchmark-runner validate-assets <asset-lock>",
        "  graphite-pdf-benchmark-runner verify-prepared <asset-lock> <candidate> <asset-root>",
        "  graphite-pdf-benchmark-runner verify-protocol <protocol-lock> <repository-root>",
        "  graphite-pdf-benchmark-runner validate <manifest>",
        "  graphite-pdf-benchmark-runner capture-zotero <manifest> <document-id> <library-kind> <library-id> <attachment-key>",
        "  graphite-pdf-benchmark-runner run <manifest> <run-dir> <timeout-policy> <candidate=executable>...",
        "  graphite-pdf-benchmark-runner measure <manifest> <output> <timeout-policy> <candidate=executable>...",
        "  graphite-pdf-benchmark-runner score <manifest> <run-dir> <aggregate-output>",
        "  graphite-pdf-benchmark-runner compare <run-dir-a> <run-dir-b>",
        "  graphite-pdf-benchmark-runner privacy <aggregate-or-operational-output>",
    ]
    .join("\n")
}

fn capture_zotero_sidecar(arguments: &[String]) -> Result<(), String> {
    let manifest_path = required_path(arguments, 0, "manifest")?;
    let document_id = arguments
        .get(1)
        .ok_or_else(|| format!("missing document identifier\n{}", usage()))?;
    let library_kind = arguments
        .get(2)
        .ok_or_else(|| format!("missing Zotero library kind\n{}", usage()))?;
    let library_id = arguments
        .get(3)
        .ok_or_else(|| format!("missing Zotero library identifier\n{}", usage()))?;
    let attachment_key = arguments
        .get(4)
        .ok_or_else(|| format!("missing Zotero attachment key\n{}", usage()))?;
    if arguments.len() != 5 {
        return Err(format!(
            "capture-zotero received unexpected arguments\n{}",
            usage()
        ));
    }
    let manifest = validate_corpus(&manifest_path)?;
    if !manifest
        .documents
        .iter()
        .any(|document| &document.id == document_id)
    {
        return Err(format!("manifest has no document {document_id}"));
    }
    if !is_neutral_document_id(document_id) {
        return Err("document identifier is not a neutral benchmark ID".to_owned());
    }
    if !matches!(library_kind.as_str(), "user" | "group") {
        return Err("Zotero library kind must be user or group".to_owned());
    }
    if library_id.is_empty() || attachment_key.is_empty() {
        return Err("Zotero library and attachment identifiers must be non-empty".to_owned());
    }
    let corpus_root = manifest_path
        .parent()
        .ok_or_else(|| "manifest has no parent directory".to_owned())?;
    let local_root = corpus_root.join("local");
    fs::create_dir_all(&local_root)
        .map_err(|error| format!("cannot create local sidecar directory: {error}"))?;
    let temporary_root = local_root.join(format!(
        ".zotero-capture-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("system time is invalid: {error}"))?
            .as_nanos()
    ));
    fs::create_dir(&temporary_root)
        .map_err(|error| format!("cannot create Zotero capture staging directory: {error}"))?;
    let result = capture_zotero_sidecar_in(
        document_id,
        library_kind,
        library_id,
        attachment_key,
        &temporary_root,
    );
    let cleanup = fs::remove_dir_all(&temporary_root)
        .map_err(|error| format!("cannot remove Zotero capture staging directory: {error}"));
    let sidecar = result?;
    cleanup?;
    let output = local_root.join(format!("{document_id}.zotero-fulltext.json"));
    write_json_atomic(&output, &sidecar)?;
    let sidecar_hash = content_hash(
        &fs::read(&output).map_err(|error| format!("cannot verify captured sidecar: {error}"))?,
    );
    println!(
        "captured read-only Zotero sidecar local/{document_id}.zotero-fulltext.json; content_sha256={}; sidecar_sha256={sidecar_hash}",
        sidecar.content_sha256.as_deref().unwrap_or("unavailable")
    );
    Ok(())
}

fn capture_zotero_sidecar_in(
    document_id: &str,
    library_kind: &str,
    library_id: &str,
    attachment_key: &str,
    staging_root: &Path,
) -> Result<ZoteroTextSidecar, String> {
    const ROOT: &str = "http://127.0.0.1:23119/api";
    let probe_headers = staging_root.join("probe.headers");
    let probe_body = staging_root.join("probe.body");
    run_curl_get(ROOT, &probe_headers, Some(&probe_body), &[])?;
    let probe = fs::read_to_string(&probe_headers)
        .map_err(|error| format!("cannot read Zotero probe headers: {error}"))?;
    let server_id = response_header(&probe, "zotero-server-id")
        .ok_or_else(|| "Zotero probe omitted Zotero-Server-ID".to_owned())?;
    let prefix = if library_kind == "user" {
        format!("users/{library_id}")
    } else {
        format!("groups/{library_id}")
    };
    let url = format!("{ROOT}/{prefix}/items/{attachment_key}/fulltext");
    let response_headers_path = staging_root.join("fulltext.headers");
    let response_body_path = staging_root.join("fulltext.json");
    run_curl_get(
        &url,
        &response_headers_path,
        Some(&response_body_path),
        &[
            ("Zotero-API-Version", "3"),
            ("Zotero-Server-ID", server_id.as_str()),
        ],
    )?;
    let headers = fs::read_to_string(&response_headers_path)
        .map_err(|error| format!("cannot read Zotero full-text headers: {error}"))?;
    if response_header(&headers, "zotero-server-id").as_deref() != Some(server_id.as_str()) {
        return Err("Zotero full-text response came from a different server identity".to_owned());
    }
    let source_version = response_header(&headers, "last-modified-version")
        .ok_or_else(|| "Zotero full-text response omitted Last-Modified-Version".to_owned())?
        .parse::<i64>()
        .map_err(|error| format!("invalid Zotero Last-Modified-Version: {error}"))?;
    let value: serde_json::Value = read_json(&response_body_path)?;
    let content = value
        .get("content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Zotero full-text response omitted content".to_owned())?
        .to_owned();
    Ok(ZoteroTextSidecar {
        schema_version: 2,
        document_id: Some(document_id.to_owned()),
        provider: "zotero-local-api-fulltext-v1".to_owned(),
        source_version,
        content_sha256: Some(content_hash(content.as_bytes())),
        content,
        indexed_pages: json_u32(&value, "indexedPages"),
        total_pages: json_u32(&value, "totalPages"),
        indexed_characters: json_u64(&value, "indexedChars"),
        total_characters: json_u64(&value, "totalChars"),
    })
}

fn run_curl_get(
    url: &str,
    headers_path: &Path,
    body_path: Option<&Path>,
    headers: &[(&str, &str)],
) -> Result<(), String> {
    let mut command = Command::new("curl");
    command
        .args([
            "-fsS",
            "--max-time",
            "30",
            "--request",
            "GET",
            "--dump-header",
        ])
        .arg(headers_path);
    if let Some(body_path) = body_path {
        command.arg("--output").arg(body_path);
    } else {
        command.arg("--output").arg("/dev/null");
    }
    for (name, value) in headers {
        command.arg("--header").arg(format!("{name}: {value}"));
    }
    let status = command
        .arg(url)
        .status()
        .map_err(|error| format!("cannot start read-only Zotero request: {error}"))?;
    if !status.success() {
        return Err(format!(
            "read-only Zotero request failed with status {status}"
        ));
    }
    Ok(())
}

fn response_header(headers: &str, name: &str) -> Option<String> {
    headers.lines().rev().find_map(|line| {
        let (candidate, value) = line.split_once(':')?;
        candidate
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_owned())
    })
}

fn json_u64(value: &serde_json::Value, field: &str) -> Option<u64> {
    value.get(field).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

fn json_u32(value: &serde_json::Value, field: &str) -> Option<u32> {
    json_u64(value, field).and_then(|value| u32::try_from(value).ok())
}

fn is_neutral_document_id(value: &str) -> bool {
    value.starts_with("paper")
        && value.len() > "paper".len()
        && value["paper".len()..]
            .chars()
            .all(|character| character.is_ascii_digit())
}

fn prepare_assets(arguments: &[String]) -> Result<(), String> {
    let lock_path = required_path(arguments, 0, "asset lock")?;
    let candidate_id = arguments
        .get(1)
        .ok_or_else(|| format!("missing candidate identifier\n{}", usage()))?;
    let asset_root = required_path(arguments, 2, "asset root")?;
    let lock: AssetLock = read_json(&lock_path)?;
    validate_asset_lock(&lock)?;
    let candidate = lock
        .candidates
        .iter()
        .find(|candidate| &candidate.candidate_id == candidate_id)
        .ok_or_else(|| format!("asset lock has no candidate {candidate_id}"))?;
    let destination_root = asset_root.join(candidate_id);
    fs::create_dir_all(&destination_root)
        .map_err(|error| format!("cannot create {}: {error}", destination_root.display()))?;
    for asset in &candidate.assets {
        if !asset.platforms.is_empty()
            && !asset.platforms.iter().any(|value| value == platform_slug())
        {
            continue;
        }
        let sha256 = asset.sha256.as_deref().ok_or_else(|| {
            format!(
                "asset {} has no frozen SHA-256 and cannot be prepared reproducibly",
                asset.id
            )
        })?;
        if !asset.url.starts_with("https://") {
            return Err(format!("asset {} URL is not HTTPS", asset.id));
        }
        let relative = Path::new(&asset.file_name);
        if relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(format!(
                "asset {} filename escapes its candidate root",
                asset.id
            ));
        }
        let destination = destination_root.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        if asset_matches(&destination, sha256, asset.byte_count)? {
            unpack_asset(asset, &destination, &destination_root)?;
            continue;
        }
        let temporary = destination.with_extension("download");
        let status = Command::new("curl")
            .args([
                "-fL",
                "--proto",
                "=https",
                "--connect-timeout",
                "30",
                "--retry",
                "3",
                "--output",
            ])
            .arg(&temporary)
            .arg(&asset.url)
            .status()
            .map_err(|error| format!("cannot start curl for asset {}: {error}", asset.id))?;
        if !status.success() {
            let _ = fs::remove_file(&temporary);
            return Err(format!("download failed for asset {}", asset.id));
        }
        if !asset_matches(&temporary, sha256, asset.byte_count)? {
            let _ = fs::remove_file(&temporary);
            return Err(format!("downloaded asset {} failed its lock", asset.id));
        }
        fs::rename(&temporary, &destination).map_err(|error| {
            format!(
                "cannot publish asset {} at {}: {error}",
                asset.id,
                destination.display()
            )
        })?;
        unpack_asset(asset, &destination, &destination_root)?;
    }
    println!("prepared checksum-verified assets for {candidate_id}");
    Ok(())
}

fn verify_prepared_assets(arguments: &[String]) -> Result<(), String> {
    let lock_path = required_path(arguments, 0, "asset lock")?;
    let candidate_id = arguments
        .get(1)
        .ok_or_else(|| format!("missing candidate identifier\n{}", usage()))?;
    let asset_root = required_path(arguments, 2, "asset root")?;
    let lock: AssetLock = read_json(&lock_path)?;
    validate_asset_lock(&lock)?;
    let candidate = lock
        .candidates
        .iter()
        .find(|candidate| &candidate.candidate_id == candidate_id)
        .ok_or_else(|| format!("asset lock has no candidate {candidate_id}"))?;
    let candidate_root = asset_root.join(candidate_id);
    let mut verified = 0usize;
    for asset in &candidate.assets {
        if !asset.platforms.is_empty()
            && !asset.platforms.iter().any(|value| value == platform_slug())
        {
            continue;
        }
        let hash = asset
            .sha256
            .as_deref()
            .ok_or_else(|| format!("asset {} has no frozen SHA-256", asset.id))?;
        let path = candidate_root.join(&asset.file_name);
        if !asset_matches(&path, hash, asset.byte_count)? {
            return Err(format!(
                "prepared asset {} does not match its lock",
                asset.id
            ));
        }
        if let Some(relative) = &asset.unpack_to {
            let unpacked = candidate_root.join(relative);
            if directory_sizes(&unpacked).0 == 0 {
                return Err(format!(
                    "prepared asset {} has no unpacked runtime content",
                    asset.id
                ));
            }
        }
        verified += 1;
    }
    let (logical_bytes, allocated_bytes) = directory_sizes(&candidate_root);
    println!(
        "verified {verified} checksum-locked assets for {candidate_id}; logical bytes {logical_bytes}; allocated bytes {}",
        allocated_bytes
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unavailable".to_owned())
    );
    Ok(())
}

fn verify_protocol_lock(arguments: &[String]) -> Result<(), String> {
    let lock_path = required_path(arguments, 0, "protocol lock")?;
    let repository_root = required_path(arguments, 1, "repository root")?;
    let lock: serde_json::Value = read_json(&lock_path)?;
    if lock
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        return Err("protocol lock schemaVersion must be 1".to_owned());
    }
    let files = lock
        .get("lockedFiles")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "protocol lock has no lockedFiles array".to_owned())?;
    if files.is_empty() {
        return Err("protocol lock has no files".to_owned());
    }
    for file in files {
        let path = file
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "protocol lock file has no path".to_owned())?;
        validate_asset_relative_path(path, "protocol-file")?;
        let expected = file
            .get("sha256")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("protocol file {path} has no SHA-256"))?;
        let bytes = fs::read(repository_root.join(path))
            .map_err(|error| format!("cannot read protocol file {path}: {error}"))?;
        if graphite_pdf_benchmark_common::content_hash(&bytes) != expected {
            return Err(format!("protocol file {path} differs from the frozen hash"));
        }
    }
    println!("verified {} frozen protocol files", files.len());
    Ok(())
}

fn validate_asset_lock(lock: &AssetLock) -> Result<(), String> {
    if lock.schema_version != 1 {
        return Err(format!(
            "unsupported asset lock schema {}",
            lock.schema_version
        ));
    }
    let mut candidate_ids = std::collections::BTreeSet::new();
    for candidate in &lock.candidates {
        if candidate.candidate_id.trim().is_empty()
            || candidate.version.trim().is_empty()
            || candidate.revision.trim().is_empty()
            || candidate.license.trim().is_empty()
        {
            return Err("asset lock candidate identity is incomplete".to_owned());
        }
        if !candidate_ids.insert(candidate.candidate_id.as_str()) {
            return Err(format!(
                "asset lock repeats candidate {}",
                candidate.candidate_id
            ));
        }
        let mut asset_ids = std::collections::BTreeSet::new();
        for asset in &candidate.assets {
            if !asset_ids.insert(asset.id.as_str()) {
                return Err(format!(
                    "candidate {} repeats asset {}",
                    candidate.candidate_id, asset.id
                ));
            }
            let hash = asset.sha256.as_deref().ok_or_else(|| {
                format!(
                    "candidate {} asset {} has no SHA-256",
                    candidate.candidate_id, asset.id
                )
            })?;
            if hash.len() != 64
                || !hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(format!(
                    "candidate {} asset {} has an invalid SHA-256",
                    candidate.candidate_id, asset.id
                ));
            }
            if !asset.url.starts_with("https://") || asset.license.trim().is_empty() {
                return Err(format!(
                    "candidate {} asset {} has incomplete provenance",
                    candidate.candidate_id, asset.id
                ));
            }
            validate_asset_relative_path(&asset.file_name, &asset.id)?;
            if let Some(relative) = &asset.unpack_to {
                validate_asset_relative_path(relative, &asset.id)?;
            }
            if asset.required_for.is_empty() {
                return Err(format!(
                    "candidate {} asset {} has no required profile",
                    candidate.candidate_id, asset.id
                ));
            }
            if asset.platforms.iter().any(|platform| {
                !matches!(
                    platform.as_str(),
                    "linux-x64" | "linux-arm64" | "mac-x64" | "mac-arm64" | "win-x64"
                )
            }) {
                return Err(format!(
                    "candidate {} asset {} names an unknown platform",
                    candidate.candidate_id, asset.id
                ));
            }
        }
        if candidate
            .relevant_issues
            .iter()
            .any(|url| !url.starts_with("https://"))
        {
            return Err(format!(
                "candidate {} has a non-HTTPS issue reference",
                candidate.candidate_id
            ));
        }
    }
    Ok(())
}

fn unpack_asset(
    asset: &graphite_pdf_benchmark_common::ExternalAsset,
    archive: &Path,
    candidate_root: &Path,
) -> Result<(), String> {
    let Some(relative) = asset.unpack_to.as_deref() else {
        return Ok(());
    };
    validate_asset_relative_path(relative, &asset.id)?;
    let destination = candidate_root.join(relative);
    fs::create_dir_all(&destination)
        .map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
    let status = Command::new("tar")
        .args(["-xzf"])
        .arg(archive)
        .arg("-C")
        .arg(&destination)
        .status()
        .map_err(|error| format!("cannot start tar for asset {}: {error}", asset.id))?;
    if !status.success() {
        return Err(format!("cannot unpack asset {}", asset.id));
    }
    Ok(())
}

fn validate_asset_relative_path(value: &str, asset_id: &str) -> Result<(), String> {
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "asset {asset_id} unpack path escapes its candidate root"
        ));
    }
    Ok(())
}

fn platform_slug() -> &'static str {
    match (env::consts::OS, env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("macos", "x86_64") => "mac-x64",
        ("macos", "aarch64") => "mac-arm64",
        ("windows", "x86_64") => "win-x64",
        _ => "unsupported-platform",
    }
}

fn asset_matches(path: &Path, sha256: &str, byte_count: Option<u64>) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    let bytes = fs::read(path)
        .map_err(|error| format!("cannot read prepared asset {}: {error}", path.display()))?;
    Ok(
        byte_count.is_none_or(|expected| bytes.len() as u64 == expected)
            && graphite_pdf_benchmark_common::content_hash(&bytes) == sha256,
    )
}

fn required_path(arguments: &[String], index: usize, label: &str) -> Result<PathBuf, String> {
    arguments
        .get(index)
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing {label}\n{}", usage()))
}

fn validate_corpus(manifest_path: &Path) -> Result<CorpusManifest, String> {
    let manifest: CorpusManifest = read_json(manifest_path)?;
    let root = manifest_path
        .parent()
        .ok_or_else(|| "manifest has no parent directory".to_owned())?;
    validate_manifest(&manifest, root).map_err(|errors| errors.join("\n"))?;
    for document in &manifest.documents {
        let anchors: AnchorSet = read_json(&root.join(&document.anchors))?;
        validate_anchor_set(&anchors, &document.id).map_err(|errors| errors.join("\n"))?;
    }
    Ok(manifest)
}

fn run_benchmark(arguments: &[String]) -> Result<(), String> {
    let manifest_path = required_path(arguments, 0, "manifest")?;
    let run_dir = required_path(arguments, 1, "run directory")?;
    let manifest = validate_corpus(&manifest_path)?;
    let corpus_root = manifest_path
        .parent()
        .ok_or_else(|| "manifest has no parent directory".to_owned())?;
    let policy = read_timeout_policy(&required_path(arguments, 2, "timeout policy")?)?;
    let adapters = parse_adapters(&arguments[3..])?;
    let handshake_timeout = Duration::from_millis(policy.ordinary_ms);
    fs::create_dir_all(&run_dir)
        .map_err(|error| format!("cannot create {}: {error}", run_dir.display()))?;
    let mut candidate_errors = Vec::new();
    for (candidate_id, executable) in adapters {
        let descriptor = match handshake(&executable, handshake_timeout) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                candidate_errors.push(format!("{candidate_id}: {error}"));
                continue;
            }
        };
        if descriptor.candidate_id != candidate_id {
            candidate_errors.push(format!(
                "adapter {} identified itself as {}",
                candidate_id, descriptor.candidate_id
            ));
            continue;
        }
        for document in &manifest.documents {
            for profile in &descriptor.supported_profiles {
                if !profile_applies(document, profile) {
                    continue;
                }
                let request = ExtractionRequest {
                    request_id: format!("{}:{}:{}", document.id, candidate_id, profile.as_str()),
                    document_id: document.id.clone(),
                    artifact_id: format!("benchmark-artifact-{}", document.id),
                    artifact_revision_id: 1,
                    input_path: absolute_utf8(&corpus_root.join(&document.file))?,
                    input_sha256: document.sha256.clone(),
                    media_type: "application/pdf".to_owned(),
                    profile: profile.clone(),
                    forced_ocr_pages: document.expected_ocr_pages.clone(),
                    zotero_baseline_path: document
                        .zotero_baseline
                        .as_ref()
                        .map(|path| absolute_utf8(&corpus_root.join(path)))
                        .transpose()?,
                };
                let timeout_context = timeout_context(document, profile, &policy);
                let timeout = Duration::from_millis(timeout_context.ceiling_ms);
                let started = Instant::now();
                let mut result = extract_once(&executable, request.clone(), timeout)
                    .unwrap_or_else(|failure| {
                        failure_envelope(&request, &descriptor, failure, &timeout_context)
                    });
                result.timing.wall_time_ms = started.elapsed().as_millis() as u64;
                if let Err(errors) = validate_extraction(&result) {
                    result.outcome = ExtractionOutcome::Suspicious;
                    result.warnings.push(format!(
                        "benchmark envelope validation: {}",
                        errors.join("; ")
                    ));
                }
                let output = run_dir
                    .join(&candidate_id)
                    .join(profile.as_str())
                    .join(format!("{}.json", document.id));
                write_json_atomic(&output, &result)?;
            }
        }
    }
    println!("wrote raw local run output under {}", run_dir.display());
    if candidate_errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "some adapters could not start; other candidate results were preserved:\n{}",
            candidate_errors.join("\n")
        ))
    }
}

fn measure_benchmark(arguments: &[String]) -> Result<(), String> {
    let manifest_path = required_path(arguments, 0, "manifest")?;
    let output = required_path(arguments, 1, "operational output")?;
    let manifest = validate_corpus(&manifest_path)?;
    let corpus_root = manifest_path
        .parent()
        .ok_or_else(|| "manifest has no parent directory".to_owned())?;
    let policy = read_timeout_policy(&required_path(arguments, 2, "timeout policy")?)?;
    let adapters = parse_adapters(&arguments[3..])?;
    let handshake_timeout = Duration::from_millis(policy.ordinary_ms);
    let mut reports = Vec::new();
    let mut candidate_errors = Vec::new();
    for (candidate_id, executable) in adapters {
        let descriptor = match handshake(&executable, handshake_timeout) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                candidate_errors.push(format!("{candidate_id}: {error}"));
                continue;
            }
        };
        if descriptor.candidate_id != candidate_id {
            candidate_errors.push(format!(
                "adapter {} identified itself as {}",
                candidate_id, descriptor.candidate_id
            ));
            continue;
        }
        for document in &manifest.documents {
            for profile in &descriptor.supported_profiles {
                if !profile_applies(document, profile) {
                    continue;
                }
                let request = ExtractionRequest {
                    request_id: format!(
                        "operational:{}:{}:{}:cold",
                        document.id,
                        candidate_id,
                        profile.as_str()
                    ),
                    document_id: document.id.clone(),
                    artifact_id: format!("benchmark-artifact-{}", document.id),
                    artifact_revision_id: 1,
                    input_path: absolute_utf8(&corpus_root.join(&document.file))?,
                    input_sha256: document.sha256.clone(),
                    media_type: "application/pdf".to_owned(),
                    profile: profile.clone(),
                    forced_ocr_pages: document.expected_ocr_pages.clone(),
                    zotero_baseline_path: document
                        .zotero_baseline
                        .as_ref()
                        .map(|path| absolute_utf8(&corpus_root.join(path)))
                        .transpose()?,
                };
                let timeout_context = timeout_context(document, profile, &policy);
                match measure_adapter(&executable, &descriptor, request, timeout_context) {
                    Ok(report) => reports.push(report),
                    Err(error) => candidate_errors.push(format!(
                        "{}:{}:{}: {error}",
                        candidate_id,
                        profile.as_str(),
                        document.id
                    )),
                }
            }
        }
    }
    reports.sort_by(|left, right| {
        (
            left.candidate_id.as_str(),
            left.profile.as_str(),
            left.document_id.as_str(),
        )
            .cmp(&(
                right.candidate_id.as_str(),
                right.profile.as_str(),
                right.document_id.as_str(),
            ))
    });
    let result = OperationalResultSet {
        schema_version: "graphite-pdf-benchmark-operational-v1".to_owned(),
        corpus_id: manifest.corpus_id,
        environment_id: environment_id(),
        operating_system_cache_control:
            "not-controlled; cold means a fresh adapter process, not a cleared OS page cache"
                .to_owned(),
        reports,
    };
    validate_operational_privacy(&result).map_err(|errors| errors.join("\n"))?;
    write_json_atomic(&output, &result)?;
    if candidate_errors.is_empty() {
        println!(
            "wrote privacy-safe operational metrics to {}",
            output.display()
        );
        Ok(())
    } else {
        Err(format!(
            "operational measurements were partially preserved:\n{}",
            candidate_errors.join("\n")
        ))
    }
}

fn measure_adapter(
    executable: &Path,
    descriptor: &CandidateDescriptor,
    request: ExtractionRequest,
    timeout_context: TimeoutContext,
) -> Result<OperationalReport, String> {
    let temporary_root = measurement_temporary_root()?;
    fs::create_dir_all(&temporary_root).map_err(|error| {
        format!(
            "cannot create operational temporary directory {}: {error}",
            temporary_root.display()
        )
    })?;
    let result = measure_adapter_in_temporary_root(
        executable,
        descriptor,
        request,
        timeout_context,
        &temporary_root,
    );
    remove_measurement_temporary_root(&temporary_root)?;
    result
}

fn measure_adapter_in_temporary_root(
    executable: &Path,
    descriptor: &CandidateDescriptor,
    request: ExtractionRequest,
    timeout_context: TimeoutContext,
    temporary_root: &Path,
) -> Result<OperationalReport, String> {
    let timeout = Duration::from_millis(timeout_context.ceiling_ms);
    let process_started = Instant::now();
    let mut command = benchmark_command(executable);
    command
        .env("TMPDIR", temporary_root)
        .env("TMP", temporary_root)
        .env("TEMP", temporary_root);
    let mut child = ManagedChild::new(
        command
            .spawn()
            .map_err(|error| format!("cannot start {}: {error}", executable.display()))?,
    );
    let process_id = child.id();
    let mut stdin = child
        .take_stdin()
        .ok_or_else(|| "adapter stdin is unavailable".to_owned())?;
    let stdout = child
        .take_stdout()
        .ok_or_else(|| "adapter stdout is unavailable".to_owned())?;
    let (response_sender, response_receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let result = line
                .map_err(|error| format!("cannot read adapter response: {error}"))
                .and_then(|line| {
                    let byte_count = line.len() as u64 + 1;
                    serde_json::from_str::<ProtocolResponse>(&line)
                        .map(|response| (response, byte_count))
                        .map_err(|error| format!("invalid adapter response: {error}"))
                });
            let failed = result.is_err();
            if response_sender.send(result).is_err() || failed {
                break;
            }
        }
    });
    let phase = Arc::new(AtomicU8::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let peaks = Arc::new(Mutex::new([PhasePeaks::default(), PhasePeaks::default()]));
    let sampler = start_resource_sampler(
        process_id,
        temporary_root.to_owned(),
        Arc::clone(&phase),
        Arc::clone(&stop),
        Arc::clone(&peaks),
    );
    send_request(
        &mut stdin,
        &ProtocolRequest::Handshake {
            protocol_version: 1,
        },
    )?;
    let (handshake_response, _) = receive_protocol_response(&response_receiver, timeout)?;
    match handshake_response {
        ProtocolResponse::Handshake { candidate, .. }
            if candidate.candidate_id == descriptor.candidate_id => {}
        ProtocolResponse::Handshake { candidate, .. } => {
            return Err(format!(
                "adapter changed identity from {} to {}",
                descriptor.candidate_id, candidate.candidate_id
            ));
        }
        ProtocolResponse::Error { message } => return Err(message),
        other => return Err(format!("unexpected handshake response {other:?}")),
    }
    send_request(
        &mut stdin,
        &ProtocolRequest::Extract {
            request: Box::new(request.clone()),
        },
    )?;
    let (cold_response, cold_bytes) = match receive_protocol_response(&response_receiver, timeout) {
        Ok(response) => response,
        Err(error) if error == "adapter timed out" => {
            let cold_wall = process_started.elapsed();
            let cold_cpu = process_tree_snapshot(process_id);
            record_phase_peak(&peaks, 0, cold_cpu, directory_sizes(temporary_root).0);
            sampler.finish()?;
            let phase_peaks = *peaks
                .lock()
                .map_err(|_| "resource sampler state is poisoned".to_owned())?;
            let (asset_logical_bytes, asset_allocated_bytes) = candidate_asset_sizes(descriptor)?;
            let mut notes = measurement_notes();
            notes.push(
                "cold extraction reached the frozen timeout; no warm request was attempted"
                    .to_owned(),
            );
            return Ok(OperationalReport {
                document_id: request.document_id,
                candidate_id: descriptor.candidate_id.clone(),
                candidate_version: descriptor.version.clone(),
                profile: request.profile,
                timeout_class: timeout_context.class,
                timeout_stage: timeout_context.stage,
                timeout_ceiling_ms: timeout_context.ceiling_ms,
                cold_start: timeout_phase_measurement(cold_wall, None, cold_cpu, phase_peaks[0]),
                warm_extraction: None,
                adapter_binary_bytes: fs::metadata(executable)
                    .map(|metadata| metadata.len())
                    .unwrap_or_default(),
                asset_logical_bytes,
                asset_allocated_bytes,
                measurement_notes: notes,
            });
        }
        Err(error) => return Err(error),
    };
    let cold_wall = process_started.elapsed();
    let cold_cpu = process_tree_snapshot(process_id);
    record_phase_peak(&peaks, 0, cold_cpu, directory_sizes(temporary_root).0);
    phase.store(1, Ordering::Release);
    let warm_started = Instant::now();
    let mut warm_request = request.clone();
    warm_request.request_id = request.request_id.replace(":cold", ":warm");
    send_request(
        &mut stdin,
        &ProtocolRequest::Extract {
            request: Box::new(warm_request),
        },
    )?;
    let warm_result = receive_protocol_response(&response_receiver, timeout);
    let warm_wall = warm_started.elapsed();
    let warm_cpu = process_tree_snapshot(process_id);
    record_phase_peak(&peaks, 1, warm_cpu, directory_sizes(temporary_root).0);
    drop(stdin);
    if let Err(error) = &warm_result {
        if error == "adapter timed out" {
            sampler.finish()?;
            let phase_peaks = *peaks
                .lock()
                .map_err(|_| "resource sampler state is poisoned".to_owned())?;
            let cold = phase_measurement(
                cold_response,
                cold_bytes,
                cold_wall,
                None,
                cold_cpu,
                phase_peaks[0],
            );
            let (asset_logical_bytes, asset_allocated_bytes) = candidate_asset_sizes(descriptor)?;
            let mut notes = measurement_notes();
            notes.push("warm extraction reached the frozen timeout".to_owned());
            return Ok(OperationalReport {
                document_id: request.document_id,
                candidate_id: descriptor.candidate_id.clone(),
                candidate_version: descriptor.version.clone(),
                profile: request.profile,
                timeout_class: timeout_context.class,
                timeout_stage: timeout_context.stage,
                timeout_ceiling_ms: timeout_context.ceiling_ms,
                cold_start: cold,
                warm_extraction: Some(timeout_phase_measurement(
                    warm_wall,
                    cold_cpu,
                    warm_cpu,
                    phase_peaks[1],
                )),
                adapter_binary_bytes: fs::metadata(executable)
                    .map(|metadata| metadata.len())
                    .unwrap_or_default(),
                asset_logical_bytes,
                asset_allocated_bytes,
                measurement_notes: notes,
            });
        }
    }
    let (warm_response, warm_bytes) = warm_result?;
    let status = child.wait_to_completion()?;
    sampler.finish()?;
    let phase_peaks = *peaks
        .lock()
        .map_err(|_| "resource sampler state is poisoned".to_owned())?;
    let mut notes = measurement_notes();
    let cold = phase_measurement(
        cold_response,
        cold_bytes,
        cold_wall,
        None,
        cold_cpu,
        phase_peaks[0],
    );
    let warm = phase_measurement(
        warm_response,
        warm_bytes,
        warm_wall,
        cold_cpu,
        warm_cpu,
        phase_peaks[1],
    );
    let (asset_logical_bytes, asset_allocated_bytes) = candidate_asset_sizes(descriptor)?;
    if !status.success() {
        notes.push(format!(
            "adapter exited with status {status} after returning both responses"
        ));
    }
    Ok(OperationalReport {
        document_id: request.document_id,
        candidate_id: descriptor.candidate_id.clone(),
        candidate_version: descriptor.version.clone(),
        profile: request.profile,
        timeout_class: timeout_context.class,
        timeout_stage: timeout_context.stage,
        timeout_ceiling_ms: timeout_context.ceiling_ms,
        cold_start: cold,
        warm_extraction: Some(warm),
        adapter_binary_bytes: fs::metadata(executable)
            .map(|metadata| metadata.len())
            .unwrap_or_default(),
        asset_logical_bytes,
        asset_allocated_bytes,
        measurement_notes: notes,
    })
}

fn benchmark_command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .env("GRAPHITE_PDF_BENCH_OFFLINE", "1")
        .env("HF_HUB_OFFLINE", "1")
        .env("HUGGINGFACE_HUB_OFFLINE", "1");
    if let Ok(root) = env::var("GRAPHITE_PDF_BENCH_ASSET_ROOT") {
        command.env("GRAPHITE_PDF_BENCH_ASSET_ROOT", root);
    }
    command
}

fn receive_protocol_response(
    receiver: &mpsc::Receiver<Result<(ProtocolResponse, u64), String>>,
    timeout: Duration,
) -> Result<(ProtocolResponse, u64), String> {
    receiver
        .recv_timeout(timeout)
        .map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => "adapter timed out".to_owned(),
            mpsc::RecvTimeoutError::Disconnected => {
                "adapter response channel disconnected".to_owned()
            }
        })?
}

fn phase_measurement(
    response: ProtocolResponse,
    response_bytes: u64,
    wall_time: Duration,
    before_cpu: Option<ProcessTreeSnapshot>,
    after_cpu: Option<ProcessTreeSnapshot>,
    peaks: PhasePeaks,
) -> OperationalPhaseMeasurement {
    let (
        outcome,
        outcome_reason,
        adapter_reported_wall_time_ms,
        provider_time_ms,
        render_time_ms,
        ocr_time_ms,
    ) = match response {
        ProtocolResponse::Extraction { result } => (
            result.outcome,
            result.outcome_reason,
            Some(result.timing.wall_time_ms),
            result.timing.provider_time_ms,
            result.timing.render_time_ms,
            result.timing.ocr_time_ms,
        ),
        ProtocolResponse::Error { message } => (
            ExtractionOutcome::Failed,
            Some(message),
            None,
            None,
            None,
            None,
        ),
        other => (
            ExtractionOutcome::Failed,
            Some(format!("unexpected operational response {other:?}")),
            None,
            None,
            None,
            None,
        ),
    };
    let after_cpu = after_cpu.map(|mut snapshot| {
        snapshot.user_ticks = snapshot
            .user_ticks
            .max(peaks.user_ticks.unwrap_or_default());
        snapshot.system_ticks = snapshot
            .system_ticks
            .max(peaks.system_ticks.unwrap_or_default());
        snapshot
    });
    let cpu = cpu_delta(before_cpu, after_cpu);
    let wall_time_ms = wall_time.as_millis() as u64;
    let wall_time_microseconds = wall_time.as_micros() as u64;
    let total_cpu_ms = cpu.map(|value| value.0.saturating_add(value.1));
    OperationalPhaseMeasurement {
        outcome,
        outcome_reason,
        wall_time_ms,
        wall_time_microseconds,
        adapter_reported_wall_time_ms,
        provider_time_ms,
        render_time_ms,
        ocr_time_ms,
        user_cpu_time_ms: cpu.map(|value| value.0),
        system_cpu_time_ms: cpu.map(|value| value.1),
        cpu_utilization_percent: total_cpu_ms.and_then(|cpu_ms| {
            (wall_time_microseconds > 0)
                .then_some(cpu_ms as f64 * 100_000.0 / wall_time_microseconds as f64)
        }),
        peak_process_tree_rss_bytes: peaks.rss_bytes,
        peak_process_tree_thread_count: peaks.thread_count,
        peak_temporary_disk_bytes: peaks.temporary_bytes,
        response_bytes,
    }
}

fn timeout_phase_measurement(
    wall_time: Duration,
    before_cpu: Option<ProcessTreeSnapshot>,
    after_cpu: Option<ProcessTreeSnapshot>,
    peaks: PhasePeaks,
) -> OperationalPhaseMeasurement {
    let cpu = cpu_delta(before_cpu, after_cpu);
    let wall_time_ms = wall_time.as_millis() as u64;
    let wall_time_microseconds = wall_time.as_micros() as u64;
    let total_cpu_ms = cpu.map(|value| value.0.saturating_add(value.1));
    OperationalPhaseMeasurement {
        outcome: ExtractionOutcome::Timeout,
        outcome_reason: Some("adapter timed out".to_owned()),
        wall_time_ms,
        wall_time_microseconds,
        adapter_reported_wall_time_ms: None,
        provider_time_ms: None,
        render_time_ms: None,
        ocr_time_ms: None,
        user_cpu_time_ms: cpu.map(|value| value.0),
        system_cpu_time_ms: cpu.map(|value| value.1),
        cpu_utilization_percent: total_cpu_ms.and_then(|cpu_ms| {
            (wall_time_microseconds > 0)
                .then_some(cpu_ms as f64 * 100_000.0 / wall_time_microseconds as f64)
        }),
        peak_process_tree_rss_bytes: peaks.rss_bytes,
        peak_process_tree_thread_count: peaks.thread_count,
        peak_temporary_disk_bytes: peaks.temporary_bytes,
        response_bytes: 0,
    }
}

fn measurement_notes() -> Vec<String> {
    let mut notes = vec![
        "cold start includes process spawn, handshake, and first extraction".to_owned(),
        "warm extraction is the second request in the same adapter process".to_owned(),
        "OS page cache was not cleared or claimed to be controlled".to_owned(),
        "temporary disk sampling covers the isolated TMPDIR tree".to_owned(),
        "Linux process-tree and temporary-disk peaks are sampled every 10 ms; shorter-lived activity may be undercounted"
            .to_owned(),
    ];
    if !cfg!(target_os = "linux") {
        notes.push(
            "process-tree RSS, CPU, and thread measurements are unavailable on this platform"
                .to_owned(),
        );
    }
    notes
}

fn record_phase_peak(
    peaks: &Arc<Mutex<[PhasePeaks; 2]>>,
    phase_index: usize,
    snapshot: Option<ProcessTreeSnapshot>,
    temporary_bytes: u64,
) {
    let Ok(mut values) = peaks.lock() else {
        return;
    };
    let current = &mut values[phase_index];
    current.temporary_bytes = current.temporary_bytes.max(temporary_bytes);
    if let Some(snapshot) = snapshot {
        current.rss_bytes = Some(
            current
                .rss_bytes
                .unwrap_or_default()
                .max(snapshot.rss_bytes),
        );
        current.thread_count = Some(
            current
                .thread_count
                .unwrap_or_default()
                .max(snapshot.thread_count),
        );
        current.user_ticks = Some(
            current
                .user_ticks
                .unwrap_or_default()
                .max(snapshot.user_ticks),
        );
        current.system_ticks = Some(
            current
                .system_ticks
                .unwrap_or_default()
                .max(snapshot.system_ticks),
        );
    }
}

struct ManagedChild {
    child: Child,
    completed: bool,
}

impl ManagedChild {
    fn new(child: Child) -> Self {
        Self {
            child,
            completed: false,
        }
    }

    fn id(&self) -> u32 {
        self.child.id()
    }

    fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    fn wait_to_completion(&mut self) -> Result<std::process::ExitStatus, String> {
        let status = self
            .child
            .wait()
            .map_err(|error| format!("cannot wait for adapter: {error}"))?;
        self.completed = true;
        Ok(status)
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if !self.completed {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

struct ResourceSampler {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl ResourceSampler {
    fn finish(mut self) -> Result<(), String> {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .map_err(|_| "resource sampler panicked".to_owned())?;
        }
        Ok(())
    }
}

impl Drop for ResourceSampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PhasePeaks {
    rss_bytes: Option<u64>,
    thread_count: Option<u64>,
    user_ticks: Option<u64>,
    system_ticks: Option<u64>,
    temporary_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct ProcessTreeSnapshot {
    user_ticks: u64,
    system_ticks: u64,
    rss_bytes: u64,
    thread_count: u64,
}

fn start_resource_sampler(
    root_process_id: u32,
    temporary_root: PathBuf,
    phase: Arc<AtomicU8>,
    stop: Arc<AtomicBool>,
    peaks: Arc<Mutex<[PhasePeaks; 2]>>,
) -> ResourceSampler {
    let sampler_stop = Arc::clone(&stop);
    let handle = thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            let phase_index = usize::from(phase.load(Ordering::Acquire).min(1));
            let snapshot = process_tree_snapshot(root_process_id);
            let temporary_bytes = directory_sizes(&temporary_root).0;
            record_phase_peak(&peaks, phase_index, snapshot, temporary_bytes);
            thread::sleep(Duration::from_millis(10));
        }
    });
    ResourceSampler {
        stop: sampler_stop,
        handle: Some(handle),
    }
}

fn cpu_delta(
    before: Option<ProcessTreeSnapshot>,
    after: Option<ProcessTreeSnapshot>,
) -> Option<(u64, u64)> {
    let after = after?;
    let before = before.unwrap_or_default();
    let ticks_per_second = clock_ticks_per_second()?;
    Some((
        ticks_to_milliseconds(
            after.user_ticks.saturating_sub(before.user_ticks),
            ticks_per_second,
        ),
        ticks_to_milliseconds(
            after.system_ticks.saturating_sub(before.system_ticks),
            ticks_per_second,
        ),
    ))
}

fn ticks_to_milliseconds(ticks: u64, ticks_per_second: u64) -> u64 {
    ticks.saturating_mul(1_000) / ticks_per_second.max(1)
}

#[cfg(unix)]
fn clock_ticks_per_second() -> Option<u64> {
    let value = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    (value > 0).then_some(value as u64)
}

#[cfg(not(unix))]
fn clock_ticks_per_second() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn process_tree_snapshot(root_process_id: u32) -> Option<ProcessTreeSnapshot> {
    let mut processes = BTreeMap::new();
    for entry in fs::read_dir("/proc").ok()? {
        let Ok(entry) = entry else {
            continue;
        };
        let Some(process_id) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some((parent_id, user_ticks, system_ticks)) = parse_linux_stat(&stat) else {
            continue;
        };
        let Ok(status) = fs::read_to_string(entry.path().join("status")) else {
            continue;
        };
        let rss_bytes = linux_status_value(&status, "VmRSS:").unwrap_or_default() * 1_024;
        let thread_count = linux_status_value(&status, "Threads:").unwrap_or(1);
        processes.insert(
            process_id,
            (parent_id, user_ticks, system_ticks, rss_bytes, thread_count),
        );
    }
    if !processes.contains_key(&root_process_id) {
        return None;
    }
    let mut selected = BTreeSet::from([root_process_id]);
    loop {
        let before = selected.len();
        for (process_id, (parent_id, ..)) in &processes {
            if selected.contains(parent_id) {
                selected.insert(*process_id);
            }
        }
        if selected.len() == before {
            break;
        }
    }
    let mut snapshot = ProcessTreeSnapshot::default();
    for process_id in selected {
        let (_, user_ticks, system_ticks, rss_bytes, thread_count) = processes[&process_id];
        snapshot.user_ticks = snapshot.user_ticks.saturating_add(user_ticks);
        snapshot.system_ticks = snapshot.system_ticks.saturating_add(system_ticks);
        snapshot.rss_bytes = snapshot.rss_bytes.saturating_add(rss_bytes);
        snapshot.thread_count = snapshot.thread_count.saturating_add(thread_count);
    }
    Some(snapshot)
}

#[cfg(not(target_os = "linux"))]
fn process_tree_snapshot(_root_process_id: u32) -> Option<ProcessTreeSnapshot> {
    None
}

fn parse_linux_stat(value: &str) -> Option<(u32, u64, u64)> {
    let fields = value
        .get(value.rfind(')')? + 2..)?
        .split_whitespace()
        .collect::<Vec<_>>();
    Some((
        fields.get(1)?.parse().ok()?,
        fields.get(11)?.parse().ok()?,
        fields.get(12)?.parse().ok()?,
    ))
}

fn linux_status_value(value: &str, field: &str) -> Option<u64> {
    value.lines().find_map(|line| {
        line.strip_prefix(field)?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

fn candidate_asset_sizes(descriptor: &CandidateDescriptor) -> Result<(u64, Option<u64>), String> {
    let Some(root) = env::var_os("GRAPHITE_PDF_BENCH_ASSET_ROOT") else {
        return Ok((0, allocated_size_supported().then_some(0)));
    };
    let candidate_root = PathBuf::from(root).join(&descriptor.candidate_id);
    Ok(directory_sizes(&candidate_root))
}

fn directory_sizes(root: &Path) -> (u64, Option<u64>) {
    if !root.exists() {
        return (0, allocated_size_supported().then_some(0));
    }
    let mut logical = 0u64;
    let mut allocated = 0u64;
    let mut pending = vec![root.to_owned()];
    while let Some(path) = pending.pop() {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            let Ok(entries) = fs::read_dir(&path) else {
                continue;
            };
            pending.extend(entries.filter_map(Result::ok).map(|entry| entry.path()));
        } else if metadata.is_file() {
            logical = logical.saturating_add(metadata.len());
            allocated = allocated.saturating_add(allocated_file_bytes(&metadata));
        }
    }
    (logical, allocated_size_supported().then_some(allocated))
}

#[cfg(unix)]
fn allocated_file_bytes(metadata: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.blocks().saturating_mul(512)
}

#[cfg(not(unix))]
fn allocated_file_bytes(_metadata: &fs::Metadata) -> u64 {
    0
}

fn allocated_size_supported() -> bool {
    cfg!(unix)
}

fn measurement_temporary_root() -> Result<PathBuf, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system time is before the Unix epoch: {error}"))?
        .as_nanos();
    Ok(env::temp_dir().join(format!(
        "graphite-pdf-benchmark-measure-{}-{timestamp}",
        std::process::id()
    )))
}

fn remove_measurement_temporary_root(path: &Path) -> Result<(), String> {
    let expected_parent = env::temp_dir();
    let parent = path.parent();
    let valid_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.starts_with("graphite-pdf-benchmark-measure-"));
    if parent != Some(expected_parent.as_path()) || !valid_name {
        return Err("refusing to remove an unrecognized measurement directory".to_owned());
    }
    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|error| format!("cannot remove {}: {error}", path.display()))?;
    }
    Ok(())
}

fn compare_runs(arguments: &[String]) -> Result<(), String> {
    let left_root = required_path(arguments, 0, "first run directory")?;
    let right_root = required_path(arguments, 1, "second run directory")?;
    let left = deterministic_results(&left_root)?;
    let right = deterministic_results(&right_root)?;
    if left != right {
        let left_keys = left.keys().cloned().collect::<Vec<_>>();
        let right_keys = right.keys().cloned().collect::<Vec<_>>();
        return Err(format!(
            "normalized runs differ; first keys {left_keys:?}, second keys {right_keys:?}"
        ));
    }
    println!("normalized extraction output is deterministic across both runs");
    Ok(())
}

fn deterministic_results(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut results = BTreeMap::new();
    for path in extraction_files(root)? {
        let mut extraction: BenchmarkExtractionEnvelopeV1 = read_json(&path)?;
        extraction.timing = Default::default();
        let relative = path
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let normalized = serde_json::to_string(&extraction).map_err(|error| error.to_string())?;
        results.insert(relative, normalized);
    }
    Ok(results)
}

fn score_run(arguments: &[String]) -> Result<(), String> {
    let manifest_path = required_path(arguments, 0, "manifest")?;
    let run_dir = required_path(arguments, 1, "run directory")?;
    let output = required_path(arguments, 2, "aggregate output")?;
    let manifest = validate_corpus(&manifest_path)?;
    let corpus_root = manifest_path
        .parent()
        .ok_or_else(|| "manifest has no parent directory".to_owned())?;
    for document in &manifest.documents {
        let anchors: AnchorSet = read_json(&corpus_root.join(&document.anchors))?;
        validate_scoring_anchors(&anchors).map_err(|errors| errors.join("\n"))?;
    }
    let mut reports = Vec::new();
    for path in extraction_files(&run_dir)? {
        let extraction: BenchmarkExtractionEnvelopeV1 = read_json(&path)?;
        let document = manifest
            .documents
            .iter()
            .find(|document| document.id == extraction.document_id)
            .ok_or_else(|| format!("unknown document {} in run", extraction.document_id))?;
        let anchors: AnchorSet = read_json(&corpus_root.join(&document.anchors))?;
        let baseline = document
            .zotero_baseline
            .as_ref()
            .map(|path| read_json(&corpus_root.join(path)))
            .transpose()?;
        reports.push(score_extraction(
            document,
            &anchors,
            &extraction,
            baseline.as_ref(),
        ));
    }
    reports.sort_by(|left, right| {
        (
            left.candidate_id.as_str(),
            left.profile.as_str(),
            left.document_id.as_str(),
        )
            .cmp(&(
                right.candidate_id.as_str(),
                right.profile.as_str(),
                right.document_id.as_str(),
            ))
    });
    let aggregate = AggregateResult {
        schema_version: "graphite-pdf-benchmark-aggregate-v1".to_owned(),
        run_id: run_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("local-run")
            .to_owned(),
        corpus_id: manifest.corpus_id,
        environment_id: environment_id(),
        reports,
        source_document_count: manifest.documents.len(),
        generated_at_utc: "not-recorded-in-reproducible-summary".to_owned(),
    };
    validate_aggregate_privacy(&aggregate).map_err(|errors| errors.join("\n"))?;
    write_json_atomic(&output, &aggregate)?;
    println!(
        "wrote privacy-safe aggregate metrics to {}",
        output.display()
    );
    Ok(())
}

fn parse_adapters(values: &[String]) -> Result<BTreeMap<String, PathBuf>, String> {
    if values.is_empty() {
        return Err("at least one candidate=executable adapter is required".to_owned());
    }
    let mut adapters = BTreeMap::new();
    for value in values {
        let (candidate, executable) = value
            .split_once('=')
            .ok_or_else(|| format!("invalid adapter specification {value}"))?;
        if candidate.is_empty() || executable.is_empty() {
            return Err(format!("invalid adapter specification {value}"));
        }
        adapters.insert(candidate.to_owned(), PathBuf::from(executable));
    }
    Ok(adapters)
}

fn handshake(executable: &Path, timeout: Duration) -> Result<CandidateDescriptor, String> {
    let response = transact(
        executable,
        vec![ProtocolRequest::Handshake {
            protocol_version: 1,
        }],
        timeout,
    )?
    .into_iter()
    .next()
    .ok_or_else(|| "adapter returned no handshake response".to_owned())?;
    match response {
        ProtocolResponse::Handshake { candidate, .. } => Ok(candidate),
        ProtocolResponse::Error { message } => Err(message),
        other => Err(format!("unexpected handshake response {other:?}")),
    }
}

fn extract_once(
    executable: &Path,
    request: ExtractionRequest,
    timeout: Duration,
) -> Result<BenchmarkExtractionEnvelopeV1, ProcessFailure> {
    let responses = transact(
        executable,
        vec![
            ProtocolRequest::Handshake {
                protocol_version: 1,
            },
            ProtocolRequest::Extract {
                request: Box::new(request),
            },
        ],
        timeout,
    )
    .map_err(classify_process_failure)?;
    match responses.into_iter().nth(1) {
        Some(ProtocolResponse::Extraction { result }) => Ok(*result),
        Some(ProtocolResponse::Error { message }) => Err(ProcessFailure::Failed(message)),
        Some(other) => Err(ProcessFailure::Failed(format!(
            "unexpected extraction response {other:?}"
        ))),
        None => Err(ProcessFailure::Crashed(
            "adapter exited without an extraction response".to_owned(),
        )),
    }
}

fn transact(
    executable: &Path,
    requests: Vec<ProtocolRequest>,
    timeout: Duration,
) -> Result<Vec<ProtocolResponse>, String> {
    let mut command = benchmark_command(executable);
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start {}: {error}", executable.display()))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "adapter stdin is unavailable".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "adapter stdout is unavailable".to_owned())?;
    for request in &requests {
        send_request(&mut stdin, request)?;
    }
    drop(stdin);
    receive_responses(&mut child, stdout, requests.len(), timeout)
}

fn send_request(input: &mut ChildStdin, request: &ProtocolRequest) -> Result<(), String> {
    serde_json::to_writer(&mut *input, request).map_err(|error| error.to_string())?;
    input.write_all(b"\n").map_err(|error| error.to_string())?;
    input.flush().map_err(|error| error.to_string())
}

fn receive_responses(
    child: &mut Child,
    stdout: ChildStdout,
    expected: usize,
    timeout: Duration,
) -> Result<Vec<ProtocolResponse>, String> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut responses = Vec::new();
        for line in BufReader::new(stdout).lines().take(expected) {
            let line = match line {
                Ok(line) => line,
                Err(error) => {
                    let _ = sender.send(Err(format!("cannot read adapter response: {error}")));
                    return;
                }
            };
            match serde_json::from_str::<ProtocolResponse>(&line) {
                Ok(response) => responses.push(response),
                Err(error) => {
                    let _ = sender.send(Err(format!("invalid adapter response: {error}")));
                    return;
                }
            }
        }
        let _ = sender.send(Ok(responses));
    });
    match receiver.recv_timeout(timeout) {
        Ok(result) => {
            let status = child
                .wait()
                .map_err(|error| format!("cannot wait for adapter: {error}"))?;
            if !status.success() {
                return Err(format!("adapter crashed with status {status}"));
            }
            result
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = child.kill();
            let _ = child.wait();
            Err("adapter timed out".to_owned())
        }
        Err(error) => Err(format!("adapter response channel failed: {error}")),
    }
}

#[derive(Debug)]
enum ProcessFailure {
    Timeout(String),
    Crashed(String),
    Failed(String),
}

fn classify_process_failure(error: String) -> ProcessFailure {
    if error.contains("timed out") {
        ProcessFailure::Timeout(error)
    } else if error.contains("crashed") || error.contains("status") {
        ProcessFailure::Crashed(error)
    } else {
        ProcessFailure::Failed(error)
    }
}

fn failure_envelope(
    request: &ExtractionRequest,
    descriptor: &CandidateDescriptor,
    failure: ProcessFailure,
    timeout_context: &TimeoutContext,
) -> BenchmarkExtractionEnvelopeV1 {
    let (outcome, reason) = match failure {
        ProcessFailure::Timeout(reason) => (
            ExtractionOutcome::Timeout,
            format!(
                "{reason}; class={:?}; stage={:?}; ceiling_ms={}",
                timeout_context.class, timeout_context.stage, timeout_context.ceiling_ms
            ),
        ),
        ProcessFailure::Crashed(reason) => (ExtractionOutcome::Crashed, reason),
        ProcessFailure::Failed(reason) => (ExtractionOutcome::Failed, reason),
    };
    BenchmarkExtractionEnvelopeV1 {
        schema_version: BENCHMARK_SCHEMA_VERSION.to_owned(),
        request_id: request.request_id.clone(),
        document_id: request.document_id.clone(),
        input_sha256: request.input_sha256.clone(),
        candidate: descriptor.clone(),
        profile: request.profile.clone(),
        outcome,
        outcome_reason: Some(reason),
        searchable_projection: None,
        document: None,
        canonical_document: None,
        mapping_losses: Vec::new(),
        warnings: Vec::new(),
        observations: Default::default(),
        timing: Default::default(),
        diagnostics: BTreeMap::new(),
    }
}

fn extraction_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|value| value.to_str()) == Some("json") {
                paths.push(path);
            }
        }
    }
    paths.sort();
    Ok(paths)
}

fn absolute_utf8(path: &Path) -> Result<String, String> {
    path.canonicalize()
        .map_err(|error| format!("cannot resolve {}: {error}", path.display()))?
        .to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("path {} is not UTF-8", path.display()))
}

fn environment_id() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("local-{}-{seconds}", env::consts::ARCH)
}

#[derive(Clone, Debug)]
struct TimeoutContext {
    class: TimeoutClass,
    stage: TimeoutStage,
    ceiling_ms: u64,
}

fn read_timeout_policy(path: &Path) -> Result<TimeoutPolicy, String> {
    let policy: TimeoutPolicy = read_json(path)?;
    if policy.schema_version != 1 {
        return Err(format!(
            "unsupported timeout policy schema {}",
            policy.schema_version
        ));
    }
    if policy.ordinary_ms == 0
        || policy.scanned_ocr_ms == 0
        || policy.extreme_stress_ms == 0
        || policy.extreme_page_threshold == 0
        || policy.scanned_ocr_page_ratio.denominator == 0
        || policy.scanned_ocr_page_ratio.numerator > policy.scanned_ocr_page_ratio.denominator
    {
        return Err("timeout policy contains an invalid zero, threshold, or ratio".to_owned());
    }
    Ok(policy)
}

fn timeout_context(
    document: &CorpusDocument,
    profile: &BenchmarkProfile,
    policy: &TimeoutPolicy,
) -> TimeoutContext {
    let extreme = document
        .expected_pages
        .is_some_and(|pages| pages >= policy.extreme_page_threshold);
    let scanned_ratio_met = document.expected_pages.is_some_and(|pages| {
        document.expected_ocr_pages.len() as u64
            * u64::from(policy.scanned_ocr_page_ratio.denominator)
            >= u64::from(pages) * u64::from(policy.scanned_ocr_page_ratio.numerator)
    });
    let class = if extreme {
        TimeoutClass::ExtremeStress
    } else if profile != &BenchmarkProfile::Native && scanned_ratio_met {
        TimeoutClass::ScannedOcr
    } else {
        TimeoutClass::Ordinary
    };
    let ceiling_ms = match class {
        TimeoutClass::Ordinary => policy.ordinary_ms,
        TimeoutClass::ScannedOcr => policy.scanned_ocr_ms,
        TimeoutClass::ExtremeStress => policy.extreme_stress_ms,
    };
    let stage = match profile {
        BenchmarkProfile::Native => TimeoutStage::Parser,
        BenchmarkProfile::ForcedOcr => TimeoutStage::OcrPipeline,
        BenchmarkProfile::RecommendedAuto if !document.expected_ocr_pages.is_empty() => {
            TimeoutStage::OcrPipeline
        }
        BenchmarkProfile::RecommendedAuto => TimeoutStage::IndeterminatePipeline,
    };
    TimeoutContext {
        class,
        stage,
        ceiling_ms,
    }
}

fn profile_applies(document: &CorpusDocument, profile: &BenchmarkProfile) -> bool {
    profile != &BenchmarkProfile::ForcedOcr || !document.expected_ocr_pages.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_specification_requires_identifier_and_path() {
        assert!(parse_adapters(&["xberg=/tmp/xberg".to_owned()]).is_ok());
        assert!(parse_adapters(&["broken".to_owned()]).is_err());
    }

    #[test]
    fn deterministic_comparison_ignores_only_timing() {
        let directory = tempfile::tempdir().expect("temp directory");
        let left = directory.path().join("left/candidate/native");
        let right = directory.path().join("right/candidate/native");
        fs::create_dir_all(&left).expect("left directory");
        fs::create_dir_all(&right).expect("right directory");
        let request = ExtractionRequest {
            request_id: "request".to_owned(),
            document_id: "document".to_owned(),
            artifact_id: "artifact".to_owned(),
            artifact_revision_id: 1,
            input_path: "ignored".to_owned(),
            input_sha256: "0".repeat(64),
            media_type: "application/pdf".to_owned(),
            profile: graphite_pdf_benchmark_common::BenchmarkProfile::Native,
            forced_ocr_pages: Vec::new(),
            zotero_baseline_path: None,
        };
        let candidate = CandidateDescriptor {
            candidate_id: "candidate".to_owned(),
            version: "1".to_owned(),
            revision: "revision".to_owned(),
            license: "MIT".to_owned(),
            adapter_version: "1".to_owned(),
            features: Vec::new(),
            supported_profiles: vec![graphite_pdf_benchmark_common::BenchmarkProfile::Native],
            configured_available: true,
            availability_reason: None,
        };
        let mut first = BenchmarkExtractionEnvelopeV1::terminal(
            &request,
            candidate.clone(),
            ExtractionOutcome::Failed,
            "same",
        );
        let mut second = first.clone();
        first.timing.wall_time_ms = 1;
        second.timing.wall_time_ms = 99;
        write_json_atomic(&left.join("document.json"), &first).expect("left output");
        write_json_atomic(&right.join("document.json"), &second).expect("right output");
        assert_eq!(
            deterministic_results(&directory.path().join("left")).expect("left results"),
            deterministic_results(&directory.path().join("right")).expect("right results")
        );
    }

    #[test]
    fn forced_ocr_applies_only_to_human_reviewed_pages() {
        let mut document = CorpusDocument {
            id: "paper01".to_owned(),
            file: "local/paper01.pdf".to_owned(),
            sha256: "0".repeat(64),
            source_url: None,
            license: "local-evaluation-only".to_owned(),
            redistributable: false,
            categories: vec!["native".to_owned()],
            language: "en".to_owned(),
            expected_pages: Some(1),
            text_layer: graphite_pdf_benchmark_common::TextLayerKind::Native,
            expected_ocr_pages: Vec::new(),
            anchors: "anchors/paper01.json".to_owned(),
            anchors_sha256: "0".repeat(64),
            zotero_baseline: None,
            zotero_baseline_sha256: None,
        };

        assert!(profile_applies(&document, &BenchmarkProfile::Native));
        assert!(profile_applies(
            &document,
            &BenchmarkProfile::RecommendedAuto
        ));
        assert!(!profile_applies(&document, &BenchmarkProfile::ForcedOcr));

        document.expected_ocr_pages = vec![1];
        assert!(profile_applies(&document, &BenchmarkProfile::ForcedOcr));
    }

    #[test]
    fn operational_timeout_remains_an_explicit_measurement() {
        let measurement = timeout_phase_measurement(
            Duration::from_millis(90_000),
            None,
            None,
            PhasePeaks {
                rss_bytes: Some(1_024),
                thread_count: Some(2),
                temporary_bytes: 512,
                ..PhasePeaks::default()
            },
        );

        assert_eq!(measurement.outcome, ExtractionOutcome::Timeout);
        assert_eq!(measurement.wall_time_ms, 90_000);
        assert_eq!(measurement.peak_process_tree_rss_bytes, Some(1_024));
        assert_eq!(measurement.peak_process_tree_thread_count, Some(2));
        assert_eq!(measurement.peak_temporary_disk_bytes, 512);
        assert_eq!(measurement.response_bytes, 0);
    }

    #[test]
    fn timeout_policy_classifies_ordinary_scanned_and_stress_documents() {
        let policy = TimeoutPolicy {
            schema_version: 1,
            ordinary_ms: 90_000,
            scanned_ocr_ms: 120_000,
            extreme_stress_ms: 120_000,
            extreme_page_threshold: 500,
            scanned_ocr_page_ratio: graphite_pdf_benchmark_common::TimeoutRatio {
                numerator: 1,
                denominator: 2,
            },
        };
        let mut document = fixture_document();
        let ordinary = timeout_context(&document, &BenchmarkProfile::Native, &policy);
        assert_eq!(ordinary.class, TimeoutClass::Ordinary);
        assert_eq!(ordinary.stage, TimeoutStage::Parser);
        assert_eq!(ordinary.ceiling_ms, 90_000);

        document.expected_ocr_pages = vec![1];
        let scanned = timeout_context(&document, &BenchmarkProfile::RecommendedAuto, &policy);
        assert_eq!(scanned.class, TimeoutClass::ScannedOcr);
        assert_eq!(scanned.stage, TimeoutStage::OcrPipeline);

        document.expected_pages = Some(1_312);
        let stress = timeout_context(&document, &BenchmarkProfile::ForcedOcr, &policy);
        assert_eq!(stress.class, TimeoutClass::ExtremeStress);
        assert_eq!(stress.ceiling_ms, 120_000);
    }

    #[test]
    fn zotero_header_parser_uses_the_final_case_insensitive_value() {
        let headers = "HTTP/1.1 302 Found\r\nZotero-Server-ID: old\r\n\r\nHTTP/1.1 200 OK\r\nzotero-server-id: current\r\nLast-Modified-Version: 42\r\n";
        assert_eq!(
            response_header(headers, "Zotero-Server-ID").as_deref(),
            Some("current")
        );
        assert_eq!(
            response_header(headers, "last-modified-version").as_deref(),
            Some("42")
        );
    }

    fn fixture_document() -> CorpusDocument {
        CorpusDocument {
            id: "paper01".to_owned(),
            file: "local/paper01.pdf".to_owned(),
            sha256: "0".repeat(64),
            source_url: None,
            license: "local-evaluation-only".to_owned(),
            redistributable: false,
            categories: vec!["native".to_owned()],
            language: "en".to_owned(),
            expected_pages: Some(1),
            text_layer: graphite_pdf_benchmark_common::TextLayerKind::Native,
            expected_ocr_pages: Vec::new(),
            anchors: "anchors/paper01.json".to_owned(),
            anchors_sha256: "0".repeat(64),
            zotero_baseline: None,
            zotero_baseline_sha256: None,
        }
    }
}
