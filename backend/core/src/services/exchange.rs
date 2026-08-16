//! 外部格式的导入导出:在一个事务里落库 Character 与内嵌世界书。
//!
//! 纯格式转换在 [`crate::exchange`];这里只负责 Asset 之间的引用回填与
//! 事务边界(计划 §6)。

use sea_orm::{ConnectionTrait, TransactionSession, TransactionTrait};

use crate::assets::character::{CharacterDefinition, CharacterV1};
use crate::assets::refs::WorldBookRef;
use crate::assets::world_book::WorldBookDefinition;
use crate::exchange::ccv2;
use crate::services::assets::{AssetHeadDto, AssetService};
use crate::services::auth::Actor;
use crate::CoreError;

/// 一次角色卡导入的结果。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, ts_rs::TS)]
pub struct ImportedCharacter {
    pub character: AssetHeadDto,
    pub world_book: Option<AssetHeadDto>,
}

pub struct ExchangeService;

impl ExchangeService {
    /// 导入 CCv2 JSON:内嵌世界书先落成独立 Asset,再把引用写进角色。
    pub async fn import_ccv2_json<C>(
        db: &C,
        actor: Actor,
        bytes: &[u8],
    ) -> Result<ImportedCharacter, CoreError>
    where
        C: ConnectionTrait + TransactionTrait,
    {
        let card = ccv2::parse_card(bytes)?;
        let (character, book) = ccv2::card_to_definitions(&card);
        let transaction = db.begin().await?;
        let imported = import_into(&transaction, actor, character, book).await?;
        transaction.commit().await?;
        Ok(imported)
    }

    /// 导入 PNG 角色卡:读 `chara` chunk 后与 JSON 同路。
    pub async fn import_ccv2_png<C>(
        db: &C,
        actor: Actor,
        png: &[u8],
    ) -> Result<ImportedCharacter, CoreError>
    where
        C: ConnectionTrait + TransactionTrait,
    {
        let json = crate::exchange::png::read_card(png)?;
        Self::import_ccv2_json(db, actor, &json).await
    }

    /// 导出 PNG 角色卡:在给定底图上写入 `chara` chunk。
    pub async fn export_ccv2_png(
        db: &impl ConnectionTrait,
        actor: Actor,
        character_id: i32,
        base_png: &[u8],
    ) -> Result<Vec<u8>, CoreError> {
        let json = Self::export_ccv2_json(db, actor, character_id).await?;
        crate::exchange::png::write_card(base_png, &json)
    }

    /// 导出 CCv2 JSON;角色引用的第一本世界书作为内嵌 character_book。
    pub async fn export_ccv2_json(
        db: &impl ConnectionTrait,
        actor: Actor,
        character_id: i32,
    ) -> Result<Vec<u8>, CoreError> {
        let loaded = AssetService::load::<CharacterDefinition>(db, actor, character_id).await?;
        let CharacterDefinition::V1(character) = loaded.definition;
        let book = match character.world_books.first() {
            Some(reference) => {
                let loaded =
                    AssetService::load::<WorldBookDefinition>(db, actor, reference.id()).await?;
                let WorldBookDefinition::V1(book) = loaded.definition;
                Some(book)
            }
            None => None,
        };
        let card = ccv2::definitions_to_card(&character, book.as_ref());
        Ok(serde_json::to_vec_pretty(&card)?)
    }
}

pub(crate) async fn import_into(
    db: &impl ConnectionTrait,
    actor: Actor,
    mut character: CharacterV1,
    book: Option<crate::assets::world_book::WorldBookV1>,
) -> Result<ImportedCharacter, CoreError> {
    let world_book = match book {
        Some(book) => {
            let name = book.name.clone();
            let head = AssetService::create(db, actor, &name, None, &WorldBookDefinition::V1(book))
                .await?;
            character.world_books.push(WorldBookRef(head.id));
            Some(head)
        }
        None => None,
    };
    let name = character.name.clone();
    let head =
        AssetService::create(db, actor, &name, None, &CharacterDefinition::V1(character)).await?;
    Ok(ImportedCharacter {
        character: head,
        world_book,
    })
}
