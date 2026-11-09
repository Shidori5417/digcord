//! Performans ölçümü için sentetik "büyük hesap" verisi (yalnızca test derlemesi).
//!
//! Gerçek kayıt yok: kimlikler, adlar ve içerik tamamen üretilmiştir; üretim deterministiktir
//! (sabit tohumlu LCG), dosya olarak commit'lenmez.

use serde_json::{Value, json};

pub const GUILDS: u64 = 100;
const CATEGORIES_PER_GUILD: u64 = 5;
const CHANNELS_PER_CATEGORY: u64 = 9; // 8 metin + 1 ses
pub const USERS: u64 = 2_000;
pub const DMS: u64 = 300;
const ROLES_PER_GUILD: u64 = 3;
pub const ME: u64 = 1_000_000;

/// Deterministik sözde-rastgele sayı üreteci.
pub struct Lcg(u64);

impl Lcg {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    pub fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }
}

/// 2026-01-01 civarı + `minutes` dakika için snowflake (`seq` alt bitlerde).
pub fn snow(minutes: u64, seq: u64) -> u64 {
    ((1_767_268_800_000 + minutes * 60_000 - 1_420_070_400_000) << 22) | (seq & 0x3f_ffff)
}

pub fn guild_id(g: u64) -> u64 {
    snow(0, 0) + (g << 22) + 1
}
fn category_id(g: u64, c: u64) -> u64 {
    10_000_000_000_000_000 + (g * 100 + c) * 1_000_003
}
pub fn channel_id(g: u64, c: u64, k: u64) -> u64 {
    20_000_000_000_000_000 + ((g * 100 + c) * 100 + k) * 1_000_033
}
fn role_id(g: u64, r: u64) -> u64 {
    30_000_000_000_000_000 + (g * 10 + r) * 1_000_037
}
pub fn user_id(i: u64) -> u64 {
    40_000_000_000_000_000 + i * 1_000_039
}
fn dm_id(i: u64) -> u64 {
    50_000_000_000_000_000 + i * 1_000_049
}

fn user(i: u64) -> Value {
    json!({"id": user_id(i).to_string(), "username": format!("kullanici{i}"),
           "global_name": if i.is_multiple_of(3) { Value::Null } else { json!(format!("Kullanıcı {i}")) }})
}

/// Tüm metin kanalları `(guild, kanal_id)`; mesaj ölçümü için.
pub fn text_channels() -> impl Iterator<Item = (u64, u64)> {
    (0..GUILDS).flat_map(|g| {
        (0..CATEGORIES_PER_GUILD).flat_map(move |c| {
            (0..CHANNELS_PER_CATEGORY - 1).map(move |k| (g, channel_id(g, c, k)))
        })
    })
}

/// ~100 sunucu, ~5000 kanal, 400 rol, 2000 kullanıcı, 300 DM içeren READY (JSON baytları).
pub fn ready_json() -> Vec<u8> {
    let mut rng = Lcg::new(7);
    let mut guilds = Vec::new();
    let mut merged = Vec::new();
    let mut read_entries = Vec::new();
    let mut settings = Vec::new();
    for g in 0..GUILDS {
        let gid = guild_id(g);
        let mut channels = Vec::new();
        let mut overrides = Vec::new();
        let mut voice_states = Vec::new();
        for c in 0..CATEGORIES_PER_GUILD {
            let cat = category_id(g, c);
            channels.push(json!({"id": cat.to_string(), "type": 4, "flags": 0,
                "permission_overwrites": [], "position": c, "name": format!("kategori-{c}")}));
            for k in 0..CHANNELS_PER_CATEGORY {
                let id = channel_id(g, c, k);
                let voice = k == CHANNELS_PER_CATEGORY - 1;
                let ow = if rng.next(5) == 0 {
                    json!([{"id": gid.to_string(), "type": 0, "allow": "0", "deny": "1024"}])
                } else {
                    json!([])
                };
                channels.push(
                    json!({"id": id.to_string(), "type": if voice { 2 } else { 0 },
                    "flags": 0, "permission_overwrites": ow, "parent_id": cat.to_string(),
                    "position": k, "name": format!("kanal-{c}-{k}"),
                    "topic": if voice { Value::Null } else { json!("Kanal konusu burada yazar") },
                    "last_message_id": snow(rng.next(5000), 1).to_string()}),
                );
                if voice && c == 0 {
                    for u in 0..3 {
                        voice_states.push(json!({"channel_id": id.to_string(),
                            "user_id": user_id(rng.next(USERS)).to_string(),
                            "session_id": format!("{u:032x}"), "self_mute": u == 0}));
                    }
                }
                read_entries.push(json!({"id": id.to_string(),
                    "last_message_id": snow(rng.next(5000), 1).to_string(),
                    "mention_count": rng.next(3) / 2}));
                if rng.next(50) == 0 {
                    overrides.push(json!({"channel_id": id.to_string(), "muted": true,
                        "message_notifications": 3, "mute_config": {"end_time": null}}));
                }
            }
        }
        let mut roles = vec![json!({"id": gid.to_string(), "name": "@everyone",
            "permissions": "1096185212685889", "position": 0, "color": 0})];
        for r in 1..=ROLES_PER_GUILD {
            roles.push(
                json!({"id": role_id(g, r).to_string(), "name": format!("rol-{r}"),
                "permissions": "0", "position": r, "color": 0}),
            );
        }
        let emojis: Vec<Value> = (0..20)
            .map(|e| {
                json!({"id": (60_000_000_000_000_000 + g * 100 + e).to_string(),
                "name": format!("emoji_{e}"), "animated": e % 7 == 0, "available": true})
            })
            .collect();
        guilds.push(
            json!({"id": gid.to_string(), "lazy": true, "large": g % 10 == 0,
            "member_count": 500 + g * 37,
            "properties": {"id": gid.to_string(), "name": format!("sunucu-{g}"), "icon": null,
                           "owner_id": user_id(g).to_string()},
            "channels": channels, "roles": roles, "emojis": emojis, "voice_states": voice_states}),
        );
        merged.push(json!([{"user_id": ME.to_string(), "nick": null,
            "roles": [role_id(g, 1).to_string()], "joined_at": "2020-01-01T00:00:00+00:00",
            "deaf": false}]));
        settings.push(json!({"guild_id": gid.to_string(), "muted": g % 17 == 0,
            "message_notifications": 3, "suppress_everyone": false, "suppress_roles": false,
            "channel_overrides": overrides}));
    }
    let private_channels: Vec<Value> = (0..DMS)
        .map(|i| {
            read_entries.push(json!({"id": dm_id(i).to_string(),
                "last_message_id": snow(rng.next(5000), 1).to_string(), "mention_count": 0}));
            json!({"id": dm_id(i).to_string(), "type": 1,
                "recipient_ids": [user_id(i * 3 % USERS).to_string()],
                "last_message_id": snow(rng.next(5000), 1).to_string()})
        })
        .collect();
    let relationships: Vec<Value> = (0..400)
        .map(|i| {
            json!({"id": user_id(i * 5 % USERS).to_string(), "type": 1 + i % 4,
            "nickname": if i % 10 == 0 { json!("takma ad") } else { Value::Null }})
        })
        .collect();
    let users: Vec<Value> = (0..USERS).map(user).collect();
    serde_json::to_vec(&json!({
        "v": 9,
        "user": {"id": ME.to_string(), "username": "ben", "global_name": "Ben",
                 "discriminator": "0", "avatar": null, "bot": false},
        "session_id": "0123456789abcdef0123456789abcdef", "session_type": "normal",
        "resume_gateway_url": "wss://gateway.example",
        "guilds": guilds, "private_channels": private_channels, "users": users,
        "merged_members": merged,
        "read_state": {"entries": read_entries, "partial": false, "version": 1},
        "user_guild_settings": {"entries": settings, "partial": false, "version": 1},
        "relationships": relationships
    }))
    .unwrap_or_default() // json! değeri her zaman serileşir
}

/// `channel` için `n` mesajlık JSON (eskiden yeniye; gömme, ek, yanıt metni, tepki dahil).
pub fn messages_json(channel: u64, guild: u64, n: u64, rng: &mut Lcg) -> Vec<u8> {
    const TEXTS: [&str; 6] = [
        "selam herkese, bugün nasılsınız?",
        "Lorem ipsum **dolor** sit amet, consectetur `adipiscing` elit; ||gizli|| ve biraz daha uzun bir cümle ki satır sarılsın.",
        "bakın şu linke https://example.com/yol/ornek?x=1 çok güzel olmuş",
        "> alıntı satırı\nve altına cevap yazdım, ```rust\nfn main() {}\n``` şeklinde",
        "tamam :tada: harika olmuş",
        "ok",
    ];
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let author = rng.next(USERS);
        let id = snow(i / 2, channel % 1000 + i * 4096);
        let mut content = TEXTS[rng.next(TEXTS.len() as u64) as usize].to_string();
        if rng.next(6) == 0 {
            content.push_str(&format!(" <@{}>", user_id(rng.next(USERS))));
        }
        let mut m = json!({"id": id.to_string(), "channel_id": channel.to_string(),
            "guild_id": guild_id(guild).to_string(), "author": user(author),
            "content": content, "type": 0, "mentions": [], "mention_roles": [],
            "mention_everyone": false, "attachments": [], "embeds": [], "reactions": []});
        if rng.next(8) == 0 {
            m["attachments"] = json!([{"id": (id + 1).to_string(), "filename": "resim.png",
                "size": 123_456, "width": 800, "height": 600,
                "url": "https://cdn.discordapp.com/attachments/1/2/resim.png?ex=abc&is=def&hm=0123456789abcdef0123456789abcdef",
                "proxy_url": "https://media.discordapp.net/attachments/1/2/resim.png?ex=abc&is=def&hm=0123456789abcdef0123456789abcdef",
                "content_type": "image/png"}]);
        }
        if rng.next(6) == 0 {
            m["embeds"] = json!([{"type": "rich", "title": "Örnek başlık",
                "description": "Gömme açıklaması, birkaç cümlelik bir metin; satır sarılması için yeterince uzun.",
                "url": "https://example.com/a", "color": 5_793_266,
                "provider": {"name": "Örnek"}, "footer": {"text": "alt bilgi"},
                "fields": [{"name": "alan", "value": "değer", "inline": true},
                           {"name": "alan2", "value": "değer2", "inline": true}],
                "thumbnail": {"url": "https://cdn.example/t.png", "width": 64, "height": 64}}]);
        }
        if rng.next(10) == 0 {
            m["reactions"] =
                json!([{"count": 3, "me": false, "emoji": {"id": null, "name": "👍"}}]);
        }
        out.push(m);
    }
    serde_json::to_vec(&out).unwrap_or_default() // json! değeri her zaman serileşir
}
