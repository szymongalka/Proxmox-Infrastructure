use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use wgtui::{
    app::{App, View},
    profile::Settings,
    ui,
};

fn press(app: &mut App, key: KeyCode) {
    app.key(KeyEvent::new(key, KeyModifiers::NONE));
}
fn render(app: &App, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| ui::draw(f, app)).unwrap();
    let buf = terminal.backend().buffer();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn layouts_resize_and_details_are_accessible_at_80_columns() {
    let mut app = App::new(None, Settings::default());
    for (w, h) in [(80, 24), (110, 30), (160, 40)] {
        let screen = render(&app, w, h);
        assert!(screen.contains("macbook-01"));
        assert!(screen.contains("telefon-01"));
        assert!(screen.contains("wgtui"));
    }
    assert!(!render(&app, 80, 24).contains("KLUCZ PUBLICZNY"));
    press(&mut app, KeyCode::Enter);
    assert!(render(&app, 80, 30).contains("KLUCZ PUBLICZNY"));
    assert!(render(&app, 60, 12).contains("Zwiększ terminal"));
}
#[test]
fn search_navigation_and_global_shortcuts_do_not_steal_form_input() {
    let mut app = App::new(None, Settings::default());
    press(&mut app, KeyCode::Down);
    assert_eq!(app.peer().unwrap().name, "telefon-01");
    press(&mut app, KeyCode::Char('/'));
    for c in "macbook".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    assert_eq!(app.filtered().len(), 1);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.filtered().len(), 3);
    press(&mut app, KeyCode::Char('a'));
    for c in "qjednak".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    assert!(!app.quit);
    if let View::Form(f) = &app.view {
        assert_eq!(f.fields[0].value, "qjednak");
    } else {
        panic!("form closed");
    }
    app.paste("\nPrivateKey = bad");
    if let View::Form(f) = &app.view {
        assert!(f.error.is_some());
    }
}
#[test]
fn qr_onboarding_has_review_and_requires_confirmation_to_discard_secret() {
    let mut app = App::new(None, Settings::default());
    press(&mut app, KeyCode::Char('a'));
    app.paste("tablet-01");
    press(&mut app, KeyCode::F(10));
    assert!(matches!(app.view, View::Review(_)));
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.view, View::Delivery(_)));
    let small = render(&app, 80, 24);
    assert!(small.contains("QR nie mieści"));
    assert!(!small.contains('█'));
    let large = render(&app, 120, 55);
    assert!(large.contains('█') || large.contains('▀') || large.contains('▄'));
    press(&mut app, KeyCode::Esc);
    assert!(matches!(app.view, View::CloseDelivery(_)));
    press(&mut app, KeyCode::Esc);
    assert!(matches!(app.view, View::Delivery(_)));
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.view, View::Main));
    assert!(app.clear_secret_screen);
}
#[test]
fn public_key_mode_returns_a_client_template_without_private_key_or_qr() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut app = App::new(None, Settings::default());
    press(&mut app, KeyCode::Char('a'));
    app.paste("public-01");
    press(&mut app, KeyCode::F(2));
    press(&mut app, KeyCode::Tab);
    app.paste(&STANDARD.encode([90u8; 32]));
    press(&mut app, KeyCode::F(10));
    press(&mut app, KeyCode::Enter);
    if let View::Delivery(d) = &app.view {
        assert!(d.qr.is_none());
        assert!(d.content.contains("<WKLEJ_KLUCZ_PRYWATNY_NA_URZADZENIU>"));
    } else {
        panic!("no public-key delivery: {}", app.note);
    }
}
