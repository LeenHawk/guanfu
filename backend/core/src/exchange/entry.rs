//! Character Book entry ↔ WorldBookEntry 的字段映射。
//!
//! 交换形态把 ST 的高级语义塞在 entry 的 `extensions` 里(snake_case 与
//! camelCase 混用),观复把已调查字段提升为具名字段,其余原样保真——
//! 只存交换形态会丢 ST 运行语义,只存内部模板会丢外部格式。

use serde::{Deserialize, Serialize};

use crate::assets::character::InjectionRole;
use crate::assets::refs::Extra;
use crate::assets::world_book::{EntryPosition, SelectiveLogic, WorldBookEntry};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ccv2BookEntry {
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub secondary_keys: Vec<String>,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub comment: String,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    #[serde(default)]
    pub insertion_order: i32,
    #[serde(default)]
    pub constant: bool,
    #[serde(default)]
    pub selective: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_sensitive: Option<bool>,
    /// `before_char` / `after_char`;更细的位置在 extensions.position。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<String>,
    #[serde(default)]
    pub extensions: Extra,
    #[serde(flatten, default)]
    pub extra: Extra,
}

fn enabled_default() -> bool {
    true
}

pub fn book_entry_from_ccv2(entry: &Ccv2BookEntry) -> WorldBookEntry {
    let ext = &entry.extensions;
    let position = match (number(ext, "position"), entry.position.as_deref()) {
        (Some(0), _) => EntryPosition::BeforeCharacter,
        (Some(1), _) => EntryPosition::AfterCharacter,
        (Some(2), _) => EntryPosition::TopAuthorNote,
        (Some(3), _) => EntryPosition::BottomAuthorNote,
        (Some(4), _) => EntryPosition::AtDepth,
        (Some(5), _) => EntryPosition::TopExampleMessages,
        (Some(6), _) => EntryPosition::BottomExampleMessages,
        (_, Some("after_char")) => EntryPosition::AfterCharacter,
        _ => EntryPosition::BeforeCharacter,
    };
    let mut extra = entry.extra.clone();
    // 已提升为具名字段的扩展键不再重复保存,其余原样带走。
    let carried: Extra = ext
        .iter()
        .filter(|(key, _)| !LIFTED.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if !carried.is_empty() {
        extra.insert(
            EXTENSIONS_KEY.to_owned(),
            serde_json::to_value(&carried).unwrap_or_default(),
        );
    }

    WorldBookEntry {
        name: entry.comment.clone(),
        content: entry.content.clone(),
        keys: entry.keys.clone(),
        secondary_keys: entry.secondary_keys.clone(),
        selective_logic: match number(ext, "selectiveLogic") {
            Some(1) => SelectiveLogic::NotAll,
            Some(2) => SelectiveLogic::NotAny,
            Some(3) => SelectiveLogic::AndAll,
            _ => SelectiveLogic::AndAny,
        },
        enabled: entry.enabled,
        constant: entry.constant,
        order: entry.insertion_order,
        position,
        depth: number(ext, "depth").map(|value| value as u32),
        role: match number(ext, "role") {
            Some(1) => Some(InjectionRole::User),
            Some(2) => Some(InjectionRole::Assistant),
            Some(0) => Some(InjectionRole::System),
            _ => None,
        },
        // useProbability 为假时概率无意义。
        probability: flag(ext, "useProbability")
            .unwrap_or(false)
            .then(|| number(ext, "probability").unwrap_or(100).clamp(0, 100) as u8),
        case_sensitive: entry
            .case_sensitive
            .or_else(|| flag(ext, "case_sensitive"))
            .unwrap_or(false),
        match_whole_words: flag(ext, "match_whole_words").unwrap_or(false),
        exclude_recursion: flag(ext, "exclude_recursion").unwrap_or(false),
        prevent_recursion: flag(ext, "prevent_recursion").unwrap_or(false),
        // ST 用 false 表示"不延迟",数字表示第 N 轮。
        delay_until_recursion: number(ext, "delay_until_recursion")
            .filter(|value| *value > 0)
            .map(|value| value as u32),
        ignore_budget: flag(ext, "ignore_budget").unwrap_or(false),
        extra,
    }
}

pub fn book_entry_to_ccv2(entry: &WorldBookEntry) -> Ccv2BookEntry {
    let mut extra = entry.extra.clone();
    let mut extensions: Extra = extra
        .remove(EXTENSIONS_KEY)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let position_code = match entry.position {
        EntryPosition::BeforeCharacter => 0,
        EntryPosition::AfterCharacter => 1,
        EntryPosition::TopAuthorNote => 2,
        EntryPosition::BottomAuthorNote => 3,
        EntryPosition::AtDepth => 4,
        EntryPosition::TopExampleMessages => 5,
        EntryPosition::BottomExampleMessages => 6,
    };
    extensions.insert("position".to_owned(), position_code.into());
    extensions.insert(
        "selectiveLogic".to_owned(),
        match entry.selective_logic {
            SelectiveLogic::AndAny => 0,
            SelectiveLogic::NotAll => 1,
            SelectiveLogic::NotAny => 2,
            SelectiveLogic::AndAll => 3,
        }
        .into(),
    );
    if let Some(depth) = entry.depth {
        extensions.insert("depth".to_owned(), depth.into());
    }
    if let Some(role) = entry.role {
        extensions.insert(
            "role".to_owned(),
            match role {
                InjectionRole::System => 0,
                InjectionRole::User => 1,
                InjectionRole::Assistant => 2,
            }
            .into(),
        );
    }
    extensions.insert(
        "useProbability".to_owned(),
        entry.probability.is_some().into(),
    );
    if let Some(probability) = entry.probability {
        extensions.insert("probability".to_owned(), probability.into());
    }
    extensions.insert("case_sensitive".to_owned(), entry.case_sensitive.into());
    extensions.insert(
        "match_whole_words".to_owned(),
        entry.match_whole_words.into(),
    );
    extensions.insert(
        "exclude_recursion".to_owned(),
        entry.exclude_recursion.into(),
    );
    extensions.insert(
        "prevent_recursion".to_owned(),
        entry.prevent_recursion.into(),
    );
    extensions.insert(
        "delay_until_recursion".to_owned(),
        match entry.delay_until_recursion {
            Some(value) => value.into(),
            None => false.into(),
        },
    );
    extensions.insert("ignore_budget".to_owned(), entry.ignore_budget.into());

    Ccv2BookEntry {
        keys: entry.keys.clone(),
        secondary_keys: entry.secondary_keys.clone(),
        content: entry.content.clone(),
        comment: entry.name.clone(),
        enabled: entry.enabled,
        insertion_order: entry.order,
        constant: entry.constant,
        selective: !entry.secondary_keys.is_empty(),
        case_sensitive: Some(entry.case_sensitive),
        position: Some(
            match entry.position {
                EntryPosition::AfterCharacter => "after_char",
                _ => "before_char",
            }
            .to_owned(),
        ),
        extensions,
        extra,
    }
}

/// 已提升为 WorldBookEntry 具名字段的扩展键。
const LIFTED: [&str; 12] = [
    "position",
    "selectiveLogic",
    "depth",
    "role",
    "probability",
    "useProbability",
    "case_sensitive",
    "match_whole_words",
    "exclude_recursion",
    "prevent_recursion",
    "delay_until_recursion",
    "ignore_budget",
];

/// `WorldBookEntry::extra` 中存放剩余 entry 扩展的键。
const EXTENSIONS_KEY: &str = "ccv2_extensions";

fn number(extensions: &Extra, key: &str) -> Option<i64> {
    extensions.get(key)?.as_i64()
}

fn flag(extensions: &Extra, key: &str) -> Option<bool> {
    extensions.get(key)?.as_bool()
}
