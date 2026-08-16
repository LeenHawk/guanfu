//! typed Asset 引用:definition 内以稳定 ID 指向其他 Asset。
//!
//! 每个引用类型携带目标 kind 的关联常量,服务层在保存和加载时据此校验
//! 目标存在且 kind 匹配(计划 §3.1);不建通用 asset_reference 表。

use serde::{Deserialize, Serialize};

use crate::entities::asset::AssetKind;

macro_rules! asset_ref {
    ($(#[$meta:meta])* $name:ident => $kind:expr) => {
        $(#[$meta])*
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
            Serialize, Deserialize, ts_rs::TS,
        )]
        #[ts(as = "i32")]
        pub struct $name(pub i32);

        impl $name {
            pub const TARGET_KIND: AssetKind = $kind;

            pub const fn id(self) -> i32 {
                self.0
            }
        }
    };
}

asset_ref!(
    /// 指向 WorldBook Asset。
    WorldBookRef => AssetKind::WorldBook
);
asset_ref!(
    /// 指向 RegexScript Asset。
    RegexScriptRef => AssetKind::RegexScript
);
asset_ref!(
    /// 指向 Media Asset。
    MediaRef => AssetKind::Media
);
asset_ref!(
    /// 指向 Character Asset。
    CharacterRef => AssetKind::Character
);
asset_ref!(
    /// 指向 Persona Asset。
    PersonaRef => AssetKind::Persona
);
asset_ref!(
    /// 指向 OpenAiChatPreset Asset。
    PresetRef => AssetKind::OpenAiChatPreset
);
asset_ref!(
    /// 指向 Pipeline Asset。
    PipelineRef => AssetKind::Pipeline
);
asset_ref!(
    /// 指向 ChatHistory Asset。
    ChatHistoryRef => AssetKind::ChatHistory
);

/// 交换格式里未识别的字段:原样保真,不自动进入模型请求。
pub type Extra = std::collections::BTreeMap<String, serde_json::Value>;
