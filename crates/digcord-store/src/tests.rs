use super::*;
use digcord_model::ThreadListSync;
use serde_json::json;

const READY: &str = include_str!("../../digcord-model/tests/fixtures/ready.json");

fn sf(n: u64) -> Snowflake {
    Snowflake(n)
}

fn user(id: u64, name: &str, global: Option<&str>) -> User {
    serde_json::from_value(json!({"id": id.to_string(), "username": name, "global_name": global}))
        .unwrap()
}

fn msg(id: u64, ch: u64, guild: Option<u64>, author: u64) -> Message {
    let mut v = json!({
        "id": id.to_string(), "channel_id": ch.to_string(),
        "author": {"id": author.to_string(), "username": format!("u{author}"), "global_name": null},
        "content": format!("m{id}"),
    });
    if let Some(g) = guild {
        v["guild_id"] = json!(g.to_string());
    }
    serde_json::from_value(v).unwrap()
}

fn msg_with_member(id: u64, ch: u64, guild: u64, author: u64, nick: &str) -> Message {
    let mut m = msg(id, ch, Some(guild), author);
    m.member = Some(serde_json::from_value(json!({"nick": nick, "roles": ["5"]})).unwrap());
    m
}

fn ids(state: &State, ch: u64) -> Vec<u64> {
    state
        .messages(sf(ch))
        .unwrap()
        .iter()
        .map(|m| m.id.0)
        .collect()
}

fn chan(id: u64, kind: u8, parent: Option<u64>, pos: i32, name: &str) -> serde_json::Value {
    let mut v = json!({"id": id.to_string(), "type": kind, "name": name, "position": pos});
    if let Some(p) = parent {
        v["parent_id"] = json!(p.to_string());
    }
    v
}

fn guild(id: u64, owner: u64, channels: Vec<serde_json::Value>, view: bool) -> Guild {
    serde_json::from_value(json!({
        "id": id.to_string(),
        "properties": {"name": format!("g{id}"), "owner_id": owner.to_string()},
        "channels": channels,
        "roles": [{"id": id.to_string(), "name": "@everyone",
                   "permissions": if view { "1024" } else { "0" }}],
        "large": false,
    }))
    .unwrap()
}

fn ready(
    guilds: Vec<serde_json::Value>,
    dms: Vec<serde_json::Value>,
    members: Vec<serde_json::Value>,
) -> Ready {
    serde_json::from_value(json!({
        "v": 9, "user": {"id": "100", "username": "me", "global_name": null},
        "session_id": "s", "resume_gateway_url": "wss://x",
        "guilds": guilds, "private_channels": dms, "merged_members": members,
        "users": [{"id": "7", "username": "alice", "global_name": "Alice"},
                  {"id": "8", "username": "bob", "global_name": null}],
    }))
    .unwrap()
}

fn gjson(id: u64, owner: u64, channels: Vec<serde_json::Value>) -> serde_json::Value {
    json!({
        "id": id.to_string(),
        "properties": {"name": format!("g{id}"), "owner_id": owner.to_string()},
        "channels": channels,
        "roles": [{"id": id.to_string(), "name": "@everyone", "permissions": "1024"}],
    })
}

/// Hazır durum: guild 1 (ağaç testi için), açık oturum.
fn state_with(guilds: Vec<serde_json::Value>, dms: Vec<serde_json::Value>) -> State {
    let mut s = State::new();
    s.load_ready(&ready(guilds, dms, vec![]));
    s
}

fn loaded_state() -> State {
    let mut s = state_with(
        vec![gjson(1, 999, vec![chan(10, 0, None, 0, "genel")])],
        vec![],
    );
    s.history_loaded(
        sf(10),
        None,
        50,
        vec![msg(3, 10, Some(1), 7), msg(2, 10, Some(1), 7)],
    );
    s
}

#[test]
fn fixture_ready() {
    let r: Ready = serde_json::from_str(READY).unwrap();
    let mut s = State::new();
    s.load_ready(&r);
    assert_eq!(s.me().unwrap().id, r.user.id);
    // Üçüncü guild kullanılamaz: listede yok.
    let g = s.guilds();
    assert_eq!(g.len(), 2);
    assert_eq!(g[0].name, "sunucu-1");
    assert_eq!(g[1].name, "sunucu-2");
    // Guild 0: iki metin kanalı @everyone deny ile gizli, kategori çocuksuz → boş.
    assert!(s.guild_tree(g[0].id).is_empty());
    // Guild 1 ağacı tutarlı: kategori varsa en az bir çocuğu izler.
    let t = s.guild_tree(g[1].id);
    for (i, it) in t.iter().enumerate() {
        if it.kind == TreeKind::Category {
            assert_eq!(it.depth, 0);
            assert_eq!(t[i + 1].depth, 1);
        }
    }
    // Kanallara guild_id atanmış.
    let cid = r.guilds[0].channels[1].id;
    assert_eq!(s.channel(cid).unwrap().guild_id, Some(r.guilds[0].id));
    assert!(s.guild_tree(sf(42)).is_empty());
}

#[test]
fn tree_order_and_depth() {
    let s = state_with(
        vec![gjson(
            1,
            999,
            vec![
                chan(20, 2, None, 0, "sesli-top"),
                chan(21, 0, None, 5, "metin-top2"),
                chan(22, 0, None, 1, "metin-top1"),
                chan(30, 4, None, 2, "kat-b"),
                chan(31, 4, None, 1, "kat-a"),
                chan(40, 2, Some(31), 0, "ses-a"),
                chan(41, 5, Some(31), 4, "duyuru-a"),
                chan(42, 0, Some(31), 3, "metin-a"),
                chan(43, 13, Some(31), 0, "sahne-a"),
                chan(50, 0, Some(30), 0, "metin-b"),
                chan(60, 4, None, 9, "bos-kat"),
            ],
        )],
        vec![],
    );
    let names: Vec<(&str, u8)> = s
        .guild_tree(sf(1))
        .iter()
        .map(|i| (i.name.as_str(), i.depth))
        .collect();
    assert_eq!(
        names,
        vec![
            ("metin-top1", 0),
            ("metin-top2", 0),
            ("sesli-top", 0),
            ("kat-a", 0),
            ("metin-a", 1),
            ("duyuru-a", 1),
            ("ses-a", 1),
            ("sahne-a", 1),
            ("kat-b", 0),
            ("metin-b", 1),
        ]
    );
    let t = s.guild_tree(sf(1));
    assert_eq!(t[3].kind, TreeKind::Category);
    assert_eq!(t[3].channel_kind, 4);
    assert_eq!(t[4].kind, TreeKind::Channel);
}

#[test]
fn visibility_overwrite_roles_and_owner() {
    // @everyone görür ama 11 kanalı deny'lı; rol 5 ile izin geri verilir.
    let hidden = |id: u64| {
        let mut c = chan(id, 0, None, 0, "gizli");
        c["permission_overwrites"] = json!([
            {"id": "1", "type": 0, "allow": "0", "deny": "1024"},
            {"id": "5", "type": 0, "allow": "1024", "deny": "0"},
        ]);
        c
    };
    let g = vec![gjson(
        1,
        999,
        vec![chan(10, 0, None, 0, "acik"), hidden(11)],
    )];
    let mut s = State::new();
    s.load_ready(&ready(g.clone(), vec![], vec![]));
    assert_eq!(s.guild_tree(sf(1)).len(), 1);

    // Rolü olan üyelik: görünür.
    let mut s = State::new();
    s.load_ready(&ready(
        g.clone(),
        vec![],
        vec![json!([{"user_id": "100", "roles": ["5"]}])],
    ));
    assert_eq!(s.guild_tree(sf(1)).len(), 2);

    // Başkasının üyeliği sayılmaz.
    s.load_ready(&ready(
        g.clone(),
        vec![],
        vec![json!([{"user_id": "101", "roles": ["5"]}])],
    ));
    assert_eq!(s.guild_tree(sf(1)).len(), 1);

    // Sahip her şeyi görür.
    let g = vec![gjson(1, 100, vec![chan(10, 0, None, 0, "a"), hidden(11)])];
    s.load_ready(&ready(g, vec![], vec![]));
    assert_eq!(s.guild_tree(sf(1)).len(), 2);

    // chunk ile kendi rollerim gelince ağaç yenilenir.
    let g = vec![gjson(1, 999, vec![chan(10, 0, None, 0, "a"), hidden(11)])];
    s.load_ready(&ready(g, vec![], vec![]));
    assert_eq!(s.guild_tree(sf(1)).len(), 1);
    let chunk: GuildMembersChunk = serde_json::from_value(json!({
        "guild_id": "1",
        "members": [{"user": {"id": "100", "username": "me"}, "roles": ["5"]}],
    }))
    .unwrap();
    s.add_members(&chunk);
    assert_eq!(s.guild_tree(sf(1)).len(), 2);
}

#[test]
fn category_without_visible_child_hidden_even_if_own_visible() {
    let mut hidden = chan(50, 0, Some(30), 0, "gizli");
    hidden["permission_overwrites"] = json!([{"id": "1", "type": 0, "allow": "0", "deny": "1024"}]);
    let s = state_with(
        vec![gjson(1, 999, vec![chan(30, 4, None, 0, "kat"), hidden])],
        vec![],
    );
    assert!(s.guild_tree(sf(1)).is_empty());
}

#[test]
fn dm_sorting_and_titles() {
    let dm = |id: u64, last: Option<u64>, rec: &[u64]| {
        json!({"id": id.to_string(), "type": 1,
               "last_message_id": last.map(|l| l.to_string()),
               "recipient_ids": rec.iter().map(|r| r.to_string()).collect::<Vec<_>>()})
    };
    let group = json!({"id": "203", "type": 3, "name": "ekip", "last_message_id": "50",
                       "recipient_ids": ["7", "8"]});
    let group2 = json!({"id": "204", "type": 3, "last_message_id": "5",
                        "recipient_ids": ["7", "8", "999"]});
    let mut s = state_with(
        vec![],
        vec![
            dm(200, Some(10), &[7]),
            dm(201, None, &[8]),
            dm(202, Some(30), &[7]),
            group,
            group2,
        ],
    );
    let order: Vec<u64> = s.dm_channels().iter().map(|c| c.0).collect();
    assert_eq!(order, vec![203, 202, 200, 204, 201]);
    assert_eq!(s.channel_title(sf(200)), "Alice");
    assert_eq!(s.channel_title(sf(201)), "bob");
    assert_eq!(s.channel_title(sf(203)), "ekip");
    assert_eq!(s.channel_title(sf(204)), "Alice, bob, 999");
    assert_eq!(s.channel_title(sf(777)), "?");

    // Yeni mesaj sıralamayı değiştirir (kanal yüklü olmasa da).
    s.message_create(msg(99, 201, None, 8));
    assert_eq!(s.dm_channels()[0], sf(201));
    assert_eq!(s.channel(sf(201)).unwrap().last_message_id, Some(sf(99)));
    assert!(s.messages(sf(201)).is_none());
    // Eski id last_message_id'yi geri almaz.
    s.message_create(msg(50, 201, None, 8));
    assert_eq!(s.channel(sf(201)).unwrap().last_message_id, Some(sf(99)));
}

#[test]
fn guild_channel_title() {
    let s = state_with(
        vec![gjson(1, 999, vec![chan(10, 0, None, 0, "genel")])],
        vec![],
    );
    assert_eq!(s.channel_title(sf(10)), "genel");
}

#[test]
fn message_create_requires_loaded_and_orders() {
    let mut s = loaded_state();
    assert_eq!(ids(&s, 10), vec![2, 3]);
    s.message_create(msg(5, 10, Some(1), 7));
    s.message_create(msg(4, 10, Some(1), 7)); // geç gelen
    s.message_create(msg(4, 10, Some(1), 7)); // yinelenen
    s.message_create(msg(5, 10, Some(1), 7));
    assert_eq!(ids(&s, 10), vec![2, 3, 4, 5]);
    assert_eq!(s.channel(sf(10)).unwrap().last_message_id, Some(sf(5)));

    // Yüklenmemiş kanal: saklanmaz, last_message_id güncellenir.
    let mut s = state_with(vec![gjson(1, 999, vec![chan(10, 0, None, 0, "g")])], vec![]);
    assert!(!s.is_loaded(sf(10)));
    s.message_create(msg(8, 10, Some(1), 7));
    assert!(!s.is_loaded(sf(10)));
    assert_eq!(s.channel(sf(10)).unwrap().last_message_id, Some(sf(8)));
}

#[test]
fn message_update_and_delete() {
    let mut s = loaded_state();
    let mut upd = msg(2, 10, Some(1), 7);
    upd.content = "yeni".into();
    s.message_update(upd);
    assert_eq!(ids(&s, 10), vec![2, 3]);
    assert_eq!(s.messages(sf(10)).unwrap()[0].content, "yeni");

    let v = s.version();
    s.message_update(msg(77, 10, Some(1), 7)); // bilinmeyen: değişiklik yok
    assert_eq!(ids(&s, 10), vec![2, 3]);
    assert_eq!(s.version(), v);

    let del = |id| MessageDelete {
        id: sf(id),
        channel_id: sf(10),
        guild_id: Some(sf(1)),
    };
    s.message_delete(&del(2));
    assert_eq!(ids(&s, 10), vec![3]);
    let v = s.version();
    s.message_delete(&del(2));
    assert_eq!(s.version(), v);
}

#[test]
fn history_paging() {
    let mut s = state_with(vec![gjson(1, 999, vec![chan(10, 0, None, 0, "g")])], vec![]);
    let page = |range: std::ops::RangeInclusive<u64>| -> Vec<Message> {
        range.rev().map(|i| msg(i, 10, Some(1), 7)).collect()
    };
    // İlk yükleme: tam sayfa → başlangıç ulaşılmadı.
    s.history_loaded(sf(10), None, 5, page(16..=20));
    assert!(s.is_loaded(sf(10)));
    assert!(!s.history_complete(sf(10)));
    assert_eq!(ids(&s, 10), vec![16, 17, 18, 19, 20]);

    // Eski sayfa (kesişen 16 ile) başa eklenir, yinelenen atılır.
    s.history_loaded(sf(10), Some(sf(16)), 5, page(11..=16));
    assert_eq!(ids(&s, 10), (11..=20).collect::<Vec<_>>());
    assert!(!s.history_complete(sf(10)));

    // Limitten az → başlangıç.
    s.history_loaded(sf(10), Some(sf(11)), 5, page(9..=10));
    assert_eq!(ids(&s, 10).first(), Some(&9));
    assert!(s.history_complete(sf(10)));

    // İlk yükleme yeniden: değiştirir ve bayrağı sıfırlar.
    s.history_loaded(sf(10), None, 5, page(30..=34));
    assert_eq!(ids(&s, 10), vec![30, 31, 32, 33, 34]);
    assert!(!s.history_complete(sf(10)));

    // Yüklenmemiş kanala eski sayfa yok sayılır.
    s.history_loaded(sf(55), Some(sf(9)), 5, page(1..=2));
    assert!(!s.is_loaded(sf(55)));
    // Boş ilk yükleme: yüklü ve tamam.
    s.history_loaded(sf(56), None, 5, vec![]);
    assert!(s.is_loaded(sf(56)));
    assert!(s.history_complete(sf(56)));
}

#[test]
fn trim_channel_keeps_newest() {
    let mut s = state_with(vec![gjson(1, 999, vec![chan(10, 0, None, 0, "g")])], vec![]);
    let msgs: Vec<Message> = (1..=300u64).rev().map(|i| msg(i, 10, Some(1), 7)).collect();
    s.history_loaded(sf(10), None, 1000, msgs);
    assert!(s.history_complete(sf(10)));
    s.trim_channel(sf(10));
    let v = ids(&s, 10);
    assert_eq!(v.len(), DEFAULT_WINDOW);
    assert_eq!(v.first(), Some(&101));
    assert_eq!(v.last(), Some(&300));
    assert!(!s.history_complete(sf(10)));

    // Küçük kanalda bayrak korunur.
    let mut s = loaded_state();
    s.history_loaded(sf(10), None, 50, vec![msg(2, 10, Some(1), 7)]);
    s.trim_channel(sf(10));
    assert!(s.history_complete(sf(10)));
    s.trim_channel(sf(999)); // bilinmeyen: sorun yok
}

#[test]
fn hard_cap_drops_oldest() {
    let mut s = state_with(vec![gjson(1, 999, vec![chan(10, 0, None, 0, "g")])], vec![]);
    let msgs: Vec<Message> = (1..=HARD_CAP as u64)
        .rev()
        .map(|i| msg(i, 10, Some(1), 7))
        .collect();
    s.history_loaded(sf(10), None, HARD_CAP * 2, msgs);
    assert!(s.history_complete(sf(10)));
    s.message_create(msg(HARD_CAP as u64 + 1, 10, Some(1), 7));
    let m = s.messages(sf(10)).unwrap();
    assert_eq!(m.len(), HARD_CAP);
    assert_eq!(m.front().unwrap().id, sf(2));
    assert!(!s.history_complete(sf(10)));
}

#[test]
fn author_name_precedence() {
    let mut s = loaded_state();
    let plain = user(7, "alice", None);
    let global = user(7, "alice", Some("Alice"));
    assert_eq!(s.author_name(Some(sf(1)), &plain), "alice");
    assert_eq!(s.author_name(Some(sf(1)), &global), "Alice");
    s.message_create(msg_with_member(9, 10, 1, 7, "Takma"));
    assert_eq!(s.author_name(Some(sf(1)), &global), "Takma");
    assert_eq!(s.author_name(Some(sf(2)), &global), "Alice"); // başka guild
    assert_eq!(s.author_name(None, &global), "Alice");
    assert_eq!(s.user(sf(7)).unwrap().username, "u7");
}

#[test]
fn unknown_members_dedupe_max_dm() {
    let mut s = state_with(
        vec![gjson(1, 999, vec![chan(10, 0, None, 0, "g")])],
        vec![json!({"id": "200", "type": 1, "recipient_ids": ["7"]})],
    );
    s.history_loaded(
        sf(10),
        None,
        50,
        vec![
            msg(6, 10, Some(1), 3),
            msg_with_member(5, 10, 1, 2, "n"),
            msg(4, 10, Some(1), 1),
            msg(3, 10, Some(1), 3),
            msg(2, 10, Some(1), 1),
        ],
    );
    // Artan id sırası: 2(u1), 3(u3), 4(u1), 5(u2 bilinen), 6(u3)
    assert_eq!(s.unknown_members(sf(10), 10), vec![sf(1), sf(3)]);
    assert_eq!(s.unknown_members(sf(10), 1), vec![sf(1)]);
    assert!(s.unknown_members(sf(10), 0).is_empty());

    let chunk: GuildMembersChunk = serde_json::from_value(json!({
        "guild_id": "1", "members": [{"user": {"id": "1", "username": "u1"}, "nick": "x"}],
    }))
    .unwrap();
    s.add_members(&chunk);
    assert_eq!(s.unknown_members(sf(10), 10), vec![sf(3)]);
    assert_eq!(s.author_name(Some(sf(1)), &user(1, "u1", None)), "x");

    s.history_loaded(sf(200), None, 50, vec![msg(2, 200, None, 7)]);
    assert!(s.unknown_members(sf(200), 10).is_empty());
    assert!(s.unknown_members(sf(404), 10).is_empty());
}

#[test]
fn upsert_guild_unknown_and_unavailable() {
    let r = ready(
        vec![json!({"id": "3", "unavailable": true})],
        vec![],
        vec![json!([])],
    );
    let mut s = State::new();
    s.load_ready(&r);
    assert!(s.guilds().is_empty());
    assert!(s.guild_tree(sf(3)).is_empty());

    // Kullanılamaz guild dolar, sırası korunur.
    s.upsert_guild(&guild(3, 999, vec![chan(30, 0, None, 0, "a")], true));
    assert_eq!(s.guilds().len(), 1);
    assert_eq!(s.guilds()[0].name, "g3");
    assert_eq!(s.guild_tree(sf(3)).len(), 1);
    assert_eq!(s.channel(sf(30)).unwrap().guild_id, Some(sf(3)));

    // Bilinmeyen guild sona eklenir.
    s.upsert_guild(&guild(4, 999, vec![chan(40, 0, None, 0, "b")], true));
    let order: Vec<u64> = s.guilds().iter().map(|g| g.id.0).collect();
    assert_eq!(order, vec![3, 4]);

    // Yeniden GUILD_CREATE: kanallar değişir, eski kanal ve mesajları gider.
    s.history_loaded(sf(30), None, 50, vec![msg(2, 30, Some(3), 7)]);
    s.upsert_guild(&guild(3, 999, vec![chan(31, 0, None, 0, "c")], true));
    assert!(s.channel(sf(30)).is_none());
    assert!(!s.is_loaded(sf(30)));
    assert_eq!(s.guild_tree(sf(3))[0].name, "c");
    assert_eq!(s.role(sf(3), sf(3)).unwrap().name, "@everyone");
    assert!(s.role(sf(3), sf(99)).is_none());
    assert!(s.role(sf(99), sf(3)).is_none());

    // Benim rollerim korunur.
    let mut hidden = chan(32, 0, None, 1, "gizli");
    hidden["permission_overwrites"] = json!([
        {"id": "3", "type": 0, "allow": "0", "deny": "1024"},
        {"id": "5", "type": 0, "allow": "1024", "deny": "0"},
    ]);
    let chunk: GuildMembersChunk = serde_json::from_value(json!({
        "guild_id": "3", "members": [{"user_id": "100", "roles": ["5"]}],
    }))
    .unwrap();
    s.add_members(&chunk);
    let mut g = guild(3, 999, vec![chan(31, 0, None, 0, "c"), hidden], true);
    g.large = true;
    s.upsert_guild(&g);
    assert_eq!(s.guild_tree(sf(3)).len(), 2);
    assert!(s.guilds()[0].large);
}

#[test]
fn upsert_and_remove_channel() {
    let mut s = state_with(vec![gjson(1, 999, vec![chan(10, 0, None, 0, "a")])], vec![]);
    let new_ch: Channel = serde_json::from_value(json!({
        "id": "11", "type": 0, "guild_id": "1", "name": "b", "position": 1,
    }))
    .unwrap();
    s.upsert_channel(&new_ch);
    assert_eq!(s.guild_tree(sf(1)).len(), 2);
    // Güncelleme (ad değişimi).
    let mut renamed = new_ch.clone();
    renamed.name = Some("b2".into());
    s.upsert_channel(&renamed);
    assert_eq!(s.guild_tree(sf(1)).len(), 2);
    assert_eq!(s.guild_tree(sf(1))[1].name, "b2");

    s.history_loaded(sf(11), None, 50, vec![msg(2, 11, Some(1), 7)]);
    s.remove_channel(sf(11));
    assert_eq!(s.guild_tree(sf(1)).len(), 1);
    assert!(s.channel(sf(11)).is_none());
    assert!(!s.is_loaded(sf(11)));

    // DM ekle / kaldır.
    let dm: Channel = serde_json::from_value(json!({
        "id": "300", "type": 1, "recipient_ids": ["7"],
    }))
    .unwrap();
    s.upsert_channel(&dm);
    assert_eq!(s.dm_channels(), &[sf(300)]);
    s.remove_channel(sf(300));
    assert!(s.dm_channels().is_empty());

    let v = s.version();
    s.remove_channel(sf(300)); // yok: sürüm değişmez
    assert_eq!(s.version(), v);
}

#[test]
fn version_increments() {
    let mut s = State::new();
    assert_eq!(s.version(), 0);
    s.load_ready(&ready(
        vec![gjson(1, 999, vec![chan(10, 0, None, 0, "a")])],
        vec![],
        vec![],
    ));
    let v1 = s.version();
    assert!(v1 > 0);
    s.history_loaded(sf(10), None, 50, vec![msg(2, 10, Some(1), 7)]);
    let v2 = s.version();
    assert!(v2 > v1);
    s.message_create(msg(3, 10, Some(1), 7));
    let v3 = s.version();
    assert!(v3 > v2);
    s.message_update(msg(3, 10, Some(1), 7));
    let v4 = s.version();
    assert!(v4 > v3);
    s.message_delete(&MessageDelete {
        id: sf(3),
        channel_id: sf(10),
        guild_id: None,
    });
    assert!(s.version() > v4);
    // Yeni oturumda sürüm geri sarmaz.
    let before = s.version();
    s.load_ready(&ready(vec![], vec![], vec![]));
    assert!(s.version() > before);
    assert!(!s.is_loaded(sf(10)));
}

#[test]
fn ready_clears_messages_and_records_users() {
    let mut s = loaded_state();
    assert!(s.is_loaded(sf(10)));
    s.load_ready(&ready(
        vec![gjson(1, 999, vec![chan(10, 0, None, 0, "g")])],
        vec![],
        vec![],
    ));
    assert!(!s.is_loaded(sf(10)));
    assert_eq!(s.user(sf(7)).unwrap().display_name(), "Alice");
    assert_eq!(s.user(sf(100)).unwrap().username, "me");
}

// ------------------------------------------------------------------ thread'ler

/// VIEW_CHANNEL + SEND_MESSAGES + SEND_MESSAGES_IN_THREADS.
const THREAD_PERMS: &str = "274877910016";

fn thread(id: u64, parent: u64, name: &str, archived: bool, locked: bool) -> serde_json::Value {
    json!({"id": id.to_string(), "type": 11, "guild_id": "1", "parent_id": parent.to_string(),
           "name": name, "last_message_id": id.to_string(), "owner_id": "7",
           "thread_metadata": {"archived": archived, "locked": locked}})
}

fn thread_ch(v: serde_json::Value) -> Channel {
    serde_json::from_value(v).unwrap()
}

fn threads_state(
    perms: &str,
    overwrites: serde_json::Value,
    threads: Vec<serde_json::Value>,
) -> State {
    let mut parent = chan(10, 0, None, 0, "genel");
    parent["permission_overwrites"] = overwrites;
    let mut g = gjson(1, 999, vec![parent, chan(20, 15, None, 1, "forum")]);
    g["roles"][0]["permissions"] = json!(perms);
    g["threads"] = json!(threads);
    state_with(vec![g], vec![])
}

fn thread_ids(s: &State, parent: u64) -> Vec<u64> {
    s.threads_of(sf(parent), None)
        .iter()
        .map(|t| t.id.0)
        .collect()
}

#[test]
fn ready_threads_hang_under_parent_not_in_tree() {
    let s = threads_state(
        THREAD_PERMS,
        json!([]),
        vec![
            thread(30, 10, "eski", false, false),
            thread(32, 10, "yeni", false, false),
            thread(31, 10, "arşiv", true, false),
            thread(40, 20, "gönderi", false, false),
        ],
    );
    // Yeniden eskiye; arşivli gizli.
    assert_eq!(thread_ids(&s, 10), vec![32, 30]);
    assert_eq!(thread_ids(&s, 20), vec![40]);
    assert!(
        s.guild_tree(sf(1))
            .iter()
            .all(|i| ![30, 31, 32, 40].contains(&i.id.0))
    );
    assert_eq!(s.thread_parent(sf(30)), Some(sf(10)));
    assert_eq!(s.channel(sf(30)).unwrap().guild_id, Some(sf(1)));
    assert_eq!(s.channel_title(sf(30)), "eski");
}

#[test]
fn thread_create_update_delete() {
    let mut s = threads_state(THREAD_PERMS, json!([]), vec![]);
    s.upsert_channel(&thread_ch(thread(30, 10, "a", false, false)));
    assert_eq!(thread_ids(&s, 10), vec![30]);

    // UPDATE üyelik ve son mesajı taşımaz: eskisi korunur.
    let mut with_member = thread(30, 10, "a", false, false);
    with_member["member"] = json!({"flags": 1});
    with_member["last_message_id"] = json!("500");
    s.upsert_channel(&thread_ch(with_member));
    s.upsert_channel(&thread_ch(thread(30, 10, "yeni ad", false, false)));
    let c = s.channel(sf(30)).unwrap();
    assert_eq!(c.name.as_deref(), Some("yeni ad"));
    assert!(c.member.is_some());
    assert_eq!(c.last_message_id, Some(sf(500)));

    // Arşivlenince listeden düşer; açık olan (`keep`) kalır.
    s.upsert_channel(&thread_ch(thread(30, 10, "yeni ad", true, true)));
    assert!(thread_ids(&s, 10).is_empty());
    assert_eq!(s.threads_of(sf(10), Some(sf(30))).len(), 1);
    assert!(s.is_thread_locked(sf(30)));

    // Silme: mesajlar ve bağ gider.
    s.history_loaded(sf(30), None, 50, vec![msg(900, 30, Some(1), 7)]);
    s.remove_channel(sf(30));
    assert!(s.channel(sf(30)).is_none() && s.messages(sf(30)).is_none());
    assert!(s.threads_of(sf(10), Some(sf(30))).is_empty());

    // Bilinmeyen guild'in thread'i yok sayılır.
    let mut other = thread(31, 10, "x", false, false);
    other["guild_id"] = json!("77");
    let v = s.version();
    s.upsert_channel(&thread_ch(other));
    assert_eq!(s.version(), v);
}

#[test]
fn parent_delete_removes_threads() {
    let mut s = threads_state(
        THREAD_PERMS,
        json!([]),
        vec![thread(30, 10, "a", false, false)],
    );
    s.remove_channel(sf(10));
    assert!(s.channel(sf(30)).is_none());
}

#[test]
fn thread_list_sync_scopes() {
    let mut s = threads_state(
        THREAD_PERMS,
        json!([]),
        vec![
            thread(30, 10, "a", false, false),
            thread(31, 10, "b", false, false),
            thread(40, 20, "f", false, false),
        ],
    );
    // Yalnızca kanal 10 senkronlanır: 31 gider, 33 gelir, forum dokunulmaz.
    let sync =
        |channel_ids: serde_json::Value, threads: Vec<serde_json::Value>| -> ThreadListSync {
            serde_json::from_value(
                json!({"guild_id": "1", "channel_ids": channel_ids, "threads": threads}),
            )
            .unwrap()
        };
    s.thread_list_sync(&sync(
        json!(["10"]),
        vec![
            thread(30, 10, "a", false, false),
            thread(33, 10, "c", false, false),
        ],
    ));
    assert_eq!(thread_ids(&s, 10), vec![33, 30]);
    assert_eq!(thread_ids(&s, 20), vec![40]);
    // Kapsamsız: tüm guild değişir.
    s.thread_list_sync(&sync(json!(null), vec![thread(41, 20, "g", false, false)]));
    assert!(thread_ids(&s, 10).is_empty());
    assert_eq!(thread_ids(&s, 20), vec![41]);
    // Bilinmeyen guild: yok sayılır.
    let mut bad = sync(json!(null), vec![]);
    bad.guild_id = sf(77);
    let v = s.version();
    s.thread_list_sync(&bad);
    assert_eq!(s.version(), v);
}

#[test]
fn rest_loaded_threads_and_archived() {
    let mut s = threads_state(THREAD_PERMS, json!([]), vec![]);
    assert!(!s.threads_fetched(sf(20)));
    // `guild_id` olmayan REST thread'i de yerleşir; yanlış üst kanallı olan atlanır.
    let mut a = thread(40, 20, "a", false, false);
    a.as_object_mut().unwrap().remove("guild_id");
    let stray = thread(41, 10, "başka", false, false);
    s.threads_loaded(sf(20), &[thread_ch(a), thread_ch(stray)], false);
    assert!(s.threads_fetched(sf(20)));
    assert_eq!(thread_ids(&s, 20), vec![40]);
    assert_eq!(s.channel(sf(40)).unwrap().guild_id, Some(sf(1)));

    // Arşivliler istenince dim olarak listelenir.
    s.threads_loaded(
        sf(10),
        &[thread_ch(thread(50, 10, "eski", true, false))],
        true,
    );
    assert!(s.archived_shown(sf(10)));
    let items = s.threads_of(sf(10), None);
    assert_eq!(items.len(), 1);
    assert!(items[0].archived);
    // Sync arşivlilere dokunmaz.
    s.thread_list_sync(&ThreadListSync {
        guild_id: sf(1),
        channel_ids: None,
        threads: vec![],
    });
    assert_eq!(thread_ids(&s, 10), vec![50]);
    // Bilinmeyen üst kanal: yok sayılır.
    s.threads_loaded(
        sf(99),
        &[thread_ch(thread(60, 99, "x", false, false))],
        false,
    );
    assert!(s.channel(sf(60)).is_none());
}

#[test]
fn thread_permissions_inherit_from_parent() {
    let deny = |bits: u64| json!([{"id": "1", "type": 0, "allow": "0", "deny": bits.to_string()}]);
    // Normal: yazabilir.
    let s = threads_state(
        THREAD_PERMS,
        json!([]),
        vec![thread(30, 10, "a", false, false)],
    );
    assert!(s.can_send(sf(30)));
    // Arşivli ama kilitsiz: yazmak thread'i açar, engellenmez.
    let s = threads_state(
        THREAD_PERMS,
        json!([]),
        vec![thread(30, 10, "a", true, false)],
    );
    assert!(s.can_send(sf(30)));
    // Üst kanalda SEND_MESSAGES_IN_THREADS reddi miras alınır (SEND_MESSAGES değil).
    let s = threads_state(
        THREAD_PERMS,
        deny(1 << 38),
        vec![thread(30, 10, "a", false, false)],
    );
    assert!(!s.can_send(sf(30)));
    assert!(s.can_send(sf(10)));
    let s = threads_state(
        THREAD_PERMS,
        deny(1 << 11),
        vec![thread(30, 10, "a", false, false)],
    );
    assert!(s.can_send(sf(30)) && !s.can_send(sf(10)));
    // Üst kanalı görme izni yok → thread'de de yok.
    let s = threads_state(
        THREAD_PERMS,
        deny(1 << 10),
        vec![thread(30, 10, "a", false, false)],
    );
    assert!(!s.my_permissions(sf(30)).contains(Permissions::VIEW_CHANNEL));
    // Kilitli: yazamaz; MANAGE_THREADS ile yazabilir.
    let s = threads_state(
        THREAD_PERMS,
        json!([]),
        vec![thread(30, 10, "a", false, true)],
    );
    assert!(!s.can_send(sf(30)));
    let s = threads_state(
        "292057779200",
        json!([]),
        vec![thread(30, 10, "a", false, true)],
    );
    assert!(s.can_send(sf(30)));
}

#[test]
fn thread_unread_counts_in_guild_and_jump_list() {
    let mut s = State::new();
    let mut parent = chan(10, 0, None, 0, "genel");
    parent["last_message_id"] = json!("5");
    let mut g = gjson(1, 999, vec![parent]);
    g["roles"][0]["permissions"] = json!(THREAD_PERMS);
    g["threads"] = json!([thread(30, 10, "a", false, false)]);
    let mut r = ready(vec![g], vec![], vec![]);
    r.read_state = serde_json::from_value(json!({"entries": [
        {"id": "10", "last_message_id": "5", "mention_count": 0},
        {"id": "30", "last_message_id": "20", "mention_count": 2},
    ]}))
    .unwrap();
    s.load_ready(&r);
    // Thread'in son mesajı 30, okunan 20 → okunmamış.
    assert!(s.is_unread(sf(30)));
    assert_eq!(s.mention_count(sf(30)), 2);
    assert_eq!(s.guild_unread(sf(1), 0), (true, 2));
    assert_eq!(s.unread_channels(0), vec![(Some(sf(1)), sf(30))]);
    s.mark_read(sf(30), sf(30));
    assert!(!s.is_unread(sf(30)));
    assert_eq!(s.guild_unread(sf(1), 0), (false, 0));
}

#[test]
fn thread_message_flow_reuses_channel_paths() {
    let mut s = threads_state(
        THREAD_PERMS,
        json!([]),
        vec![thread(30, 10, "a", false, false)],
    );
    s.history_loaded(sf(30), None, 50, vec![msg(900, 30, Some(1), 7)]);
    s.message_create(msg(901, 30, Some(1), 7));
    assert_eq!(ids(&s, 30), vec![900, 901]);
    assert_eq!(s.channel(sf(30)).unwrap().last_message_id, Some(sf(901)));
}
