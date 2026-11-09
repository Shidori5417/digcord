//! Kanal izinleri. Algoritma Discord geliştirici dokümanlarındaki
//! "Permission Overwrites" hesabıdır: temel izinler (@everyone + roller, sahip,
//! yönetici) → @everyone overwrite → rol overwrite'ları (toplu) → üye overwrite'ı.

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer};

use crate::Snowflake;

/// İzin bit alanı. JSON'da string olarak gelir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Permissions(pub u64);

impl Permissions {
    pub const ADMINISTRATOR: Self = Self(1 << 3);
    pub const VIEW_CHANNEL: Self = Self(1 << 10);
    pub const SEND_MESSAGES: Self = Self(1 << 11);
    pub const READ_MESSAGE_HISTORY: Self = Self(1 << 16);
    pub const ADD_REACTIONS: Self = Self(1 << 6);
    pub const MANAGE_MESSAGES: Self = Self(1 << 13);
    pub const ATTACH_FILES: Self = Self(1 << 15);
    pub const CONNECT: Self = Self(1 << 20);
    pub const MANAGE_THREADS: Self = Self(1 << 34);
    pub const SEND_MESSAGES_IN_THREADS: Self = Self(1 << 38);
    pub const ALL: Self = Self(u64::MAX);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    fn apply(self, allow: Self, deny: Self) -> Self {
        Self((self.0 & !deny.0) | allow.0)
    }
}

impl<'de> Deserialize<'de> for Permissions {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Permissions;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("string veya sayı olarak izin bit alanı")
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Permissions, E> {
                Ok(Permissions(v))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Permissions, E> {
                v.parse().map(Permissions).map_err(E::custom)
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Role {
    pub id: Snowflake,
    pub name: String,
    pub permissions: Permissions,
    #[serde(default)]
    pub position: i32,
    #[serde(default)]
    pub color: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(from = "u8")]
pub enum OverwriteKind {
    Role,
    Member,
}

impl From<u8> for OverwriteKind {
    fn from(v: u8) -> Self {
        if v == 1 { Self::Member } else { Self::Role }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Overwrite {
    pub id: Snowflake,
    #[serde(rename = "type")]
    pub kind: OverwriteKind,
    pub allow: Permissions,
    pub deny: Permissions,
}

/// Bir kullanıcının bir guild'deki izin bağlamı.
pub struct MemberContext<'a> {
    pub guild_id: Snowflake,
    pub owner_id: Option<Snowflake>,
    pub user_id: Snowflake,
    pub member_roles: &'a [Snowflake],
    pub roles: &'a [Role],
}

impl MemberContext<'_> {
    /// Kanal overwrite'larından önceki guild düzeyi izinler.
    pub fn base(&self) -> Permissions {
        if self.owner_id == Some(self.user_id) {
            return Permissions::ALL;
        }
        // @everyone rolünün id'si guild id'sine eşittir.
        let bits = self
            .roles
            .iter()
            .filter(|r| r.id == self.guild_id || self.member_roles.contains(&r.id))
            .fold(0, |acc, r| acc | r.permissions.0);
        let perms = Permissions(bits);
        if perms.contains(Permissions::ADMINISTRATOR) {
            Permissions::ALL
        } else {
            perms
        }
    }

    /// Kanal için son izinler.
    pub fn channel(&self, overwrites: &[Overwrite]) -> Permissions {
        let base = self.base();
        if base.contains(Permissions::ADMINISTRATOR) {
            return Permissions::ALL;
        }
        let mut perms = base;
        if let Some(o) = overwrites.iter().find(|o| o.id == self.guild_id) {
            perms = perms.apply(o.allow, o.deny);
        }
        let (allow, deny) = overwrites
            .iter()
            .filter(|o| o.kind == OverwriteKind::Role && self.member_roles.contains(&o.id))
            .fold((0, 0), |(a, d), o| (a | o.allow.0, d | o.deny.0));
        perms = perms.apply(Permissions(allow), Permissions(deny));
        if let Some(o) = overwrites
            .iter()
            .find(|o| o.kind == OverwriteKind::Member && o.id == self.user_id)
        {
            perms = perms.apply(o.allow, o.deny);
        }
        perms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUILD: Snowflake = Snowflake(1);
    const ME: Snowflake = Snowflake(100);
    const MOD: Snowflake = Snowflake(10);
    const MUTED: Snowflake = Snowflake(11);
    const V: Permissions = Permissions::VIEW_CHANNEL;
    const SEND: Permissions = Permissions::SEND_MESSAGES;

    fn role(id: Snowflake, perms: u64) -> Role {
        Role {
            id,
            name: String::new(),
            permissions: Permissions(perms),
            position: 0,
            color: 0,
        }
    }

    fn ow(id: Snowflake, kind: OverwriteKind, allow: Permissions, deny: Permissions) -> Overwrite {
        Overwrite {
            id,
            kind,
            allow,
            deny,
        }
    }

    fn ctx<'a>(member_roles: &'a [Snowflake], roles: &'a [Role]) -> MemberContext<'a> {
        MemberContext {
            guild_id: GUILD,
            owner_id: Some(Snowflake(999)),
            user_id: ME,
            member_roles,
            roles,
        }
    }

    #[test]
    fn deserializes_string_bits() {
        let r: Role = serde_json::from_str(
            r#"{"id":"1","name":"@everyone","permissions":"1024","position":0,"color":0,"x":1}"#,
        )
        .expect("rol");
        assert_eq!(r.permissions, V);
        let o: Overwrite =
            serde_json::from_str(r#"{"id":"5","type":1,"allow":"0","deny":"1024"}"#).expect("ow");
        assert_eq!(o.kind, OverwriteKind::Member);
    }

    #[test]
    fn everyone_and_roles_combine() {
        let roles = [role(GUILD, V.0), role(MOD, SEND.0)];
        assert_eq!(ctx(&[], &roles).base(), V);
        assert_eq!(ctx(&[MOD], &roles).base(), Permissions(V.0 | SEND.0));
    }

    #[test]
    fn owner_and_admin_see_everything() {
        let roles = [role(GUILD, 0), role(MOD, Permissions::ADMINISTRATOR.0)];
        let deny_all = [ow(GUILD, OverwriteKind::Role, Permissions(0), V)];
        assert_eq!(ctx(&[MOD], &roles).channel(&deny_all), Permissions::ALL);
        let mut owner = ctx(&[], &roles);
        owner.owner_id = Some(ME);
        assert!(owner.channel(&deny_all).contains(V));
    }

    #[test]
    fn overwrite_order_everyone_roles_member() {
        let roles = [role(GUILD, V.0), role(MOD, 0), role(MUTED, 0)];
        // @everyone gizler, MOD rolü gösterir → görür.
        let ows = [
            ow(GUILD, OverwriteKind::Role, Permissions(0), V),
            ow(MOD, OverwriteKind::Role, V, Permissions(0)),
        ];
        assert!(!ctx(&[], &roles).channel(&ows).contains(V));
        assert!(ctx(&[MOD], &roles).channel(&ows).contains(V));

        // Rol overwrite'ları toplanır, sonra uygulanır: aynı bit hem allow hem deny ise allow kazanır.
        let ows = [
            ow(MOD, OverwriteKind::Role, V, Permissions(0)),
            ow(MUTED, OverwriteKind::Role, Permissions(0), V),
        ];
        assert!(ctx(&[MOD, MUTED], &roles).channel(&ows).contains(V));

        // Üye overwrite'ı en son ve en güçlü.
        let ows = [
            ow(MOD, OverwriteKind::Role, V, Permissions(0)),
            ow(ME, OverwriteKind::Member, Permissions(0), V),
        ];
        assert!(!ctx(&[MOD], &roles).channel(&ows).contains(V));
    }

    #[test]
    fn member_overwrite_for_other_user_ignored() {
        let roles = [role(GUILD, V.0)];
        let ows = [ow(Snowflake(555), OverwriteKind::Member, Permissions(0), V)];
        assert!(ctx(&[], &roles).channel(&ows).contains(V));
    }
}
