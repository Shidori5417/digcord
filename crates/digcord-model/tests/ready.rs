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
