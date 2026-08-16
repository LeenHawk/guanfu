//! WorldBook definition:有序 entries,逐 entry 成 chunk。
//!
//! 角色内嵌世界书只是 CCv2 交换形态,导入时创建独立 WorldBook Asset。

use serde::{Deserialize, Serialize};

use super::character::InjectionRole;
use super::refs::Extra;
use super::{join_items, split_items, AssetDefinition, ChunkContents, Manifest, SplitManifest};
use crate::entities::asset::AssetKind;
use crate::CoreError;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "version")]
pub enum WorldBookDefinition {
    V1(WorldBookV1),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct WorldBookV1 {
    pub name: String,
    pub description: String,
    /// 扫描聊天历史的深度上限;None 表示用运行配置缺省。
    pub scan_depth: Option<u32>,
    pub token_budget: Option<u32>,
    pub recursive_scanning: bool,
    pub entries: Vec<WorldBookEntry>,
    #[serde(default)]
    pub extra: Extra,
}

/// 单条世界书条目——也是内容寻址的可编辑单元(HashEdit 的锚点)。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct WorldBookEntry {
    pub name: String,
    pub content: String,
    pub keys: Vec<String>,
    pub secondary_keys: Vec<String>,
    pub selective_logic: SelectiveLogic,
    pub enabled: bool,
    /// 常驻激活,不参与关键词匹配。
    pub constant: bool,
    /// 插入顺序;越小越靠前。
    pub order: i32,
    pub position: EntryPosition,
    /// `position = at_depth` 时的深度与角色。
    pub depth: Option<u32>,
    pub role: Option<InjectionRole>,
    /// 激活概率百分比(0-100);None 表示必然激活。
    pub probability: Option<u8>,
    pub case_sensitive: bool,
    pub match_whole_words: bool,
    /// 递归控制:不被递归扫描触发 / 自身不触发递归 / 延迟到第 N 轮递归。
    pub exclude_recursion: bool,
    pub prevent_recursion: bool,
    pub delay_until_recursion: Option<u32>,
    /// 不计入预算上限。
    pub ignore_budget: bool,
    #[serde(default)]
    pub extra: Extra,
}

/// 次关键词的组合逻辑;ST 以 0-3 编码。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum SelectiveLogic {
    #[default]
    AndAny,
    NotAll,
    NotAny,
    AndAll,
}

/// 注入位置;ST 以 0-7 编码,outlet 后置不建模。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum EntryPosition {
    #[default]
    BeforeCharacter,
    AfterCharacter,
    TopAuthorNote,
    BottomAuthorNote,
    AtDepth,
    TopExampleMessages,
    BottomExampleMessages,
}

impl AssetDefinition for WorldBookDefinition {
    const KIND: AssetKind = AssetKind::WorldBook;

    fn split(&self) -> Result<SplitManifest, CoreError> {
        let Self::V1(book) = self;
        let (hashes, chunks) = split_items(&book.entries)?;
        let mut manifest = Manifest {
            fields: serde_json::to_value(BookHead {
                version: "V1",
                name: &book.name,
                description: &book.description,
                scan_depth: book.scan_depth,
                token_budget: book.token_budget,
                recursive_scanning: book.recursive_scanning,
                extra: &book.extra,
            })?,
            chunk_lists: Default::default(),
        };
        manifest.chunk_lists.insert(ENTRIES.to_owned(), hashes);
        Ok(SplitManifest { manifest, chunks })
    }

    fn join(manifest: &Manifest, chunks: &ChunkContents) -> Result<Self, CoreError> {
        let head: OwnedBookHead = serde_json::from_value(manifest.fields.clone())?;
        Ok(Self::V1(WorldBookV1 {
            name: head.name,
            description: head.description,
            scan_depth: head.scan_depth,
            token_budget: head.token_budget,
            recursive_scanning: head.recursive_scanning,
            entries: join_items(manifest, chunks, ENTRIES)?,
            extra: head.extra,
        }))
    }
}

const ENTRIES: &str = "entries";

/// manifest.fields 的骨架:entries 以 chunk 列表存放,不进 fields。
#[derive(Serialize)]
struct BookHead<'a> {
    version: &'static str,
    name: &'a str,
    description: &'a str,
    scan_depth: Option<u32>,
    token_budget: Option<u32>,
    recursive_scanning: bool,
    extra: &'a Extra,
}

#[derive(Deserialize)]
struct OwnedBookHead {
    name: String,
    description: String,
    scan_depth: Option<u32>,
    token_budget: Option<u32>,
    recursive_scanning: bool,
    #[serde(default)]
    extra: Extra,
}
