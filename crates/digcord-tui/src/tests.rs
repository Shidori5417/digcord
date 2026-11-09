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
            "roles": [{"id": GUILD.to_string(), "name": "@everyone", "permissions": "35904", "position": 0}],
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
