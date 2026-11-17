use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::model::RawObject;

const CHILD_ITEM_TYPES: &[&str] = &["attachment", "note", "annotation"];

#[derive(Clone, Debug)]
pub struct NormalizedCollection {
    pub key: String,
    pub version: i64,
    pub name: String,
    pub parent_key: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NormalizedCreator {
    pub position: usize,
    pub creator_type: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub field_mode_name: Option<String>,
    pub display_name: String,
}

#[derive(Clone, Debug)]
pub struct NormalizedTag {
    pub value: String,
    pub tag_type: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct NormalizedRelation {
    pub predicate: String,
    pub target: String,
}

#[derive(Clone, Debug)]
pub struct NormalizedItem {
    pub key: String,
    pub version: i64,
    pub item_type: String,
    pub parent_key: Option<String>,
    pub title: String,
    pub abstract_text: Option<String>,
    pub date: Option<String>,
    pub year: Option<i32>,
    pub venue: Option<String>,
    pub volume: Option<String>,
    pub issue: Option<String>,
    pub pages: Option<String>,
    pub publisher: Option<String>,
    pub doi: Option<String>,
    pub isbn: Option<String>,
    pub issn: Option<String>,
    pub url: Option<String>,
    pub date_added: Option<String>,
    pub date_modified: Option<String>,
    pub is_trashed: bool,
    pub creators: Vec<NormalizedCreator>,
    pub tags: Vec<NormalizedTag>,
    pub collection_keys: Vec<String>,
    pub relations: Vec<NormalizedRelation>,
    pub note_content: Option<String>,
    pub content_type: Option<String>,
    pub filename: Option<String>,
    pub link_mode: Option<String>,
    pub attachment_available: bool,
}

impl NormalizedItem {
    pub fn is_top_level_research_item(&self) -> bool {
        !CHILD_ITEM_TYPES.contains(&self.item_type.as_str()) && self.parent_key.is_none()
    }
}

pub fn normalize_collection(object: &RawObject) -> AppResult<NormalizedCollection> {
    let data = object_data(object)?;
    Ok(NormalizedCollection {
        key: object.key.clone(),
        version: object.version,
        name: text(data, "name").unwrap_or_else(|| "Untitled collection".to_owned()),
        parent_key: optional_text(data, "parentCollection"),
    })
}

pub fn normalize_item(object: &RawObject) -> AppResult<NormalizedItem> {
    let data = object_data(object)?;
    let item_type = text(data, "itemType").unwrap_or_else(|| "document".to_owned());
    let title = text(data, "title")
        .or_else(|| text(data, "shortTitle"))
        .unwrap_or_else(|| fallback_title(&item_type));
    let date = optional_text(data, "date");
    let creators = data
        .get("creators")
        .and_then(Value::as_array)
        .map(|values| values.iter().enumerate().map(normalize_creator).collect())
        .unwrap_or_default();
    let tags = data
        .get("tags")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(normalize_tag).collect())
        .unwrap_or_default();
    let collection_keys = data
        .get("collections")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let relations = normalize_relations(data.get("relations"));
    let path = optional_text(data, "path");
    let link_mode = optional_text(data, "linkMode");
    let attachment_available = item_type == "attachment"
        && path.is_some()
        && !matches!(link_mode.as_deref(), Some("linked_url"));

    Ok(NormalizedItem {
        key: object.key.clone(),
        version: object.version,
        item_type,
        parent_key: optional_text(data, "parentItem"),
        title,
        abstract_text: optional_text(data, "abstractNote"),
        year: date.as_deref().and_then(parse_year),
        date,
        venue: venue(data),
        volume: optional_text(data, "volume"),
        issue: optional_text(data, "issue"),
        pages: optional_text(data, "pages"),
        publisher: optional_text(data, "publisher"),
        doi: optional_text(data, "DOI"),
        isbn: optional_text(data, "ISBN"),
        issn: optional_text(data, "ISSN"),
        url: optional_text(data, "url"),
        date_added: optional_text(data, "dateAdded"),
        date_modified: optional_text(data, "dateModified"),
        is_trashed: data
            .get("deleted")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || data.get("deleted").and_then(Value::as_i64) == Some(1),
        creators,
        tags,
        collection_keys,
        relations,
        note_content: optional_text(data, "note"),
        content_type: optional_text(data, "contentType"),
        filename: optional_text(data, "filename"),
        link_mode,
        attachment_available,
    })
}

fn object_data(object: &RawObject) -> AppResult<&Value> {
    object.json.get("data").ok_or_else(|| {
        AppError::InvalidZoteroResponse(format!(
            "{} object {} has no data field",
            object.object_type, object.key
        ))
    })
}

fn normalize_creator((position, value): (usize, &Value)) -> NormalizedCreator {
    let first_name = optional_text(value, "firstName");
    let last_name = optional_text(value, "lastName");
    let field_mode_name = optional_text(value, "name");
    let display_name = field_mode_name.clone().unwrap_or_else(|| {
        [first_name.as_deref(), last_name.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
    });
    NormalizedCreator {
        position,
        creator_type: text(value, "creatorType").unwrap_or_else(|| "contributor".to_owned()),
        first_name,
        last_name,
        field_mode_name,
        display_name: if display_name.is_empty() {
            "Unknown creator".to_owned()
        } else {
            display_name
        },
    }
}

fn normalize_tag(value: &Value) -> Option<NormalizedTag> {
    let value_text = value.get("tag").and_then(Value::as_str)?.trim();
    if value_text.is_empty() {
        return None;
    }
    Some(NormalizedTag {
        value: value_text.to_owned(),
        tag_type: value.get("type").and_then(Value::as_i64),
    })
}

fn normalize_relations(value: Option<&Value>) -> Vec<NormalizedRelation> {
    let Some(object) = value.and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for (predicate, target_value) in object {
        match target_value {
            Value::String(target) => result.push(NormalizedRelation {
                predicate: predicate.clone(),
                target: target.clone(),
            }),
            Value::Array(targets) => {
                for target in targets.iter().filter_map(Value::as_str) {
                    result.push(NormalizedRelation {
                        predicate: predicate.clone(),
                        target: target.to_owned(),
                    });
                }
            }
            _ => {}
        }
    }
    result
}

fn venue(data: &Value) -> Option<String> {
    [
        "publicationTitle",
        "conferenceName",
        "proceedingsTitle",
        "university",
        "institution",
        "repository",
        "libraryCatalog",
    ]
    .iter()
    .find_map(|field| optional_text(data, field))
}

fn fallback_title(item_type: &str) -> String {
    match item_type {
        "attachment" => "Untitled attachment".to_owned(),
        "note" => "Untitled note".to_owned(),
        _ => "Untitled Zotero item".to_owned(),
    }
}

fn parse_year(value: &str) -> Option<i32> {
    let bytes = value.as_bytes();
    for window in bytes.windows(4) {
        if window.iter().all(u8::is_ascii_digit) {
            let text = std::str::from_utf8(window).ok()?;
            let year = text.parse::<i32>().ok()?;
            if (1000..=2999).contains(&year) {
                return Some(year);
            }
        }
    }
    None
}

fn optional_text(data: &Value, field: &str) -> Option<String> {
    text(data, field).filter(|value| !value.trim().is_empty())
}

fn text(data: &Value, field: &str) -> Option<String> {
    data.get(field).and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use crate::model::RawObject;

    use super::{normalize_item, parse_year};

    #[test]
    fn parses_partial_zotero_dates_without_inventing_precision() {
        assert_eq!(parse_year("Spring 2019"), Some(2019));
        assert_eq!(parse_year("n.d."), None);
    }

    #[test]
    fn preserves_field_mode_creators_and_optional_metadata() {
        let object = RawObject {
            key: "ITEM001".to_owned(),
            version: 7,
            object_type: "items".to_owned(),
            json: serde_json::json!({
                "data": {
                    "itemType": "report",
                    "title": "A local report",
                    "date": "2024-04",
                    "creators": [{
                        "creatorType": "author",
                        "name": "Earth Science Consortium"
                    }],
                    "tags": [{"tag": "Crustal imaging", "type": 1}],
                    "collections": ["COLL001"]
                }
            }),
        };

        let item = normalize_item(&object).expect("normalized item");
        assert_eq!(item.year, Some(2024));
        assert_eq!(item.creators[0].display_name, "Earth Science Consortium");
        assert_eq!(item.tags[0].tag_type, Some(1));
        assert!(item.is_top_level_research_item());
    }

    #[test]
    fn keeps_child_resources_out_of_top_level_research_items() {
        let object = RawObject {
            key: "ATTACH01".to_owned(),
            version: 2,
            object_type: "items".to_owned(),
            json: serde_json::json!({
                "data": {
                    "itemType": "attachment",
                    "parentItem": "ITEM001",
                    "title": "Full text PDF",
                    "contentType": "application/pdf",
                    "linkMode": "imported_file",
                    "path": "storage:paper.pdf"
                }
            }),
        };

        let item = normalize_item(&object).expect("normalized attachment");
        assert!(!item.is_top_level_research_item());
        assert!(item.attachment_available);
    }
}
