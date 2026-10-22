//! Render the actual Ratatui demo buffer: preview [main|peer|tunnel|review|qr|error] [width] [height].
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Terminal,
    backend::TestBackend,
    style::{Color, Modifier},
};
use wgtui::{app::App, profile::Settings, ui};
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn color(c: Color, background: bool) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Black => "#000000".into(),
        Color::White => "#ffffff".into(),
        _ => if background { "#090a09" } else { "#c6c6c6" }.into(),
    }
}
fn press(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let screen = args.first().map(String::as_str).unwrap_or("main");
    let w: u16 = args
        .get(1)
        .map(|s| s.parse().expect("width"))
        .unwrap_or(120);
    let h: u16 = args
        .get(2)
        .map(|s| s.parse().expect("height"))
        .unwrap_or(if screen == "qr" { 55 } else { 34 });
    let mut app = App::new(None, Settings::default());
    match screen {
        "main" => {}
        "tunnel" => press(&mut app, KeyCode::Char('n')),
        "peer" | "error" | "review" | "qr" => {
            press(&mut app, KeyCode::Char('a'));
            if screen != "error" {
                app.paste("tablet-01");
            }
            if matches!(screen, "error" | "review" | "qr") {
                press(&mut app, KeyCode::F(10));
            }
            if screen == "qr" {
                press(&mut app, KeyCode::Enter);
            }
        }
        _ => panic!("unknown preview screen"),
    }
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    println!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\"><rect width=\"100%\" height=\"100%\" fill=\"#090a09\"/>",
        w as u32 * 10 + 40,
        h as u32 * 22 + 40
    );
    for y in 0..h {
        for x in 0..w {
            let cell = &buffer[(x, y)];
            let bg = color(cell.bg, true);
            if bg != "#090a09" {
                println!(
                    "<rect x=\"{}\" y=\"{}\" width=\"10\" height=\"22\" fill=\"{bg}\"/>",
                    20 + x as u32 * 10,
                    20 + y as u32 * 22
                );
            }
        }
    }
    println!("<g font-family=\"JetBrains Mono,monospace\" font-size=\"16\">");
    for y in 0..h {
        for x in 0..w {
            let cell = &buffer[(x, y)];
            if cell.symbol() == " " {
                continue;
            }
            let px = 20 + x as u32 * 10;
            let py = 20 + y as u32 * 22;
            let fg = color(cell.fg, false);
            if matches!(cell.symbol(), "█" | "▀" | "▄") {
                let (offset, height) = match cell.symbol() {
                    "▀" => (0, 11),
                    "▄" => (11, 11),
                    _ => (0, 22),
                };
                println!(
                    "<rect x=\"{px}\" y=\"{}\" width=\"10\" height=\"{height}\" fill=\"{fg}\"/>",
                    py + offset
                );
                continue;
            }
            println!(
                "<text x=\"{px}\" y=\"{}\" fill=\"{fg}\" font-weight=\"{}\" {}>{}</text>",
                py + 16,
                if cell.modifier.contains(Modifier::BOLD) {
                    600
                } else {
                    400
                },
                if cell.modifier.contains(Modifier::UNDERLINED) {
                    "text-decoration=\"underline\""
                } else {
                    ""
                },
                escape(cell.symbol())
            );
        }
    }
    println!("</g></svg>");
}
