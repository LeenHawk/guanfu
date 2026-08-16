//! 聊天历史的写入路径:追加是小写入,修订以内容哈希锚定。

use guanfu_core::assets::chat_history::{
    ChatHistoryDefinition, ChatHistoryV1, ChatMessage, MESSAGES,
};
use guanfu_core::assets::edit::{HashEdit, HashEditOp};
use guanfu_core::assets::{content_hash, ChunkHash};
use guanfu_core::entities::chunk;
use guanfu_core::llm::ir::generation::InputContent;
use guanfu_core::services::assets::{AssetService, LoadedAsset};
use guanfu_core::CoreError;
use sea_orm::EntityTrait;

fn user(text: &str, at: i64) -> ChatMessage {
    ChatMessage::User {
        content: vec![InputContent::Text {
            text: text.to_owned(),
        }],
        created_at_ms: at,
    }
}

fn unit(message: &ChatMessage) -> serde_json::Value {
    serde_json::to_value(message).unwrap()
}

fn anchor(message: &ChatMessage) -> ChunkHash {
    content_hash(message).unwrap()
}

fn texts(definition: &ChatHistoryDefinition) -> Vec<String> {
    let ChatHistoryDefinition::V1(history) = definition;
    history
        .messages
        .iter()
        .map(|message| match message {
            ChatMessage::User { content, .. } => match &content[0] {
                InputContent::Text { text } => text.clone(),
                _ => unreachable!("probe only writes text"),
            },
            _ => unreachable!("probe only writes user turns"),
        })
        .collect()
}

async fn db() -> sea_orm::DatabaseConnection {
    let state = guanfu_core::AppState::initialize(guanfu_core::AppConfig {
        database_url: "sqlite::memory:".to_owned(),
        asset_root: std::env::temp_dir().join("guanfu-test-assets"),
        llm: guanfu_core::LlmConfig::default(),
    })
    .await
    .unwrap();
    state.db
}

#[tokio::test]
async fn appends_turns_and_revises_by_content_anchor() {
    let db = db().await;
    let first = user("hello", 1);
    let history = ChatHistoryDefinition::V1(ChatHistoryV1 {
        title: "probe".into(),
        messages: vec![first.clone()],
        ..Default::default()
    });
    let head = AssetService::create(&db, "chat", None, &history)
        .await
        .unwrap();

    // 追加一轮:只新增该消息的 chunk,历史其余部分结构共享。
    let second = user("how are you", 2);
    let before = chunk::Entity::find().all(&db).await.unwrap().len();
    let revision = AssetService::append_units(&db, head.id, 1, MESSAGES, &[unit(&second)], None)
        .await
        .unwrap();
    assert_eq!(revision, 2);
    assert_eq!(
        chunk::Entity::find().all(&db).await.unwrap().len(),
        before + 1,
        "appending one turn writes exactly one chunk"
    );

    let loaded: LoadedAsset<ChatHistoryDefinition> =
        AssetService::load(&db, head.id).await.unwrap();
    assert_eq!(texts(&loaded.definition), ["hello", "how are you"]);

    // 修订:按内容哈希锚定,替换 + 在其后插入。
    let edits = vec![
        HashEdit {
            target_hash: anchor(&first),
            op: HashEditOp::Replace {
                new_content: unit(&user("hi", 1)),
            },
        },
        HashEdit {
            target_hash: anchor(&second),
            op: HashEditOp::InsertAfter {
                new_content: unit(&user("still there?", 3)),
            },
        },
    ];
    let revision = AssetService::revise_units(&db, head.id, 2, MESSAGES, &edits, None)
        .await
        .unwrap();
    assert_eq!(revision, 3);
    let revised: LoadedAsset<ChatHistoryDefinition> =
        AssetService::load(&db, head.id).await.unwrap();
    assert_eq!(
        texts(&revised.definition),
        ["hi", "how are you", "still there?"]
    );

    // 锚点已被替换:重放同一批指令显式报 stale,不静默错位。
    let stale = AssetService::revise_units(&db, head.id, 3, MESSAGES, &edits, None).await;
    assert!(matches!(stale, Err(CoreError::HashEditStale { .. })));

    // 旧修订仍可完整取回。
    let original: LoadedAsset<ChatHistoryDefinition> =
        AssetService::load_revision(&db, head.id, 1).await.unwrap();
    assert_eq!(texts(&original.definition), ["hello"]);
}
