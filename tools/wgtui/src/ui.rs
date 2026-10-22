use crate::{
    app::{App, Delivery, FieldKind, Form, View, now},
    config::{Peer, join_ips},
    system::PeerStats,
    theme::{self, AMBER, BG, BORDER, BORDER_STRONG, ELEVATED, ERROR, MUTED, PRIMARY, STRONG},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, Padding, Paragraph, Row, Table, TableState, Wrap,
    },
};
use unicode_width::UnicodeWidthStr;

fn accent() -> Style {
    Style::default().fg(AMBER)
}
fn muted() -> Style {
    Style::default().fg(MUTED)
}
fn bold() -> Style {
    Style::default().fg(STRONG).add_modifier(Modifier::BOLD)
}

fn card<'a>() -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER_STRONG))
        .style(theme::card())
}

fn shortcuts() -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (key, label)) in [
        ("a", "dodaj"),
        ("e", "edytuj"),
        ("Spacja", "stan"),
        ("d", "usuń"),
        ("/", "szukaj"),
        ("?", "pomoc"),
        ("q", "wyjdź"),
    ]
    .into_iter()
    .enumerate()
    {
        spans.push(Span::styled(
            format!(" {key} "),
            if index == 0 {
                theme::button()
            } else {
                Style::default().fg(PRIMARY).bg(ELEVATED)
            },
        ));
        spans.push(Span::styled(format!(" {label} "), muted()));
    }
    Line::from(spans)
}

pub fn bytes(value: u64) -> String {
    if value < 1024 {
        format!("{value} B")
    } else if value < 1024 * 1024 {
        format!("{:.1} KiB", value as f64 / 1024.)
    } else if value < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", value as f64 / 1048576.)
    } else {
        format!("{:.2} GiB", value as f64 / 1073741824.)
    }
}
pub fn age(handshake: u64) -> String {
    if handshake == 0 {
        "nigdy".into()
    } else if handshake > now() {
        "czas w przyszłości".into()
    } else {
        let delta = now() - handshake;
        if delta < 60 {
            format!("{delta} s")
        } else if delta < 3600 {
            format!("{} min", delta / 60)
        } else if delta < 86400 {
            format!("{} h", delta / 3600)
        } else {
            format!("{} d", delta / 86400)
        }
    }
}
pub fn peer_status(peer: &Peer, stat: Option<&PeerStats>, active: u64) -> (&'static str, Color) {
    if !peer.enabled {
        ("wyłączony", MUTED)
    } else {
        match stat {
            Some(s) if s.handshake > 0 && s.handshake <= now() && now() - s.handshake <= active => {
                ("aktywny ostatnio", AMBER)
            }
            Some(s) if s.handshake > 0 => ("brak niedawnej aktywności", MUTED),
            _ => ("nigdy nie nawiązał handshake’u", MUTED),
        }
    }
}

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.render_widget(Block::default().style(theme::base()), area);
    if area.width < 80 || area.height < 24 {
        frame.render_widget(Paragraph::new("wgtui\n\nZwiększ terminal do co najmniej 80 × 24.\nStan i formularze pozostają zachowane.").style(accent()),area);
        return;
    }
    let content = Rect::new(area.x + 2, area.y + 1, area.width - 4, area.height - 2);
    match &app.view {
        View::Main | View::Search => main_screen(frame, app, content),
        View::Form(form) => form_screen(frame, form, content),
        View::Review(p) => text_screen(
            frame,
            content,
            "Podgląd zmiany",
            &p.preview,
            p.scroll,
            "Enter zastosuj   ↑/↓ przewijaj   Esc anuluj",
            Some(&app.note),
        ),
        View::Delivery(d) => delivery_screen(frame, d, content, app.store.is_none()),
        View::CloseDelivery(_) => text_screen(
            frame,
            content,
            "Zamknąć jednorazowy profil?",
            "Upewnij się, że profil został zaimportowany na urządzeniu.\n\nKlucz prywatny klienta nie został zapisany. Po zamknięciu\nnie będzie można ponownie wyświetlić tego QR.\n\nPeer pozostanie skonfigurowany na serwerze. Jeśli profil\nnie został odebrany, usuń peera i dodaj go ponownie.",
            0,
            "Enter profil odebrany, zamknij   Esc wróć do QR",
            None,
        ),
        View::Busy => text_screen(
            frame,
            content,
            "Stosowanie zmiany…",
            "Zapis i weryfikacja konfiguracji w toku.\n\nNie zamykaj aplikacji. Monitoring działa w tle.",
            0,
            "Trwa operacja",
            None,
        ),
        View::Help => text_screen(
            frame,
            content,
            "Klawiatura i obsługa",
            HELP,
            0,
            "Dowolny klawisz wraca",
            None,
        ),
        View::Details => {
            let parts = Layout::vertical([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(2),
            ])
            .split(content);
            heading(frame, parts[0], "Szczegóły peera", "");
            details(frame, app, parts[1]);
            frame.render_widget(
                Paragraph::new("Dowolny klawisz wraca").style(muted()),
                parts[2],
            );
        }
        View::Message(text) => text_screen(
            frame,
            content,
            "Informacja",
            text,
            0,
            "Dowolny klawisz wraca",
            None,
        ),
    }
}

fn heading(frame: &mut Frame, area: Rect, title: &str, tag: &str) {
    let line = Line::from(vec![
        Span::styled("wgtui", bold()),
        Span::styled("_", accent().add_modifier(Modifier::BOLD)),
        Span::styled("  /  ", muted()),
        Span::styled(title, bold()),
        Span::styled(format!("   {tag}"), muted()),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn main_screen(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Min(8),
        Constraint::Length(2),
        Constraint::Length(2),
    ])
    .split(area);
    heading(
        frame,
        chunks[0],
        "WireGuard",
        if app.store.is_none() {
            "DEMO"
        } else {
            "LOCAL · LIVE"
        },
    );
    let mut tabs = vec![Span::styled(
        if app.focus_interfaces { "› " } else { "  " },
        accent(),
    )];
    let start = app.interface.saturating_sub(1);
    for (index, iface) in app.data.interfaces.iter().enumerate().skip(start).take(3) {
        tabs.push(Span::styled(
            format!(" {} ", if iface.stats.is_some() { "●" } else { "○" }),
            if iface.stats.is_some() {
                accent()
            } else {
                muted()
            },
        ));
        tabs.push(Span::styled(
            format!("{}  ", iface.name),
            if index == app.interface {
                Style::default()
                    .fg(PRIMARY)
                    .bg(ELEVATED)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                muted()
            },
        ));
    }
    if app.data.interfaces.is_empty() {
        tabs.push(Span::styled("Brak tuneli · n tworzy pierwszy", muted()));
    }
    frame.render_widget(Paragraph::new(Line::from(tabs)), chunks[1]);
    if let Some(iface) = app.iface() {
        let active = iface
            .peers
            .iter()
            .filter(|p| {
                peer_status(
                    p,
                    iface
                        .stats
                        .as_ref()
                        .and_then(|s| s.peers.get(&p.public_key)),
                    app.settings.active_seconds,
                )
                .1 == AMBER
            })
            .count();
        let port = iface
            .stats
            .as_ref()
            .map(|s| s.listen_port.to_string())
            .unwrap_or_else(|| "—".into());
        let text = format!(
            "{} ostatnio aktywnych  /  {} peerów    UDP {}    autostart {}{}",
            active,
            iface.peers.len(),
            port,
            if iface.autostart { "tak" } else { "nie" },
            if iface.error.is_some() {
                "    tylko podgląd"
            } else {
                ""
            }
        );
        frame.render_widget(Paragraph::new(text).style(muted()), chunks[2]);
    }
    let wide = frame.area().width >= 110;
    let panels = if wide {
        Layout::horizontal([Constraint::Percentage(64), Constraint::Percentage(36)])
            .spacing(2)
            .split(chunks[3])
    } else {
        Layout::horizontal([Constraint::Percentage(100)]).split(chunks[3])
    };
    let peers = app.filtered();
    let rows = peers.iter().map(|peer| {
        let stat = app
            .iface()
            .and_then(|i| i.stats.as_ref())
            .and_then(|s| s.peers.get(&peer.public_key));
        let (status, color) = peer_status(peer, stat, app.settings.active_seconds);
        let name = Cell::from(Text::from(vec![
            Line::styled(peer.label(), if peer.enabled { bold() } else { muted() }),
            Line::styled(
                format!(
                    "{} · {}",
                    if !peer.enabled {
                        "○"
                    } else if color == AMBER {
                        "●"
                    } else {
                        "·"
                    },
                    stat.map(|s| age(s.handshake))
                        .unwrap_or_else(|| if peer.enabled { "nigdy" } else { "wyłączony" }.into())
                ),
                Style::default().fg(color),
            ),
        ]));
        let ip = Cell::from(Text::from(
            peer.allowed_ips
                .iter()
                .map(|n| Line::from(n.addr().to_string()))
                .collect::<Vec<_>>(),
        ));
        let (rx, tx) = stat.map(|s| (s.rx, s.tx)).unwrap_or_default();
        let rates = app
            .iface()
            .and_then(|i| app.rates.get(&(i.name.clone(), peer.public_key.clone())))
            .copied()
            .unwrap_or_default();
        let transfer = Cell::from(Text::from(vec![
            Line::from(format!("↓ {}   ↑ {}", bytes(rx), bytes(tx))),
            Line::styled(
                format!("↓ {}/s  ↑ {}/s", bytes(rates.0), bytes(rates.1)),
                muted(),
            ),
        ]));
        let _ = status;
        Row::new(vec![name, ip, transfer]).height(3)
    });
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(32),
            Constraint::Percentage(25),
            Constraint::Percentage(43),
        ],
    )
    .block(
        card()
            .title(Line::styled(" PEERY ", muted()))
            .padding(Padding::horizontal(1)),
    )
    .header(
        Row::new(["PEER / HANDSHAKE", "ADRES", "TRANSFER / PRĘDKOŚĆ"])
            .style(muted())
            .height(1),
    )
    .row_highlight_style(Style::default().bg(ELEVATED))
    .highlight_symbol("› ");
    let mut state = TableState::default().with_selected(if peers.is_empty() {
        None
    } else {
        Some(app.selected)
    });
    frame.render_stateful_widget(table, panels[0], &mut state);
    if peers.is_empty() {
        frame.render_widget(
            Paragraph::new(if app.query.is_empty() {
                "\n\nJeszcze bez peerów. Naciśnij a, aby dodać urządzenie."
            } else {
                "\n\nBrak wyników. Esc usuwa filtr."
            })
            .style(muted()),
            card().inner(panels[0]),
        );
    }
    if wide {
        details(frame, app, panels[1]);
    }
    let note = if matches!(app.view, View::Search) {
        format!("Szukaj: {}▏", app.query)
    } else if let Some(e) = &app.monitor_error {
        format!("Odczyt niedostępny: {e}")
    } else if !app.query.is_empty() {
        format!(
            "Filtr: {}  ·  {} wyników  ·  Esc wyczyść",
            app.query,
            peers.len()
        )
    } else {
        app.note.clone()
    };
    frame.render_widget(
        Paragraph::new(note)
            .style(if app.monitor_error.is_some() {
                Style::default().fg(ERROR)
            } else {
                muted()
            })
            .wrap(Wrap { trim: false }),
        chunks[4],
    );
    frame.render_widget(Paragraph::new(vec![shortcuts(),Line::styled("n tunel   p profil   s start/stop   b autostart   [ / ] interfejs   Enter szczegóły",muted())]),chunks[5]);
}

fn details(frame: &mut Frame, app: &App, area: Rect) {
    let block = card()
        .title(Line::styled(" SZCZEGÓŁY ", muted()))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let area = Rect::new(inner.x, inner.y, inner.width, inner.height);
    let mut lines = Vec::new();
    if let Some(peer) = app.peer() {
        let stat = app
            .iface()
            .and_then(|i| i.stats.as_ref())
            .and_then(|s| s.peers.get(&peer.public_key));
        let (status, color) = peer_status(peer, stat, app.settings.active_seconds);
        lines.extend([
            Line::styled(peer.label(), bold()),
            Line::styled(status, Style::default().fg(color)),
            Line::from(""),
            Line::styled("OSTATNI HANDSHAKE", muted()),
            Line::from(
                stat.map(|s| age(s.handshake))
                    .unwrap_or_else(|| "nigdy".into()),
            ),
            Line::from(""),
            Line::styled("ENDPOINT", muted()),
            Line::from(stat.map(|s| s.endpoint.as_str()).unwrap_or("—")),
            Line::from(""),
            Line::styled("ALLOWED IPS", muted()),
            Line::from(join_ips(&peer.allowed_ips)),
            Line::from(""),
            Line::styled("KLUCZ PUBLICZNY", muted()),
        ]);
        let width = area.width.max(1) as usize;
        for chunk in peer.public_key.as_bytes().chunks(width) {
            lines.push(Line::from(String::from_utf8_lossy(chunk).into_owned()));
        }
        lines.push(Line::from(""));
        lines.push(Line::styled(
            format!("Keepalive serwera: {} s", peer.keepalive),
            muted(),
        ));
    } else {
        lines.push(Line::styled("Wybierz peera", muted()));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn form_screen(frame: &mut Frame, form: &Form, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(4),
        Constraint::Length(2),
    ])
    .split(area);
    heading(frame, chunks[0], form.title(), "");
    let panel = card()
        .title(Line::styled(" PARAMETRY ", muted()))
        .padding(Padding::horizontal(1));
    let body = panel.inner(chunks[1]);
    frame.render_widget(panel, chunks[1]);
    let visible = (body.height / 3).max(1) as usize;
    let start = if form.focus >= visible {
        form.focus - visible + 1
    } else {
        0
    };
    for (index, field) in form.fields.iter().enumerate().skip(start).take(visible) {
        let y = body.y + ((index - start) * 3) as u16;
        let row = Rect::new(body.x, y, body.width, 3);
        let parts = Layout::horizontal([Constraint::Length(23), Constraint::Min(1)]).split(row);
        frame.render_widget(
            Paragraph::new(format!(
                "{}{}",
                if index == form.focus { "› " } else { "  " },
                field.label
            ))
            .style(if index == form.focus {
                accent()
            } else {
                muted()
            }),
            Rect::new(parts[0].x, parts[0].y + 1, parts[0].width, 1),
        );
        let focused = index == form.focus;
        let input = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(if focused {
                PRIMARY
            } else if field.kind == FieldKind::ReadOnly {
                BORDER
            } else {
                BORDER_STRONG
            }))
            .style(
                Style::default()
                    .bg(BG)
                    .fg(if field.kind == FieldKind::ReadOnly {
                        MUTED
                    } else {
                        STRONG
                    }),
            )
            .padding(Padding::horizontal(1));
        let input_area = input.inner(parts[1]);
        frame.render_widget(input, parts[1]);
        let (display, cursor) = visible_value(
            &field.value,
            if focused { field.cursor } else { 0 },
            input_area.width.saturating_sub(1) as usize,
        );
        let val = if field.kind == FieldKind::Bool {
            format!(
                "[{}] {}",
                if field.is_true() { "×" } else { " " },
                field.value
            )
        } else {
            display
        };
        frame.render_widget(Paragraph::new(val), input_area);
        if focused && field.kind == FieldKind::Text {
            frame.set_cursor_position((input_area.x + cursor as u16, input_area.y));
        }
    }
    let hint = if let Some(error) = &form.error {
        format!("Popraw formularz: {error}")
    } else {
        format!(
            "{}\nPole {} z {}",
            form.fields[form.focus].hint,
            form.focus + 1,
            form.fields.len()
        )
    };
    frame.render_widget(
        Paragraph::new(hint)
            .style(if form.error.is_some() {
                Style::default().fg(ERROR)
            } else {
                muted()
            })
            .wrap(Wrap { trim: false }),
        chunks[2],
    );
    frame.render_widget(
        Paragraph::new("Tab / ↑↓ pole   F10 / Ctrl+S podgląd   F2 tryb klucza   Esc anuluj")
            .style(Style::default().fg(PRIMARY)),
        chunks[3],
    );
}

fn visible_value(value: &str, cursor: usize, width: usize) -> (String, usize) {
    let cursor = cursor.min(value.len());
    let mut start = 0;
    while UnicodeWidthStr::width(&value[start..cursor]) >= width.max(1) && start < cursor {
        start += value[start..].chars().next().unwrap().len_utf8();
    }
    let mut output = String::new();
    for c in value[start..].chars() {
        if UnicodeWidthStr::width(output.as_str())
            + unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
            > width
        {
            break;
        }
        output.push(c);
    }
    (output, UnicodeWidthStr::width(&value[start..cursor]))
}

fn text_screen(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    text: &str,
    scroll: u16,
    footer: &str,
    note: Option<&str>,
) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(3),
        Constraint::Length(2),
    ])
    .split(area);
    heading(frame, chunks[0], title, "");
    let panel = card().padding(Padding::horizontal(1));
    let body = panel.inner(chunks[1]);
    frame.render_widget(panel, chunks[1]);
    frame.render_widget(
        Paragraph::new(text)
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false }),
        body,
    );
    if let Some(note) = note {
        frame.render_widget(
            Paragraph::new(note)
                .style(muted())
                .wrap(Wrap { trim: false }),
            chunks[2],
        );
    }
    frame.render_widget(
        Paragraph::new(footer).style(Style::default().fg(PRIMARY)),
        chunks[3],
    );
}

fn delivery_screen(frame: &mut Frame, d: &Delivery, area: Rect, demo: bool) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(2),
        Constraint::Length(3),
        Constraint::Length(2),
    ])
    .split(area);
    heading(
        frame,
        chunks[0],
        if d.qr.is_some() {
            "Jednorazowy profil klienta"
        } else {
            "Dane do konfiguracji klienta"
        },
        if demo { "DEMO" } else { "" },
    );
    let panel = card().padding(Padding::horizontal(1));
    let body = panel.inner(chunks[1]);
    frame.render_widget(panel, chunks[1]);
    if d.show_text || d.qr.is_none() {
        frame.render_widget(
            Paragraph::new(d.content.as_str())
                .scroll((d.scroll, 0))
                .wrap(Wrap { trim: false }),
            body,
        );
    } else if let Some(qr) = &d.qr {
        let (width, height) = qr.terminal_size();
        if body.width < width || body.height < height {
            let minimum_width = frame
                .area()
                .width
                .saturating_sub(body.width)
                .saturating_add(width)
                .max(80);
            let minimum_height = frame
                .area()
                .height
                .saturating_sub(body.height)
                .saturating_add(height)
                .max(24);
            frame.render_widget(Paragraph::new(format!("QR nie mieści się w tym oknie.\n\nZwiększ terminal do co najmniej {minimum_width} × {minimum_height}.\nNaciśnij t, aby zobaczyć tekst profilu.")).style(muted()),body);
        } else {
            let qr_area = Rect::new(body.x + (body.width - width) / 2, body.y, width, height);
            frame.render_widget(
                Block::default().style(Style::default().bg(Color::White).fg(Color::Black)),
                qr_area,
            );
            for row in 0..height as usize {
                for col in 0..width as usize {
                    let module = |y: usize| col >= 4 && y >= 4 && qr.dark(col - 4, y - 4);
                    let top = module(row * 2);
                    let bottom = module(row * 2 + 1);
                    let symbol = match (top, bottom) {
                        (true, true) => "█",
                        (true, false) => "▀",
                        (false, true) => "▄",
                        _ => " ",
                    };
                    if let Some(cell) = frame
                        .buffer_mut()
                        .cell_mut((qr_area.x + col as u16, qr_area.y + row as u16))
                    {
                        cell.set_symbol(symbol)
                            .set_fg(Color::Black)
                            .set_bg(Color::White);
                    }
                }
            }
        }
    }
    let note = if d.qr.is_some() {
        "Klucz klienta istnieje tylko w pamięci. Zaimportuj profil przed zamknięciem.\nQR nie będzie dostępny ponownie. Nic nie jest kopiowane do schowka."
    } else {
        "Klucz prywatny utwórz i pozostaw na urządzeniu klienta.\nWpis [Peer] serwera został zastosowany po zatwierdzeniu."
    };
    frame.render_widget(Paragraph::new(note).style(muted()), chunks[2]);
    frame.render_widget(
        Paragraph::new("t QR / tekst   ↑↓ przewijaj tekst   Esc zakończ")
            .style(Style::default().fg(PRIMARY)),
        chunks[3],
    );
}

const HELP: &str = "↑/↓ lub j/k   wybór peera           [ / ] lub ←/→  zmiana interfejsu\nTab           przełącz fokus        Enter           szczegóły\na             dodaj peera           e               edytuj nazwę/adres\nSpacja        włącz / wyłącz        d               usuń peera\nn             nowy tunel            p               profil klientów\ns             uruchom / zatrzymaj   b               autostart systemd\n/             wyszukaj              Esc             wróć / usuń filtr\nq             wyjdź\n\nFORMULARZE   Tab / Shift+Tab wybór pola · F10 lub Ctrl+S podgląd\n             F2 zmienia QR / klucz publiczny · Ctrl+U czyści pole\n\nAktywność jest oceną na podstawie handshake’u, nie testem dostępności.\nWireGuard milczy przy braku ruchu; stary handshake nie dowodzi awarii.\n\nQR i klucz klienta są jednorazowe. Po zamknięciu profilu można\nodzyskać dostęp wyłącznie przez dodanie nowego klucza klienta.\n\nV1 nie zarządza NAT, forwardingiem ani zaporą. Konfiguracje innych\nmenedżerów i interfejsy poza wgXXadminXX są tylko do podglądu.";
