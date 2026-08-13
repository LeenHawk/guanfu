use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set,
    TransactionSession, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::entities::{channel, credential, routing_rule};
use crate::CoreError;

/// 对适配层暴露的渠道视图。
#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct ChannelDto {
    pub id: i32,
    pub name: String,
    pub base_url: String,
    pub enabled: bool,
}

/// 凭证视图；不含 secret，避免泄露到前端。
#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct CredentialDto {
    pub id: i32,
    pub channel_id: i32,
    pub label: String,
    pub weight: i32,
    pub disabled: bool,
    pub failure_count: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct NewChannel {
    pub name: String,
    pub base_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct NewCredential {
    pub channel_id: i32,
    pub label: String,
    pub secret: String,
    pub weight: i32,
}

impl From<channel::Model> for ChannelDto {
    fn from(m: channel::Model) -> Self {
        Self {
            id: m.id,
            name: m.name,
            base_url: m.base_url,
            enabled: m.enabled,
        }
    }
}

impl From<credential::Model> for CredentialDto {
    fn from(m: credential::Model) -> Self {
        Self {
            id: m.id,
            channel_id: m.channel_id,
            label: m.label,
            weight: m.weight,
            disabled: m.disabled,
            failure_count: m.failure_count,
        }
    }
}

/// 渠道与凭证的持久化管理，接口与传输层无关。
pub struct ChannelService;

impl ChannelService {
    pub async fn create_channel(
        db: &impl ConnectionTrait,
        input: NewChannel,
    ) -> Result<ChannelDto, CoreError> {
        let m = channel::ActiveModel {
            name: Set(input.name),
            base_url: Set(input.base_url),
            enabled: Set(true),
            created_at: Set(OffsetDateTime::now_utc()),
            ..Default::default()
        }
        .insert(db)
        .await?;
        Ok(m.into())
    }

    pub async fn list_channels(db: &impl ConnectionTrait) -> Result<Vec<ChannelDto>, CoreError> {
        Ok(channel::Entity::find()
            .all(db)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub async fn set_channel_enabled(
        db: &impl ConnectionTrait,
        id: i32,
        enabled: bool,
    ) -> Result<(), CoreError> {
        let m = channel::Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or(CoreError::ChannelNotFound(id))?;
        let mut am: channel::ActiveModel = m.into();
        am.enabled = Set(enabled);
        am.update(db).await?;
        Ok(())
    }

    /// 删除渠道及其全部凭证（同一事务）。
    pub async fn delete_channel(
        db: &(impl ConnectionTrait + TransactionTrait),
        id: i32,
    ) -> Result<(), CoreError> {
        let txn = db.begin().await?;
        routing_rule::Entity::delete_many()
            .filter(routing_rule::Column::ChannelId.eq(id))
            .exec(&txn)
            .await?;
        credential::Entity::delete_many()
            .filter(credential::Column::ChannelId.eq(id))
            .exec(&txn)
            .await?;
        channel::Entity::delete_by_id(id).exec(&txn).await?;
        txn.commit().await?;
        Ok(())
    }

    pub async fn add_credential(
        db: &impl ConnectionTrait,
        input: NewCredential,
    ) -> Result<CredentialDto, CoreError> {
        channel::Entity::find_by_id(input.channel_id)
            .one(db)
            .await?
            .ok_or(CoreError::ChannelNotFound(input.channel_id))?;
        let m = credential::ActiveModel {
            channel_id: Set(input.channel_id),
            label: Set(input.label),
            secret: Set(input.secret),
            weight: Set(input.weight),
            disabled: Set(false),
            failure_count: Set(0),
            ..Default::default()
        }
        .insert(db)
        .await?;
        Ok(m.into())
    }

    pub async fn list_credentials(
        db: &impl ConnectionTrait,
        channel_id: i32,
    ) -> Result<Vec<CredentialDto>, CoreError> {
        Ok(credential::Entity::find()
            .filter(credential::Column::ChannelId.eq(channel_id))
            .all(db)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub async fn remove_credential(db: &impl ConnectionTrait, id: i32) -> Result<(), CoreError> {
        credential::Entity::delete_by_id(id).exec(db).await?;
        Ok(())
    }
}
