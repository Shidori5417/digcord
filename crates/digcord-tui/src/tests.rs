use std::sync::Arc;

use digcord_model::{Message, Ready, Snowflake};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

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
    // "gizli" @everyone'a kapalı → listede yok; ses kanalı listede ama açılamaz.
    assert_eq!(labels, ["▾ GENEL", "# sohbet", "# duyurular", "🔊 Ses"]);
    // Ses kanalında Enter bir şey yapmaz.
    assert!(
        press(
            &mut app,
            &[
                KeyCode::Char('j'),
                KeyCode::Char('j'),
                KeyCode::Char('j'),
                KeyCode::Enter
            ]
        )
        .is_empty()
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
