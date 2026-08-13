use gproxy_protocol::{Operation, OperationKey, OperationKind};

use crate::entities::routing_rule::{self, RouteImplementation, RoutingKind, RoutingOperation};
use crate::CoreError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteDecision {
    Passthrough,
    TransformTo(OperationKey),
    Local,
    Unsupported,
}

/// 在一个渠道的持久化路由矩阵中查找源协议单元格。缺行即 unsupported。
pub fn decide(
    rules: &[routing_rule::Model],
    source: OperationKey,
) -> Result<RouteDecision, CoreError> {
    for rule in rules.iter().filter(|rule| rule.enabled) {
        let (rule_source, decision) = compile_rule(rule)?;
        if rule_source != source {
            continue;
        }
        return Ok(decision);
    }
    Ok(RouteDecision::Unsupported)
}

pub fn compile_rule(
    rule: &routing_rule::Model,
) -> Result<(OperationKey, RouteDecision), CoreError> {
    let source = OperationKey::try_new(rule.operation.into(), rule.kind.into())
        .map_err(|e| invalid_rule(rule.id, &e.to_string()))?;
    let decision = match rule.implementation {
        RouteImplementation::Passthrough => RouteDecision::Passthrough,
        RouteImplementation::Local => RouteDecision::Local,
        RouteImplementation::Unsupported => RouteDecision::Unsupported,
        RouteImplementation::TransformTo => {
            let operation = rule
                .dest_operation
                .map(Into::into)
                .unwrap_or(source.operation());
            let kind = rule
                .dest_kind
                .ok_or_else(|| invalid_rule(rule.id, "transform_to requires dest_kind"))
                .map(Into::into)?;
            let target = OperationKey::try_new(operation, kind)
                .map_err(|e| invalid_rule(rule.id, &e.to_string()))?;
            RouteDecision::TransformTo(target)
        }
    };
    Ok((source, decision))
}

pub fn store_operation(value: Operation) -> Result<RoutingOperation, CoreError> {
    value
        .try_into()
        .map_err(|()| CoreError::InvalidRoutingRule {
            id: None,
            reason: format!("unsupported operation {value:?}"),
        })
}

pub fn store_kind(value: OperationKind) -> Result<RoutingKind, CoreError> {
    value
        .try_into()
        .map_err(|()| CoreError::InvalidRoutingRule {
            id: None,
            reason: format!("unsupported operation kind {value:?}"),
        })
}

fn invalid_rule(id: i32, reason: &str) -> CoreError {
    CoreError::InvalidRoutingRule {
        id: Some(id),
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use gproxy_protocol::{ContentGenerationKind, Operation};
    use time::OffsetDateTime;

    use super::*;

    fn source() -> OperationKey {
        OperationKey::content_generation(
            Operation::GenerateContent,
            ContentGenerationKind::OpenAiResponses,
        )
    }

    fn rule(
        implementation: RouteImplementation,
        dest_kind: Option<RoutingKind>,
    ) -> routing_rule::Model {
        routing_rule::Model {
            id: 1,
            channel_id: 1,
            operation: RoutingOperation::GenerateContent,
            kind: RoutingKind::OpenAiResponses,
            implementation,
            dest_operation: None,
            dest_kind,
            sort_order: 0,
            enabled: true,
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
        }
    }

    #[test]
    fn missing_rule_is_unsupported() {
        assert_eq!(decide(&[], source()).unwrap(), RouteDecision::Unsupported);
    }

    #[test]
    fn transform_resolves_destination_key() {
        let decision = decide(
            &[rule(
                RouteImplementation::TransformTo,
                Some(RoutingKind::ClaudeMessages),
            )],
            source(),
        )
        .unwrap();
        assert_eq!(
            decision,
            RouteDecision::TransformTo(OperationKey::content_generation(
                Operation::GenerateContent,
                ContentGenerationKind::ClaudeMessages,
            ))
        );
    }
}
