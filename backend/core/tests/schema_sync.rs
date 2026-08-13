//! entity-first 全链路冒烟：sync 建表 → service 层 CRUD。

use guanfu_core::services::channels::{ChannelService, NewChannel, NewCredential};

#[tokio::test]
async fn entity_first_sync_and_crud() {
    let db = guanfu_core::db::connect("sqlite::memory:").await.unwrap();
    guanfu_core::db::sync_schema(&db).await.unwrap();
    // sync 是只增操作，重复执行应幂等。
    guanfu_core::db::sync_schema(&db).await.unwrap();

    let ch = ChannelService::create_channel(
        &db,
        NewChannel {
            name: "main".into(),
            provider: "claude".into(),
            base_url: "https://api.anthropic.com".into(),
        },
    )
    .await
    .unwrap();

    let cred = ChannelService::add_credential(
        &db,
        NewCredential {
            channel_id: ch.id,
            label: "k1".into(),
            secret: "sk-test".into(),
            weight: 1,
        },
    )
    .await
    .unwrap();
    assert_eq!(cred.channel_id, ch.id);

    let creds = ChannelService::list_credentials(&db, ch.id).await.unwrap();
    assert_eq!(creds.len(), 1);

    ChannelService::delete_channel(&db, ch.id).await.unwrap();
    assert!(ChannelService::list_channels(&db).await.unwrap().is_empty());
    assert!(ChannelService::list_credentials(&db, ch.id)
        .await
        .unwrap()
        .is_empty());
}
