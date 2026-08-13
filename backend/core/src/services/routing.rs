use gproxy_protocol::OperationKey;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::entities::routing_rule::{RouteImplementation, RoutingKind, RoutingOperation};
use crate::entities::{channel, routing_rule};
use crate::llm::routing;
use crate::CoreError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct OperationKeyDto {
    pub operation: RoutingOperation,
    pub kind: RoutingKind,
}

impl TryFrom<OperationKeyDto> for OperationKey {
    type Error = CoreError;

    fn try_from(value: OperationKeyDto) -> Result<Self, Self::Error> {
        OperationKey::try_new(value.operation.into(), value.kind.into()).map_err(|error| {
            CoreError::InvalidRoutingRule {
                id: None,
                reason: error.to_string(),
            }
        })
    }
}

impl TryFrom<OperationKey> for OperationKeyDto {
    type Error = CoreError;

    fn try_from(value: OperationKey) -> Result<Self, Self::Error> {
        Ok(Self {
            operation: routing::store_operation(value.operation())?,
            kind: routing::store_kind(value.kind())?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoutingImplementation {
    Passthrough,
    TransformTo { target: OperationKeyDto },
    Local,
    Unsupported,
}

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct RoutingRuleDto {
    pub id: i32,
    pub channel_id: i32,
    pub source: OperationKeyDto,
    pub implementation: RoutingImplementation,
    pub sort_order: i32,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct PutRoutingRule {
    pub channel_id: i32,
    pub source: OperationKeyDto,
    pub implementation: RoutingImplementation,
    pub sort_order: i32,
    pub enabled: bool,
}

pub struct RoutingService;

impl RoutingService {
    /// 按源 `(operation, kind)` 原子 upsert 一个路由单元格。
    pub async fn put_rule(
        db: &impl ConnectionTrait,
        input: PutRoutingRule,
    ) -> Result<RoutingRuleDto, CoreError> {
        channel::Entity::find_by_id(input.channel_id)
            .one(db)
            .await?
            .ok_or(CoreError::ChannelNotFound(input.channel_id))?;

        let now = OffsetDateTime::now_utc();
        let source: OperationKey = input.source.try_into()?;
        let operation = routing::store_operation(source.operation())?;
        let kind = routing::store_kind(source.kind())?;
        let (implementation, dest_operation, dest_kind) = encode(&input.implementation)?;
        let model = routing_rule::ActiveModel {
            channel_id: Set(input.channel_id),
            operation: Set(operation),
            kind: Set(kind),
            implementation: Set(implementation),
            dest_operation: Set(dest_operation),
            dest_kind: Set(dest_kind),
            sort_order: Set(input.sort_order),
            enabled: Set(input.enabled),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        routing_rule::Entity::insert(model)
            .on_conflict(
                OnConflict::columns([
                    routing_rule::Column::ChannelId,
                    routing_rule::Column::Operation,
                    routing_rule::Column::Kind,
                ])
                .update_columns([
                    routing_rule::Column::Implementation,
                    routing_rule::Column::DestOperation,
                    routing_rule::Column::DestKind,
                    routing_rule::Column::SortOrder,
                    routing_rule::Column::Enabled,
                    routing_rule::Column::UpdatedAt,
                ])
                .to_owned(),
            )
            .exec_without_returning(db)
            .await?;

        let model = routing_rule::Entity::find()
            .filter(routing_rule::Column::ChannelId.eq(input.channel_id))
            .filter(routing_rule::Column::Operation.eq(operation))
            .filter(routing_rule::Column::Kind.eq(kind))
            .one(db)
            .await?
            .expect("upserted routing rule must exist");
        decode(model)
    }

    pub async fn list_rules(
        db: &impl ConnectionTrait,
        channel_id: i32,
    ) -> Result<Vec<RoutingRuleDto>, CoreError> {
        routing_rule::Entity::find()
            .filter(routing_rule::Column::ChannelId.eq(channel_id))
            .order_by_asc(routing_rule::Column::SortOrder)
            .all(db)
            .await?
            .into_iter()
            .map(decode)
            .collect()
    }

    pub async fn remove_rule(db: &impl ConnectionTrait, id: i32) -> Result<(), CoreError> {
        routing_rule::Entity::delete_by_id(id).exec(db).await?;
        Ok(())
    }
}

fn encode(
    value: &RoutingImplementation,
) -> Result<
    (
        RouteImplementation,
        Option<RoutingOperation>,
        Option<RoutingKind>,
    ),
    CoreError,
> {
    Ok(match value {
        RoutingImplementation::Passthrough => (RouteImplementation::Passthrough, None, None),
        RoutingImplementation::Local => (RouteImplementation::Local, None, None),
        RoutingImplementation::Unsupported => (RouteImplementation::Unsupported, None, None),
        RoutingImplementation::TransformTo { target } => (
            RouteImplementation::TransformTo,
            Some(target.operation),
            Some(target.kind),
        ),
    })
}

fn decode(model: routing_rule::Model) -> Result<RoutingRuleDto, CoreError> {
    let (source, decision) = routing::compile_rule(&model)?;
    let implementation = match decision {
        routing::RouteDecision::Passthrough => RoutingImplementation::Passthrough,
        routing::RouteDecision::TransformTo(target) => RoutingImplementation::TransformTo {
            target: target.try_into()?,
        },
        routing::RouteDecision::Local => RoutingImplementation::Local,
        routing::RouteDecision::Unsupported => RoutingImplementation::Unsupported,
    };
    Ok(RoutingRuleDto {
        id: model.id,
        channel_id: model.channel_id,
        source: source.try_into()?,
        implementation,
        sort_order: model.sort_order,
        enabled: model.enabled,
    })
}
