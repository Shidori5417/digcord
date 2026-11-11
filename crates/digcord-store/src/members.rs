//! Kanal üye listesi (`GUILD_MEMBER_LIST_UPDATE`): guild başına güncel liste; SYNC / INSERT /
//! UPDATE / DELETE / INVALIDATE uygulanır. Dizinler grup başlıkları dahil düz listedir.

use digcord_model::{GroupCount, ListItem, ListOp, MemberListUpdate, Snowflake};

use crate::{MemberInfo, State};

/// Bozuk/kötü niyetli dizinlerin belleği şişirmemesi için listenin üst boyutu.
const MAX_ITEMS: usize = 100_000;

/// Bir guild'in şu anki üye listesi. `None` öğeler henüz alınmamış (SYNC beklenen) konumlardır.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MemberList {
    pub list_id: String,
    pub member_count: u32,
    pub online_count: u32,
    pub groups: Vec<GroupCount>,
    items: Vec<Option<ListItem>>,
}

impl MemberList {
    pub fn items(&self) -> &[Option<ListItem>] {
        &self.items
    }

    fn pad_to(&mut self, len: usize) -> bool {
        if len > MAX_ITEMS {
            return false;
        }
        if self.items.len() < len {
            self.items.resize(len, None);
        }
        true
    }

    fn apply(&mut self, op: &ListOp) {
        match op {
            ListOp::Sync { start, end, items } => {
                let Some(stop) = start.checked_add(items.len()) else {
                    return;
                };
                if !self.pad_to(stop) {
                    return;
                }
                for (slot, item) in self.items[*start..stop].iter_mut().zip(items) {
                    *slot = Some(item.clone());
                }
                // İstenen aralıktan az öğe döndüyse liste orada biter.
                if end.saturating_sub(*start) + 1 > items.len() {
                    self.items.truncate(stop);
                }
            }
            ListOp::Insert { index, item } => {
                if *index > MAX_ITEMS || !self.pad_to(*index) {
                    return;
                }
                if self.items.len() < MAX_ITEMS {
                    self.items.insert(*index, Some(item.clone()));
                }
            }
            ListOp::Update { index, item } => {
                if self.pad_to(index.saturating_add(1)) {
                    self.items[*index] = Some(item.clone());
                }
            }
            ListOp::Delete { index } => {
                if *index < self.items.len() {
                    self.items.remove(*index);
                }
            }
            ListOp::Invalidate { start, end } => {
                let end = (*end).min(self.items.len().saturating_sub(1));
                if *start < self.items.len() && *start <= end {
                    for slot in &mut self.items[*start..=end] {
                        *slot = None;
                    }
                }
            }
        }
    }
}

impl State {
    /// Guild'in güncel üye listesi (hiç alınmadıysa `None`).
    pub fn member_list(&self, guild: Snowflake) -> Option<&MemberList> {
        self.member_lists.get(&guild)
    }

    /// Listeyi günceller; liste kimliği değiştiyse (başka kanal/izin kümesi) eskisi atılır.
    /// Üyelerin kullanıcıları ve takma adları yazar adı çözümlemesi için de kaydedilir.
    pub fn apply_member_list(&mut self, update: &MemberListUpdate) {
        self.version += 1;
        for op in &update.ops {
            let items: &[ListItem] = match op {
                ListOp::Sync { items, .. } => items,
                ListOp::Insert { item, .. } | ListOp::Update { item, .. } => {
                    std::slice::from_ref(item)
                }
                ListOp::Delete { .. } | ListOp::Invalidate { .. } => &[],
            };
            for item in items {
                if let ListItem::Member(m) = item {
                    self.users.insert(m.user.id, m.user.clone());
                    self.members.insert(
                        (update.guild_id, m.user.id),
                        MemberInfo {
                            nick: m.nick.clone(),
                            roles: m.roles.clone(),
                        },
                    );
                }
            }
        }
        let list = self.member_lists.entry(update.guild_id).or_default();
        if list.list_id != update.list_id {
            *list = MemberList {
                list_id: update.list_id.clone(),
                ..MemberList::default()
            };
        }
        list.member_count = update.member_count;
        list.online_count = update.online_count;
        list.groups.clone_from(&update.groups);
        for op in &update.ops {
            list.apply(op);
        }
    }

    /// Üyenin rollerinden görünen rengi: rengi olan en yüksek konumlu rol (`0` = renksiz).
    pub fn member_color(&self, guild: Snowflake, roles: &[Snowflake]) -> Option<u32> {
        self.guilds
            .get(&guild)?
            .roles
            .iter()
            .filter(|r| r.color != 0 && roles.contains(&r.id))
            .max_by_key(|r| r.position)
            .map(|r| r.color)
    }
}

#[cfg(test)]
mod tests {
    use digcord_model::{Guild, Presence};
    use serde_json::{Value, json};

    use super::*;

    fn sf(n: u64) -> Snowflake {
        Snowflake(n)
    }

    fn member(id: u64, nick: Option<&str>) -> Value {
        json!({"member": {"user": {"id": id.to_string(), "username": format!("u{id}"), "global_name": null},
                          "roles": ["7"], "nick": nick, "presence": {"status": "idle"}}})
    }

    fn group(id: &str) -> Value {
        json!({"group": {"id": id}})
    }

    fn update(list_id: &str, ops: Value) -> MemberListUpdate {
        let d = json!({"id": list_id, "guild_id": "1", "member_count": 9, "online_count": 3,
                       "groups": [{"id": "online", "count": 2}], "ops": ops});
        MemberListUpdate::parse(&d.to_string()).expect("geçerli")
    }

    /// Listenin kısa gösterimi: `G` grup, `.` boş, `u<id>` üye.
    fn shape(st: &State) -> Vec<String> {
        st.member_list(sf(1))
            .expect("liste")
            .items()
            .iter()
            .map(|i| match i {
                None => ".".to_owned(),
                Some(ListItem::Group { .. }) => "G".to_owned(),
                Some(ListItem::Member(m)) => format!("u{}", m.user.id),
            })
            .collect()
    }

    fn sync(st: &mut State) {
        st.apply_member_list(&update(
            "a",
            json!([{"op": "SYNC", "range": [0, 99],
                    "items": [group("online"), member(1, None), member(2, Some("Iki")), group("offline")]}]),
        ));
    }

    #[test]
    fn sync_then_insert_update_delete() {
        let mut st = State::new();
        sync(&mut st);
        assert_eq!(shape(&st), ["G", "u1", "u2", "G"]);
        let l = st.member_list(sf(1)).expect("liste");
        assert_eq!((l.member_count, l.online_count, l.groups.len()), (9, 3, 1));

        st.apply_member_list(&update(
            "a",
            json!([{"op": "INSERT", "index": 2, "item": member(3, None)}]),
        ));
        assert_eq!(shape(&st), ["G", "u1", "u3", "u2", "G"]);
        st.apply_member_list(&update(
            "a",
            json!([{"op": "UPDATE", "index": 1, "item": member(4, None)}]),
        ));
        assert_eq!(shape(&st), ["G", "u4", "u3", "u2", "G"]);
        st.apply_member_list(&update("a", json!([{"op": "DELETE", "index": 0}])));
        assert_eq!(shape(&st), ["u4", "u3", "u2", "G"]);
    }

    #[test]
    fn invalidate_clears_range_and_sync_refills() {
        let mut st = State::new();
        sync(&mut st);
        st.apply_member_list(&update("a", json!([{"op": "INVALIDATE", "range": [1, 2]}])));
        assert_eq!(shape(&st), ["G", ".", ".", "G"]);
        st.apply_member_list(&update(
            "a",
            json!([{"op": "SYNC", "range": [1, 2], "items": [member(8, None), member(9, None)]}]),
        ));
        assert_eq!(shape(&st), ["G", "u8", "u9", "G"]);
    }

    #[test]
    fn short_sync_truncates_and_far_sync_pads() {
        let mut st = State::new();
        sync(&mut st);
        // [0,99] istendi, 1 öğe döndü: liste orada biter.
        st.apply_member_list(&update(
            "a",
            json!([{"op": "SYNC", "range": [0, 99], "items": [group("online")]}]),
        ));
        assert_eq!(shape(&st), ["G"]);
        // İkinci sayfa: araya boş yer eklenir.
        st.apply_member_list(&update(
            "a",
            json!([{"op": "SYNC", "range": [3, 5], "items": [member(7, None), member(8, None), member(9, None)]}]),
        ));
        assert_eq!(shape(&st), ["G", ".", ".", "u7", "u8", "u9"]);
    }

    #[test]
    fn out_of_range_ops_are_safe() {
        let mut st = State::new();
        sync(&mut st);
        st.apply_member_list(&update(
            "a",
            json!([
                {"op": "DELETE", "index": 50},
                {"op": "INVALIDATE", "range": [40, 90]},
                {"op": "INVALIDATE", "range": [3, 1]},
                {"op": "UPDATE", "index": 6, "item": member(5, None)},
                {"op": "INSERT", "index": 9, "item": member(6, None)},
                {"op": "INSERT", "index": 99999999, "item": member(6, None)},
                {"op": "SYNC", "range": [99999999, 99999999], "items": [member(6, None)]},
            ]),
        ));
        assert_eq!(
            shape(&st),
            ["G", "u1", "u2", "G", ".", ".", "u5", ".", ".", "u6"]
        );
        // Boş liste üzerinde de panik yok.
        let mut empty = State::new();
        empty.apply_member_list(&update(
            "z",
            json!([{"op": "DELETE", "index": 0}, {"op": "INVALIDATE", "range": [0, 0]}]),
        ));
        assert!(shape(&empty).is_empty());
    }

    #[test]
    fn new_list_id_replaces_old_list() {
        let mut st = State::new();
        sync(&mut st);
        st.apply_member_list(&update(
            "b",
            json!([{"op": "SYNC", "range": [0, 99], "items": [group("offline")]}]),
        ));
        let l = st.member_list(sf(1)).expect("liste");
        assert_eq!(l.list_id, "b");
        assert_eq!(l.items().len(), 1);
    }

    #[test]
    fn records_users_and_nicks_for_author_names() {
        let mut st = State::new();
        sync(&mut st);
        let u = st.user(sf(2)).expect("kullanıcı kaydedildi");
        assert_eq!(st.author_name(Some(sf(1)), u), "Iki");
        let Some(Some(ListItem::Member(m))) = st.member_list(sf(1)).map(|l| &l.items()[1]) else {
            panic!("üye bekleniyordu");
        };
        assert_eq!(m.presence, Presence::Idle);
    }

    #[test]
    fn member_color_picks_highest_colored_role() {
        let mut st = State::new();
        let g: Guild = serde_json::from_value(json!({
            "id": "1",
            "properties": {"name": "G", "owner_id": "5"},
            "roles": [
                {"id": "7", "name": "alt", "permissions": "0", "position": 1, "color": 255},
                {"id": "8", "name": "ust", "permissions": "0", "position": 5, "color": 65280},
                {"id": "9", "name": "renksiz", "permissions": "0", "position": 9, "color": 0},
            ],
        }))
        .expect("guild");
        st.upsert_guild(&g);
        assert_eq!(st.member_color(sf(1), &[sf(7), sf(9)]), Some(255));
        assert_eq!(st.member_color(sf(1), &[sf(7), sf(8), sf(9)]), Some(65280));
        assert_eq!(st.member_color(sf(1), &[sf(9)]), None);
        assert_eq!(st.member_color(sf(2), &[sf(7)]), None);
        assert_eq!(st.role(sf(1), sf(8)).map(|r| r.name.as_str()), Some("ust"));
    }
}
