//! CCv2 交换格式:外部 JSON ↔ typed definition 的纯函数层。
//!
//! 只做格式转换,不碰数据库;Asset 的事务性落库在
//! [`crate::services::exchange`]。`data.*` 优先于顶层兼容副本,未识别字段
//! 原样保真(计划 §6)。

use serde::{Deserialize, Serialize};

use super::entry::{book_entry_from_ccv2, book_entry_to_ccv2, Ccv2BookEntry};
use crate::assets::character::{CharacterV1, DepthPrompt, InjectionRole};
use crate::assets::refs::Extra;
use crate::assets::world_book::WorldBookV1;
use crate::CoreError;

pub const SPEC: &str = "chara_card_v2";
pub const SPEC_VERSION: &str = "2.0";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ccv2Card {
    pub spec: String,
    pub spec_version: String,
    pub data: Ccv2Data,
    /// V1 兼容层在顶层重复的字段;导入时只用于诊断,不作为事实来源。
    #[serde(flatten, default)]
    pub legacy: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ccv2Data {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub personality: String,
    #[serde(default)]
    pub scenario: String,
    #[serde(default)]
    pub first_mes: String,
    #[serde(default)]
    pub mes_example: String,
    #[serde(default)]
    pub creator_notes: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub post_history_instructions: String,
    #[serde(default)]
    pub alternate_greetings: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub creator: String,
    #[serde(default)]
    pub character_version: String,
    #[serde(default)]
    pub extensions: Extra,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_book: Option<Ccv2Book>,
    /// 非 V2 声明字段(如 `group_only_greetings`)一律保真。
    #[serde(flatten, default)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ccv2Book {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_depth: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u32>,
    #[serde(default)]
    pub recursive_scanning: bool,
    #[serde(default)]
    pub entries: Vec<Ccv2BookEntry>,
    #[serde(default)]
    pub extensions: Extra,
}

/// 解析 CCv2 JSON;spec 不匹配即结构化错误。
pub fn parse_card(bytes: &[u8]) -> Result<Ccv2Card, CoreError> {
    let card: Ccv2Card = serde_json::from_slice(bytes)?;
    if card.spec != SPEC {
        return Err(CoreError::UnsupportedExchangeFormat {
            format: card.spec.clone(),
        });
    }
    Ok(card)
}

/// 角色卡 → Character definition + 可选的内嵌世界书。
///
/// 世界书引用留空:调用方先落库 WorldBook Asset,再把 ID 填回。
pub fn card_to_definitions(card: &Ccv2Card) -> (CharacterV1, Option<WorldBookV1>) {
    let data = &card.data;
    let mut greetings = Vec::with_capacity(1 + data.alternate_greetings.len());
    // 顺序即语义:首项是 first_mes,其余保持 alternate_greetings 的次序。
    greetings.push(data.first_mes.clone());
    greetings.extend(data.alternate_greetings.iter().cloned());

    let mut extensions = data.extensions.clone();
    let depth_prompt = extensions
        .remove("depth_prompt")
        .and_then(|value| serde_json::from_value::<RawDepthPrompt>(value).ok())
        .filter(|raw| !raw.prompt.is_empty())
        .map(|raw| DepthPrompt {
            prompt: raw.prompt,
            depth: raw.depth,
            role: match raw.role.as_str() {
                "user" => InjectionRole::User,
                "assistant" => InjectionRole::Assistant,
                _ => InjectionRole::System,
            },
        });

    let mut extra = data.extra.clone();
    // extensions 与非 V2 字段分开保真,导出时可原样还原。
    extra.insert(
        EXTENSIONS_KEY.to_owned(),
        serde_json::to_value(&extensions).unwrap_or_default(),
    );

    let character = CharacterV1 {
        name: data.name.clone(),
        description: data.description.clone(),
        personality: data.personality.clone(),
        scenario: data.scenario.clone(),
        creator_notes: data.creator_notes.clone(),
        system_prompt: data.system_prompt.clone(),
        post_history_instructions: data.post_history_instructions.clone(),
        greetings,
        example_dialogue: data.mes_example.clone(),
        tags: data.tags.clone(),
        creator: data.creator.clone(),
        character_version: data.character_version.clone(),
        depth_prompt,
        world_books: Vec::new(),
        regex_scripts: Vec::new(),
        avatar: None,
        extra,
    };

    let book = data.character_book.as_ref().map(|book| WorldBookV1 {
        name: if book.name.is_empty() {
            data.name.clone()
        } else {
            book.name.clone()
        },
        description: book.description.clone(),
        scan_depth: book.scan_depth,
        token_budget: book.token_budget,
        recursive_scanning: book.recursive_scanning,
        entries: book.entries.iter().map(book_entry_from_ccv2).collect(),
        extra: book.extensions.clone(),
    });

    (character, book)
}

/// Character definition(+ 可选世界书)→ 角色卡。
pub fn definitions_to_card(character: &CharacterV1, book: Option<&WorldBookV1>) -> Ccv2Card {
    let mut extra = character.extra.clone();
    let mut extensions: Extra = extra
        .remove(EXTENSIONS_KEY)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    if let Some(depth_prompt) = &character.depth_prompt {
        extensions.insert(
            "depth_prompt".to_owned(),
            serde_json::json!({
                "prompt": depth_prompt.prompt,
                "depth": depth_prompt.depth,
                "role": match depth_prompt.role {
                    InjectionRole::System => "system",
                    InjectionRole::User => "user",
                    InjectionRole::Assistant => "assistant",
                },
            }),
        );
    }

    let mut greetings = character.greetings.iter();
    let first_mes = greetings.next().cloned().unwrap_or_default();

    Ccv2Card {
        spec: SPEC.to_owned(),
        spec_version: SPEC_VERSION.to_owned(),
        data: Ccv2Data {
            name: character.name.clone(),
            description: character.description.clone(),
            personality: character.personality.clone(),
            scenario: character.scenario.clone(),
            first_mes,
            mes_example: character.example_dialogue.clone(),
            creator_notes: character.creator_notes.clone(),
            system_prompt: character.system_prompt.clone(),
            post_history_instructions: character.post_history_instructions.clone(),
            alternate_greetings: greetings.cloned().collect(),
            tags: character.tags.clone(),
            creator: character.creator.clone(),
            character_version: character.character_version.clone(),
            extensions,
            character_book: book.map(|book| Ccv2Book {
                name: book.name.clone(),
                description: book.description.clone(),
                scan_depth: book.scan_depth,
                token_budget: book.token_budget,
                recursive_scanning: book.recursive_scanning,
                entries: book.entries.iter().map(book_entry_to_ccv2).collect(),
                extensions: book.extra.clone(),
            }),
            extra,
        },
        legacy: Extra::new(),
    }
}

/// `CharacterV1::extra` 中存放 CCv2 `data.extensions` 的键。
const EXTENSIONS_KEY: &str = "ccv2_extensions";

#[derive(Deserialize)]
struct RawDepthPrompt {
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    depth: u32,
    #[serde(default)]
    role: String,
}
