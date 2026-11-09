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
