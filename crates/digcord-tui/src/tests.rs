use std::sync::Arc;

use digcord_model::{Message, Ready, Snowflake};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Color;

use super::*;

const GUILD: u64 = 1;
const ME: u64 = 100;
const FRIEND: u64 = 200;
const SOHBET: u64 = 11;
const DM: u64 = 50;
/// 2026-01-01T12:00:00Z civarı; id'den zaman damgası türetmek için.
const BASE_MS: u64 = 1_767_268_800_000;

fn ready() -> Ready {
    serde_json::from_value(serde_json::json!({
        "v": 9,
        "user": {"id": ME.to_string(), "username": "ben", "global_name": "Ben"},
        "session_id": "s", "resume_gateway_url": "wss://x",
        "guilds": [{
            "id": GUILD.to_string(), "large": false,
            "properties": {"name": "Test Sunucusu", "owner_id": "999"},
            // VIEW_CHANNEL + SEND_MESSAGES + ADD_REACTIONS + ATTACH_FILES
            "roles": [{"id": GUILD.to_string(), "name": "@everyone", "permissions": "35904", "position": 0},
                       {"id": "7", "name": "Moderatör", "permissions": "0", "position": 1, "color": 16711680}],
            "channels": [
                {"id": "10", "type": 4, "name": "Genel", "position": 0},
                {"id": SOHBET.to_string(), "type": 0, "name": "sohbet", "parent_id": "10", "position": 0, "topic": "konu"},
                {"id": "12", "type": 5, "name": "duyurular", "parent_id": "10", "position": 1},
                {"id": "13", "type": 2, "name": "Ses", "parent_id": "10", "position": 2},
                {"id": "14", "type": 0, "name": "gizli", "parent_id": "10", "position": 3,
                 "permission_overwrites": [{"id": GUILD.to_string(), "type": 0, "allow": "0", "deny": "1024"}]}
            ]
        }],
        "private_channels": [{"id": DM.to_string(), "type": 1, "recipient_ids": [FRIEND.to_string()], "last_message_id": null}],
        "users": [{"id": FRIEND.to_string(), "username": "arkadas", "global_name": "Arkadaş"}],
        "merged_members": [[{"user_id": ME.to_string(), "roles": []}]]
    }))
    .expect("ready")
}

fn msg(seq: u64, minutes: u64, author: u64, content: &str) -> Message {
    let ms = BASE_MS + minutes * 60_000;
    let id = ((ms - 1_420_070_400_000) << 22) + seq;
    let name = if author == ME { "ben" } else { "arkadas" };
    serde_json::from_value(serde_json::json!({
        "id": id.to_string(), "channel_id": SOHBET.to_string(), "guild_id": GUILD.to_string(),
        "author": {"id": author.to_string(), "username": name, "global_name": null},
        "content": content
    }))
    .expect("mesaj")
}

fn shared_with(messages: Vec<Message>, complete: bool) -> Arc<Shared> {
    let shared = Arc::new(Shared::default());
    {
        let mut st = shared.state.write().expect("kilit");
        st.load_ready(&ready());
        let limit = if complete {
            messages.len() + 1
        } else {
            messages.len()
        };
        let mut newest_first = messages;
        newest_first.reverse();
        st.history_loaded(Snowflake(SOHBET), None, limit, newest_first);
    }
    shared
}

/// Snapshot'lar makineden bağımsız olsun diye UTC.
fn test_app(shared: Arc<Shared>) -> App {
    let mut app = App::new(shared);
    app.tz = jiff::tz::TimeZone::UTC;
    app
}

fn key(code: KeyCode) -> UiEvent {
    UiEvent::Input(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

fn press(app: &mut App, codes: &[KeyCode]) -> Vec<Action> {
    codes.iter().filter_map(|c| app.handle(key(*c))).collect()
}

fn draw(app: &App, w: u16, h: u16) -> TestBackend {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("test terminali");
    terminal.draw(|f| app.draw(f)).expect("çizim");
    terminal.backend().clone()
}

/// Sunucuyu seçip #sohbet'i açar.
fn open_sohbet(app: &mut App) -> Vec<Action> {
    // Guilds: j (sunucu) → Enter (kanallara) → j (sohbet; 0 = kategori) → Enter
    press(
        app,
        &[
            KeyCode::Char('j'),
            KeyCode::Enter,
            KeyCode::Char('j'),
            KeyCode::Enter,
        ],
    )
}

#[test]
fn empty_layout_snapshot() {
    let app = test_app(Arc::new(Shared::default()));
    insta::assert_snapshot!(draw(&app, 80, 14));
}

#[test]
fn online_status_snapshot() {
    let mut app = test_app(Arc::new(Shared::default()));
    app.handle(UiEvent::Status(Status::Online {
        user: "kullanici".into(),
        guilds: 9,
        channels: 120,
    }));
    insta::assert_snapshot!(draw(&app, 80, 14));
}

#[test]
fn login_masks_token_snapshot() {
    let mut app = test_app(Arc::new(Shared::default()));
    app.handle(UiEvent::ShowLogin {
        error: Some("Token geçersiz, tekrar dene".into()),
    });
    for c in "gizli".chars() {
        app.handle(key(KeyCode::Char(c)));
    }
    let screen = draw(&app, 80, 14);
    assert!(!format!("{screen}").contains("gizli"));
    insta::assert_snapshot!(screen);
}

#[test]
fn login_submit_and_edit() {
    let mut app = test_app(Arc::new(Shared::default()));
    app.handle(UiEvent::ShowLogin { error: None });
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        None,
        "boş token gönderilmez"
    );
    for c in "abcx".chars() {
        app.handle(key(KeyCode::Char(c)));
    }
    app.handle(key(KeyCode::Backspace));
    app.handle(UiEvent::Input(Event::Paste("d\n".into())));
    app.handle(key(KeyCode::Char('q')));
    assert!(!app.should_quit, "giriş ekranında q yazılır");
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        Some(Action::SubmitToken("abcdq".into()))
    );
    assert!(matches!(app.screen, Screen::Main));
    assert_eq!(app.status, Status::Connecting);
}

#[test]
fn quit_keys() {
    let mut app = test_app(Arc::new(Shared::default()));
    app.handle(UiEvent::ShowLogin { error: None });
    app.handle(key(KeyCode::Esc));
    assert!(app.should_quit);

    for ev in [
        key(KeyCode::Char('q')),
        UiEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ))),
    ] {
        let mut app = test_app(Arc::new(Shared::default()));
        app.handle(ev);
        assert!(app.should_quit);
    }
}

#[test]
fn channel_entries_respect_permissions_and_collapse() {
    let mut app = test_app(shared_with(vec![], true));
    press(&mut app, &[KeyCode::Char('j'), KeyCode::Enter]);
    let labels: Vec<String> = app.entries().into_iter().map(|e| e.label).collect();
    // "gizli" @everyone'a kapalı → listede yok.
    assert_eq!(labels, ["▾ GENEL", "# sohbet", "# duyurular", "🔊 Ses"]);
    // Ses kanalında Enter katılır.
    assert_eq!(
        press(
            &mut app,
            &[
                KeyCode::Char('j'),
                KeyCode::Char('j'),
                KeyCode::Char('j'),
                KeyCode::Enter
            ]
        ),
        [Action::JoinVoice {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(13)
        }]
    );
    // Kategoriyi katla.
    app.chan_sel = 0;
    press(&mut app, &[KeyCode::Enter]);
    let labels: Vec<String> = app.entries().into_iter().map(|e| e.label).collect();
    assert_eq!(labels, ["▸ GENEL"]);
}

#[test]
fn dm_list_and_open() {
    let mut app = test_app(shared_with(vec![], true));
    // guild_sel 0 = DM'ler
    let actions = press(&mut app, &[KeyCode::Enter, KeyCode::Enter]);
    assert_eq!(
        actions,
        [Action::OpenChannel {
            guild: None,
            channel: Snowflake(DM),
            previous: None
        }]
    );
    assert_eq!(app.entries()[0].label, "@ Arkadaş");
}

#[test]
fn opening_channel_emits_action_with_previous() {
    let mut app = test_app(shared_with(vec![], true));
    let a = open_sohbet(&mut app);
    assert_eq!(
        a,
        [Action::OpenChannel {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(SOHBET),
            previous: None
        }]
    );
    assert_eq!(app.focus, Focus::Messages);
    // Duyurular'a geç: önceki kanal bildirilir.
    let a = press(
        &mut app,
        &[KeyCode::BackTab, KeyCode::Char('j'), KeyCode::Enter],
    );
    assert_eq!(
        a,
        [Action::OpenChannel {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(12),
            previous: Some(Snowflake(SOHBET))
        }]
    );
}

#[test]
fn message_view_snapshot() {
    let msgs = vec![
        msg(1, 0, FRIEND, "selam herkese"),
        msg(2, 1, FRIEND, "**kalın** ve `kod`"),
        msg(3, 2, ME, "naber <@200>"),
        msg(4, 20, FRIEND, "yedi dakikadan sonra yeni grup"),
    ];
    let mut app = test_app(shared_with(msgs, true));
    open_sohbet(&mut app);
    insta::assert_snapshot!(draw(&app, 100, 16));
}

/// Ekran satırları (kenar çubuğu dahil); metin aramaları için.
fn screen_rows(backend: &TestBackend) -> Vec<String> {
    format!("{backend}").lines().map(str::to_owned).collect()
}

fn row_index(rows: &[String], needle: &str) -> Option<usize> {
    rows.iter().position(|r| r.contains(needle))
}

/// Okunma kaydı `read_upto`'ya kadar olan, ardından yeni mesajlar gelen sohbet.
fn unread_channel(read_upto: usize, msgs: &[Message]) -> Arc<Shared> {
    let shared = shared_with(msgs[..read_upto].to_vec(), true);
    {
        let mut st = shared.state.write().expect("kilit");
        st.mark_read(Snowflake(SOHBET), msgs[read_upto - 1].id);
        for m in &msgs[read_upto..] {
            st.message_create(m.clone());
        }
    }
    shared
}

#[test]
fn date_separators_between_days_with_consistent_heights() {
    let msgs = vec![
        msg(1, 0, FRIEND, "bir"),
        msg(2, 5, FRIEND, "iki"),
        msg(3, 24 * 60, FRIEND, "üç"),
        msg(4, 24 * 60 + 1, FRIEND, "dört"),
    ];
    let mut app = test_app(shared_with(msgs, true));
    open_sohbet(&mut app);
    let rows = screen_rows(&draw(&app, 100, 20));
    let d1 = row_index(&rows, "── 1 Ocak 2026 ──").expect("ilk gün");
    let d2 = row_index(&rows, "── 2 Ocak 2026 ──").expect("ikinci gün");
    assert!(d1 < d2);
    // Aynı gün içinde ikinci ayırıcı yok.
    assert_eq!(
        rows.iter().filter(|r| r.contains("Ocak 2026 ──")).count(),
        2
    );
    // Ayırıcının hemen altında yazar başlığı: gün atlayınca gruplama kesilir.
    assert!(rows[d2 + 1].contains("arkadas"), "{}", rows[d2 + 1]);
    assert!(rows[d1 + 1].contains("arkadas"));
    // Dar ekranda da kaydırma yükseklikleri ayırıcıları içerir: en eskiye çıkılabilir.
    draw(&app, 100, 10);
    app.scroll_up(1000);
    let rows = screen_rows(&draw(&app, 100, 10));
    assert!(row_index(&rows, "kanalın başlangıcı").is_some());
}

#[test]
fn new_messages_line_is_placed_kept_and_cleared() {
    let msgs: Vec<Message> = (1..=4)
        .map(|i| msg(i, i, FRIEND, &format!("m{i}")))
        .collect();
    let shared = unread_channel(2, &msgs);
    let mut app = test_app(Arc::clone(&shared));
    open_sohbet(&mut app);
    let rows = screen_rows(&draw(&app, 100, 20));
    let line = row_index(&rows, "Yeni mesajlar").expect("çizgi");
    assert!(rows[line + 1].contains("arkadas"), "çizgi m3'ün üstünde");
    assert!(rows[line + 2].contains("m3"), "{}", rows[line + 2]);
    assert!(row_index(&rows, "m2").is_some_and(|r| r < line));
    // Renk: tema `new_messages` yuvası (kırmızı).
    let buf = draw(&app, 100, 20);
    let col = rows[line].chars().position(|c| c == 'Y').expect("Y sütunu");
    assert_eq!(buf.buffer()[(col as u16, line as u16)].fg, Color::Red);

    // Canlı okundu işareti çizgiyi silmez.
    shared
        .state
        .write()
        .expect("kilit")
        .mark_read(Snowflake(SOHBET), msgs[3].id);
    let rows = screen_rows(&draw(&app, 100, 20));
    assert!(row_index(&rows, "Yeni mesajlar").is_some());

    // G / End siler.
    press(&mut app, &[KeyCode::Char('G')]);
    let rows = screen_rows(&draw(&app, 100, 20));
    assert!(row_index(&rows, "Yeni mesajlar").is_none());
}

#[test]
fn new_messages_line_cleared_on_channel_switch_and_absent_when_read() {
    let msgs: Vec<Message> = (1..=3)
        .map(|i| msg(i, i, FRIEND, &format!("m{i}")))
        .collect();
    let mut app = test_app(unread_channel(1, &msgs));
    open_sohbet(&mut app);
    assert!(app.new_marker.is_some());
    app.open_channel(Some(Snowflake(GUILD)), Snowflake(12));
    assert!(app.new_marker.is_none());
    // Okunmuş kanalı açınca çizgi yok.
    let mut app = test_app(shared_with(msgs, true));
    open_sohbet(&mut app);
    assert!(app.new_marker.is_none());
    let rows = screen_rows(&draw(&app, 100, 20));
    assert!(row_index(&rows, "Yeni mesajlar").is_none());
}

#[test]
fn author_role_color_bot_badge_and_reply_line() {
    let mut colored = msg(1, 0, FRIEND, "renkli");
    colored.member = serde_json::from_value(serde_json::json!({"roles": ["7"]})).expect("üye");
    let mut bot = msg(2, 1, ME, "ben botum");
    bot.author.bot = true;
    let mut reply = msg(3, 2, FRIEND, "cevap");
    reply.kind = 19;
    reply.referenced_message = Some(Box::new(msg(1, 0, FRIEND, "renkli")));
    let mut app = test_app(shared_with(vec![colored, bot, reply], true));
    open_sohbet(&mut app);
    let backend = draw(&app, 100, 20);
    let rows = screen_rows(&backend);
    // Moderatör rolü kırmızı: yazar adı Rgb(255,0,0).
    let r = row_index(&rows, "arkadas  01.01.2026 12:00").expect("renkli başlık");
    let col = rows[r].chars().position(|c| c == 'a').expect("ad");
    // Kenar çubuğu çok baytlı karakterler içerebilir; sütun = karakter sırası (hepsi dar).
    assert_eq!(
        backend.buffer()[(col as u16, r as u16)].fg,
        Color::Rgb(255, 0, 0)
    );
    assert!(
        row_index(&rows, "ben BOT  01.01.2026 12:01").is_some(),
        "{rows:#?}"
    );
    assert!(
        row_index(&rows, "╭─ arkadas: renkli").is_some(),
        "{rows:#?}"
    );
}

#[test]
fn message_extras_snapshot() {
    let mut bot = msg(2, 1, ME, "**BOT** mesajı");
    bot.author.bot = true;
    let mut reply = msg(4, 24 * 60 + 3, FRIEND, "cevap <@100>");
    reply.kind = 19;
    reply.referenced_message = Some(Box::new(msg(2, 1, ME, "ilk <@200>")));
    let msgs = vec![
        msg(1, 0, FRIEND, "dünden kalan"),
        bot,
        msg(3, 24 * 60, FRIEND, "bugün"),
        reply,
    ];
    let mut app = test_app(unread_channel(2, &msgs));
    open_sohbet(&mut app);
    insta::assert_snapshot!(draw(&app, 100, 22));
}

#[test]
fn scroll_up_down_and_follow() {
    let msgs: Vec<Message> = (0..100)
        .map(|i| msg(i, i * 10, FRIEND, &format!("mesaj {i}")))
        .collect();
    let mut app = test_app(shared_with(msgs, true));
    open_sohbet(&mut app);
    draw(&app, 100, 12);
    assert_eq!(app.scroll, Scroll::default());
    let bottom = format!("{}", draw(&app, 100, 12));
    assert!(bottom.contains("mesaj 99"));

    press(&mut app, &[KeyCode::PageUp, KeyCode::PageUp]);
    assert!(app.scroll.anchor.is_some());
    let up = format!("{}", draw(&app, 100, 12));
    assert!(!up.contains("mesaj 99"), "{up}");
    assert!(up.contains("[geçmiş]"));

    // Aşağı inince tekrar en alta yapışır.
    for _ in 0..10 {
        press(&mut app, &[KeyCode::PageDown]);
    }
    assert_eq!(app.scroll, Scroll::default());

    // En üste kadar kaydırma en eski mesajda durur, başlangıç işareti görünür.
    for _ in 0..200 {
        press(&mut app, &[KeyCode::PageUp]);
    }
    let top = format!("{}", draw(&app, 100, 12));
    assert!(top.contains("mesaj 0"), "{top}");
    assert!(top.contains("kanalın başlangıcı"), "{top}");
    press(&mut app, &[KeyCode::Char('G')]);
    assert_eq!(app.scroll, Scroll::default());
}

#[test]
fn requests_older_when_top_visible_and_incomplete() {
    let msgs = vec![msg(1, 0, FRIEND, "tek mesaj")];
    let mut app = test_app(shared_with(msgs, false));
    open_sohbet(&mut app);
    draw(&app, 100, 12);
    assert_eq!(
        app.take_older_request(),
        Some(Action::LoadOlder {
            channel: Snowflake(SOHBET)
        })
    );
    assert_eq!(app.take_older_request(), None, "istek bir kez");

    // Geçmiş tamsa istek yok.
    let mut app = test_app(shared_with(vec![msg(1, 0, FRIEND, "x")], true));
    open_sohbet(&mut app);
    draw(&app, 100, 12);
    assert_eq!(app.take_older_request(), None);
}

#[test]
fn store_changed_clears_dirty_flag() {
    let shared = Arc::new(Shared::default());
    assert!(shared.mark_dirty());
    assert!(!shared.mark_dirty(), "bildirim birikmez");
    let mut app = test_app(shared.clone());
    app.handle(UiEvent::StoreChanged);
    assert!(shared.mark_dirty());
}

/// Kabul ölçümü (ROADMAP Aşama 2): 10 000 mesajlık kanal, 200×60, kare süresi.
/// `cargo test -p digcord-tui --release -- --ignored --nocapture frame_time`
#[test]
#[ignore]
fn frame_time_10k_messages() {
    use std::time::Instant;
    let text = "Lorem ipsum **dolor** sit amet, <@200> consectetur `adipiscing` elit; \
                https://example.com/yol ||gizli|| ve biraz daha uzun bir cümle ki satır sarılsın.";
    let msgs: Vec<Message> = (0..10_000)
        .map(|i| msg(i, i / 3, if i % 5 == 0 { ME } else { FRIEND }, text))
        .collect();
    let mut app = test_app(shared_with(msgs, true));
    open_sohbet(&mut app);
    let mut terminal = Terminal::new(TestBackend::new(200, 60)).expect("terminal");

    let mut measure = |app: &mut App, label: &str, scroll: Option<usize>| {
        if let Some(n) = scroll {
            app.scroll_up(n);
        }
        // Isınma
        terminal.draw(|f| app.draw(f)).expect("çizim");
        let n = 200;
        let start = Instant::now();
        for _ in 0..n {
            terminal.draw(|f| app.draw(f)).expect("çizim");
        }
        let per = start.elapsed() / n;
        println!("{label}: kare başına {per:?}");
        per
    };
    let bottom = measure(&mut app, "en alt", None);
    let middle = measure(&mut app, "~5000 mesaj yukarı", Some(5_000 * 4));
    let mut scroll_total = std::time::Duration::ZERO;
    for _ in 0..100 {
        let start = Instant::now();
        app.scroll_up(30);
        terminal.draw(|f| app.draw(f)).expect("çizim");
        scroll_total += start.elapsed();
    }
    println!("kaydırma + çizim (30 satır): {:?}", scroll_total / 100);
    assert!(bottom.as_millis() < 4 && middle.as_millis() < 4);
}

/// Bellek (RSS, `/proc/self/statm`; `unsafe` gerektirmez), MiB.
fn rss_mib() -> f64 {
    let s = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: f64 = s
        .split_whitespace()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(0.0);
    pages * 4096.0 / 1_048_576.0
}

/// Zirve RSS (`VmHWM`), MiB.
fn peak_rss_mib() -> f64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|l| l.split_whitespace().next())
        .and_then(|kb| kb.parse::<f64>().ok())
        .map_or(0.0, |kb| kb / 1024.0)
}

/// Sentetik büyük hesap (100 sunucu / ~5000 kanal / 2000 kullanıcı / 300 DM): ayrıştırma,
/// `load_ready`, bellek ve kare süresi. Çalıştırma:
/// `cargo test -p digcord-tui --release -- --ignored --nocapture large_account_profile`
#[test]
#[ignore]
fn large_account_profile() {
    use crate::perf_synth as syn;
    use std::time::Instant;

    // Üretim ayrı iş parçacığında (kendi malloc arena'sı): üretim çöpü ölçülen yığını
    // kirletmesin. Taban RSS üretimden sonra alınır.
    let (json, raw_msgs) = std::thread::spawn(|| {
        let json = syn::ready_json();
        let mut rng = syn::Lcg::new(99);
        let raw: Vec<(u64, u64, Vec<u8>)> = syn::text_channels()
            .step_by(8)
            .take(50)
            .map(|(g, ch)| (g, ch, syn::messages_json(ch, g, 200, &mut rng)))
            .collect();
        (json, raw)
    })
    .join()
    .expect("üretim");
    let base = rss_mib();
    println!(
        "READY JSON: {:.2} MiB; RSS taban {base:.1} MiB",
        json.len() as f64 / 1_048_576.0
    );
    let before_parse = rss_mib();

    let t = Instant::now();
    let ready: Ready = serde_json::from_slice(&json).expect("ready ayrıştırma");
    println!("(a) READY JSON ayrıştırma: {:?}", t.elapsed());
    println!(
        "    kanal {}, sunucu {}, DM {}, kullanıcı {}, read_state {}",
        ready.channel_count(),
        ready.guilds.len(),
        ready.private_channels.len(),
        ready.users.len(),
        ready.read_state.entries.len()
    );
    println!(
        "    RSS ayrıştırma sonrası: +{:.1} MiB",
        rss_mib() - before_parse
    );

    let shared = Arc::new(Shared::default());
    let before_load = rss_mib();
    {
        let mut st = shared.state.write().expect("kilit");
        let t = Instant::now();
        st.load_ready(&ready);
        println!("(b) State::load_ready: {:?}", t.elapsed());
    }
    drop(ready);
    drop(json);
    let after_load = rss_mib();
    println!(
        "(c) RSS: yükleme öncesi {before_load:.1}, ready bırakıldıktan sonra {after_load:.1} MiB (üst sınır, başlangıca göre +{:.1})",
        after_load - base
    );

    // 50 kanal x 200 mesaj
    let mut parse_total = std::time::Duration::ZERO;
    let bytes: usize = raw_msgs.iter().map(|(_, _, r)| r.len()).sum();
    let before_msgs = rss_mib();
    {
        let mut st = shared.state.write().expect("kilit");
        for (_, ch, raw) in &raw_msgs {
            let t = Instant::now();
            let msgs: Vec<Message> = serde_json::from_slice(raw).expect("mesajlar");
            parse_total += t.elapsed();
            st.history_loaded(Snowflake(*ch), None, 201, msgs);
        }
    }
    println!(
        "    50x200 mesaj: JSON {:.1} MiB, ayrıştırma {parse_total:?}",
        bytes as f64 / 1_048_576.0
    );
    println!(
        "(d) RSS 10k mesaj sonrası: {:.1} MiB (mesajlar +{:.1} MiB); zirve RSS {:.1} MiB",
        rss_mib(),
        rss_mib() - before_msgs,
        peak_rss_mib()
    );

    // (e) bir kare: ilk sunucunun ilk kanalı (mesajları yüklü)
    let mut app = test_app(shared);
    open_sohbet(&mut app);
    let mut terminal = Terminal::new(TestBackend::new(200, 60)).expect("terminal");
    let t = Instant::now();
    terminal.draw(|f| app.draw(f)).expect("çizim");
    println!("(e) ilk kare (soğuk): {:?}", t.elapsed());
    let n = 200;
    let t = Instant::now();
    for _ in 0..n {
        terminal.draw(|f| app.draw(f)).expect("çizim");
    }
    println!("    kare başına (sıcak): {:?}", t.elapsed() / n);
}

fn type_text(app: &mut App, text: &str) -> Vec<Action> {
    text.chars()
        .filter_map(|c| app.handle(key(KeyCode::Char(c))))
        .collect()
}

#[test]
fn insert_send_and_typing_throttle() {
    let mut app = test_app(shared_with(vec![msg(1, 0, FRIEND, "selam")], true));
    open_sohbet(&mut app);
    press(&mut app, &[KeyCode::Char('i')]);
    assert_eq!(app.mode, compose::Mode::Insert);
    let typed = type_text(&mut app, "merhaba");
    // İlk tuşta bir kez "yazıyor", sonra 8 sn içinde tekrar yok.
    assert_eq!(
        typed,
        [Action::Typing {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(SOHBET)
        }]
    );
    assert_eq!(
        press(&mut app, &[KeyCode::Enter]),
        [Action::SendMessage {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(SOHBET),
            content: "merhaba".into(),
            reply_to: None
        }]
    );
    assert!(app.editor.is_empty());
    assert_eq!(
        app.mode,
        compose::Mode::Insert,
        "gönderdikten sonra yazmaya devam"
    );
    // Boş Enter bir şey göndermez.
    assert!(press(&mut app, &[KeyCode::Enter]).is_empty());
    press(&mut app, &[KeyCode::Esc]);
    assert_eq!(app.mode, compose::Mode::Normal);
}

#[test]
fn select_reply_edit_delete_react_copy() {
    let msgs = vec![msg(1, 0, FRIEND, "soru"), msg(2, 1, ME, "benim mesajım")];
    let mut app = test_app(shared_with(msgs.clone(), true));
    open_sohbet(&mut app);
    draw(&app, 100, 20);
    press(&mut app, &[KeyCode::Char('v')]);
    assert_eq!(app.mode, compose::Mode::Select);
    assert_eq!(app.selected, Some(msgs[1].id), "en alttaki mesajdan başlar");

    // Kendi mesajım: e → düzenleme, metin dolu gelir; aynı metinle Enter bir şey göndermez.
    press(&mut app, &[KeyCode::Char('e')]);
    assert_eq!(app.target, compose::Target::Edit(msgs[1].id));
    assert_eq!(app.editor.text(), "benim mesajım");
    assert!(press(&mut app, &[KeyCode::Enter]).is_empty());
    press(&mut app, &[KeyCode::Char('v')].map(|_| KeyCode::Esc));
    press(&mut app, &[KeyCode::Char('v')]);

    // y → kopyala
    assert_eq!(
        press(&mut app, &[KeyCode::Char('y')]),
        [Action::Copy("benim mesajım".into())]
    );
    // d d → onay → e → sil
    assert!(press(&mut app, &[KeyCode::Char('d'), KeyCode::Char('d')]).is_empty());
    assert_eq!(app.mode, compose::Mode::ConfirmDelete(msgs[1].id));
    assert!(format!("{}", draw(&app, 100, 20)).contains("silinsin mi"));
    assert_eq!(
        press(&mut app, &[KeyCode::Char('e')]),
        [Action::DeleteMessage {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(SOHBET),
            message: msgs[1].id
        }]
    );

    // k → arkadaşın mesajı; e düzenlemez, d d silmez (izin yok), r yanıt.
    press(&mut app, &[KeyCode::Char('k')]);
    assert_eq!(app.selected, Some(msgs[0].id));
    press(&mut app, &[KeyCode::Char('e')]);
    assert_eq!(app.mode, compose::Mode::Select);
    assert!(app.notice.is_some());
    press(&mut app, &[KeyCode::Char('d'), KeyCode::Char('d')]);
    assert_eq!(app.mode, compose::Mode::Select);

    // + → tepki istemi; ":+1" Enter → 👍 ekle.
    press(&mut app, &[KeyCode::Char('+')]);
    assert_eq!(app.mode, compose::Mode::React);
    type_text(&mut app, "+1");
    assert_eq!(
        press(&mut app, &[KeyCode::Enter]),
        [Action::React {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(SOHBET),
            message: msgs[0].id,
            emoji: digcord_model::ReactionEmoji::unicode("👍"),
            add: true
        }]
    );

    press(&mut app, &[KeyCode::Char('r')]);
    assert_eq!(app.target, compose::Target::Reply(msgs[0].id));
    assert!(format!("{}", draw(&app, 100, 20)).contains("yanıtlanıyor: arkadas"));
    type_text(&mut app, "cevap");
    assert_eq!(
        press(&mut app, &[KeyCode::Enter]),
        [Action::SendMessage {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(SOHBET),
            content: "cevap".into(),
            reply_to: Some(msgs[0].id)
        }]
    );
}

#[test]
fn up_on_empty_input_edits_last_own_message() {
    let msgs = vec![msg(1, 0, ME, "eski"), msg(2, 1, FRIEND, "araya")];
    let mut app = test_app(shared_with(msgs.clone(), true));
    open_sohbet(&mut app);
    press(&mut app, &[KeyCode::Char('i'), KeyCode::Up]);
    assert_eq!(app.target, compose::Target::Edit(msgs[0].id));
    assert_eq!(app.editor.text(), "eski");
    type_text(&mut app, "!");
    assert_eq!(
        press(&mut app, &[KeyCode::Enter]),
        [Action::EditMessage {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(SOHBET),
            message: msgs[0].id,
            content: "eski!".into()
        }]
    );
    assert_eq!(app.mode, compose::Mode::Normal);
}

#[test]
fn mention_autocomplete_popup_and_tab() {
    let mut app = test_app(shared_with(vec![msg(1, 0, FRIEND, "selam")], true));
    open_sohbet(&mut app);
    press(&mut app, &[KeyCode::Char('i')]);
    type_text(&mut app, "naber @ark");
    let screen = format!("{}", draw(&app, 100, 20));
    assert!(screen.contains("@arkadas"), "{screen}");
    press(&mut app, &[KeyCode::Tab]);
    assert_eq!(app.editor.text(), format!("naber <@{FRIEND}> "));
    // Emoji: ":fi" → aday listesi, Tab unicode emoji ekler.
    type_text(&mut app, ":fire");
    assert!(!app.candidates().is_empty());
    press(&mut app, &[KeyCode::Tab]);
    assert!(app.editor.text().ends_with("🔥 "), "{}", app.editor.text());
}

#[test]
fn no_send_permission_blocks_insert() {
    let shared = shared_with(vec![], true);
    let mut app = test_app(shared);
    // duyurular (12): @everyone 3072 = VIEW+SEND → yazılabilir; izni kaldırmak için yeni ready gerekmez:
    // DM'de her zaman yazılabilir, sunucu kanalında izin yoksa giriş kapalı olmalı.
    open_sohbet(&mut app);
    {
        let mut st = app.shared.state.write().expect("kilit");
        let mut ready = ready();
        ready.guilds[0].roles[0].permissions = digcord_model::Permissions::VIEW_CHANNEL;
        st.load_ready(&ready);
    }
    press(&mut app, &[KeyCode::Char('i')]);
    assert_eq!(app.mode, compose::Mode::Normal);
    assert!(app.notice.as_deref().is_some_and(|n| n.contains("izni")));
    assert!(format!("{}", draw(&app, 100, 12)).contains("yazma iznin yok"));
}

#[test]
fn upload_command() {
    let mut app = test_app(shared_with(vec![], true));
    open_sohbet(&mut app);
    press(&mut app, &[KeyCode::Esc, KeyCode::Char(':')]);
    assert_eq!(app.mode, compose::Mode::Command);
    type_text(&mut app, "upload /tmp/a.png bak");
    let a = press(&mut app, &[KeyCode::Enter]);
    assert!(matches!(
        &a[..],
        [Action::Upload { path, content, .. }] if path == std::path::Path::new("/tmp/a.png") && content == "bak"
    ));
    press(&mut app, &[KeyCode::Char(':')]);
    type_text(&mut app, "q");
    press(&mut app, &[KeyCode::Enter]);
    assert!(app.should_quit);
}

#[test]
fn switcher_fuzzy_opens_channel_and_moves_selection() {
    let mut app = test_app(shared_with(vec![], true));
    let ctrl_k = UiEvent::Input(Event::Key(KeyEvent::new(
        KeyCode::Char('k'),
        KeyModifiers::CONTROL,
    )));
    app.handle(ctrl_k);
    assert_eq!(app.mode, compose::Mode::Switcher);
    type_text(&mut app, "duy");
    let screen = format!("{}", draw(&app, 100, 24));
    assert!(screen.contains("#duyurular · Test Sunucusu"), "{screen}");
    let a = press(&mut app, &[KeyCode::Enter]);
    assert_eq!(
        a,
        [Action::OpenChannel {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(12),
            previous: None
        }]
    );
    assert_eq!(app.guild_sel, 1, "şerit sunucuya geçti");
    assert_eq!(app.entries()[app.chan_sel].id, Snowflake(12));
    assert_eq!(app.mode, compose::Mode::Normal);
}

#[test]
fn unread_mentions_shown_and_alt_down_jumps() {
    let shared = shared_with(vec![msg(1, 0, FRIEND, "eski")], true);
    let mut app = test_app(shared.clone());
    {
        let mut st = shared.state.write().expect("kilit");
        // Okunma kaydı: sohbet 1. mesaja kadar okunmuş.
        st.mark_read(Snowflake(SOHBET), msg(1, 0, FRIEND, "").id);
        let mut m = msg(2, 5, FRIEND, "selam <@100>");
        m.mentions = vec![digcord_model::User {
            id: Snowflake(ME),
            username: "ben".into(),
            global_name: None,
            bot: false,
        }];
        st.message_create(m);
    }
    press(&mut app, &[KeyCode::Char('j'), KeyCode::Enter]);
    let screen = format!("{}", draw(&app, 100, 14));
    assert!(screen.contains("# sohbet (1)"), "{screen}");
    let alt_down = UiEvent::Input(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::ALT)));
    assert_eq!(
        app.handle(alt_down),
        Some(Action::OpenChannel {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(SOHBET),
            previous: None
        })
    );
}

#[test]
fn friends_section_in_dm_panel_and_focus_events() {
    let shared = Arc::new(Shared::default());
    {
        let mut st = shared.state.write().expect("kilit");
        let mut r = ready();
        r.relationships = serde_json::from_value(serde_json::json!([
            {"id": FRIEND.to_string(), "type": 1, "nickname": null},
            {"id": "300", "type": 1, "nickname": "Takma"}
        ]))
        .expect("ilişkiler");
        st.load_ready(&r);
    }
    let mut app = test_app(shared);
    let labels: Vec<String> = app.entries().into_iter().map(|e| e.label).collect();
    assert_eq!(
        labels,
        ["@ Arkadaş", "▾ ARKADAŞLAR (2)", "  Arkadaş", "  Takma"]
    );
    // DM'si olan arkadaşta Enter DM'yi açar; olmayanda bir şey olmaz.
    press(&mut app, &[KeyCode::Enter]);
    app.chan_sel = 2;
    assert_eq!(
        press(&mut app, &[KeyCode::Enter]),
        [Action::OpenChannel {
            guild: None,
            channel: Snowflake(DM),
            previous: None
        }]
    );
    app.focus = Focus::Channels;
    app.chan_sel = 3;
    assert!(press(&mut app, &[KeyCode::Enter]).is_empty());

    assert_eq!(
        app.handle(UiEvent::Input(Event::FocusLost)),
        Some(Action::Focus(false))
    );
    assert_eq!(
        app.handle(UiEvent::Input(Event::FocusGained)),
        Some(Action::Focus(true))
    );
}

/// Resimli mesaj: 400×200 piksellik ek; yarım blok yedeğinde hücre 10×20 → 40×10 hücre,
/// `image_max_rows = 4` ile 16×4.
fn image_msg(seq: u64, minutes: u64) -> Message {
    let mut m = msg(seq, minutes, FRIEND, "resim");
    m.attachments = serde_json::from_value(serde_json::json!([{
        "id": "9", "filename": "a.png", "size": 10, "width": 400, "height": 200,
        "content_type": "image/png",
        "url": "https://cdn.discordapp.com/attachments/11/9/a.png?ex=1&is=2&hm=3&",
        "proxy_url": "https://media.discordapp.net/attachments/11/9/a.png?ex=1&is=2&hm=3&"
    }]))
    .expect("ek");
    m
}

fn image_app(msgs: Vec<Message>) -> App {
    let mut app = test_app(shared_with(msgs, true));
    app.images = media::Images::new(
        Some(ratatui_image::picker::Picker::halfblocks()),
        Options {
            images: ImageMode::Thumbnail,
            image_max_rows: 4,
            ..Options::default()
        },
    );
    open_sohbet(&mut app);
    app
}

fn png_bytes() -> Vec<u8> {
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(160, 80)
        .write_to(&mut png, image::ImageFormat::Png)
        .expect("png");
    png.into_inner()
}

fn row_text(backend: &TestBackend, y: u16) -> String {
    let buf = backend.buffer();
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol().to_owned())
        .collect()
}

#[test]
fn image_requested_once_and_drawn_in_place() {
    let msgs = vec![image_msg(1, 0), msg(2, 1, FRIEND, "sonra")];
    let mut app = image_app(msgs.clone());
    let screen = draw(&app, 100, 24);
    let jobs = app.images.take_wanted();
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        jobs[0].url(),
        "https://media.discordapp.net/attachments/11/9/a.png?ex=1&is=2&hm=3&width=160&height=80"
    );
    assert_eq!(jobs[0].origin(), Some((Snowflake(SOHBET), msgs[0].id)));
    let loading = (0..24).find(|y| row_text(&screen, *y).contains("yükleniyor…"));
    let label = (0..24)
        .find(|y| row_text(&screen, *y).contains("[ek: a.png 400×200]"))
        .expect("etiket");
    assert_eq!(loading, Some(label + 1), "resim etiketin hemen altında");

    // Aynı çizim tekrarında yeni istek yok; resim gelince yerinde çizilir.
    draw(&app, 100, 24);
    assert!(app.images.take_wanted().is_empty());
    let job = jobs.into_iter().next().expect("iş");
    app.handle(job.encode(&png_bytes()));
    let screen = draw(&app, 100, 24);
    assert!(!row_text(&screen, label + 1).contains("yükleniyor"));
    // Sonraki mesaj resmin 4 satırının altında.
    assert!(row_text(&screen, label + 5).contains("sonra"));
}

#[test]
fn partly_visible_image_is_not_drawn() {
    // Resimli mesajın altında ekranı dolduracak kadar mesaj: resmin üstü ekran dışında kalsın.
    let mut msgs = vec![image_msg(1, 0)];
    for i in 0..5 {
        msgs.push(msg(10 + i, 1, FRIEND, &format!("satır {i}")));
    }
    let app = image_app(msgs);
    // Mesaj alanı 7 satır: alttaki 5 mesaj + resmin son 2 satırı.
    let screen = draw(&app, 100, 12);
    assert!(
        app.images.take_wanted().is_empty(),
        "kısmen görünen resim istenmez"
    );
    assert!(row_text(&screen, 1).contains("[resim]"));
}

#[test]
fn viewer_opens_on_o_and_external_on_second_o() {
    let msgs = vec![image_msg(1, 0)];
    let mut app = image_app(msgs);
    draw(&app, 100, 24);
    press(&mut app, &[KeyCode::Char('v'), KeyCode::Char('o')]);
    assert_eq!(app.mode, compose::Mode::Viewer);
    let screen = draw(&app, 100, 24);
    assert!(row_text(&screen, 0).contains("a.png (1/1)"));
    let actions = press(&mut app, &[KeyCode::Char('o')]);
    assert_eq!(
        actions,
        [Action::OpenExternal(
            "https://cdn.discordapp.com/attachments/11/9/a.png?ex=1&is=2&hm=3&".into()
        )]
    );
    press(&mut app, &[KeyCode::Esc]);
    assert_eq!(app.mode, compose::Mode::Select);
}

#[test]
fn o_opens_link_when_images_off() {
    let msgs = vec![msg(1, 0, FRIEND, "bak https://example.com/x")];
    let mut app = test_app(shared_with(msgs, true));
    open_sohbet(&mut app);
    draw(&app, 100, 20);
    let actions = press(&mut app, &[KeyCode::Char('v'), KeyCode::Char('o')]);
    assert_eq!(
        actions,
        [Action::OpenExternal("https://example.com/x".into())]
    );
}

#[test]
fn voice_members_status_and_commands() {
    let shared = shared_with(vec![], true);
    {
        let mut st = shared.state.write().expect("kilit");
        let vs: digcord_model::VoiceState = serde_json::from_value(serde_json::json!({
            "guild_id": GUILD.to_string(), "channel_id": "13", "user_id": FRIEND.to_string(),
            "session_id": "x", "self_mute": true
        }))
        .expect("ses durumu");
        st.voice_state_update(&vs);
        st.set_speaking(Snowflake(FRIEND), true);
        st.set_voice_call(Some(digcord_store::VoiceCall {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(13),
            phase: digcord_store::VoicePhase::Connected { e2ee: true },
            self_mute: false,
            self_deaf: true,
        }));
    }
    let mut app = test_app(shared);
    press(&mut app, &[KeyCode::Char('j'), KeyCode::Enter]);
    let entries = app.entries();
    let member = entries.last().expect("üye");
    assert_eq!(member.label, "  Arkadaş 🎤✕");
    assert_eq!(member.kind, EntryKind::VoiceMember { speaking: true });
    // Zaten içindeyken Enter yeniden katılmaz.
    app.chan_sel = 4 - 1;
    assert!(press(&mut app, &[KeyCode::Enter]).is_empty());
    let screen = draw(&app, 120, 20);
    let status: String = (0..120)
        .map(|x| screen.buffer()[(x, 19)].symbol().to_owned())
        .collect();
    // Geniş emoji iki hücre kaplar; boşluklar atılarak karşılaştırılır.
    let compact: String = status.split_whitespace().collect();
    assert!(compact.contains("🔊Ses·bağlı(uçtanuca)🎧✕"), "{status}");
    // Komutlar ve kısayollar.
    for (cmd, action) in [
        ("leave", Action::LeaveVoice),
        ("devices", Action::ListDevices),
        ("mute", Action::ToggleMute),
        ("deaf", Action::ToggleDeaf),
    ] {
        press(&mut app, &[KeyCode::Char(':')]);
        let mut keys: Vec<KeyCode> = cmd.chars().map(KeyCode::Char).collect();
        keys.push(KeyCode::Enter);
        assert_eq!(press(&mut app, &keys), [action]);
    }
    let shortcut = UiEvent::Input(Event::Key(KeyEvent::new(
        KeyCode::Char('M'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    )));
    assert_eq!(app.handle(shortcut), Some(Action::ToggleMute));

    // :vol arkadaş 50 → seviye 0.5; bilinmeyen ad → uyarı.
    let run = |app: &mut App, cmd: &str| {
        press(app, &[KeyCode::Char(':')]);
        let mut keys: Vec<KeyCode> = cmd.chars().map(KeyCode::Char).collect();
        keys.push(KeyCode::Enter);
        press(app, &keys)
    };
    assert_eq!(
        run(&mut app, "vol arkadaş 50"),
        [Action::SetVolume {
            user: Snowflake(FRIEND),
            volume: 0.5
        }]
    );
    assert!(run(&mut app, "vol kimse 50").is_empty());
    assert!(app.notice.as_deref().is_some_and(|n| n.contains("kimse")));

    // Bas-konuş: basma ve bırakma olayları.
    app.push_to_talk = true;
    let ptt = |kind| {
        let mut k = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL);
        k.kind = kind;
        UiEvent::Input(Event::Key(k))
    };
    use ratatui::crossterm::event::KeyEventKind as K;
    assert_eq!(app.handle(ptt(K::Press)), Some(Action::PushToTalk(true)));
    assert_eq!(app.handle(ptt(K::Repeat)), None);
    assert_eq!(app.handle(ptt(K::Release)), Some(Action::PushToTalk(false)));
}

// ------------------------------------------------------------ tema, kısayol, ayarlar

fn key_mod(code: KeyCode, mods: KeyModifiers) -> UiEvent {
    UiEvent::Input(Event::Key(KeyEvent::new(code, mods)))
}

fn run_command(app: &mut App, cmd: &str) -> Vec<Action> {
    press(app, &[KeyCode::Char(':')]);
    let mut keys: Vec<KeyCode> = cmd.chars().map(KeyCode::Char).collect();
    keys.push(KeyCode::Enter);
    press(app, &keys)
}

fn overrides(pairs: &[(&str, &str)]) -> Keymap {
    let list: Vec<(String, Vec<String>)> = pairs
        .iter()
        .map(|(a, k)| ((*a).to_owned(), vec![(*k).to_owned()]))
        .collect();
    let (map, warnings) = Keymap::from_overrides(&list);
    assert!(warnings.is_empty(), "{warnings:?}");
    map
}

#[test]
fn custom_keys_replace_normal_mode_defaults() {
    let mut app = test_app(shared_with(vec![], true));
    app.keys = overrides(&[
        ("quit", "x"),
        ("move_down", "n"),
        ("switcher", "ctrl+o"),
        ("command_mode", ";"),
    ]);
    // Eski tuşlar artık işlemez.
    press(&mut app, &[KeyCode::Char('q'), KeyCode::Char('j')]);
    assert!(!app.should_quit);
    assert_eq!(app.guild_sel, 0);
    app.handle(key(KeyCode::Char('n')));
    assert_eq!(app.guild_sel, 1);
    app.handle(key_mod(KeyCode::Char('k'), KeyModifiers::CONTROL));
    assert_eq!(app.mode, compose::Mode::Normal);
    app.handle(key_mod(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert_eq!(app.mode, compose::Mode::Switcher);
    app.handle(key(KeyCode::Esc));
    app.handle(key(KeyCode::Char(';')));
    assert_eq!(app.mode, compose::Mode::Command);
    app.handle(key(KeyCode::Esc));
    app.handle(key(KeyCode::Char('x')));
    assert!(app.should_quit);
}

#[test]
fn custom_keys_replace_select_mode_defaults_and_global_keys() {
    let mut app = test_app(shared_with(vec![msg(1, 0, ME, "merhaba")], true));
    open_sohbet(&mut app);
    app.keys = overrides(&[
        ("reply", "R"),
        ("toggle_mute", "f5"),
        ("select_mode", "s"),
        ("back", "q"),
    ]);
    press(&mut app, &[KeyCode::Char('s')]);
    assert_eq!(app.mode, compose::Mode::Select);
    // `r` artık yanıtlamaz; Shift+R yanıtlar.
    app.handle(key(KeyCode::Char('r')));
    assert_eq!(app.mode, compose::Mode::Select);
    app.handle(key_mod(KeyCode::Char('R'), KeyModifiers::SHIFT));
    assert_eq!(app.mode, compose::Mode::Insert);
    app.handle(key(KeyCode::Esc));
    press(&mut app, &[KeyCode::Char('s'), KeyCode::Char('q')]);
    assert_eq!(app.mode, compose::Mode::Normal);
    // Genel tuş her modda çalışır; eskisi artık çalışmaz.
    assert_eq!(app.handle(key(KeyCode::F(5))), Some(Action::ToggleMute));
    let old = key_mod(
        KeyCode::Char('M'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    assert_eq!(app.handle(old), None);
    // Ctrl+C her zaman çıkar.
    app.handle(key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.should_quit);
}

fn setting_items() -> Vec<SettingItem> {
    vec![
        SettingItem::new(
            "behavior.send_typing",
            SettingValue::Bool(true),
            SettingKind::Bool,
            true,
        ),
        SettingItem::new(
            "ui.image_max_rows",
            SettingValue::Int(10),
            SettingKind::Int {
                min: 1,
                max: 100,
                step: 1,
            },
            false,
        ),
        SettingItem::new(
            "ui.images",
            SettingValue::Str("thumbnail".into()),
            SettingKind::Choice(
                ["off", "thumbnail", "full"]
                    .map(|s| SettingValue::Str(s.into()))
                    .to_vec(),
            ),
            false,
        ),
        SettingItem::new(
            "ui.theme",
            SettingValue::Str("default".into()),
            SettingKind::ReadOnly,
            false,
        ),
    ]
}

#[test]
fn settings_screen_navigation_edits_and_save() {
    let mut app = test_app(Arc::new(Shared::default()));
    app.settings = settings::State::new(setting_items());
    assert!(run_command(&mut app, "settings").is_empty());
    assert_eq!(app.mode, compose::Mode::Settings);
    insta::assert_snapshot!("settings_screen", draw(&app, 90, 10));

    // Canlı ayar: Enter değiştirir ve eylem üretir.
    assert_eq!(
        press(&mut app, &[KeyCode::Enter]),
        [Action::ApplySetting {
            key: "behavior.send_typing".into(),
            value: SettingValue::Bool(false)
        }]
    );
    // Yeniden başlatma gerektiren ayar: eylem yok, not var.
    assert!(press(&mut app, &[KeyCode::Char('j'), KeyCode::Char('+')]).is_empty());
    assert_eq!(app.settings.items[1].value, SettingValue::Int(11));
    assert_eq!(app.notice.as_deref(), Some("yeniden başlatınca geçerli"));
    // Enter sayıyı değiştirmez; sınırlar aşılmaz.
    press(&mut app, &[KeyCode::Enter]);
    assert_eq!(app.settings.items[1].value, SettingValue::Int(11));
    app.settings.items[1].value = SettingValue::Int(100);
    press(&mut app, &[KeyCode::Char('+')]);
    assert_eq!(app.settings.items[1].value, SettingValue::Int(100));
    // Seçenek döngüsü; salt okunur ayar değişmez.
    press(&mut app, &[KeyCode::Char('j'), KeyCode::Enter]);
    assert_eq!(
        app.settings.items[2].value,
        SettingValue::Str("full".into())
    );
    press(&mut app, &[KeyCode::Char('j'), KeyCode::Enter]);
    assert_eq!(
        app.settings.items[3].value,
        SettingValue::Str("default".into())
    );
    // Aşağı sınır: son satırdan öteye geçmez.
    press(&mut app, &[KeyCode::Char('j'), KeyCode::Char('j')]);
    assert_eq!(app.settings.sel, 3);

    // `s`: sadece değişenler kaydedilir; ikinci `s` boş.
    assert_eq!(
        press(&mut app, &[KeyCode::Char('s')]),
        [Action::SaveSettings(vec![
            ("behavior.send_typing".into(), SettingValue::Bool(false)),
            ("ui.image_max_rows".into(), SettingValue::Int(100)),
            ("ui.images".into(), SettingValue::Str("full".into())),
        ])]
    );
    assert!(press(&mut app, &[KeyCode::Char('s')]).is_empty());
    assert_eq!(app.notice.as_deref(), Some("Kaydedilecek değişiklik yok"));

    // Esc kapatır; komut modundaki tuşlar ayarlar ekranında etkisizdir.
    press(&mut app, &[KeyCode::Esc]);
    assert_eq!(app.mode, compose::Mode::Normal);
}

#[test]
fn theme_command_and_event_update_colors() {
    let mut app = test_app(shared_with(vec![msg(1, 0, FRIEND, "selam")], true));
    app.settings = settings::State::new(setting_items());
    assert_eq!(
        run_command(&mut app, "theme light"),
        [Action::LoadTheme("light".into())]
    );
    assert!(run_command(&mut app, "theme").is_empty());
    assert!(
        app.notice
            .as_deref()
            .is_some_and(|n| n.contains("Kullanım"))
    );

    open_sohbet(&mut app);
    let mut theme = Theme::default();
    for (slot, value) in [
        ("background", "#101820"),
        ("author", "magenta"),
        ("border_focused", "#00ff00"),
        ("status_bg", "blue"),
    ] {
        theme.set(slot, value).expect("geçerli yuva");
    }
    app.handle(UiEvent::Theme {
        name: "koyu".into(),
        theme: Box::new(theme),
    });
    assert_eq!(app.notice.as_deref(), Some("Tema: koyu"));
    // Tema kaydedilecek ayar olarak işaretlenir.
    assert_eq!(
        app.settings.items[3].value,
        SettingValue::Str("koyu".into())
    );

    let screen = draw(&app, 80, 12);
    insta::assert_snapshot!("themed_message_view", screen);
    let buf = screen.buffer();
    // Odaktaki mesaj paneli çerçevesi (sol üst köşe: sütun 30) yeşil, arka plan koyu.
    assert_eq!(buf[(30, 0)].fg, Color::Rgb(0, 255, 0));
    assert_eq!(buf[(31, 0)].bg, Color::Rgb(16, 24, 32));
    // Yazar adı macenta.
    assert_eq!(buf[(31, 6)].symbol(), "a");
    assert_eq!(buf[(31, 6)].fg, Color::Magenta);
    // Durum çubuğu mavi arka planlı.
    assert_eq!(buf[(5, 11)].bg, Color::Blue);
}

#[test]
fn default_theme_keeps_the_builtin_colors() {
    let mut app = test_app(shared_with(vec![msg(1, 0, FRIEND, "selam")], true));
    open_sohbet(&mut app);
    let screen = draw(&app, 80, 12);
    let buf = screen.buffer();
    assert_eq!(buf[(30, 0)].fg, Color::Cyan, "odaktaki çerçeve");
    assert_eq!(buf[(0, 0)].fg, Color::Reset, "odaksız çerçeve");
    assert_eq!(buf[(31, 0)].bg, Color::Reset);
}

// ------------------------------------------------------------------ thread'ler

const THREAD_SOHBET: u64 = 30;
const THREAD_LOCKED: u64 = 31;
const FORUM: u64 = 20;
const BUSY: u64 = 12;

fn thread_json(
    id: u64,
    parent: u64,
    name: &str,
    archived: bool,
    locked: bool,
) -> serde_json::Value {
    serde_json::json!({"id": id.to_string(), "type": 11, "guild_id": GUILD.to_string(),
        "parent_id": parent.to_string(), "name": name, "last_message_id": id.to_string(),
        "thread_metadata": {"archived": archived, "locked": locked}})
}

/// #sohbet (2 aktif + 1 arşivli thread), #kalabalik (5 thread), forum (thread'siz).
fn threads_shared() -> Arc<Shared> {
    threads_shared_with(&[])
}

/// [`threads_shared`] + ek kanallar (ör. ses kanalı).
fn threads_shared_with(extra_channels: &[serde_json::Value]) -> Arc<Shared> {
    let mut r = serde_json::to_value(serde_json::json!({
        "v": 9,
        "user": {"id": ME.to_string(), "username": "ben", "global_name": "Ben"},
        "session_id": "s", "resume_gateway_url": "wss://x",
        "guilds": [{
            "id": GUILD.to_string(), "large": false,
            "properties": {"name": "Test Sunucusu", "owner_id": "999"},
            // VIEW + SEND + SEND_MESSAGES_IN_THREADS + ADD_REACTIONS + ATTACH_FILES
            "roles": [{"id": GUILD.to_string(), "name": "@everyone", "permissions": "274877942848", "position": 0}],
            "channels": [
                {"id": "10", "type": 4, "name": "Genel", "position": 0},
                {"id": SOHBET.to_string(), "type": 0, "name": "sohbet", "parent_id": "10", "position": 0},
                {"id": BUSY.to_string(), "type": 0, "name": "kalabalik", "parent_id": "10", "position": 1},
                {"id": FORUM.to_string(), "type": 15, "name": "forum", "parent_id": "10", "position": 2}
            ],
            "threads": [
                thread_json(THREAD_SOHBET, SOHBET, "konu bir", false, false),
                thread_json(THREAD_LOCKED, SOHBET, "kilitli konu", false, true),
                thread_json(32, SOHBET, "arşivli", true, false),
                thread_json(40, BUSY, "a", false, false),
                thread_json(41, BUSY, "b", false, false),
                thread_json(42, BUSY, "c", false, false),
                thread_json(43, BUSY, "d", false, false),
                thread_json(44, BUSY, "e", false, false)
            ]
        }],
        "private_channels": [],
        "merged_members": [[{"user_id": ME.to_string(), "roles": []}]],
        "read_state": {"entries": [
            {"id": BUSY.to_string(), "last_message_id": "0", "mention_count": 0},
            {"id": "43", "last_message_id": "1", "mention_count": 2}
        ]}
    }))
    .expect("json");
    // Okunma kaydı: kalabalık kanalın 43 numaralı thread'i okunmamış (son mesaj 43 > 1).
    r["v"] = serde_json::json!(9);
    if let Some(chs) = r["guilds"][0]["channels"].as_array_mut() {
        chs.extend_from_slice(extra_channels);
    }
    let ready: Ready = serde_json::from_value(r).expect("ready");
    let shared = Arc::new(Shared::default());
    shared.state.write().expect("kilit").load_ready(&ready);
    shared
}

fn labels(app: &App) -> Vec<String> {
    app.entries().into_iter().map(|e| e.label).collect()
}

fn to_channels(app: &mut App) {
    press(app, &[KeyCode::Char('j'), KeyCode::Enter]);
}

#[test]
fn threads_nest_under_parent_with_auto_collapse() {
    let mut app = test_app(threads_shared());
    to_channels(&mut app);
    // #sohbet: 2 aktif thread (≤3) açık, arşivli gizli. #kalabalik: 5 thread → kapalı, sayı ipucu.
    assert_eq!(
        labels(&app),
        [
            "▾ GENEL",
            "# sohbet",
            "└ kilitli konu 🔒",
            "└ konu bir",
            "# kalabalik ▸5 ",
            "≡ forum ▸",
        ]
        .map(|l| l.trim_end().to_owned())
    );
    let entries = app.entries();
    assert_eq!(entries[2].depth, 2);
    assert!(entries[2].dim && !entries[3].dim);
    // Kapalı üst satırda okunmamış/bahsetme toplanır.
    assert!(entries[4].unread);
    assert_eq!(entries[4].mentions, 2);
    // `:threads` kalabalık kanala geçince açar, tekrar kapatır.
    app.chan_sel = 4;
    assert!(run_command(&mut app, "threads").is_empty());
    assert_eq!(labels(&app).len(), 6 + 5);
    run_command(&mut app, "threads");
    assert_eq!(labels(&app).len(), 6);
    // Açıkken üst satır okunmamışı kendi durumundan alır.
    run_command(&mut app, "threads");
    assert!(!app.entries()[4].unread || app.entries()[4].mentions == 0);
    // Metin kanalında da kapatılabilir.
    app.chan_sel = 1;
    run_command(&mut app, "threads");
    assert!(labels(&app).contains(&"# sohbet ▸2".to_owned()));
}

#[test]
fn opening_thread_emits_open_channel_and_reveals_it() {
    let mut app = test_app(threads_shared());
    to_channels(&mut app);
    // chan_sel 3 = "└ konu bir"
    let actions = press(
        &mut app,
        &[
            KeyCode::Char('j'),
            KeyCode::Char('j'),
            KeyCode::Char('j'),
            KeyCode::Enter,
        ],
    );
    assert_eq!(
        actions,
        [Action::OpenChannel {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(THREAD_SOHBET),
            previous: None
        }]
    );
    // Kalabalık kanalın kapalı thread'i dışarıdan (Alt+↓ gibi) açılırsa liste açılır.
    let action = app.open_channel(Some(Snowflake(GUILD)), Snowflake(42));
    assert!(matches!(action, Action::OpenChannel { .. }));
    assert!(labels(&app).contains(&"└ c".to_owned()));
    assert_eq!(app.entries()[app.chan_sel].id, Snowflake(42));
}

#[test]
fn forum_activation_fetches_then_lists_threads() {
    let shared = threads_shared();
    let mut app = test_app(shared.clone());
    to_channels(&mut app);
    app.chan_sel = 5; // forum
    let actions = press(&mut app, &[KeyCode::Enter]);
    assert_eq!(
        actions,
        [Action::LoadThreads {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(FORUM),
            archived: false
        }]
    );
    assert_eq!(labels(&app).last().map(String::as_str), Some("≡ forum"));
    // Yanıt gelir: gönderiler forumun altında.
    let threads: Vec<digcord_model::Channel> = serde_json::from_value(serde_json::json!([
        thread_json(60, FORUM, "ilk gönderi", false, false),
        thread_json(61, FORUM, "ikinci", false, false)
    ]))
    .expect("threads");
    shared
        .state
        .write()
        .expect("kilit")
        .threads_loaded(Snowflake(FORUM), &threads, false);
    let l = labels(&app);
    assert_eq!(&l[l.len() - 3..], ["≡ forum", "└ ikinci", "└ ilk gönderi"]);
    // Kapat/aç: artık alınmış, yeni istek yok.
    assert!(press(&mut app, &[KeyCode::Enter]).is_empty());
    assert_eq!(labels(&app).len(), 6);
    assert!(press(&mut app, &[KeyCode::Enter]).is_empty());
    assert_eq!(labels(&app).len(), 8);
    // Forum gönderisi normal kanal gibi açılır.
    app.chan_sel = 6;
    let actions = press(&mut app, &[KeyCode::Enter]);
    assert!(matches!(
        actions[..],
        [Action::OpenChannel {
            channel: Snowflake(61),
            ..
        }]
    ));
}

#[test]
fn archived_command_targets_parent_channel() {
    let mut app = test_app(threads_shared());
    to_channels(&mut app);
    app.chan_sel = 1; // #sohbet
    let actions = run_command(&mut app, "archived");
    assert_eq!(
        actions,
        [Action::LoadThreads {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(SOHBET),
            archived: true
        }]
    );
    // Thread'deyken üst kanalı hedefler.
    app.open_channel(Some(Snowflake(GUILD)), Snowflake(THREAD_SOHBET));
    app.focus = Focus::Messages;
    let actions = run_command(&mut app, "archived");
    assert!(matches!(
        actions[..],
        [Action::LoadThreads {
            channel: Snowflake(SOHBET),
            archived: true,
            ..
        }]
    ));
    // Kategori / thread'siz hedef: bildirim, eylem yok.
    app.focus = Focus::Channels;
    app.open = None;
    app.chan_sel = 0;
    assert!(run_command(&mut app, "archived").is_empty());
    assert!(app.notice.as_deref().unwrap_or("").contains(":archived"));
}

#[test]
fn archived_threads_appear_dim_after_load() {
    let shared = threads_shared();
    let mut app = test_app(shared.clone());
    to_channels(&mut app);
    let old: Vec<digcord_model::Channel> =
        serde_json::from_value(serde_json::json!([thread_json(
            50,
            SOHBET,
            "çok eski",
            true,
            false
        )]))
        .expect("threads");
    shared
        .state
        .write()
        .expect("kilit")
        .threads_loaded(Snowflake(SOHBET), &old, true);
    let entries = app.entries();
    let e = entries
        .iter()
        .find(|e| e.id == Snowflake(50))
        .expect("arşivli");
    assert!(e.dim);
    // Önceden gizli olan arşivli thread de artık listede.
    assert!(entries.iter().any(|e| e.id == Snowflake(32) && e.dim));
}

#[test]
fn locked_thread_cannot_send_and_shows_notice() {
    let mut app = test_app(threads_shared());
    to_channels(&mut app);
    // "└ kilitli konu 🔒" (chan_sel 2)
    let actions = press(
        &mut app,
        &[KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Enter],
    );
    assert!(matches!(
        actions[..],
        [Action::OpenChannel {
            channel: Snowflake(THREAD_LOCKED),
            ..
        }]
    ));
    press(&mut app, &[KeyCode::Char('i')]);
    assert_eq!(app.mode, compose::Mode::Normal);
    assert_eq!(app.notice.as_deref(), Some("Bu thread kilitli; yazamazsın"));
    let text = format!("{}", draw(&app, 100, 16));
    assert!(text.contains("bu thread kilitli"), "{text}");

    // Kilitsiz thread'e yazılabilir.
    app.open_channel(Some(Snowflake(GUILD)), Snowflake(THREAD_SOHBET));
    press(&mut app, &[KeyCode::Char('i')]);
    assert_eq!(app.mode, compose::Mode::Insert);
    let mut sent = press(
        &mut app,
        &[KeyCode::Char('h'), KeyCode::Char('i'), KeyCode::Enter],
    );
    assert!(matches!(
        sent.pop(),
        Some(Action::SendMessage {
            channel: Snowflake(THREAD_SOHBET),
            ..
        })
    ));
}

#[test]
fn thread_channel_panel_snapshot() {
    let mut app = test_app(threads_shared());
    to_channels(&mut app);
    insta::assert_snapshot!(draw(&app, 60, 12));
}

// ------------------------------------------------------------ arama ve sabitlenenler

fn hit_in(channel: u64, seq: u64, minutes: u64, author: u64, content: &str) -> Message {
    let mut m = msg(seq, minutes, author, content);
    m.channel_id = Snowflake(channel);
    if channel == DM {
        m.guild_id = None;
    }
    m
}

/// #sohbet açık, mesaj penceresinde 3 mesaj; komut verilmiş, sonuçlar gelmiş.
fn search_app(hits: Vec<Message>, total: u64) -> (App, u64) {
    let msgs = vec![
        msg(1, 0, FRIEND, "pencere bir"),
        msg(2, 1, FRIEND, "pencere iki"),
        msg(3, 2, ME, "pencere üç"),
    ];
    let mut app = test_app(shared_with(msgs, true));
    open_sohbet(&mut app);
    draw(&app, 100, 16);
    let actions = run_command(&mut app, "search kahve from:200");
    let Some(Action::Search { seq, .. }) = actions.first() else {
        panic!("Action::Search bekleniyordu: {actions:?}");
    };
    let seq = *seq;
    app.handle(UiEvent::SearchResults { seq, total, hits });
    (app, seq)
}

#[test]
fn search_command_resolves_filters_into_one_action() {
    let mut app = test_app(shared_with(vec![msg(1, 0, FRIEND, "x")], true));
    open_sohbet(&mut app);
    let actions = run_command(
        &mut app,
        "search çay şeker from:<@200> in:#SOHBET has:link has:image",
    );
    let [
        Action::Search {
            guild,
            channel,
            query,
            offset,
            ..
        },
    ] = actions.as_slice()
    else {
        panic!("tek Action::Search bekleniyordu: {actions:?}");
    };
    assert_eq!(*guild, Some(Snowflake(GUILD)));
    assert_eq!(*channel, Snowflake(SOHBET));
    assert_eq!(*offset, 0);
    assert_eq!(query.content, "çay şeker");
    assert_eq!(query.author, Some(Snowflake(FRIEND)));
    assert_eq!(query.channel, Some(Snowflake(SOHBET)));
    assert_eq!(
        query.has,
        vec![digcord_model::Has::Link, digcord_model::Has::Image]
    );
    assert_eq!(app.mode, compose::Mode::Search);

    // @ad ile kullanıcı çözümü: store'daki bilinen kullanıcılar arasında.
    let mut app = test_app(shared_with(vec![msg(1, 0, FRIEND, "x")], true));
    open_sohbet(&mut app);
    let actions = run_command(&mut app, "search from:@arkadas");
    assert!(matches!(
        actions.as_slice(),
        [Action::Search { query, .. }] if query.author == Some(Snowflake(FRIEND))
    ));
}

#[test]
fn search_command_validation_emits_nothing() {
    let mut app = test_app(shared_with(vec![msg(1, 0, FRIEND, "x")], true));
    // Kanal açık değilken.
    assert!(run_command(&mut app, "search a").is_empty());
    assert_eq!(app.notice.as_deref(), Some("Önce bir kanal aç"));
    open_sohbet(&mut app);
    for (cmd, needle) in [
        ("search", "Kullanım"),
        ("search has:video a", "Bilinmeyen has"),
        ("search in:#yok a", "kanal bulunamadı"),
        ("search from:@kimse a", "kullanıcı bulunamadı"),
    ] {
        assert!(run_command(&mut app, cmd).is_empty(), "{cmd}");
        assert!(
            app.notice.as_deref().is_some_and(|n| n.contains(needle)),
            "{cmd}: {:?}",
            app.notice
        );
        assert_eq!(app.mode, compose::Mode::Normal);
    }
}

#[test]
fn search_in_dm_uses_channel_scope_and_rejects_in_filter() {
    let mut app = test_app(Arc::new(Shared::default()));
    app.shared
        .state
        .write()
        .expect("kilit")
        .load_ready(&ready());
    // DM'i aç: DM panelinde ilk satır.
    press(&mut app, &[KeyCode::Enter, KeyCode::Enter]);
    assert_eq!(app.open.map(|o| o.channel), Some(Snowflake(DM)));
    let actions = run_command(&mut app, "search merhaba");
    assert!(matches!(
        actions.as_slice(),
        [Action::Search { guild: None, channel, .. }] if *channel == Snowflake(DM)
    ));
    press(&mut app, &[KeyCode::Esc]);
    assert!(run_command(&mut app, "search in:#sohbet a").is_empty());
    assert!(app.notice.as_deref().is_some_and(|n| n.contains("in:")));
}

#[test]
fn results_navigation_paging_and_stale_responses() {
    let hits: Vec<Message> = (0..3)
        .map(|i| hit_in(SOHBET, 10 + i, 30 + i, FRIEND, &format!("sonuç {i}")))
        .collect();
    let (mut app, seq) = search_app(hits, 60);
    assert_eq!(app.search.as_ref().map(|v| v.hits.len()), Some(3));

    press(
        &mut app,
        &[KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Char('j')],
    );
    assert_eq!(
        app.search.as_ref().map(|v| v.sel),
        Some(2),
        "son satırda durur"
    );
    press(&mut app, &[KeyCode::Char('k')]);
    assert_eq!(app.search.as_ref().map(|v| v.sel), Some(1));
    press(&mut app, &[KeyCode::Char('g')]);
    assert_eq!(app.search.as_ref().map(|v| v.sel), Some(0));

    // p ilk sayfada istek yapmaz.
    assert!(press(&mut app, &[KeyCode::Char('p')]).is_empty());
    // n: offset 25, yeni seq; yükleme sürerken n/p yok sayılır.
    let actions = press(&mut app, &[KeyCode::Char('n')]);
    let [
        Action::Search {
            seq: seq2,
            offset,
            query,
            guild,
            ..
        },
    ] = actions.as_slice()
    else {
        panic!("{actions:?}");
    };
    assert_eq!(*offset, 25);
    assert_eq!(*guild, Some(Snowflake(GUILD)));
    assert_eq!(query.content, "kahve");
    assert!(*seq2 > seq);
    assert!(press(&mut app, &[KeyCode::Char('n')]).is_empty());
    // Eski isteğin geç yanıtı yok sayılır.
    app.handle(UiEvent::SearchResults {
        seq,
        total: 1,
        hits: vec![],
    });
    assert!(app.search.as_ref().is_some_and(|v| v.loading));
    app.handle(UiEvent::SearchResults {
        seq: *seq2,
        total: 60,
        hits: vec![hit_in(SOHBET, 20, 40, ME, "ikinci sayfa")],
    });
    assert!(
        app.search
            .as_ref()
            .is_some_and(|v| !v.loading && v.offset == 25)
    );
    // 25 + 25 < 60: bir sayfa daha var; 50'de son.
    let actions = press(&mut app, &[KeyCode::Char('n')]);
    assert!(matches!(
        actions.as_slice(),
        [Action::Search { offset: 50, .. }]
    ));
    let seq3 = app.search.as_ref().map(|v| v.seq()).expect("açık");
    app.handle(UiEvent::SearchResults {
        seq: seq3,
        total: 60,
        hits: vec![],
    });
    assert!(press(&mut app, &[KeyCode::Char('n')]).is_empty());
    assert_eq!(app.notice.as_deref(), Some("Son sayfa"));
    // Geri.
    let actions = press(&mut app, &[KeyCode::Char('p')]);
    assert!(matches!(
        actions.as_slice(),
        [Action::Search { offset: 25, .. }]
    ));
}

#[test]
fn failed_search_shows_error_and_esc_closes() {
    let (mut app, seq) = search_app(vec![], 0);
    app.handle(UiEvent::SearchFailed {
        seq,
        error: "Arama dizini hazırlanıyor, birazdan tekrar dene".into(),
    });
    let screen = format!("{}", draw(&app, 90, 8));
    assert!(screen.contains("dizini hazırlanıyor"), "{screen}");
    // Enter boş listede bir şey yapmaz.
    assert!(press(&mut app, &[KeyCode::Enter]).is_empty());
    assert_eq!(app.mode, compose::Mode::Search);
    assert!(press(&mut app, &[KeyCode::Esc]).is_empty());
    assert_eq!(app.mode, compose::Mode::Normal);
    assert!(app.search.is_none());
}

#[test]
fn enter_on_loaded_message_selects_it_without_network() {
    let hits = vec![hit_in(SOHBET, 2, 1, FRIEND, "pencere iki")];
    let (mut app, _) = search_app(hits, 1);
    let want = app
        .shared
        .read()
        .messages(Snowflake(SOHBET))
        .and_then(|m| m.get(1).map(|m| m.id))
        .expect("ikinci mesaj");
    assert!(press(&mut app, &[KeyCode::Enter]).is_empty());
    assert_eq!(app.mode, compose::Mode::Select);
    assert_eq!(app.selected, Some(want));
    assert!(app.pending_jump.is_none());
    assert!(app.search.is_none());
}

#[test]
fn enter_on_unloaded_message_requests_around_then_scrolls_on_store_change() {
    // Pencerede olmayan, çok daha eski bir mesaj.
    let mut old = hit_in(SOHBET, 7, 0, FRIEND, "çok eski");
    old.id = Snowflake(old.id.0 - (3_000_u64 << 22));
    let target = old.id;
    let (mut app, _) = search_app(vec![old], 1);
    let actions = press(&mut app, &[KeyCode::Enter]);
    assert_eq!(
        actions,
        [Action::JumpTo {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(SOHBET),
            message: target,
            previous: None,
        }]
    );
    assert_eq!(app.pending_jump, Some((Snowflake(SOHBET), target)));
    assert_eq!(app.mode, compose::Mode::Normal);

    // Denetleyici around sonucunu store'a yazar; TUI StoreChanged ile hizalanır.
    {
        let mut st = app.shared.state.write().expect("kilit");
        // Hedefin çevresi: iki öncesi, kendisi, iki sonrası (en yeniden en eskiye).
        let around: Vec<Message> = (0..5)
            .rev()
            .map(|i| {
                let mut m = msg(50 + i, 0, FRIEND, &format!("çevre {i}"));
                m.id = Snowflake(target.0 + (i << 22) - (2 << 22));
                m
            })
            .collect();
        assert!(around.iter().any(|m| m.id == target));
        let with_target = around;
        st.around_loaded(Snowflake(SOHBET), 50, with_target);
    }
    app.handle(UiEvent::StoreChanged);
    assert_eq!(app.selected, Some(target));
    assert_eq!(app.mode, compose::Mode::Select);
    assert!(app.pending_jump.is_none());
}

#[test]
fn jump_failed_clears_pending_jump() {
    let mut old = hit_in(SOHBET, 7, 0, FRIEND, "silinmiş");
    old.id = Snowflake(old.id.0 - (3_000_u64 << 22));
    let (mut app, _) = search_app(vec![old], 1);
    press(&mut app, &[KeyCode::Enter]);
    assert!(app.pending_jump.is_some());
    app.handle(UiEvent::JumpFailed(
        "Mesaj bulunamadı (silinmiş olabilir)".into(),
    ));
    assert!(app.pending_jump.is_none());
    assert_eq!(
        app.notice.as_deref(),
        Some("Mesaj bulunamadı (silinmiş olabilir)")
    );
}

#[test]
fn jump_to_other_channel_opens_it_and_trims_previous() {
    let hit = hit_in(DM, 5, 3, FRIEND, "dm içinde");
    let (mut app, _) = search_app(vec![hit.clone()], 1);
    let actions = press(&mut app, &[KeyCode::Enter]);
    assert_eq!(
        actions,
        [Action::JumpTo {
            guild: None,
            channel: Snowflake(DM),
            message: hit.id,
            previous: Some(Snowflake(SOHBET)),
        }]
    );
    assert_eq!(app.open.map(|o| o.channel), Some(Snowflake(DM)));
    assert_eq!(app.open.and_then(|o| o.guild), None);
}

#[test]
fn pins_command_and_overlay() {
    let mut app = test_app(shared_with(vec![msg(1, 0, FRIEND, "x")], true));
    assert!(run_command(&mut app, "pins").is_empty());
    open_sohbet(&mut app);
    let actions = run_command(&mut app, "pins");
    let [
        Action::LoadPins {
            seq,
            guild,
            channel,
        },
    ] = actions.as_slice()
    else {
        panic!("{actions:?}");
    };
    assert_eq!(
        (*guild, *channel),
        (Some(Snowflake(GUILD)), Snowflake(SOHBET))
    );
    // n/p sabitlenenlerde sayfa istemez.
    app.handle(UiEvent::SearchResults {
        seq: *seq,
        total: 2,
        hits: vec![
            hit_in(SOHBET, 1, 0, FRIEND, "sabit bir"),
            hit_in(SOHBET, 2, 1, ME, "sabit iki"),
        ],
    });
    assert!(press(&mut app, &[KeyCode::Char('n'), KeyCode::Char('p')]).is_empty());
    let screen = format!("{}", draw(&app, 90, 8));
    assert!(screen.contains("Sabitlenenler (sohbet) — 2"), "{screen}");
    assert!(screen.contains("sabit iki"), "{screen}");
    // Enter: yüklü mesaja atlar.
    assert!(press(&mut app, &[KeyCode::Enter]).is_empty());
    assert_eq!(app.mode, compose::Mode::Select);
}

#[test]
fn results_overlay_snapshot() {
    let multi = hit_in(
        SOHBET,
        3,
        5,
        FRIEND,
        "kahve\nçok   uzun bir mesaj ".repeat(8).trim(),
    );
    let hits = vec![
        hit_in(SOHBET, 1, 0, FRIEND, "kahve içelim mi bugün?"),
        hit_in(SOHBET, 2, 1, ME, "kahve makinesi bozuldu"),
        multi,
        hit_in(DM, 4, 7, FRIEND, "dm'de kahve"),
        hit_in(SOHBET, 5, 9, FRIEND, ""),
    ];
    let (mut app, _) = search_app(hits, 31);
    press(&mut app, &[KeyCode::Char('j')]);
    insta::assert_snapshot!("search_results_overlay", draw(&app, 80, 10));
}

#[test]
fn g_returns_from_detached_window_and_insert_is_blocked() {
    let mut app = test_app(shared_with(vec![msg(1, 0, FRIEND, "yeni")], true));
    open_sohbet(&mut app);
    draw(&app, 100, 16);
    // Kanalın en yenisi 'yeni'den ilerde; around ile eski bir pencere.
    {
        let mut st = app.shared.state.write().expect("kilit");
        st.message_create(msg(9, 600, FRIEND, "çok yeni"));
        let mut old = msg(5, 0, FRIEND, "eski");
        old.id = Snowflake(old.id.0 - (5_000_u64 << 22));
        st.around_loaded(Snowflake(SOHBET), 50, vec![old]);
        assert!(st.is_detached(Snowflake(SOHBET)));
    }
    // Yazmak engellenir.
    press(&mut app, &[KeyCode::Char('i')]);
    assert_eq!(app.mode, compose::Mode::Normal);
    assert!(app.notice.as_deref().is_some_and(|n| n.contains("G")));
    // G: ilk sayfayı yeniden iste.
    let actions = press(&mut app, &[KeyCode::Char('G')]);
    assert_eq!(
        actions,
        [Action::OpenChannel {
            guild: Some(Snowflake(GUILD)),
            channel: Snowflake(SOHBET),
            previous: None,
        }]
    );
    // Güncel pencerede G sadece kaydırır.
    {
        let mut st = app.shared.state.write().expect("kilit");
        st.history_loaded(
            Snowflake(SOHBET),
            None,
            10,
            vec![msg(9, 600, FRIEND, "çok yeni")],
        );
    }
    assert!(press(&mut app, &[KeyCode::Char('G')]).is_empty());
}

// ------------------------------------------------------------ kilitlenme koruması

/// Çizim ve girdi yolları `Shared::read()`'i iç içe çağırmamalı (yazar bekliyorsa kilitlenir).
/// Sunucu seçili, ses üyeleri, thread'ler ve açık kanal: `draw_channels` → `entries_in` yolu.
#[test]
fn draw_with_guild_voice_threads_and_open_channel_takes_one_read() {
    let shared = threads_shared_with(&[serde_json::json!(
        {"id": "13", "type": 2, "name": "ses", "parent_id": "10", "position": 3}
    )]);
    {
        let mut st = shared.state.write().expect("kilit");
        let vs: digcord_model::VoiceState = serde_json::from_value(serde_json::json!({
            "guild_id": GUILD.to_string(), "channel_id": "13", "user_id": FRIEND.to_string(),
            "session_id": "x"
        }))
        .expect("ses durumu");
        st.voice_state_update(&vs);
    }
    let mut app = test_app(shared);
    to_channels(&mut app);
    // #sohbet'i aç (thread'li kanal), sonra Insert'te tamamlama + durum çubuğu da çizilsin.
    press(&mut app, &[KeyCode::Char('j'), KeyCode::Enter]);
    assert!(app.open.is_some());
    draw(&app, 100, 20);
    press(&mut app, &[KeyCode::Char('i'), KeyCode::Char('@')]);
    draw(&app, 100, 20);
    press(&mut app, &[KeyCode::Esc, KeyCode::Tab]);
    // Kanal panelinden :threads / :archived ve Enter yolları.
    run_command(&mut app, "threads");
    run_command(&mut app, "archived");
    draw(&app, 100, 20);
}

#[test]
#[should_panic(expected = "iç içe Shared::read()")]
fn nested_shared_read_panics_in_debug() {
    let shared = Shared::default();
    let _outer = shared.read();
    let _inner = shared.read();
}

#[test]
fn sequential_shared_reads_are_fine() {
    let shared = Shared::default();
    drop(shared.read());
    drop(shared.read());
    let _ = shared.read().guilds().len();
    let _ = shared.read().guilds().len();
}

// ------------------------------------------------------------------ üye paneli

/// Sentetik üye listesi: rol grubu (renkli), çevrimiçi ve çevrimdışı grupları.
fn member_update() -> digcord_model::MemberListUpdate {
    let member = |id: u64, name: &str, nick: Option<&str>, roles: &[&str], status: &str| {
        serde_json::json!({"member": {
            "user": {"id": id.to_string(), "username": format!("kul{id}"), "global_name": name},
            "roles": roles, "nick": nick, "presence": {"status": status}}})
    };
    let d = serde_json::json!({
        "id": "everyone", "guild_id": GUILD.to_string(), "member_count": 5, "online_count": 3,
        "groups": [{"id": "7", "count": 1}, {"id": "online", "count": 2}, {"id": "offline", "count": 2}],
        "ops": [{"op": "SYNC", "range": [0, 99], "items": [
            {"group": {"id": "7"}},
            member(300, "Mod Kişi", Some("Takma"), &["7"], "dnd"),
            {"group": {"id": "online"}},
            member(301, "Ayşe", None, &[], "online"),
            member(302, "Boşta", None, &[], "idle"),
            {"group": {"id": "offline"}},
            member(303, "山田太郎山田太郎山田太郎山田太郎", None, &[], "offline"),
            {"member": {"user": {"id": "304", "username": "sadece_kullanici"}, "roles": []}},
        ]}]
    });
    digcord_model::MemberListUpdate::parse(&d.to_string()).expect("liste")
}

fn members_app() -> App {
    let shared = shared_with(vec![], true);
    shared
        .state
        .write()
        .expect("kilit")
        .apply_member_list(&member_update());
    let mut app = test_app(shared);
    open_sohbet(&mut app);
    app
}

#[test]
fn member_panel_snapshot() {
    let app = members_app();
    insta::assert_snapshot!(draw(&app, 100, 14));
}

#[test]
fn member_panel_colors_and_markers() {
    let app = members_app();
    let screen = draw(&app, 100, 14);
    let buf = screen.buffer();
    // Panel x=80..100; iç alan x=81. Satır 1: rol başlığı, 2: renkli üye (takma ad).
    let row = |y: u16| -> String { (80..100).map(|x| buf[(x, y)].symbol()).collect() };
    assert!(row(1).contains("MODERATÖR — 1"), "{}", row(1));
    assert!(row(2).contains("Takma"), "{}", row(2));
    assert_eq!(buf[(83, 2)].fg, Color::Rgb(255, 0, 0));
    // Rolsüz üye renksiz; işaret rengi duruma göre.
    assert_eq!(buf[(83, 4)].fg, Color::Reset);
    assert_eq!(buf[(81, 2)].fg, Color::Red);
    assert_eq!(buf[(81, 4)].fg, Color::Green);
    assert_eq!(buf[(81, 5)].fg, Color::Yellow);
    assert!(row(7).contains("○"), "{}", row(7));
    // Görünen ad: takma ad > global ad > kullanıcı adı.
    assert!(row(8).contains("sadece_kullanici") || row(9).contains("sadece_kullanici"));
}

#[test]
fn member_panel_truncates_wide_chars_to_panel_width() {
    let app = members_app();
    let screen = draw(&app, 100, 14);
    let buf = screen.buffer();
    // Sağ kenar çerçevesi (x=99) hiçbir satırda taşan içerikle bozulmamalı.
    for y in 1..12 {
        assert_eq!(buf[(99, y)].symbol(), "│", "satır {y}");
    }
    let all: String = (0..14)
        .flat_map(|y| (80..100).map(move |x| (x, y)))
        .map(|(x, y)| buf[(x, y)].symbol().to_owned())
        .collect();
    assert!(all.contains('…'), "uzun ad kısaltılmalı");
    assert!(!all.contains("山田太郎山田太郎山田太郎山田太郎"));
}

#[test]
fn member_panel_toggle_by_key_and_command() {
    let mut app = members_app();
    assert!(format!("{}", draw(&app, 100, 14)).contains("Üyeler"));
    let alt_m = || {
        UiEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Char('m'),
            KeyModifiers::ALT,
        )))
    };
    assert_eq!(app.handle(alt_m()), Some(Action::MembersVisible(false)));
    let hidden = format!("{}", draw(&app, 100, 14));
    assert!(!hidden.contains("Üyeler"));
    assert_eq!(app.handle(alt_m()), Some(Action::MembersVisible(true)));
    assert_eq!(
        run_command(&mut app, "members"),
        [Action::MembersVisible(false)]
    );
    assert_eq!(
        run_command(&mut app, "members"),
        [Action::MembersVisible(true)]
    );
    // Yeniden atanabilir.
    let (keys, warnings) = Keymap::from_overrides(&[("toggle_members".into(), vec!["f4".into()])]);
    assert!(warnings.is_empty(), "{warnings:?}");
    app.keys = keys;
    assert_eq!(
        app.handle(key(KeyCode::F(4))),
        Some(Action::MembersVisible(false))
    );
}

#[test]
fn member_panel_hidden_when_narrow_or_disabled() {
    let mut app = members_app();
    assert!(!format!("{}", draw(&app, 99, 14)).contains("Üyeler"));
    app.show_members = false;
    assert!(!format!("{}", draw(&app, 120, 14)).contains("Üyeler"));
}

#[test]
fn member_panel_shows_dm_recipients() {
    let mut app = test_app(shared_with(vec![], true));
    press(&mut app, &[KeyCode::Enter, KeyCode::Enter]);
    let screen = draw(&app, 100, 14);
    let buf = screen.buffer();
    let row = |y: u16| -> String { (80..100).map(|x| buf[(x, y)].symbol()).collect() };
    assert!(row(1).contains("Üyeler — 1"), "{}", row(1));
    assert!(row(2).contains("Arkadaş"), "{}", row(2));
    insta::assert_snapshot!(draw(&app, 100, 8));
}
