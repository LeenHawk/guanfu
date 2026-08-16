use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::assets::{AssetDefinition, ChunkContents, ChunkHash, Manifest, SplitManifest};
use crate::entities::asset::AssetKind;
use crate::entities::{asset, asset_revision, chunk};
use crate::CoreError;

/// 对适配层暴露的 Asset 头视图。
#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct AssetHeadDto {
    pub id: i32,
    pub kind: AssetKind,
    pub name: String,
    pub head_revision: i32,
}

impl From<asset::Model> for AssetHeadDto {
    fn from(m: asset::Model) -> Self {
        Self {
            id: m.id,
            kind: m.kind,
            name: m.name,
            head_revision: m.head_revision,
        }
    }
}

/// 装载结果:头信息 + 实际读取的 revision + typed definition。
#[derive(Clone, Debug)]
pub struct LoadedAsset<D> {
    pub head: AssetHeadDto,
    pub revision: i32,
    pub definition: D,
}

pub struct AssetService;

impl AssetService {
    pub async fn create<D: AssetDefinition>(
        db: &impl ConnectionTrait,
        name: &str,
        created_by_run_id: Option<i32>,
        definition: &D,
    ) -> Result<AssetHeadDto, CoreError> {
        let SplitManifest { manifest, chunks } = definition.split()?;
        write_chunks(db, &chunks).await?;
        let now = OffsetDateTime::now_utc();
        let head = asset::ActiveModel {
            kind: Set(D::KIND),
            name: Set(name.to_owned()),
            head_revision: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await?;
        insert_revision(db, head.id, 1, &manifest, created_by_run_id).await?;
        Ok(head.into())
    }

    pub async fn list(
        db: &impl ConnectionTrait,
        kind: Option<AssetKind>,
    ) -> Result<Vec<AssetHeadDto>, CoreError> {
        let mut query = asset::Entity::find().order_by_asc(asset::Column::Id);
        if let Some(kind) = kind {
            query = query.filter(asset::Column::Kind.eq(kind));
        }
        Ok(query.all(db).await?.into_iter().map(Into::into).collect())
    }

    pub async fn load<D: AssetDefinition>(
        db: &impl ConnectionTrait,
        id: i32,
    ) -> Result<LoadedAsset<D>, CoreError> {
        let head = find_head(db, id, Some(D::KIND)).await?;
        let revision = head.head_revision;
        Self::load_at(db, head, revision).await
    }

    pub async fn load_revision<D: AssetDefinition>(
        db: &impl ConnectionTrait,
        id: i32,
        revision: i32,
    ) -> Result<LoadedAsset<D>, CoreError> {
        let head = find_head(db, id, Some(D::KIND)).await?;
        Self::load_at(db, head, revision).await
    }

    async fn load_at<D: AssetDefinition>(
        db: &impl ConnectionTrait,
        head: asset::Model,
        revision: i32,
    ) -> Result<LoadedAsset<D>, CoreError> {
        let manifest = load_manifest(db, head.id, revision).await?;
        let chunks = load_chunks(db, &manifest).await?;
        let definition = D::join(&manifest, &chunks)?;
        Ok(LoadedAsset {
            head: head.into(),
            revision,
            definition,
        })
    }

    /// 以 CAS 提交新修订:`expected_head` 不匹配即冲突,由调用方重读重试。
    #[tracing::instrument(skip(db, definition), fields(kind = ?D::KIND))]
    pub async fn commit<D: AssetDefinition>(
        db: &impl ConnectionTrait,
        id: i32,
        expected_head: i32,
        created_by_run_id: Option<i32>,
        definition: &D,
    ) -> Result<AssetHeadDto, CoreError> {
        let head = find_head(db, id, Some(D::KIND)).await?;
        if head.head_revision != expected_head {
            return Err(CoreError::AssetRevisionConflict {
                id,
                expected: expected_head,
            });
        }
        let SplitManifest { manifest, chunks } = definition.split()?;
        let next =
            commit_manifest(db, id, expected_head, manifest, chunks, created_by_run_id).await?;
        Ok(AssetHeadDto {
            id,
            kind: D::KIND,
            name: head.name,
            head_revision: next,
        })
    }

    /// 追加:新增 chunk + 新 manifest,不重写既有单元。
    ///
    /// 每轮聊天就是这条路径——写入量与新增内容成正比,与历史长度无关。
    #[tracing::instrument(skip(db, units), fields(count = units.len()))]
    pub async fn append_units(
        db: &impl ConnectionTrait,
        id: i32,
        expected_head: i32,
        list: &str,
        units: &[serde_json::Value],
        created_by_run_id: Option<i32>,
    ) -> Result<i32, CoreError> {
        let manifest = load_manifest(db, id, expected_head).await?;
        let (hashes, payloads) = crate::assets::split_items(units)?;
        let mut manifest = manifest;
        manifest
            .chunk_lists
            .entry(list.to_owned())
            .or_default()
            .extend(hashes);
        commit_manifest(db, id, expected_head, manifest, payloads, created_by_run_id).await
    }

    /// 修订:以 HashEdit 锚定内容而非下标,一批指令原子生效。
    #[tracing::instrument(skip(db, edits), fields(count = edits.len()))]
    pub async fn revise_units(
        db: &impl ConnectionTrait,
        id: i32,
        expected_head: i32,
        list: &str,
        edits: &[crate::assets::edit::HashEdit],
        created_by_run_id: Option<i32>,
    ) -> Result<i32, CoreError> {
        let manifest = load_manifest(db, id, expected_head).await?;
        let (manifest, payloads) = crate::assets::edit::apply_hash_edits(&manifest, list, edits)?;
        commit_manifest(db, id, expected_head, manifest, payloads, created_by_run_id).await
    }

    /// fork:新 asset 指向复制的 manifest,chunk 全部结构共享。
    pub async fn fork(
        db: &impl ConnectionTrait,
        source_id: i32,
        revision: Option<i32>,
        name: &str,
    ) -> Result<AssetHeadDto, CoreError> {
        let source = find_head(db, source_id, None).await?;
        let revision = revision.unwrap_or(source.head_revision);
        let manifest = load_manifest(db, source_id, revision).await?;
        let now = OffsetDateTime::now_utc();
        let head = asset::ActiveModel {
            kind: Set(source.kind),
            name: Set(name.to_owned()),
            head_revision: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await?;
        insert_revision(db, head.id, 1, &manifest, None).await?;
        Ok(head.into())
    }

    /// 删除头指针与修订;chunk 共享,留给显式维护操作清理。
    pub async fn delete(db: &impl ConnectionTrait, id: i32) -> Result<(), CoreError> {
        asset_revision::Entity::delete_many()
            .filter(asset_revision::Column::AssetId.eq(id))
            .exec(db)
            .await?;
        asset::Entity::delete_by_id(id).exec(db).await?;
        Ok(())
    }

    /// 保存二进制内容为 Media Asset。
    ///
    /// 先写对象再提交元数据:外部存储写入不进数据库事务,失败留下的孤儿
    /// 对象由显式维护操作清理(计划 §2.2)。
    pub async fn create_media(
        db: &impl ConnectionTrait,
        store: &dyn crate::assets::AssetStore,
        name: &str,
        mime_type: &str,
        filename: Option<String>,
        bytes: &[u8],
    ) -> Result<AssetHeadDto, CoreError> {
        let hash = crate::assets::chunk_hash(bytes);
        store.put(&hash, bytes).await?;
        let size = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
        chunk::Entity::insert(chunk::ActiveModel {
            hash: Set(hash.0.clone()),
            location: Set(chunk::ChunkLocation::Store),
            bytes: Set(None),
            size: Set(size),
        })
        .on_conflict(
            OnConflict::column(chunk::Column::Hash)
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(db)
        .await?;
        Self::create(
            db,
            name,
            None,
            &crate::assets::MediaDefinition::V1(crate::assets::media::MediaV1 {
                hash,
                mime_type: mime_type.to_owned(),
                size: bytes.len() as u64,
                filename,
                extra: Default::default(),
            }),
        )
        .await
    }

    /// 读取 Media Asset 的元数据与字节。
    pub async fn read_media(
        db: &impl ConnectionTrait,
        store: &dyn crate::assets::AssetStore,
        id: i32,
    ) -> Result<(crate::assets::media::MediaV1, Vec<u8>), CoreError> {
        let loaded = Self::load::<crate::assets::MediaDefinition>(db, id).await?;
        let crate::assets::MediaDefinition::V1(media) = loaded.definition;
        let bytes = store.get(&media.hash).await?;
        Ok((media, bytes))
    }
}

/// 共享的 CAS 提交路径:写 chunk → 插入不可变修订 → 移动头指针。
async fn commit_manifest(
    db: &impl ConnectionTrait,
    id: i32,
    expected_head: i32,
    manifest: Manifest,
    chunks: Vec<crate::assets::ChunkPayload>,
    created_by_run_id: Option<i32>,
) -> Result<i32, CoreError> {
    write_chunks(db, &chunks).await?;
    let next = expected_head + 1;
    // revision 主键冲突即并发提交抢先,视作 CAS 失败。
    insert_revision(db, id, next, &manifest, created_by_run_id)
        .await
        .map_err(|error| match error {
            CoreError::Db(_) => CoreError::AssetRevisionConflict {
                id,
                expected: expected_head,
            },
            other => other,
        })?;
    let moved = asset::Entity::update_many()
        .col_expr(asset::Column::HeadRevision, next.into())
        .col_expr(asset::Column::UpdatedAt, OffsetDateTime::now_utc().into())
        .filter(asset::Column::Id.eq(id))
        .filter(asset::Column::HeadRevision.eq(expected_head))
        .exec(db)
        .await?;
    if moved.rows_affected == 0 {
        return Err(CoreError::AssetRevisionConflict {
            id,
            expected: expected_head,
        });
    }
    Ok(next)
}

async fn find_head(
    db: &impl ConnectionTrait,
    id: i32,
    expected: Option<AssetKind>,
) -> Result<asset::Model, CoreError> {
    let head = asset::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(CoreError::AssetNotFound(id))?;
    if let Some(expected) = expected {
        if head.kind != expected {
            return Err(CoreError::AssetKindMismatch {
                id,
                expected,
                found: head.kind,
            });
        }
    }
    Ok(head)
}

async fn load_manifest(
    db: &impl ConnectionTrait,
    asset_id: i32,
    revision: i32,
) -> Result<Manifest, CoreError> {
    let row = asset_revision::Entity::find_by_id((asset_id, revision))
        .one(db)
        .await?
        .ok_or(CoreError::AssetRevisionNotFound {
            id: asset_id,
            revision,
        })?;
    Ok(serde_json::from_value(row.manifest)?)
}

async fn insert_revision(
    db: &impl ConnectionTrait,
    asset_id: i32,
    revision: i32,
    manifest: &Manifest,
    created_by_run_id: Option<i32>,
) -> Result<(), CoreError> {
    asset_revision::ActiveModel {
        asset_id: Set(asset_id),
        revision: Set(revision),
        manifest: Set(serde_json::to_value(manifest)?),
        created_by_run_id: Set(created_by_run_id),
        created_at: Set(OffsetDateTime::now_utc()),
    }
    .insert(db)
    .await?;
    Ok(())
}

/// chunk 按哈希幂等写入。
async fn write_chunks(
    db: &impl ConnectionTrait,
    chunks: &[crate::assets::ChunkPayload],
) -> Result<(), CoreError> {
    for payload in chunks {
        let size = i64::try_from(payload.bytes.len()).unwrap_or(i64::MAX);
        let insert = chunk::Entity::insert(chunk::ActiveModel {
            hash: Set(payload.hash.0.clone()),
            location: Set(chunk::ChunkLocation::Db),
            bytes: Set(Some(payload.bytes.clone())),
            size: Set(size),
        })
        .on_conflict(
            OnConflict::column(chunk::Column::Hash)
                .do_nothing()
                .to_owned(),
        );
        insert.exec_without_returning(db).await?;
    }
    Ok(())
}

async fn load_chunks(
    db: &impl ConnectionTrait,
    manifest: &Manifest,
) -> Result<ChunkContents, CoreError> {
    let hashes: Vec<String> = manifest
        .referenced_chunks()
        .map(|hash| hash.0.clone())
        .collect();
    let mut contents = ChunkContents::new();
    if hashes.is_empty() {
        return Ok(contents);
    }
    let rows = chunk::Entity::find()
        .filter(chunk::Column::Hash.is_in(hashes))
        .all(db)
        .await?;
    for row in rows {
        if let Some(bytes) = row.bytes {
            contents.insert(ChunkHash(row.hash), bytes);
        }
    }
    Ok(contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::character::{CharacterDefinition, CharacterV1};
    use crate::assets::world_book::{WorldBookDefinition, WorldBookEntry, WorldBookV1};

    fn book(entries: &[&str]) -> WorldBookDefinition {
        WorldBookDefinition::V1(WorldBookV1 {
            name: "eldoria".into(),
            entries: entries
                .iter()
                .map(|content| WorldBookEntry {
                    content: (*content).into(),
                    enabled: true,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
    }

    fn entry_contents(definition: &WorldBookDefinition) -> Vec<String> {
        let WorldBookDefinition::V1(book) = definition;
        book.entries.iter().map(|e| e.content.clone()).collect()
    }

    async fn db() -> sea_orm::DatabaseConnection {
        let db = crate::db::connect("sqlite::memory:").await.unwrap();
        crate::db::sync_schema(&db).await.unwrap();
        db
    }

    #[tokio::test]
    async fn round_trip_cas_fork_and_kind_boundary() {
        let db = db().await;
        let initial = book(&["a", "b"]);
        let head = AssetService::create(&db, "wb", None, &initial)
            .await
            .unwrap();
        assert_eq!(head.head_revision, 1);

        let loaded: LoadedAsset<WorldBookDefinition> =
            AssetService::load(&db, head.id).await.unwrap();
        assert_eq!(loaded.definition, initial);

        // 追加 = 新 chunk + 新 manifest;旧修订仍可取回。
        let appended = book(&["a", "b", "c"]);
        let head2 = AssetService::commit(&db, head.id, 1, None, &appended)
            .await
            .unwrap();
        assert_eq!(head2.head_revision, 2);
        let old: LoadedAsset<WorldBookDefinition> =
            AssetService::load_revision(&db, head.id, 1).await.unwrap();
        assert_eq!(entry_contents(&old.definition), ["a", "b"]);

        // CAS:过期头指针显式冲突。
        let stale = AssetService::commit(&db, head.id, 1, None, &appended).await;
        assert!(matches!(
            stale,
            Err(CoreError::AssetRevisionConflict { .. })
        ));

        // fork 结构共享:不产生新 chunk。
        let chunks_before = chunk::Entity::find().all(&db).await.unwrap().len();
        let fork = AssetService::fork(&db, head.id, None, "wb-fork")
            .await
            .unwrap();
        assert_eq!(
            chunks_before,
            chunk::Entity::find().all(&db).await.unwrap().len()
        );
        let forked: LoadedAsset<WorldBookDefinition> =
            AssetService::load(&db, fork.id).await.unwrap();
        assert_eq!(entry_contents(&forked.definition), ["a", "b", "c"]);

        // kind 与 definition 不匹配 → 结构化错误。
        let mismatch = AssetService::load::<CharacterDefinition>(&db, head.id).await;
        assert!(matches!(mismatch, Err(CoreError::AssetKindMismatch { .. })));

        // 体量小的 kind 全内联:零 chunk。
        let character = CharacterDefinition::V1(CharacterV1 {
            name: "seraphina".into(),
            greetings: vec!["hello".into()],
            ..Default::default()
        });
        let head3 = AssetService::create(&db, "char", None, &character)
            .await
            .unwrap();
        let reloaded: LoadedAsset<CharacterDefinition> =
            AssetService::load(&db, head3.id).await.unwrap();
        assert_eq!(reloaded.definition, character);
        assert_eq!(
            chunk::Entity::find().all(&db).await.unwrap().len(),
            chunks_before
        );
    }
}
