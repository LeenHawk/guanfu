//! entity-first 全链路冒烟：sync 建表 → service 层 CRUD。

use gproxy_protocol::{ContentGenerationKind, Operation, OperationKey};
use guanfu_core::services::channels::{ChannelService, NewChannel, NewCredential};
use guanfu_core::services::routing::{PutRoutingRule, RoutingImplementation, RoutingService};

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

    let source = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::OpenAiResponses,
    );
    let target = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::ClaudeMessages,
    );
    RoutingService::put_rule(
        &db,
        PutRoutingRule {
            channel_id: ch.id,
            source,
            implementation: RoutingImplementation::TransformTo { target },
            sort_order: 0,
            enabled: true,
        },
    )
    .await
    .unwrap();
    let rules = RoutingService::list_rules(&db, ch.id).await.unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].source, source);
    assert_eq!(
        rules[0].implementation,
        RoutingImplementation::TransformTo { target }
    );

    ChannelService::delete_channel(&db, ch.id).await.unwrap();
    assert!(ChannelService::list_channels(&db).await.unwrap().is_empty());
    assert!(ChannelService::list_credentials(&db, ch.id)
        .await
        .unwrap()
        .is_empty());
    assert!(RoutingService::list_rules(&db, ch.id)
        .await
        .unwrap()
        .is_empty());
}
