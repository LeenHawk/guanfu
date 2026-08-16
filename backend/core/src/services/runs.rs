//! Run 服务:槽位解析与 run 行的生命周期。
//!
//! 输入槽位按 pipeline 声明校验 kind 并钉住 revision,因而 run 可复现;
//! 输出的 manifest 手术在 [`crate::services::assets`],这里只记录结果
//! 槽位与终态(计划 §5)。

use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::assets::pipeline::{InputSlot, PipelineDefinition};
use crate::entities::asset;
use crate::entities::run::{self, RunStatus};
use crate::CoreError;

/// 发起 run 时对一个槽位的绑定。
#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct SlotBinding {
    pub slot: String,
    pub asset_ids: Vec<i32>,
}

/// 解析后的槽位:revision 已钉住。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ResolvedSlot {
    pub slot: String,
    pub asset_id: i32,
    pub revision: i32,
}

pub struct RunService;

impl RunService {
    /// 按 pipeline 的槽位签名校验绑定并钉住 revision。
    pub async fn resolve_slots(
        db: &impl ConnectionTrait,
        pipeline: &PipelineDefinition,
        bindings: &[SlotBinding],
    ) -> Result<Vec<ResolvedSlot>, CoreError> {
        let PipelineDefinition::V1(pipeline) = pipeline;
        for binding in bindings {
            if !pipeline.inputs.iter().any(|slot| slot.name == binding.slot) {
                return Err(slot_error(&binding.slot, "pipeline declares no such slot"));
            }
        }
        let mut resolved = Vec::new();
        for slot in &pipeline.inputs {
            let ids = bindings
                .iter()
                .find(|binding| binding.slot == slot.name)
                .map(|binding| binding.asset_ids.as_slice())
                .unwrap_or_default();
            if ids.is_empty() {
                if slot.required {
                    return Err(slot_error(&slot.name, "required slot is unbound"));
                }
                continue;
            }
            if !slot.many && ids.len() > 1 {
                return Err(slot_error(&slot.name, "slot accepts a single asset"));
            }
            for id in ids {
                resolved.push(ResolvedSlot {
                    slot: slot.name.clone(),
                    asset_id: *id,
                    revision: pin_revision(db, *id, slot).await?,
                });
            }
        }
        Ok(resolved)
    }

    pub async fn start(
        db: &impl ConnectionTrait,
        pipeline_asset_id: i32,
        inputs: &[ResolvedSlot],
    ) -> Result<run::Model, CoreError> {
        Ok(run::ActiveModel {
            pipeline_asset_id: Set(pipeline_asset_id),
            status: Set(RunStatus::Running),
            inputs: Set(serde_json::to_value(inputs)?),
            outputs: Set(serde_json::json!([])),
            error: Set(None),
            usage: Set(None),
            created_at: Set(OffsetDateTime::now_utc()),
            finished_at: Set(None),
            ..Default::default()
        }
        .insert(db)
        .await?)
    }

    /// 成功终态:记录输出槽位(已提交的 Asset 修订)与用量。
    pub async fn succeed(
        db: &impl ConnectionTrait,
        run_id: i32,
        outputs: &[ResolvedSlot],
        usage: Option<serde_json::Value>,
    ) -> Result<(), CoreError> {
        terminal(
            db,
            run_id,
            RunStatus::Succeeded,
            serde_json::to_value(outputs)?,
            None,
            usage,
        )
        .await
    }

    pub async fn fail(
        db: &impl ConnectionTrait,
        run_id: i32,
        error: &crate::error::ApiError,
    ) -> Result<(), CoreError> {
        terminal(
            db,
            run_id,
            RunStatus::Failed,
            serde_json::json!([]),
            Some(serde_json::to_value(error)?),
            None,
        )
        .await
    }

    pub async fn cancel(db: &impl ConnectionTrait, run_id: i32) -> Result<(), CoreError> {
        terminal(
            db,
            run_id,
            RunStatus::Cancelled,
            serde_json::json!([]),
            None,
            None,
        )
        .await
    }

    pub async fn get(db: &impl ConnectionTrait, run_id: i32) -> Result<run::Model, CoreError> {
        run::Entity::find_by_id(run_id)
            .one(db)
            .await?
            .ok_or(CoreError::RunNotFound(run_id))
    }

    pub async fn list(db: &impl ConnectionTrait) -> Result<Vec<run::Model>, CoreError> {
        use sea_orm::QueryOrder;
        Ok(run::Entity::find()
            .order_by_desc(run::Column::Id)
            .all(db)
            .await?)
    }
}

async fn pin_revision(
    db: &impl ConnectionTrait,
    asset_id: i32,
    slot: &InputSlot,
) -> Result<i32, CoreError> {
    let head = asset::Entity::find_by_id(asset_id)
        .one(db)
        .await?
        .ok_or(CoreError::AssetNotFound(asset_id))?;
    if head.kind != slot.kind {
        return Err(CoreError::AssetKindMismatch {
            id: asset_id,
            expected: slot.kind,
            found: head.kind,
        });
    }
    Ok(head.head_revision)
}

fn slot_error(slot: &str, reason: &str) -> CoreError {
    CoreError::InvalidRunBinding {
        slot: slot.to_owned(),
        reason: reason.to_owned(),
    }
}

async fn terminal(
    db: &impl ConnectionTrait,
    run_id: i32,
    status: RunStatus,
    outputs: serde_json::Value,
    error: Option<serde_json::Value>,
    usage: Option<serde_json::Value>,
) -> Result<(), CoreError> {
    let mut update = run::Entity::update_many()
        .col_expr(run::Column::Status, status.into())
        .col_expr(run::Column::Outputs, outputs.into())
        .col_expr(
            run::Column::FinishedAt,
            Some(OffsetDateTime::now_utc()).into(),
        )
        .filter(run::Column::Id.eq(run_id));
    if let Some(error) = error {
        update = update.col_expr(run::Column::Error, Some(error).into());
    }
    if let Some(usage) = usage {
        update = update.col_expr(run::Column::Usage, Some(usage).into());
    }
    update.exec(db).await?;
    Ok(())
}
