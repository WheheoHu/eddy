use serde::{Deserialize, Serialize};

use crate::hlc::Hlc;

/// One entry of the operation log. Every local change is recorded as an op
/// before state is updated; merges resolve per (entity, entity_id, field) by
/// the highest HLC.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Op {
    pub op_id: String,
    pub hlc: Hlc,
    pub entity: Entity,
    pub entity_id: String,
    pub field: String,
    pub value: serde_json::Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Entity {
    Item,
    Progress,
    Annotation,
    Tag,
    ItemTag,
    Series,
    Source,
    Setting,
    /// Metadata of an immutable content version; its blobs travel separately.
    ContentVersion,
    /// A synced AI result (translation, summary).
    Artifact,
}

impl Entity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Entity::Item => "item",
            Entity::Progress => "progress",
            Entity::Annotation => "annotation",
            Entity::Tag => "tag",
            Entity::ItemTag => "item_tag",
            Entity::Series => "series",
            Entity::Source => "source",
            Entity::Setting => "setting",
            Entity::ContentVersion => "content_version",
            Entity::Artifact => "artifact",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "item" => Entity::Item,
            "progress" => Entity::Progress,
            "annotation" => Entity::Annotation,
            "tag" => Entity::Tag,
            "item_tag" => Entity::ItemTag,
            "series" => Entity::Series,
            "source" => Entity::Source,
            "setting" => Entity::Setting,
            "content_version" => Entity::ContentVersion,
            "artifact" => Entity::Artifact,
            _ => return None,
        })
    }
}

/// Field names used in ops. Keeping them in one place keeps clients and the
/// server in agreement.
pub mod fields {
    pub const CREATE: &str = "create";
    pub const DELETED: &str = "deleted";
    pub const READ: &str = "read";
    pub const STARRED: &str = "starred";
    pub const FINISHED: &str = "finished";
    pub const VERSION: &str = "version";
    pub const SERIES: &str = "series";
    pub const TITLE: &str = "title";
    pub const LOCATOR: &str = "locator";
    pub const STATE: &str = "state";
    pub const FULL_TEXT: &str = "full_text";
    pub const VALUE: &str = "value";
}
