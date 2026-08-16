//! CCv2 交换验收:用 SillyTavern 样本卡跑导入 → 落库 → 导出。
//!
//! 验收标准不是字节相同,而是标准字段、greetings 顺序、entry 顺序与未知
//! extensions 均不丢失(计划 §9)。

use guanfu_core::assets::character::CharacterDefinition;
use guanfu_core::assets::refs::WorldBookRef;
use guanfu_core::assets::world_book::WorldBookDefinition;
use guanfu_core::exchange::{ccv2, png};
use guanfu_core::services::assets::{AssetService, LoadedAsset};
use guanfu_core::services::auth::Actor;
use guanfu_core::services::exchange::ExchangeService;

const CARD: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../samples/SillyTavern/default/content/default_Seraphina.png"
);

fn actor() -> Actor {
    Actor {
        user_id: 1,
        is_admin: false,
    }
}

async fn db() -> sea_orm::DatabaseConnection {
    guanfu_core::AppState::initialize(guanfu_core::AppConfig {
        database_url: "sqlite::memory:".to_owned(),
        asset_root: std::env::temp_dir().join("guanfu-test-assets"),
        llm: guanfu_core::LlmConfig::default(),
    })
    .await
    .unwrap()
    .db
}

#[tokio::test]
async fn imports_and_exports_a_sillytavern_card() {
    // samples/ 是 gitignored 的第三方样本(275 MB),干净克隆与 CI 上没有,
    // 所以缺失时跳过而不是把整条流水线判失败。
    let Ok(card_png) = std::fs::read(CARD) else {
        eprintln!("SKIP: {CARD} not present");
        return;
    };
    // 同时含 chara 与 ccv3 时只读 chara。
    let json = png::read_card(&card_png).unwrap();
    let source = ccv2::parse_card(&json).unwrap();
    let db = db().await;

    let imported = ExchangeService::import_ccv2_json(&db, actor(), &json)
        .await
        .unwrap();
    let book_head = imported
        .world_book
        .expect("the sample card embeds a character book");

    // 内嵌世界书成为独立 Asset,并被角色引用。
    let character: LoadedAsset<CharacterDefinition> =
        AssetService::load(&db, actor(), imported.character.id)
            .await
            .unwrap();
    let CharacterDefinition::V1(character) = &character.definition;
    assert_eq!(character.name, source.data.name);
    assert_eq!(character.world_books, vec![WorldBookRef(book_head.id)]);

    // greetings 顺序:首项是 first_mes,其余保持 alternate_greetings 次序。
    assert_eq!(character.greetings[0], source.data.first_mes);
    assert_eq!(
        character.greetings[1..],
        source.data.alternate_greetings[..]
    );

    // entry 顺序与内容保持。
    let book: LoadedAsset<WorldBookDefinition> = AssetService::load(&db, actor(), book_head.id)
        .await
        .unwrap();
    let WorldBookDefinition::V1(book) = &book.definition;
    let source_entries = &source.data.character_book.as_ref().unwrap().entries;
    assert_eq!(book.entries.len(), source_entries.len());
    for (ours, theirs) in book.entries.iter().zip(source_entries) {
        assert_eq!(ours.content, theirs.content);
        assert_eq!(ours.keys, theirs.keys);
    }

    // 导出:标准字段与非 V2 字段(group_only_greetings)均回到卡上。
    let exported = ExchangeService::export_ccv2_json(&db, actor(), imported.character.id)
        .await
        .unwrap();
    let round_tripped = ccv2::parse_card(&exported).unwrap();
    assert_eq!(round_tripped.spec, source.spec);
    assert_eq!(round_tripped.data.name, source.data.name);
    assert_eq!(round_tripped.data.first_mes, source.data.first_mes);
    assert_eq!(
        round_tripped.data.alternate_greetings,
        source.data.alternate_greetings
    );
    assert_eq!(
        round_tripped.data.extra.get("group_only_greetings"),
        source.data.extra.get("group_only_greetings"),
        "non-V2 fields survive the round trip"
    );
    assert_eq!(
        round_tripped.data.extensions.get("world"),
        source.data.extensions.get("world"),
        "unknown extensions survive the round trip"
    );
    let exported_entries = round_tripped.data.character_book.unwrap().entries;
    assert_eq!(exported_entries.len(), source_entries.len());
    assert_eq!(exported_entries[0].keys, source_entries[0].keys);
    assert_eq!(
        exported_entries[0].extensions.get("group"),
        source_entries[0].extensions.get("group"),
        "entry extensions that guanfu does not model are carried through"
    );

    // PNG round-trip:写回底图后仍能读出同一张卡,且只留一份角色数据。
    let exported_png =
        ExchangeService::export_ccv2_png(&db, actor(), imported.character.id, &card_png)
            .await
            .unwrap();
    let reread = ccv2::parse_card(&png::read_card(&exported_png).unwrap()).unwrap();
    assert_eq!(reread.data.name, source.data.name);
    assert_eq!(reread.data.first_mes, source.data.first_mes);
    let reimported = ExchangeService::import_ccv2_png(&db, actor(), &exported_png)
        .await
        .unwrap();
    assert_ne!(reimported.character.id, imported.character.id);
}
