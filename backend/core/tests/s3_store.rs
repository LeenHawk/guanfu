//! S3 兼容端点的读写回归;没有配 `GUANFU_S3_*` 时跳过。
//!
//! 对象存储没法在本地伪造得有意义,所以这条依赖真实端点:
//! `GUANFU_S3_ENDPOINT` / `_BUCKET` / `_ACCESS_KEY_ID` / `_SECRET_ACCESS_KEY`。
use guanfu_core::assets::chunk_hash;
use guanfu_core::assets::{AssetStore, S3AssetStore};

#[tokio::test]
async fn round_trips_against_a_real_bucket() {
    let Some(config) = S3AssetStore::from_env() else {
        eprintln!("SKIP: no S3 env");
        return;
    };
    let store = S3AssetStore::new(config);
    let bytes = format!("guanfu probe {}", std::process::id()).into_bytes();
    let hash = chunk_hash(&bytes);

    assert!(!store.exists(&hash).await, "fresh hash must not exist");
    store.put(&hash, &bytes).await.expect("put");
    assert!(store.exists(&hash).await, "put then exists");
    let read = store.get(&hash).await.expect("get");
    assert_eq!(read, bytes, "bytes round trip");
    store.put(&hash, &bytes).await.expect("put is idempotent");
    store.delete(&hash).await.expect("delete");
    assert!(!store.exists(&hash).await, "deleted");
    store.delete(&hash).await.expect("delete is idempotent");
    let missing = store.get(&hash).await;
    assert!(missing.is_err(), "missing object errors");
    println!("S3 ROUND TRIP OK");
}

/// 服务层接入:Media Asset 的字节要真的落到对象存储并能读回。
#[tokio::test]
async fn media_assets_land_in_the_object_store() {
    use guanfu_core::services::assets::AssetService;
    use std::sync::Arc;

    let Some(config) = S3AssetStore::from_env() else {
        eprintln!("SKIP: no S3 env");
        return;
    };
    let store: Arc<dyn AssetStore> = Arc::new(S3AssetStore::new(config));
    let state = guanfu_core::AppState::initialize(guanfu_core::AppConfig {
        database_url: "sqlite::memory:".to_owned(),
        asset_root: std::env::temp_dir().join("guanfu-test-assets"),
        llm: guanfu_core::LlmConfig::default(),
    })
    .await
    .unwrap();

    let bytes = format!("png-ish bytes {}", std::process::id()).into_bytes();
    let head = AssetService::create_media(
        &state.db,
        store.as_ref(),
        "probe",
        "image/png",
        Some("probe.png".to_owned()),
        &bytes,
    )
    .await
    .expect("create media");

    let (media, read) = AssetService::read_media(&state.db, store.as_ref(), head.id)
        .await
        .expect("read media");
    assert_eq!(read, bytes, "bytes survive the object store");
    assert_eq!(media.mime_type, "image/png");
    assert_eq!(media.size, bytes.len() as u64);

    store.delete(&media.hash).await.expect("cleanup");
    println!("S3 MEDIA ASSET OK");
}
