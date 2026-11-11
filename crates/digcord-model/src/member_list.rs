//! `GUILD_MEMBER_LIST_UPDATE`: kanal üye listesi (op 37 `channels` aboneliğine yanıt).
//! Biçim web istemcisinden gözlendi; `INVALIDATE` gözlenmedi (doğrulanmadı, PROTOCOL_NOTES §4.6).

use serde::Deserialize;

use crate::{Snowflake, User};

/// Üyenin çevrimiçi durumu. Sunucu `presence` göndermezse çevrimdışı sayılır.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Presence {
    Online,
    Idle,
    Dnd,
    #[default]
    Offline,
}

impl Presence {
    fn parse(s: &str) -> Self {
        match s {
            "online" => Self::Online,
            "idle" => Self::Idle,
            "dnd" => Self::Dnd,
            _ => Self::Offline,
        }
    }
}

/// Liste grubunun kimliği: bir rol ya da `online` / `offline`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupId {
    Role(Snowflake),
    Online,
    Offline,
}

/// Listedeki bir üye satırı (yalnızca gösterim için gerekenler).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListMember {
    pub user: User,
    pub nick: Option<String>,
    pub roles: Vec<Snowflake>,
    pub presence: Presence,
}

/// Düz listedeki bir öğe: grup başlığı ya da üye.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListItem {
    Group { id: GroupId, count: Option<u32> },
    Member(Box<ListMember>),
}

/// `groups` dizisindeki özet (başlık sayıları).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupCount {
    pub id: GroupId,
    pub count: u32,
}

/// Güncelleme işlemi. Dizinler düz listedeki (grup + üye) konumlardır.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListOp {
    Sync {
        start: usize,
        end: usize,
        items: Vec<ListItem>,
    },
    Insert {
        index: usize,
        item: ListItem,
    },
    Update {
        index: usize,
        item: ListItem,
    },
    Delete {
        index: usize,
    },
    /// Aralık geçersiz: istemci o aralığı atar (yeniden SYNC bekler).
    Invalidate {
        start: usize,
        end: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberListUpdate {
    pub guild_id: Snowflake,
    /// Liste kimliği (`everyone` ya da izin kümesine ait özet).
    pub list_id: String,
    pub member_count: u32,
    pub online_count: u32,
    pub groups: Vec<GroupCount>,
    pub ops: Vec<ListOp>,
}

// ------------------------------------------------------------------ ham biçim

#[derive(Deserialize)]
struct RawUpdate {
    guild_id: Snowflake,
    id: String,
    #[serde(default)]
    member_count: u32,
    #[serde(default)]
    online_count: u32,
    #[serde(default)]
    groups: Vec<RawGroup>,
    #[serde(default)]
    ops: Vec<RawOp>,
}

#[derive(Deserialize)]
struct RawGroup {
    id: String,
    #[serde(default)]
    count: Option<u32>,
}

#[derive(Deserialize)]
struct RawOp {
    op: String,
    #[serde(default)]
    range: Option<(usize, usize)>,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    items: Vec<RawItem>,
    #[serde(default)]
    item: Option<RawItem>,
}

#[derive(Deserialize)]
struct RawItem {
    group: Option<RawGroup>,
    member: Option<RawMember>,
}

#[derive(Deserialize)]
struct RawMember {
    user: User,
    #[serde(default)]
    nick: Option<String>,
    #[serde(default)]
    roles: Vec<Snowflake>,
    #[serde(default)]
    presence: Option<RawPresence>,
}

#[derive(Deserialize)]
struct RawPresence {
    #[serde(default)]
    status: String,
}

fn group_id(s: &str) -> Option<GroupId> {
    match s {
        "online" => Some(GroupId::Online),
        "offline" => Some(GroupId::Offline),
        other => other.parse().ok().map(|n| GroupId::Role(Snowflake(n))),
    }
}

impl RawItem {
    fn convert(self) -> Option<ListItem> {
        if let Some(g) = self.group {
            return Some(ListItem::Group {
                id: group_id(&g.id)?,
                count: g.count,
            });
        }
        let m = self.member?;
        Some(ListItem::Member(Box::new(ListMember {
            user: m.user,
            nick: m.nick.filter(|n| !n.is_empty()),
            roles: m.roles,
            presence: m
                .presence
                .map_or(Presence::Offline, |p| Presence::parse(&p.status)),
        })))
    }
}

impl RawOp {
    /// Tanınmayan işlem `Ok(None)` (atlanır); tanınan ama eksik/bozuk işlem `Err`.
    fn convert(self) -> Result<Option<ListOp>, ()> {
        let item = |i: Option<RawItem>| i.and_then(RawItem::convert).ok_or(());
        Ok(Some(match self.op.as_str() {
            "SYNC" => {
                let (start, end) = self.range.ok_or(())?;
                let items = self
                    .items
                    .into_iter()
                    .map(|i| i.convert().ok_or(()))
                    .collect::<Result<_, _>>()?;
                ListOp::Sync { start, end, items }
            }
            "INSERT" => ListOp::Insert {
                index: self.index.ok_or(())?,
                item: item(self.item)?,
            },
            "UPDATE" => ListOp::Update {
                index: self.index.ok_or(())?,
                item: item(self.item)?,
            },
            "DELETE" => ListOp::Delete {
                index: self.index.ok_or(())?,
            },
            "INVALIDATE" => {
                let (start, end) = self.range.ok_or(())?;
                ListOp::Invalidate { start, end }
            }
            _ => return Ok(None),
        }))
    }
}

impl MemberListUpdate {
    /// Gateway `d` nesnesini çevirir. Bozuk biçim `None` (olay atlanır, bağlantı sürer).
    pub fn parse(d: &str) -> Option<Self> {
        let raw: RawUpdate = serde_json::from_str(d).ok()?;
        let groups = raw
            .groups
            .iter()
            .map(|g| {
                Some(GroupCount {
                    id: group_id(&g.id)?,
                    count: g.count.unwrap_or(0),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let mut ops = Vec::with_capacity(raw.ops.len());
        for op in raw.ops {
            if let Some(op) = op.convert().ok()? {
                ops.push(op);
            }
        }
        Some(Self {
            guild_id: raw.guild_id,
            list_id: raw.id,
            member_count: raw.member_count,
            online_count: raw.online_count,
            groups,
            ops,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYNC: &str = r#"{"id":"abc123","guild_id":"10","member_count":5,"online_count":2,
      "groups":[{"id":"77","count":1},{"id":"online","count":1},{"id":"offline","count":3}],
      "ops":[{"op":"SYNC","range":[0,99],"items":[
        {"group":{"id":"77","count":1}},
        {"member":{"user":{"id":"1","username":"ayse","global_name":"Ayşe"},"roles":["77"],
                   "nick":"A","presence":{"status":"dnd","activities":[]}}},
        {"group":{"id":"online","count":1}},
        {"member":{"user":{"id":"2","username":"bora"},"roles":[],"nick":null,
                   "presence":{"status":"online"}}},
        {"group":{"id":"offline"}}]}]}"#;

    #[test]
    fn parses_sync_with_groups_and_members() {
        let u = MemberListUpdate::parse(SYNC).expect("geçerli");
        assert_eq!(u.guild_id, Snowflake(10));
        assert_eq!(u.list_id, "abc123");
        assert_eq!((u.member_count, u.online_count), (5, 2));
        assert_eq!(u.groups.len(), 3);
        assert_eq!(u.groups[0].id, GroupId::Role(Snowflake(77)));
        assert_eq!(u.groups[2].count, 3);
        let [ListOp::Sync { start, end, items }] = &u.ops[..] else {
            panic!("tek SYNC bekleniyordu");
        };
        assert_eq!((*start, *end, items.len()), (0, 99, 5));
        assert!(matches!(
            &items[0],
            ListItem::Group {
                id: GroupId::Role(Snowflake(77)),
                count: Some(1)
            }
        ));
        let ListItem::Member(m) = &items[1] else {
            panic!("üye bekleniyordu");
        };
        assert_eq!(m.nick.as_deref(), Some("A"));
        assert_eq!(m.presence, Presence::Dnd);
        assert_eq!(m.roles, vec![Snowflake(77)]);
        assert_eq!(m.user.display_name(), "Ayşe");
        assert!(matches!(
            &items[4],
            ListItem::Group {
                id: GroupId::Offline,
                count: None
            }
        ));
    }

    #[test]
    fn missing_presence_is_offline() {
        let d = r#"{"id":"x","guild_id":"1","ops":[{"op":"INSERT","index":3,"item":
          {"member":{"user":{"id":"2","username":"u"},"roles":[]}}}]}"#;
        let u = MemberListUpdate::parse(d).expect("geçerli");
        let [
            ListOp::Insert {
                index: 3,
                item: ListItem::Member(m),
            },
        ] = &u.ops[..]
        else {
            panic!("INSERT bekleniyordu");
        };
        assert_eq!(m.presence, Presence::Offline);
    }

    #[test]
    fn parses_update_delete_invalidate_and_skips_unknown_ops() {
        let d = r#"{"id":"x","guild_id":"1","ops":[
          {"op":"UPDATE","index":1,"item":{"group":{"id":"online","count":4}}},
          {"op":"DELETE","index":2},
          {"op":"INVALIDATE","range":[0,99]},
          {"op":"GELECEK_ISLEM","index":1}]}"#;
        let u = MemberListUpdate::parse(d).expect("geçerli");
        assert_eq!(u.ops.len(), 3);
        assert!(matches!(u.ops[0], ListOp::Update { index: 1, .. }));
        assert_eq!(u.ops[1], ListOp::Delete { index: 2 });
        assert_eq!(u.ops[2], ListOp::Invalidate { start: 0, end: 99 });
    }

    #[test]
    fn malformed_is_none() {
        for d in [
            "{}",
            "[]",
            r#"{"id":"x"}"#,
            r#"{"id":"x","guild_id":"1","ops":[{"op":"SYNC","items":[]}]}"#,
            r#"{"id":"x","guild_id":"1","ops":[{"op":"DELETE"}]}"#,
            r#"{"id":"x","guild_id":"1","ops":[{"op":"INSERT","index":0}]}"#,
            r#"{"id":"x","guild_id":"1","ops":[{"op":"SYNC","range":[0,1],"items":[{"x":1}]}]}"#,
            r#"{"id":"x","guild_id":"1","groups":[{"id":"zzz","count":1}]}"#,
            r#"{"id":"x","guild_id":"1","ops":[{"op":"DELETE","index":-1}]}"#,
        ] {
            assert!(MemberListUpdate::parse(d).is_none(), "{d}");
        }
    }
}
