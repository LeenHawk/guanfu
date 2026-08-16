//! 账号与会话。
//!
//! 首个注册者成为管理员,之后的账号只能由管理员创建——对外服务不能靠
//! "谁先访问谁就是管理员"之外的自助注册,那等于把入口敞开。
//!
//! 令牌只在签发时出现一次,库里存的是它的 sha256:数据库泄露不等于
//! 会话可被冒充。口令用 argon2id,盐与参数都在编码串里。

// 用 argon2 re-export 的 rand_core,避免和另一版 rand 在同一图里打架。
use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};

use crate::entities::{session, user};
use crate::CoreError;

/// 会话有效期;到期需要重新登录。
const SESSION_TTL_DAYS: i64 = 30;

/// 请求的发起者。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Actor {
    pub user_id: i32,
    pub is_admin: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct UserDto {
    pub id: i32,
    pub name: String,
    pub is_admin: bool,
}

impl From<user::Model> for UserDto {
    fn from(m: user::Model) -> Self {
        Self {
            id: m.id,
            name: m.name,
            is_admin: m.is_admin,
        }
    }
}

/// 登录成功的返回;`token` 只在此刻出现一次。
#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct SessionDto {
    pub token: String,
    pub user: UserDto,
}

/// 会话视图;`id` 是令牌的哈希,不是令牌本身。
#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct SessionSummary {
    pub id: String,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    /// 是否为发起本次请求的会话。
    pub current: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct Credentials {
    pub name: String,
    pub password: String,
}

pub struct AuthService;

impl AuthService {
    /// 是否还没有任何账号——前端据此决定显示"注册管理员"还是"登录"。
    pub async fn needs_setup(db: &impl ConnectionTrait) -> Result<bool, CoreError> {
        Ok(user::Entity::find().count(db).await? == 0)
    }

    /// 注册。首个账号自动成为管理员;之后必须由管理员发起。
    pub async fn register(
        db: &impl ConnectionTrait,
        actor: Option<Actor>,
        input: &Credentials,
        make_admin: bool,
    ) -> Result<UserDto, CoreError> {
        let first = Self::needs_setup(db).await?;
        if !first && !actor.is_some_and(|actor| actor.is_admin) {
            return Err(CoreError::Forbidden {
                reason: "only an administrator can create accounts".to_owned(),
            });
        }
        let name = input.name.trim();
        if name.is_empty() || input.password.len() < 8 {
            return Err(CoreError::InvalidCredentials {
                reason: "name must be present and password at least 8 characters".to_owned(),
            });
        }
        if user::Entity::find()
            .filter(user::Column::Name.eq(name))
            .one(db)
            .await?
            .is_some()
        {
            return Err(CoreError::InvalidCredentials {
                reason: "that name is taken".to_owned(),
            });
        }

        let created = user::ActiveModel {
            name: Set(name.to_owned()),
            password_hash: Set(hash_password(&input.password)?),
            is_admin: Set(first || make_admin),
            created_at: Set(OffsetDateTime::now_utc()),
            ..Default::default()
        }
        .insert(db)
        .await?;
        Ok(created.into())
    }

    pub async fn login(
        db: &impl ConnectionTrait,
        input: &Credentials,
    ) -> Result<SessionDto, CoreError> {
        let found = user::Entity::find()
            .filter(user::Column::Name.eq(input.name.trim()))
            .one(db)
            .await?;
        // 用户不存在与口令错误返回同一个错误,不泄露账号是否存在。
        let user = found.ok_or_else(invalid_login)?;
        verify_password(&input.password, &user.password_hash)?;

        let token = new_token();
        session::ActiveModel {
            token_hash: Set(token_hash(&token)),
            user_id: Set(user.id),
            created_at: Set(OffsetDateTime::now_utc()),
            expires_at: Set(OffsetDateTime::now_utc() + Duration::days(SESSION_TTL_DAYS)),
        }
        .insert(db)
        .await?;
        Ok(SessionDto {
            token,
            user: user.into(),
        })
    }

    /// 用令牌换取发起者;过期会话视同未登录。
    pub async fn actor_for(
        db: &impl ConnectionTrait,
        token: &str,
    ) -> Result<(Actor, UserDto), CoreError> {
        let session = session::Entity::find_by_id(token_hash(token))
            .one(db)
            .await?
            .filter(|session| session.expires_at > OffsetDateTime::now_utc())
            .ok_or_else(invalid_login)?;
        let user = user::Entity::find_by_id(session.user_id)
            .one(db)
            .await?
            .ok_or_else(invalid_login)?;
        Ok((
            Actor {
                user_id: user.id,
                is_admin: user.is_admin,
            },
            user.into(),
        ))
    }

    /// 列出自己的会话;`current` 标出发起本次请求的那一个。
    ///
    /// 只回 token 的哈希前缀作标识——完整令牌不该再出现第二次,
    /// 哪怕是给本人看。
    pub async fn list_sessions(
        db: &impl ConnectionTrait,
        actor: Actor,
        current_token: Option<&str>,
    ) -> Result<Vec<SessionSummary>, CoreError> {
        let current = current_token.map(token_hash);
        Ok(session::Entity::find()
            .filter(session::Column::UserId.eq(actor.user_id))
            .order_by_desc(session::Column::CreatedAt)
            .all(db)
            .await?
            .into_iter()
            .map(|row| SessionSummary {
                current: current.as_deref() == Some(row.token_hash.as_str()),
                id: row.token_hash,
                created_at_ms: to_millis(row.created_at),
                expires_at_ms: to_millis(row.expires_at),
            })
            .collect())
    }

    /// 吊销一个会话。只能吊销自己的——`id` 是会话哈希,
    /// 猜到别人的哈希也动不了别人的会话。
    pub async fn revoke_session(
        db: &impl ConnectionTrait,
        actor: Actor,
        id: &str,
    ) -> Result<(), CoreError> {
        session::Entity::delete_many()
            .filter(session::Column::TokenHash.eq(id))
            .filter(session::Column::UserId.eq(actor.user_id))
            .exec(db)
            .await?;
        Ok(())
    }

    /// 吊销自己的全部会话(可保留当前这一个)。
    pub async fn revoke_all_sessions(
        db: &impl ConnectionTrait,
        actor: Actor,
        keep_token: Option<&str>,
    ) -> Result<u64, CoreError> {
        let mut query =
            session::Entity::delete_many().filter(session::Column::UserId.eq(actor.user_id));
        if let Some(keep) = keep_token {
            query = query.filter(session::Column::TokenHash.ne(token_hash(keep)));
        }
        Ok(query.exec(db).await?.rows_affected)
    }

    pub async fn logout(db: &impl ConnectionTrait, token: &str) -> Result<(), CoreError> {
        session::Entity::delete_by_id(token_hash(token))
            .exec(db)
            .await?;
        Ok(())
    }

    pub async fn list_users(db: &impl ConnectionTrait) -> Result<Vec<UserDto>, CoreError> {
        Ok(user::Entity::find()
            .order_by_asc(user::Column::Id)
            .all(db)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// 桌面壳的本地用户:单用户进程,不登录也要有归属主体。
    pub async fn local_actor(db: &impl ConnectionTrait) -> Result<Actor, CoreError> {
        const LOCAL: &str = "local";
        if let Some(existing) = user::Entity::find()
            .filter(user::Column::Name.eq(LOCAL))
            .one(db)
            .await?
        {
            return Ok(Actor {
                user_id: existing.id,
                is_admin: existing.is_admin,
            });
        }
        // 口令随机且不外发:桌面端不经登录端点,这个账号只作归属用。
        let created = user::ActiveModel {
            name: Set(LOCAL.to_owned()),
            password_hash: Set(hash_password(&new_token())?),
            is_admin: Set(true),
            created_at: Set(OffsetDateTime::now_utc()),
            ..Default::default()
        }
        .insert(db)
        .await?;
        Ok(Actor {
            user_id: created.id,
            is_admin: true,
        })
    }
}

fn invalid_login() -> CoreError {
    CoreError::InvalidCredentials {
        reason: "invalid name or password".to_owned(),
    }
}

fn hash_password(password: &str) -> Result<String, CoreError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| CoreError::InvalidCredentials {
            reason: error.to_string(),
        })
}

fn verify_password(password: &str, encoded: &str) -> Result<(), CoreError> {
    let parsed = PasswordHash::new(encoded).map_err(|_| invalid_login())?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| invalid_login())
}

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn to_millis(at: OffsetDateTime) -> i64 {
    (at.unix_timestamp_nanos() / 1_000_000) as i64
}

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}
