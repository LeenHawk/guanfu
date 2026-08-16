use sea_orm::entity::prelude::*;

/// 账号。
///
/// 首个注册者成为管理员;之后的账号只能由管理员创建(见
/// `AuthService::register`)。
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "user")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique)]
    pub name: String,
    /// argon2 编码串,含盐与参数;不是可逆的。
    pub password_hash: String,
    pub is_admin: bool,
    pub created_at: TimeDateTimeWithTimeZone,
    #[sea_orm(has_many)]
    pub sessions: HasMany<super::session::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
