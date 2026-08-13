use chrono::{DateTime, Duration, Utc};

use crate::entities::credential;

/// 上游失败分类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// 凭证失效（401/403）：较长冷却。
    AuthDead,
    /// 限流（429）：指数退避冷却。
    RateLimited,
    /// 瞬时故障（408/5xx）：换下一个凭证。
    Transient,
    /// 请求本身有问题（其余 4xx）：不做 failover，直接报错。
    Fatal,
}

/// None 表示成功。
pub fn classify_status(status: u16) -> Option<FailureKind> {
    match status {
        200..=399 => None,
        401 | 403 => Some(FailureKind::AuthDead),
        429 => Some(FailureKind::RateLimited),
        408 | 500..=599 => Some(FailureKind::Transient),
        _ => Some(FailureKind::Fatal),
    }
}

/// 失败后的冷却时长；Transient/Fatal 不设冷却。
pub fn cooldown_after(kind: FailureKind, failure_count: i32) -> Option<Duration> {
    match kind {
        FailureKind::AuthDead => Some(Duration::minutes(10)),
        FailureKind::RateLimited => {
            let secs = 30i64.saturating_mul(1 << failure_count.clamp(0, 5));
            Some(Duration::seconds(secs.min(15 * 60)))
        }
        FailureKind::Transient | FailureKind::Fatal => None,
    }
}

/// 可用性 = 未禁用且冷却已过期。
pub fn is_usable(c: &credential::Model, now: DateTime<Utc>) -> bool {
    !c.disabled && c.cooldown_until.is_none_or(|t| t <= now)
}

/// 凭证排序：过滤不可用 → 权重降序（同权重按 id 稳定）→ 按轮换偏移旋转。
/// 确定性、无随机，便于测试与复现。
pub fn order_credentials(
    creds: &[credential::Model],
    rotation: usize,
    now: DateTime<Utc>,
) -> Vec<&credential::Model> {
    let mut usable: Vec<&credential::Model> = creds.iter().filter(|c| is_usable(c, now)).collect();
    usable.sort_by(|a, b| b.weight.cmp(&a.weight).then(a.id.cmp(&b.id)));
    if usable.is_empty() {
        return usable;
    }
    let offset = rotation % usable.len();
    usable.rotate_left(offset);
    usable
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cred(id: i32, weight: i32, disabled: bool) -> credential::Model {
        credential::Model {
            id,
            channel_id: 1,
            label: format!("c{id}"),
            secret: String::new(),
            weight,
            disabled,
            failure_count: 0,
            cooldown_until: None,
            last_used_at: None,
        }
    }

    #[test]
    fn ordering_filters_sorts_and_rotates() {
        let now = Utc::now();
        let creds = vec![cred(1, 1, false), cred(2, 5, false), cred(3, 5, true)];
        let ordered = order_credentials(&creds, 0, now);
        assert_eq!(ordered.iter().map(|c| c.id).collect::<Vec<_>>(), vec![2, 1]);
        let rotated = order_credentials(&creds, 1, now);
        assert_eq!(rotated.iter().map(|c| c.id).collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn classification_matrix() {
        assert_eq!(classify_status(200), None);
        assert_eq!(classify_status(401), Some(FailureKind::AuthDead));
        assert_eq!(classify_status(429), Some(FailureKind::RateLimited));
        assert_eq!(classify_status(503), Some(FailureKind::Transient));
        assert_eq!(classify_status(400), Some(FailureKind::Fatal));
    }
}
