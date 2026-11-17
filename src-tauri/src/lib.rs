mod agent;
mod bibliography;
mod citation;
mod commands;
mod corpus;
mod database;
mod demo;
mod document;
mod error;
#[cfg(feature = "pdf-search-experiment")]
pub mod experiments;
pub mod extraction;
pub mod knowledge;
mod model;
mod normalize;
#[cfg(test)]
mod real_demo_tests;
mod semantic;
mod sync;
mod zotero;

use corpus::CorpusCoordinator;
use database::ResearchDatabase;
use document::{respond_to_document_request, DocumentCoordinator};
use semantic::SemanticCoordinator;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use sync::SyncCoordinator;
use tauri::Manager;
use tokio::sync::Mutex;

use agent::AgentCoordinator;
use citation::CitationCoordinator;
use extraction::shadow::ShadowExtractionCoordinator;

const CURRENT_APP_IDENTIFIER: &str = "org.graphite.research";
const DEVELOPMENT_APP_IDENTIFIER: &str = "org.graphite.research.dev";
const LEGACY_APP_IDENTIFIER: &str = "org.graphy.research";
const CURRENT_DATABASE_FILENAME: &str = "graphite-research.sqlite3";
const LEGACY_DATABASE_FILENAME: &str = "graphy-research.sqlite3";
const CURRENT_SEMANTIC_MARKER: &str = "graphite-semantic-model.json";
const LEGACY_SEMANTIC_MARKER: &str = "graphy-semantic-model.json";
const SEMANTIC_MODEL_DIRECTORY: &str = "semantic-models";
const SEMANTIC_INDEX_DIRECTORY: &str = "semantic-indexes";
const AGENT_MODEL_DIRECTORY: &str = "agent-models";

#[derive(Debug, Eq, PartialEq)]
struct ApplicationStoragePaths {
    data_root: PathBuf,
    semantic_model_root: PathBuf,
    semantic_index_root: PathBuf,
    agent_model_root: PathBuf,
}

#[derive(Clone)]
struct AppState {
    database: ResearchDatabase,
    demo: demo::DemoCoordinator,
    sync: SyncCoordinator,
    corpus: CorpusCoordinator,
    documents: DocumentCoordinator,
    citations: CitationCoordinator,
    semantic: SemanticCoordinator,
    agent: AgentCoordinator,
    shadow: ShadowExtractionCoordinator,
    docling: extraction::docling::DoclingCoordinator,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .register_asynchronous_uri_scheme_protocol(
            "graphite-document",
            |context, request, responder| {
                let database = context.app_handle().state::<AppState>().database.clone();
                respond_to_document_request(database, request, responder);
            },
        )
        .setup(|app| {
            let app_identifier = app.config().identifier.clone();
            let development_or_debug_build = cfg!(dev) || cfg!(debug_assertions);
            validate_runtime_identity(&app_identifier, development_or_debug_build)?;
            let app_data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| format!("Graphite app-data directory failed: {error}"))?;
            let app_local_data_dir = app
                .path()
                .app_local_data_dir()
                .map_err(|error| format!("Graphite local-data directory failed: {error}"))?;
            let storage = prepare_application_storage_paths(
                &app_identifier,
                &app_data_dir,
                &app_local_data_dir,
            )?;
            let database =
                ResearchDatabase::initialize(storage.data_root.join(CURRENT_DATABASE_FILENAME))
                    .map_err(|error| error.to_string())?;
            let operation_guard = Arc::new(Mutex::new(()));
            let demo = demo::DemoCoordinator::new(
                database.clone(),
                storage.data_root.join("real-demo-pdfs"),
                operation_guard.clone(),
            );
            let sync = SyncCoordinator::new(database.clone(), operation_guard.clone())
                .map_err(|error| error.to_string())?;
            let corpus = CorpusCoordinator::new(database.clone(), operation_guard.clone())
                .map_err(|error| error.to_string())?;
            let documents =
                DocumentCoordinator::new(database.clone()).map_err(|error| error.to_string())?;
            let citations = CitationCoordinator::new(database.clone(), operation_guard.clone());
            let semantic = SemanticCoordinator::new(
                database.clone(),
                storage.semantic_model_root,
                storage.semantic_index_root,
            )
            .map_err(|error| error.to_string())?;
            let agent =
                AgentCoordinator::new(database.clone(), semantic.clone(), storage.agent_model_root)
                    .map_err(|error| error.to_string())?;
            let shadow = ShadowExtractionCoordinator::new(database.clone())
                .map_err(|error| error.to_string())?;
            let docling = extraction::docling::DoclingCoordinator::new(
                database.clone(),
                storage.data_root.join("docling"),
            )
            .map_err(|error| error.to_string())?;
            app.manage(AppState {
                database,
                demo,
                sync,
                corpus,
                documents,
                citations,
                semantic,
                agent,
                shadow,
                docling,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::pdf_export::export_pdf_copy,
            commands::demo::activate_demo_library,
            commands::demo::activate_zotero_library,
            commands::demo::get_demo_status,
            commands::demo::get_demo_archive_info,
            commands::demo::download_demo_pdfs,
            commands::demo::cancel_demo_preparation,
            commands::demo::import_demo_pdfs,
            commands::demo::store_demo_document_pages,
            commands::library::load_workspace,
            commands::lenses::list_lenses,
            commands::lenses::save_lens,
            commands::lenses::delete_lens,
            commands::library::probe_zotero,
            commands::library::sync_zotero,
            commands::library::rebuild_zotero_cache,
            commands::library::set_active_library,
            commands::semantic::search_library,
            commands::semantic::install_semantic_model,
            commands::semantic::index_semantic_corpus,
            commands::semantic::rebuild_semantic_index,
            commands::semantic::cancel_semantic_indexing,
            commands::semantic::query_similar_papers,
            commands::semantic::get_paper_semantic_coverage,
            commands::semantic::get_semantic_relationship_evidence,
            commands::library::query_library_items,
            commands::library::load_graph_projection,
            commands::document::sync_document_corpus,
            commands::document::rebuild_document_index,
            commands::document::resolve_document,
            commands::document::get_library_preparation_documents,
            commands::document::store_document_pdf_pages,
            commands::document::get_docling_status,
            commands::document::index_document_extraction,
            commands::document::extract_document_docling,
            commands::document::cancel_document_docling,
            commands::document::select_document_extraction,
            commands::document::get_document_text,
            commands::document::get_reader_state,
            commands::document::save_reader_state,
            commands::citation::index_citation_corpus,
            commands::citation::rebuild_citation_index,
            commands::citation::reresolve_citations,
            commands::citation::query_references,
            commands::citation::query_cited_by,
            commands::citation::query_shared_references,
            commands::citation::query_co_cited,
            commands::citation::load_research_neighborhood,
            commands::citation::get_relationship_evidence,
            commands::citation::get_connection_details,
            commands::citation::review_reference_resolution,
            commands::citation::query_missing_works,
            commands::agent::get_agent_environment,
            commands::agent::save_agent_settings,
            commands::agent::install_agent_model,
            commands::agent::unload_agent_model,
            commands::agent::check_external_agent_provider,
            commands::agent::run_research_agent,
            commands::agent::cancel_research_agent,
            commands::agent::list_agent_conversations,
            commands::agent::load_agent_conversation,
            commands::agent::delete_agent_conversation,
            commands::agent::export_agent_conversation,
            commands::shadow::run_pdf_shadow_pilot,
            commands::shadow::cancel_pdf_shadow_pilot,
        ])
        .run(tauri::generate_context!())
        .expect("Graphite failed while running the Tauri application");
}

fn validate_runtime_identity(
    identifier: &str,
    development_or_debug_build: bool,
) -> Result<(), String> {
    match (identifier, development_or_debug_build) {
        (DEVELOPMENT_APP_IDENTIFIER, _) | (CURRENT_APP_IDENTIFIER, false) => Ok(()),
        (CURRENT_APP_IDENTIFIER, true) => Err(concat!(
            "A Graphite development or debug build cannot use the production ",
            "application identity. Run the documented desktop development ",
            "command so development data remains isolated."
        )
        .to_owned()),
        _ => Err(format!(
            "Graphite was built with the unsupported application identity '{identifier}'."
        )),
    }
}

fn prepare_application_storage_paths(
    identifier: &str,
    app_data_dir: &Path,
    app_local_data_dir: &Path,
) -> Result<ApplicationStoragePaths, String> {
    let data_root = select_persistent_data_root(app_data_dir, app_local_data_dir)?;
    migrate_legacy_app_data_for_identity(identifier, app_data_dir, &data_root)?;
    Ok(resolve_application_storage_paths(app_data_dir, &data_root))
}

fn resolve_application_storage_paths(
    app_data_dir: &Path,
    data_root: &Path,
) -> ApplicationStoragePaths {
    let semantic_model_root = select_managed_directory(
        data_root.join(SEMANTIC_MODEL_DIRECTORY),
        &[app_data_dir.join(SEMANTIC_MODEL_DIRECTORY)],
    );
    let agent_model_root = select_managed_directory(
        data_root.join(AGENT_MODEL_DIRECTORY),
        &[app_data_dir.join(AGENT_MODEL_DIRECTORY)],
    );
    let semantic_index_root = select_managed_directory(
        data_root.join(SEMANTIC_INDEX_DIRECTORY),
        &[app_data_dir.join(SEMANTIC_INDEX_DIRECTORY)],
    );

    ApplicationStoragePaths {
        data_root: data_root.to_path_buf(),
        semantic_model_root,
        semantic_index_root,
        agent_model_root,
    }
}

fn select_persistent_data_root(
    app_data_dir: &Path,
    app_local_data_dir: &Path,
) -> Result<PathBuf, String> {
    if app_data_dir == app_local_data_dir {
        return Ok(app_local_data_dir.to_path_buf());
    }

    let roaming_has_database = has_current_database(app_data_dir);
    let local_has_database = has_current_database(app_local_data_dir);
    match (roaming_has_database, local_has_database) {
        (true, true) => {
            return Err(format!(
                "Graphite found databases in both '{}' and '{}'. Close Graphite, \
                 back up both directories, and resolve the duplicate data roots \
                 before starting the application.",
                app_data_dir.display(),
                app_local_data_dir.display()
            ));
        }
        (true, false) => return Ok(app_data_dir.to_path_buf()),
        (false, true) => return Ok(app_local_data_dir.to_path_buf()),
        (false, false) => {}
    }

    let roaming_has_state = has_application_state(app_data_dir);
    let local_has_state = has_application_state(app_local_data_dir);
    match (roaming_has_state, local_has_state) {
        (true, true) => Err(format!(
            "Graphite found application state in both '{}' and '{}'. Close \
             Graphite, back up both directories, and resolve the duplicate data \
             roots before starting the application.",
            app_data_dir.display(),
            app_local_data_dir.display()
        )),
        (true, false) => Ok(app_data_dir.to_path_buf()),
        (false, true) => Ok(app_local_data_dir.to_path_buf()),
        (false, false) => Ok(app_local_data_dir.to_path_buf()),
    }
}

fn has_current_database(root: &Path) -> bool {
    root.join(CURRENT_DATABASE_FILENAME).is_file()
}

fn has_application_state(root: &Path) -> bool {
    has_current_database(root)
        || root.join(SEMANTIC_MODEL_DIRECTORY).is_dir()
        || root.join(SEMANTIC_INDEX_DIRECTORY).is_dir()
        || root.join(AGENT_MODEL_DIRECTORY).is_dir()
}

fn select_managed_directory(preferred: PathBuf, existing: &[PathBuf]) -> PathBuf {
    if preferred.is_dir() {
        return preferred;
    }
    existing
        .iter()
        .find(|path| path.is_dir())
        .cloned()
        .unwrap_or(preferred)
}

fn migrate_legacy_app_data_for_identity(
    identifier: &str,
    app_data_dir: &Path,
    destination_root: &Path,
) -> Result<(), String> {
    if identifier != CURRENT_APP_IDENTIFIER {
        return Ok(());
    }
    if has_application_state(destination_root) {
        return Ok(());
    }
    let parent = app_data_dir.parent().ok_or_else(|| {
        format!(
            "Graphite cannot determine the parent of its application data directory: {}",
            app_data_dir.display()
        )
    })?;
    let legacy_root = parent.join(LEGACY_APP_IDENTIFIER);
    migrate_legacy_app_data(&legacy_root, destination_root)
}

fn migrate_legacy_app_data(legacy_root: &Path, app_data_dir: &Path) -> Result<(), String> {
    if legacy_root.is_dir() && legacy_root != app_data_dir {
        std::fs::create_dir_all(app_data_dir).map_err(|error| {
            format!("Graphite could not create its application data directory: {error}")
        })?;
        let entries = std::fs::read_dir(legacy_root).map_err(|error| {
            format!("Graphite could not inspect the previous application data: {error}")
        })?;
        let migrate_database = !app_data_dir.join(CURRENT_DATABASE_FILENAME).exists();
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!("Graphite could not inspect a previous data entry: {error}")
            })?;
            let entry_name = entry.file_name();
            if is_legacy_database_entry(&entry_name) && !migrate_database {
                continue;
            }
            let target = app_data_dir.join(migrated_entry_name(&entry_name));
            if target.exists() {
                continue;
            }
            std::fs::rename(entry.path(), &target).map_err(|error| {
                format!(
                    "Graphite could not migrate '{}' to '{}': {error}",
                    entry.path().display(),
                    target.display()
                )
            })?;
        }
    }

    migrate_named_file(
        app_data_dir,
        LEGACY_DATABASE_FILENAME,
        CURRENT_DATABASE_FILENAME,
    )?;
    migrate_named_file(
        &app_data_dir.join("semantic-models"),
        LEGACY_SEMANTIC_MARKER,
        CURRENT_SEMANTIC_MARKER,
    )?;
    Ok(())
}

fn migrated_entry_name(name: &OsStr) -> OsString {
    let Some(name) = name.to_str() else {
        return name.to_os_string();
    };
    let Some(suffix) = name.strip_prefix(LEGACY_DATABASE_FILENAME) else {
        return name.into();
    };
    format!("{CURRENT_DATABASE_FILENAME}{suffix}").into()
}

fn is_legacy_database_entry(name: &OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| name.starts_with(LEGACY_DATABASE_FILENAME))
}

fn migrate_named_file(directory: &Path, old_name: &str, new_name: &str) -> Result<(), String> {
    let old_path = directory.join(old_name);
    let new_path = directory.join(new_name);
    if !old_path.exists() || new_path.exists() {
        return Ok(());
    }
    std::fs::rename(&old_path, &new_path).map_err(|error| {
        format!(
            "Graphite could not migrate '{}' to '{}': {error}",
            old_path.display(),
            new_path.display()
        )
    })
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    fn temporary_identity_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "graphite-identity-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ))
    }

    #[test]
    fn repository_configs_separate_development_and_production_identities() {
        let production: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("production config");
        let development: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.dev.conf.json"))
                .expect("development config");

        assert_eq!(production["identifier"], CURRENT_APP_IDENTIFIER);
        assert_eq!(development["identifier"], DEVELOPMENT_APP_IDENTIFIER);
        assert_ne!(production["identifier"], development["identifier"]);
    }

    #[test]
    fn documented_desktop_command_uses_the_development_launcher() {
        let package: serde_json::Value =
            serde_json::from_str(include_str!("../../package.json")).expect("package metadata");

        assert_eq!(
            package["scripts"]["desktop:dev"],
            "node scripts/desktop-dev.mjs"
        );
    }

    #[test]
    fn runtime_identity_policy_fails_closed_for_production_development_runs() {
        assert!(validate_runtime_identity(DEVELOPMENT_APP_IDENTIFIER, true).is_ok());
        assert!(validate_runtime_identity(CURRENT_APP_IDENTIFIER, false).is_ok());
        assert!(validate_runtime_identity(CURRENT_APP_IDENTIFIER, true).is_err());
        assert!(validate_runtime_identity("org.example.unexpected", false).is_err());
    }

    #[test]
    fn new_install_uses_machine_local_storage() {
        let root = temporary_identity_root("new-storage");
        let roaming = root.join("roaming").join(CURRENT_APP_IDENTIFIER);
        let local = root.join("local").join(CURRENT_APP_IDENTIFIER);
        let paths = prepare_application_storage_paths(DEVELOPMENT_APP_IDENTIFIER, &roaming, &local)
            .expect("resolve new application storage");

        assert_eq!(paths.data_root, local);
        assert_eq!(
            paths.semantic_model_root,
            local.join(SEMANTIC_MODEL_DIRECTORY)
        );
        assert_eq!(paths.agent_model_root, local.join(AGENT_MODEL_DIRECTORY));
        assert_eq!(
            paths.semantic_index_root,
            local.join(SEMANTIC_INDEX_DIRECTORY)
        );
    }

    #[test]
    fn existing_application_storage_is_used_in_place() {
        let root = temporary_identity_root("existing-storage");
        let roaming = root.join("roaming").join(CURRENT_APP_IDENTIFIER);
        let local = root.join("local").join(CURRENT_APP_IDENTIFIER);
        std::fs::create_dir_all(roaming.join(SEMANTIC_MODEL_DIRECTORY))
            .expect("existing semantic model directory");
        std::fs::create_dir_all(roaming.join(AGENT_MODEL_DIRECTORY))
            .expect("existing agent model directory");
        std::fs::write(roaming.join(CURRENT_DATABASE_FILENAME), b"database")
            .expect("existing database");

        let paths = prepare_application_storage_paths(DEVELOPMENT_APP_IDENTIFIER, &roaming, &local)
            .expect("resolve existing application storage");

        assert_eq!(paths.data_root, roaming);
        assert_eq!(
            paths.semantic_model_root,
            roaming.join(SEMANTIC_MODEL_DIRECTORY)
        );
        assert_eq!(paths.agent_model_root, roaming.join(AGENT_MODEL_DIRECTORY));
        assert_eq!(
            std::fs::read(roaming.join(CURRENT_DATABASE_FILENAME)).expect("existing database"),
            b"database"
        );
        assert!(!local.exists());
        std::fs::remove_dir_all(root).expect("remove storage fixture");
    }

    #[test]
    fn database_takes_priority_over_auxiliary_state_in_another_root() {
        let root = temporary_identity_root("database-priority");
        let roaming = root.join("roaming").join(CURRENT_APP_IDENTIFIER);
        let local = root.join("local").join(CURRENT_APP_IDENTIFIER);
        std::fs::create_dir_all(&roaming).expect("roaming data directory");
        std::fs::write(roaming.join(CURRENT_DATABASE_FILENAME), b"database")
            .expect("roaming database");
        std::fs::create_dir_all(local.join(SEMANTIC_MODEL_DIRECTORY))
            .expect("local model directory");

        let data_root =
            select_persistent_data_root(&roaming, &local).expect("select database root");

        assert_eq!(data_root, roaming);
        std::fs::remove_dir_all(root).expect("remove storage fixture");
    }

    #[test]
    fn existing_roaming_models_and_indexes_are_reused_with_a_local_database() {
        let root = temporary_identity_root("split-existing-storage");
        let roaming = root.join("roaming").join(CURRENT_APP_IDENTIFIER);
        let local = root.join("local").join(CURRENT_APP_IDENTIFIER);
        std::fs::create_dir_all(roaming.join(SEMANTIC_MODEL_DIRECTORY))
            .expect("roaming semantic model directory");
        std::fs::create_dir_all(roaming.join(SEMANTIC_INDEX_DIRECTORY))
            .expect("roaming semantic index directory");
        std::fs::create_dir_all(roaming.join(AGENT_MODEL_DIRECTORY))
            .expect("roaming agent model directory");
        std::fs::create_dir_all(&local).expect("local data directory");
        std::fs::write(local.join(CURRENT_DATABASE_FILENAME), b"database").expect("local database");

        let paths = prepare_application_storage_paths(DEVELOPMENT_APP_IDENTIFIER, &roaming, &local)
            .expect("resolve compatible split storage");

        assert_eq!(paths.data_root, local);
        assert_eq!(
            paths.semantic_model_root,
            roaming.join(SEMANTIC_MODEL_DIRECTORY)
        );
        assert_eq!(
            paths.semantic_index_root,
            roaming.join(SEMANTIC_INDEX_DIRECTORY)
        );
        assert_eq!(paths.agent_model_root, roaming.join(AGENT_MODEL_DIRECTORY));
        std::fs::remove_dir_all(root).expect("remove storage fixture");
    }

    #[test]
    fn duplicate_databases_fail_without_modifying_either_root() {
        let root = temporary_identity_root("duplicate-databases");
        let roaming = root.join("roaming").join(CURRENT_APP_IDENTIFIER);
        let local = root.join("local").join(CURRENT_APP_IDENTIFIER);
        std::fs::create_dir_all(&roaming).expect("roaming data directory");
        std::fs::create_dir_all(&local).expect("local data directory");
        std::fs::write(roaming.join(CURRENT_DATABASE_FILENAME), b"roaming")
            .expect("roaming database");
        std::fs::write(local.join(CURRENT_DATABASE_FILENAME), b"local").expect("local database");

        let error = select_persistent_data_root(&roaming, &local)
            .expect_err("duplicate databases must fail closed");

        assert!(error.contains("databases in both"));
        assert_eq!(
            std::fs::read(roaming.join(CURRENT_DATABASE_FILENAME))
                .expect("roaming database remains"),
            b"roaming"
        );
        assert_eq!(
            std::fs::read(local.join(CURRENT_DATABASE_FILENAME)).expect("local database remains"),
            b"local"
        );
        std::fs::remove_dir_all(root).expect("remove storage fixture");
    }

    #[test]
    fn development_identity_never_runs_previous_product_migration() {
        let root = temporary_identity_root("development-migration");
        let legacy_root = root.join(LEGACY_APP_IDENTIFIER);
        let development_root = root.join(DEVELOPMENT_APP_IDENTIFIER);
        std::fs::create_dir_all(&legacy_root).expect("legacy data directory");
        std::fs::write(legacy_root.join(LEGACY_DATABASE_FILENAME), b"database")
            .expect("legacy database");

        migrate_legacy_app_data_for_identity(
            DEVELOPMENT_APP_IDENTIFIER,
            &development_root,
            &development_root,
        )
        .expect("skip development migration");

        assert!(legacy_root.join(LEGACY_DATABASE_FILENAME).is_file());
        assert!(!development_root.exists());
        std::fs::remove_dir_all(root).expect("remove migration fixture");
    }

    #[test]
    fn migrates_previous_application_data_without_overwriting_new_data() {
        let root = temporary_identity_root("move");
        let legacy_root = root.join(LEGACY_APP_IDENTIFIER);
        let current_root = root.join(CURRENT_APP_IDENTIFIER);
        std::fs::create_dir_all(legacy_root.join("semantic-models"))
            .expect("legacy data directory");
        std::fs::write(legacy_root.join(LEGACY_DATABASE_FILENAME), b"database")
            .expect("legacy database");
        std::fs::write(
            legacy_root.join(format!("{LEGACY_DATABASE_FILENAME}-wal")),
            b"write-ahead log",
        )
        .expect("legacy database sidecar");
        std::fs::write(
            legacy_root
                .join("semantic-models")
                .join(LEGACY_SEMANTIC_MARKER),
            b"model marker",
        )
        .expect("legacy semantic marker");
        std::fs::create_dir_all(&current_root).expect("current data directory");
        std::fs::write(legacy_root.join("existing.txt"), b"legacy")
            .expect("legacy conflict fixture");
        std::fs::write(current_root.join("existing.txt"), b"current")
            .expect("current conflict fixture");

        migrate_legacy_app_data_for_identity(CURRENT_APP_IDENTIFIER, &current_root, &current_root)
            .expect("migrate application data");

        assert_eq!(
            std::fs::read(current_root.join(CURRENT_DATABASE_FILENAME)).expect("migrated database"),
            b"database"
        );
        assert!(current_root
            .join(format!("{CURRENT_DATABASE_FILENAME}-wal"))
            .is_file());
        assert!(current_root
            .join("semantic-models")
            .join(CURRENT_SEMANTIC_MARKER)
            .is_file());
        assert_eq!(
            std::fs::read(current_root.join("existing.txt")).expect("preserved current fixture"),
            b"current"
        );
        assert!(legacy_root.join("existing.txt").is_file());
        std::fs::remove_dir_all(root).expect("remove migration fixture");
    }

    #[test]
    fn never_mixes_previous_database_sidecars_into_a_current_database() {
        let root = temporary_identity_root("database-conflict");
        let legacy_root = root.join(LEGACY_APP_IDENTIFIER);
        let current_root = root.join(CURRENT_APP_IDENTIFIER);
        std::fs::create_dir_all(&legacy_root).expect("legacy data directory");
        std::fs::create_dir_all(&current_root).expect("current data directory");
        std::fs::write(legacy_root.join(LEGACY_DATABASE_FILENAME), b"legacy")
            .expect("legacy database");
        std::fs::write(
            legacy_root.join(format!("{LEGACY_DATABASE_FILENAME}-wal")),
            b"legacy write-ahead log",
        )
        .expect("legacy database sidecar");
        std::fs::write(current_root.join(CURRENT_DATABASE_FILENAME), b"current")
            .expect("current database");

        migrate_legacy_app_data_for_identity(CURRENT_APP_IDENTIFIER, &current_root, &current_root)
            .expect("inspect application data");

        assert_eq!(
            std::fs::read(current_root.join(CURRENT_DATABASE_FILENAME))
                .expect("current database remains"),
            b"current"
        );
        assert!(!current_root
            .join(format!("{CURRENT_DATABASE_FILENAME}-wal"))
            .exists());
        assert!(legacy_root.join(LEGACY_DATABASE_FILENAME).is_file());
        assert!(legacy_root
            .join(format!("{LEGACY_DATABASE_FILENAME}-wal"))
            .is_file());
        std::fs::remove_dir_all(root).expect("remove migration fixture");
    }

    #[test]
    fn current_local_data_prevents_unrelated_legacy_migration() {
        let root = temporary_identity_root("current-before-legacy");
        let roaming = root.join("roaming").join(CURRENT_APP_IDENTIFIER);
        let legacy_root = root.join("roaming").join(LEGACY_APP_IDENTIFIER);
        let local = root.join("local").join(CURRENT_APP_IDENTIFIER);
        std::fs::create_dir_all(&legacy_root).expect("legacy data directory");
        std::fs::write(legacy_root.join(LEGACY_DATABASE_FILENAME), b"legacy")
            .expect("legacy database");
        std::fs::create_dir_all(&local).expect("local data directory");
        std::fs::write(local.join(CURRENT_DATABASE_FILENAME), b"current")
            .expect("current local database");

        let paths = prepare_application_storage_paths(CURRENT_APP_IDENTIFIER, &roaming, &local)
            .expect("resolve current storage without migration");

        assert_eq!(paths.data_root, local);
        assert_eq!(
            std::fs::read(legacy_root.join(LEGACY_DATABASE_FILENAME))
                .expect("legacy database remains"),
            b"legacy"
        );
        assert!(!roaming.exists());
        std::fs::remove_dir_all(root).expect("remove migration fixture");
    }

    #[test]
    fn production_migration_targets_the_selected_machine_local_root() {
        let root = temporary_identity_root("production-routing");
        let roaming = root.join("roaming").join(CURRENT_APP_IDENTIFIER);
        let legacy_root = root.join("roaming").join(LEGACY_APP_IDENTIFIER);
        let local = root.join("local").join(CURRENT_APP_IDENTIFIER);
        std::fs::create_dir_all(&legacy_root).expect("legacy data directory");
        std::fs::write(legacy_root.join(LEGACY_DATABASE_FILENAME), b"legacy")
            .expect("legacy database");

        let paths = prepare_application_storage_paths(CURRENT_APP_IDENTIFIER, &roaming, &local)
            .expect("migrate production data");

        assert_eq!(paths.data_root, local);
        assert_eq!(
            std::fs::read(local.join(CURRENT_DATABASE_FILENAME)).expect("migrated local database"),
            b"legacy"
        );
        assert!(!roaming.exists());
        assert!(!legacy_root.join(LEGACY_DATABASE_FILENAME).exists());
        std::fs::remove_dir_all(root).expect("remove migration fixture");
    }
}
