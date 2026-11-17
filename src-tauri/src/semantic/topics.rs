//! Discover persistent library-wide topics from existing paper embeddings.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::vector_index::VectorIndex;
use crate::error::{AppError, AppResult};
use crate::model::Topic;

const VERSION: &str = "semantic-topics-v1";
const FIT_LIMIT: usize = 512;
const MAX_TOPICS: usize = 12;
const ITERATIONS: usize = 24;
const SILHOUETTE_LIMIT: usize = 128;
// Reject weakly separated candidates; a narrow corpus may have no clear map.
// This engineering gate is not a classification confidence score.
const MIN_SILHOUETTE: f32 = 0.1;
const MIN_SIMILARITY: f32 = 0.4;
const MIN_MARGIN: f32 = 0.02;
const COLORS: [&str; 12] = [
    "#5dcfd1", "#7a9de8", "#a483db", "#d57983", "#5eb69f", "#d1a15c", "#d48767", "#72b8c7",
    "#b5a1ce", "#96ad80", "#c99d9d", "#8aa7bf",
];
const STOP: &str = "a an and are as at be by can for from how in into is it its of on or our that the their these this to using use used via was we were with without study studies results research analysis approach based method methods paper new application applications data model models";

#[derive(Serialize, Deserialize)]
struct Run {
    fingerprint: String,
    topics: Vec<Topic>,
    assignments: HashMap<String, String>,
    representatives: HashMap<String, Vec<String>>,
    signatures: HashMap<String, String>,
    analyzed: usize,
    eligible: usize,
}

struct Paper {
    id: String,
    text: String,
    key: u64,
    title: String,
    signature: String,
}

fn inputs(connection: &Connection, library: &str) -> AppResult<(String, Vec<Paper>, usize)> {
    let mut statement = connection.prepare(
        "SELECT i.id,i.title,COALESCE(i.abstract_text,''),i.source_version,
         p.vector_key,COALESCE(p.source_fingerprint,''),COALESCE(p.status,''),
         COALESCE(e.id,''),COALESCE(v.active,0)
         FROM research_items i
         LEFT JOIN embedding_profiles e ON e.active=1
         LEFT JOIN paper_semantic_representations p ON p.item_id=i.id AND p.embedding_profile_id=e.id
         LEFT JOIN semantic_vector_entries v ON v.vector_key=p.vector_key
         WHERE i.library_id=?1 AND i.is_trashed=0 ORDER BY i.id")?;
    let mut identity = VERSION.to_owned();
    let mut papers = Vec::new();
    let mut eligible = 0;
    for row in statement.query_map([library], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, i64>(8)?,
        ))
    })? {
        let (id, title, abstract_text, version, key, hash, status, profile, active) = row?;
        eligible += 1;
        identity.push_str(
            &serde_json::to_string(&(
                &id,
                &title,
                &abstract_text,
                version,
                &hash,
                &status,
                &profile,
                active,
            ))
            .map_err(|e| AppError::Semantic(e.to_string()))?,
        );
        if status == "active" && active == 1 {
            if let Some(key) = key {
                papers.push(Paper {
                    id,
                    text: format!("{title}. {abstract_text}"),
                    key: key as u64,
                    title: title.clone(),
                    signature: crate::database::content_hash(&format!(
                        "{profile}:{hash}:{title}:{abstract_text}:{version}"
                    )),
                });
            }
        }
    }
    Ok((crate::database::content_hash(&identity), papers, eligible))
}

pub(crate) fn unassigned() -> Topic {
    Topic {
        id: "uncategorized".into(),
        name: "Not analyzed / no clear topic".into(),
        short_name: "Unassigned".into(),
        color: "#929a9c".into(),
        origin: "uncategorized".into(),
        analyzed: None,
        eligible: None,
        representatives: Vec::new(),
    }
}

/// Load only a map matching the current source revisions and active profile.
pub(crate) fn load(
    connection: &Connection,
    library: &str,
) -> AppResult<(Vec<Topic>, HashMap<String, String>)> {
    let (fingerprint, current, eligible) = inputs(connection, library)?;
    let signatures: HashMap<_, _> = current.into_iter().map(|p| (p.id, p.signature)).collect();
    let value: Option<String> = connection
        .query_row(
            "SELECT value FROM app_meta WHERE key=?1",
            [format!("{VERSION}:{library}")],
            |row| row.get(0),
        )
        .optional()?;
    // Derived cache corruption is recoverable; the next refresh replaces it.
    let run = value
        .map(|value| serde_json::from_str::<Run>(&value))
        .transpose()
        .unwrap_or(None);
    let mut neutral = unassigned();
    neutral.analyzed = Some(signatures.len());
    neutral.eligible = Some(eligible);
    let failure: Option<String> = connection
        .query_row(
            "SELECT value FROM app_meta WHERE key=?1",
            [format!("{VERSION}:error:{library}")],
            |r| r.get(0),
        )
        .optional()?;
    if failure.is_some() {
        neutral.name = "Topic analysis failed. Retry library preparation.".into();
    }

    Ok(match run {
        Some(mut run) => {
            neutral.analyzed = Some(
                run.signatures
                    .iter()
                    .filter(|(id, signature)| signatures.get(*id) == Some(*signature))
                    .count(),
            );
            run.assignments.retain(|id, _| {
                signatures.get(id) == run.signatures.get(id) && signatures.contains_key(id)
            });
            if run.assignments.is_empty() {
                if failure.is_none() {
                    neutral.name = if run.fingerprint == fingerprint {
                        "No clear semantic groups".into()
                    } else {
                        "Topic map needs preparation".into()
                    };
                }
                (vec![neutral], HashMap::new())
            } else {
                let mut topics = vec![neutral];
                topics.extend(run.topics);
                (topics, run.assignments)
            }
        }
        _ => (vec![neutral], HashMap::new()),
    })
}

/// Compute a complete replacement in the existing indexing background task.
pub(super) fn refresh(
    connection: &Connection,
    library: &str,
    profile: &str,
    index: &dyn VectorIndex,
    cancelled: &AtomicBool,
) -> AppResult<bool> {
    match refresh_inner(connection, library, profile, index, cancelled) {
        Ok(changed) => {
            if cancelled.load(Ordering::Acquire) {
                return Ok(changed);
            }
            let cleared = connection.execute(
                "DELETE FROM app_meta WHERE key=?1",
                [format!("{VERSION}:error:{library}")],
            )?;
            Ok(changed || cleared > 0)
        }
        Err(error) => {
            let changed = connection.execute(
                "INSERT INTO app_meta(key,value) VALUES (?1,?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value
                 WHERE value<>excluded.value",
                params![format!("{VERSION}:error:{library}"), error.to_string()],
            )?;
            Ok(changed > 0)
        }
    }
}

fn refresh_inner(
    connection: &Connection,
    library: &str,
    profile: &str,
    index: &dyn VectorIndex,
    cancelled: &AtomicBool,
) -> AppResult<bool> {
    let (fingerprint, papers, eligible) = inputs(connection, library)?;
    let old: Option<String> = connection
        .query_row(
            "SELECT value FROM app_meta WHERE key=?1",
            [format!("{VERSION}:{library}")],
            |r| r.get(0),
        )
        .optional()?;
    if old
        .as_ref()
        .and_then(|v| serde_json::from_str::<Run>(v).ok())
        .is_some_and(|r| r.fingerprint == fingerprint)
    {
        return Ok(false);
    }
    let mut vectors = Vec::new();
    let mut available = Vec::new();
    for paper in papers {
        if cancelled.load(Ordering::Acquire) {
            return Ok(false);
        }
        if let Some(vector) = index.vector(profile, "paper", paper.key)? {
            if let Some(vector) = normalize(&vector) {
                vectors.push(vector);
                available.push(paper);
            }
        }
    }
    let centers = discover(&vectors, cancelled);
    if cancelled.load(Ordering::Acquire) {
        return Ok(false);
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, vector) in vectors.iter().enumerate() {
        let ranked = rank(vector, &centers);
        if let Some(&(best, similarity)) = ranked.first() {
            if similarity >= MIN_SIMILARITY
                && (ranked.len() == 1 || similarity - ranked[1].1 >= MIN_MARGIN)
            {
                groups.entry(best).or_default().push(i);
            }
        }
    }
    let texts: Vec<_> = available.iter().map(|p| phrases(&p.text)).collect();
    let mut frequencies = HashMap::new();
    for text in &texts {
        for phrase in text.keys() {
            *frequencies.entry(phrase.clone()).or_insert(0usize) += 1;
        }
    }
    let mut assignments = HashMap::new();
    let mut topics = Vec::new();
    let mut representatives = HashMap::new();
    let mut labels = std::collections::HashSet::new();
    for (center, members) in groups {
        if cancelled.load(Ordering::Acquire) {
            return Ok(false);
        }
        if members.len() < 2 {
            continue;
        }
        let label = label(&texts, &members, &labels, &frequencies);
        let Some(label) = label else {
            continue;
        };
        labels.insert(label.clone());
        let mut nearest = members.clone();
        nearest.sort_by(|a, b| {
            dot(&vectors[*b], &centers[center])
                .total_cmp(&dot(&vectors[*a], &centers[center]))
                .then(a.cmp(b))
        });
        let id = format!(
            "semantic-topic:{}",
            crate::database::content_hash(&available[nearest[0]].id)
        );
        representatives.insert(
            id.clone(),
            nearest
                .iter()
                .take(3)
                .map(|i| available[*i].id.clone())
                .collect::<Vec<_>>(),
        );
        for i in members {
            assignments.insert(available[i].id.clone(), id.clone());
        }
        topics.push(Topic {
            id,
            name: label.clone(),
            short_name: label,
            color: COLORS[topics.len() % COLORS.len()].into(),
            origin: "semantic-cluster".into(),
            analyzed: Some(available.len()),
            eligible: Some(eligible),
            representatives: nearest
                .iter()
                .take(3)
                .map(|i| available[*i].title.clone())
                .collect(),
        });
    }
    let run = Run {
        fingerprint,
        topics,
        assignments,
        representatives,
        signatures: available
            .iter()
            .map(|p| (p.id.clone(), p.signature.clone()))
            .collect(),
        analyzed: available.len(),
        eligible,
    };
    // One SQLite replacement publishes only complete results; failures retain the old row.
    if !cancelled.load(Ordering::Acquire) {
        connection.execute(
            "INSERT INTO app_meta(key,value) VALUES (?1,?2)
            ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![
                format!("{VERSION}:{library}"),
                serde_json::to_string(&run).map_err(|e| AppError::Semantic(e.to_string()))?
            ],
        )?;
    }
    Ok(true)
}

fn normalize(vector: &[f32]) -> Option<Vec<f32>> {
    let norm = dot(vector, vector).sqrt();
    (norm.is_finite() && norm > 0.0).then(|| vector.iter().map(|v| v / norm).collect())
}
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
fn rank(vector: &[f32], centers: &[Vec<f32>]) -> Vec<(usize, f32)> {
    let mut scores: Vec<_> = centers
        .iter()
        .enumerate()
        .map(|(i, c)| (i, dot(vector, c)))
        .collect();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    scores
}
fn discover(vectors: &[Vec<f32>], cancelled: &AtomicBool) -> Vec<Vec<f32>> {
    if vectors.len() < 4 {
        return Vec::new();
    }
    let fit: Vec<_> = (0..vectors.len())
        .step_by(vectors.len().div_ceil(FIT_LIMIT))
        .map(|i| vectors[i].clone())
        .collect();
    let mut winner = Vec::new();
    let mut best = MIN_SILHOUETTE;
    for k in 2..=MAX_TOPICS.min(fit.len() / 2) {
        if cancelled.load(Ordering::Acquire) {
            return Vec::new();
        }
        let mut centers = vec![fit[0].clone()];
        while centers.len() < k {
            let next = fit
                .iter()
                .enumerate()
                .min_by(|(ia, a), (ib, b)| {
                    rank(a, &centers)[0]
                        .1
                        .total_cmp(&rank(b, &centers)[0].1)
                        .then(ia.cmp(ib))
                })
                .unwrap()
                .0;
            centers.push(fit[next].clone());
        }
        let mut assignments = vec![usize::MAX; fit.len()];
        for _ in 0..ITERATIONS {
            if cancelled.load(Ordering::Acquire) {
                return Vec::new();
            }
            let next: Vec<_> = fit.iter().map(|v| rank(v, &centers)[0].0).collect();
            if assignments == next {
                break;
            }
            assignments = next;
            for (group, center) in centers.iter_mut().enumerate() {
                let mut sum = vec![0.0; center.len()];
                for (_, v) in fit
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| assignments[*i] == group)
                {
                    for (s, v) in sum.iter_mut().zip(v) {
                        *s += v;
                    }
                }
                if let Some(mean) = normalize(&sum) {
                    *center = mean;
                }
            }
        }
        let mut score = 0.0;
        let mut count = 0;
        for i in (0..fit.len()).step_by(fit.len().div_ceil(SILHOUETTE_LIMIT)) {
            let mut totals = vec![(0.0, 0usize); k];
            for j in 0..fit.len() {
                if i != j {
                    totals[assignments[j]].0 += 1.0 - dot(&fit[i], &fit[j]);
                    totals[assignments[j]].1 += 1;
                }
            }
            let own = totals[assignments[i]];
            if own.1 == 0 {
                count += 1;
                continue;
            }
            let a = own.0 / own.1 as f32;
            let b = totals
                .iter()
                .enumerate()
                .filter(|(g, t)| *g != assignments[i] && t.1 > 0)
                .map(|(_, t)| t.0 / t.1 as f32)
                .min_by(f32::total_cmp)
                .unwrap_or(a);
            score += (b - a) / a.max(b).max(f32::EPSILON);
            count += 1;
        }
        score /= count.max(1) as f32;
        if score > best {
            best = score;
            winner = centers;
        }
    }
    winner
}
fn phrases(text: &str) -> BTreeMap<String, usize> {
    let stop: std::collections::HashSet<_> = STOP.split_whitespace().collect();
    let lower = text.to_lowercase();
    let words: Vec<_> = lower
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .take(400)
        .collect();
    let mut phrases = BTreeMap::new();
    for n in 1..=3 {
        for phrase in words.windows(n) {
            if phrase.iter().all(|w| w.len() > 2 && !stop.contains(w)) {
                *phrases.entry(phrase.join(" ")).or_default() += 1;
            }
        }
    }
    phrases
}
fn label(
    texts: &[BTreeMap<String, usize>],
    members: &[usize],
    used: &std::collections::HashSet<String>,
    frequencies: &HashMap<String, usize>,
) -> Option<String> {
    let mut scores: BTreeMap<String, f32> = BTreeMap::new();
    for &i in members {
        for (phrase, count) in &texts[i] {
            let frequency = frequencies.get(phrase).copied().unwrap_or(1);
            let score = (*count as f32).ln_1p()
                * ((texts.len() as f32 + 1.0) / (frequency as f32 + 1.0)).ln_1p()
                * (phrase.split_whitespace().count() as f32).sqrt();
            *scores.entry(phrase.clone()).or_default() += score;
        }
    }
    scores
        .into_iter()
        .filter(|(s, _)| {
            !used.contains(s)
                && s.split_whitespace().count() >= 2
                && members
                    .iter()
                    .filter(|i| texts[**i].contains_key(s))
                    .count()
                    >= 2
        })
        .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(s, _)| s)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn semantic_groups_are_deterministic_and_do_not_force_homogeneous_data() {
        let cancelled = AtomicBool::new(false);
        let vectors = [
            vec![1.0, 0.0],
            vec![0.99, 0.01],
            vec![0.98, 0.02],
            vec![0.0, 1.0],
            vec![0.01, 0.99],
            vec![0.02, 0.98],
        ];
        let vectors: Vec<_> = vectors.iter().map(|v| normalize(v).unwrap()).collect();
        assert_eq!(
            discover(&vectors, &cancelled),
            discover(&vectors, &cancelled)
        );
        assert_eq!(discover(&vectors, &cancelled).len(), 2);
        assert!(discover(&vec![vec![1.0, 0.0]; 4], &cancelled).is_empty());
        assert!(discover(&vectors[..2], &cancelled).is_empty());
        cancelled.store(true, Ordering::Release);
        assert!(discover(&vectors, &cancelled).is_empty());
    }
    #[test]
    fn labels_use_distinctive_source_phrases() {
        let texts = vec![
            phrases("Surface wave tomography"),
            phrases("Surface wave dispersion"),
            phrases("Neural networks detection"),
        ];
        let value = label(&texts, &[0, 1], &Default::default(), &Default::default()).unwrap();
        assert!(value.contains("surface") || value.contains("wave"));
        assert!(normalize(&[f32::NAN]).is_none());
    }
    #[test]
    #[ignore = "requires GRAPHITE_TOPIC_MODEL_CACHE pointing to an already installed model; never installs"]
    fn real_demo_topic_quality_checkpoint() {
        use crate::semantic::{EmbeddingProvider, LocalFastEmbedProvider, SemanticCoordinator};
        let model = std::path::PathBuf::from(std::env::var("GRAPHITE_TOPIC_MODEL_CACHE").unwrap());
        assert!(LocalFastEmbedProvider::new(model.clone())
            .unwrap()
            .installed_metadata()
            .unwrap()
            .is_some());
        let directory = tempfile::tempdir().unwrap();
        let database =
            crate::database::ResearchDatabase::initialize(directory.path().join("test.sqlite3"))
                .unwrap();
        crate::demo::catalog::register(&database).unwrap();
        crate::demo::catalog::activate(&database, true).unwrap();
        let coordinator =
            SemanticCoordinator::new(database.clone(), model, directory.path().join("vectors"))
                .unwrap();
        coordinator.install_model().unwrap();
        let workspace = coordinator
            .index(&crate::model::SemanticIndexRequest {
                max_papers: Some(64),
                max_documents: Some(1),
            })
            .unwrap();
        let groups: Vec<_> = workspace
            .snapshot
            .topics
            .iter()
            .filter(|t| t.origin == "semantic-cluster")
            .collect();
        for group in &groups {
            println!(
                "Topic: {} | examples: {:?}",
                group.name, group.representatives
            );
        }
        println!(
            "Assigned {} / {} papers",
            workspace
                .snapshot
                .items
                .iter()
                .filter(|p| p.topics[0] != "uncategorized")
                .count(),
            workspace.snapshot.total_item_count
        );
        // The narrow title-only demo may correctly abstain rather than invent groups.
        if groups.is_empty() {
            assert_eq!(
                workspace.snapshot.topics[0].name,
                "No clear semantic groups"
            );
        }
        let provider = LocalFastEmbedProvider::new(std::path::PathBuf::from(
            std::env::var("GRAPHITE_TOPIC_MODEL_CACHE").unwrap(),
        ))
        .unwrap();
        let texts = [
            "Surface wave tomography measures crustal shear velocity",
            "Surface wave tomography from ambient noise correlations",
            "Surface wave dispersion imaging of crustal structure",
            "Cancer immunotherapy activates tumor-specific immune cells",
            "Cancer immunotherapy targets malignant tumor cells",
            "Tumor immunotherapy improves immune response against cancer",
            "Battery electrolytes enable lithium ion transport",
            "Lithium battery electrolytes and charge transport",
            "Lithium ion battery electrodes and electrolytes",
        ];
        let vectors = provider
            .embed_documents(&texts.iter().map(|s| s.to_string()).collect::<Vec<_>>())
            .unwrap();
        let centers = discover(&vectors, &AtomicBool::new(false));
        assert!(
            centers.len() >= 2,
            "independent mixed-topic fixture must separate"
        );
        assert_ne!(
            rank(&vectors[0], &centers)[0].0,
            rank(&vectors[3], &centers)[0].0
        );
        assert_ne!(
            rank(&vectors[0], &centers)[0].0,
            rank(&vectors[6], &centers)[0].0
        );

        assert!(groups.len() <= MAX_TOPICS);
        let connection = database.connect().unwrap();
        let original = load(&connection, crate::demo::catalog::LIBRARY_ID).unwrap();
        assert!(!refresh(
            &connection,
            crate::demo::catalog::LIBRARY_ID,
            &workspace.semantic.active_profile.unwrap().id,
            coordinator.vectors.as_ref(),
            &AtomicBool::new(false)
        )
        .unwrap());
        assert_eq!(
            original.1,
            load(&connection, crate::demo::catalog::LIBRARY_ID)
                .unwrap()
                .1
        );

        // Exercise labels and persistence through the same real embedding pipeline.
        let ids: Vec<String> = connection
            .prepare("SELECT id FROM research_items ORDER BY id LIMIT 9")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        connection
            .execute("UPDATE research_items SET is_trashed=1", [])
            .unwrap();
        for (id, text) in ids.iter().zip(texts) {
            connection
                .execute(
                    "UPDATE research_items SET title=?2,abstract_text='',is_trashed=0,
                 source_version=source_version+1 WHERE id=?1",
                    params![id, text],
                )
                .unwrap();
        }
        let mixed = coordinator
            .index(&crate::model::SemanticIndexRequest {
                max_papers: Some(64),
                max_documents: Some(1),
            })
            .unwrap();
        let labels: Vec<_> = mixed
            .snapshot
            .topics
            .iter()
            .filter(|t| t.origin == "semantic-cluster")
            .map(|t| t.name.as_str())
            .collect();
        println!("Mixed-subject source labels: {labels:?}");
        assert!(labels.iter().any(|label| label.contains("surface wave")));
        assert!(labels
            .iter()
            .any(|label| label.contains("cancer immunotherapy")));
        assert!(labels.iter().any(|label| label.contains("battery")));
        assert!(mixed
            .snapshot
            .items
            .iter()
            .all(|p| p.topics[0] != "uncategorized"));
    }
}
