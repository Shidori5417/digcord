//! Anonimleştirilmiş gerçek READY kaydından (tools/anonymize_ready.py) ayrıştırma testi.

use digcord_model::{Ready, Snowflake};

const FIXTURE: &str = include_str!("fixtures/ready.json");

#[test]
fn parses_anonymized_ready() {
    let ready: Ready = serde_json::from_str(FIXTURE).expect("READY ayrıştırılmalı");
    assert_eq!(ready.v, 9);
    assert_eq!(ready.user.username, "kullanici");
    assert_eq!(ready.user.global_name.as_deref(), Some("Kullanıcı"));
    assert_eq!(
        ready.resume_gateway_url,
        "wss://gateway-us-east1-d.discord.gg"
    );

    assert_eq!(ready.guilds.len(), 3);
    let g = &ready.guilds[0];
    assert_eq!(g.id, Snowflake(100_000_000_000_000_000));
    assert_eq!(
        g.properties.as_ref().map(|p| p.name.as_str()),
        Some("sunucu-1")
    );
    assert_eq!(g.channels[1].parent_id, Some(g.channels[0].id));
    assert_eq!(g.channels[0].kind, 4);

    let unavailable = &ready.guilds[2];
    assert!(unavailable.unavailable);
    assert!(unavailable.properties.is_none() && unavailable.channels.is_empty());

    assert_eq!(ready.private_channels.len(), 2);
    assert_eq!(ready.channel_count(), 4);
}

#[test]
fn permissions_from_anonymized_ready() {
    use digcord_model::{MemberContext, Permissions};
    let ready: Ready = serde_json::from_str(FIXTURE).expect("READY");
    let g = &ready.guilds[0];
    let me = &ready.merged_members[0][0];
    assert_eq!(me.user_id, Some(ready.user.id));
    let ctx = MemberContext {
        guild_id: g.id,
        owner_id: g.properties.as_ref().and_then(|p| p.owner_id),
        user_id: ready.user.id,
        member_roles: &me.roles,
        roles: &g.roles,
    };
    let visible: Vec<bool> = g
        .channels
        .iter()
        .map(|c| {
            ctx.channel(&c.permission_overwrites)
                .contains(Permissions::VIEW_CHANNEL)
        })
        .collect();
    // Gerçek veride @everyone bu iki kanalı gizliyor ve kullanıcının rolü açmıyor.
    assert_eq!(visible, [true, false, false]);
}

#[test]
fn parses_read_state_settings_and_relationships() {
    let ready: Ready = serde_json::from_value(serde_json::json!({
        "v": 9, "session_id": "s", "resume_gateway_url": "wss://x",
        "user": {"id": "1", "username": "a", "global_name": null},
        "read_state": {"version": 1, "partial": false, "entries": [
            {"id": "10", "last_message_id": "99", "mention_count": 2, "flags": 0, "last_pin_timestamp": "x"},
            {"id": "11", "last_message_id": 0, "mention_count": 0},
            {"id": "12", "last_message_id": null, "mention_count": 0},
            {"id": "13", "badge_count": 0, "last_acked_id": "5", "read_state_type": 4}
        ]},
        "user_guild_settings": {"version": 1, "partial": false, "entries": [
            {"guild_id": "7", "muted": true, "mute_config": {"end_time": null, "selected_time_window": -1},
             "message_notifications": 1, "suppress_everyone": true, "suppress_roles": false,
             "channel_overrides": [{"channel_id": "10", "muted": false, "mute_config": null, "message_notifications": 3, "collapsed": false, "flags": 0}]},
            {"guild_id": null, "muted": false, "message_notifications": 0, "channel_overrides": []}
        ]},
        "relationships": [{"id": "5", "type": 1, "user_id": "5", "nickname": null, "is_spam_request": false}]
    }))
    .expect("ready");
    let rs = &ready.read_state.entries;
    assert_eq!(
        (rs[0].last_message_id, rs[0].mention_count),
        (Some(Snowflake(99)), 2)
    );
    assert_eq!(rs[1].last_message_id, Some(Snowflake(0)));
    assert_eq!(rs[2].last_message_id, None);
    assert_eq!(rs[3].read_state_type, Some(4));
    let g = &ready.user_guild_settings.entries[0];
    assert!(g.muted && g.suppress_everyone && g.message_notifications == 1);
    assert_eq!(g.channel_overrides[0].message_notifications, 3);
    assert!(ready.user_guild_settings.entries[1].guild_id.is_none());
    assert_eq!(ready.relationships[0].kind, 1);
    let ack: digcord_model::MessageAck =
        serde_json::from_str(r#"{"channel_id":"1","message_id":"2","version":3}"#).expect("ack");
    assert_eq!(ack.mention_count, 0);
}
