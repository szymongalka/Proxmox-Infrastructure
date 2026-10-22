use anyhow::{Result, bail, ensure};
use clap::Parser;
use crossterm::{
    cursor::Show,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    style::ResetColor,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, widgets::Clear};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use wgtui::{
    app::{App, View, monitor},
    profile::Settings,
    secret::protect_process,
    store::Store,
    system::RealBackend,
    ui,
};

#[derive(Parser)]
#[command(
    version,
    about = "Minimalistyczny menedżer WireGuarda · Linux · lokalnie i w sesji SSH"
)]
struct Args {
    /// Tryb demonstracyjny bez uprawnień, procesów systemowych i zapisu
    #[arg(long)]
    demo: bool,
    /// Katalog ustawień i szablonów (nie konfiguracji serwera)
    #[arg(long, default_value = "/opt/custom-tools/wgtui")]
    data_dir: PathBuf,
    /// Interfejs wybrany po uruchomieniu
    #[arg(long)]
    interface: Option<String>,
}

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            event::DisableBracketedPaste,
            Show,
            ResetColor,
            LeaveAlternateScreen
        );
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("wgtui: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args = Args::parse();
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "Wymagany terminal interaktywny; przez SSH użyj sesji z TTY"
    );
    protect_process()?;
    let store = if args.demo {
        None
    } else {
        if !cfg!(target_os = "linux") {
            bail!("Zarządzanie WireGuardem wymaga Linuksa. Na tym systemie użyj --demo");
        }
        ensure!(
            unsafe { libc::geteuid() } == 0,
            "Uruchom sudo wgtui lub wybierz --demo"
        );
        Some(Store::open(
            args.data_dir,
            PathBuf::from("/etc/wireguard"),
            Arc::new(RealBackend),
        )?)
    };
    let settings = if let Some(store) = &store {
        store.settings()?
    } else {
        Settings::default()
    };
    let rx = store.clone().map(monitor);
    let mut app = App::new(store, settings);
    app.requested_interface = args.interface;
    enable_raw_mode()?;
    let _guard = TerminalGuard;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        event::EnableBracketedPaste
    )?;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            event::DisableBracketedPaste,
            Show,
            ResetColor,
            LeaveAlternateScreen
        );
        previous(info);
    }));
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut last_demo = Instant::now();
    loop {
        if let Some(rx) = &rx {
            while let Ok(result) = rx.try_recv() {
                match result {
                    Ok(data) => app.update(data),
                    Err(e) => app.monitor_error = Some(e.to_string()),
                }
            }
        }
        app.poll_completion();
        if last_demo.elapsed() >= Duration::from_secs(1) {
            app.tick_demo();
            last_demo = Instant::now();
        }
        if app.clear_secret_screen {
            // Overwrite both retained Ratatui buffers as well as the terminal image.
            terminal.draw(|f| f.render_widget(Clear, f.area()))?;
            terminal.draw(|f| f.render_widget(Clear, f.area()))?;
            terminal.clear()?;
            app.clear_secret_screen = false;
        }
        terminal.draw(|frame| ui::draw(frame, &app))?;
        if app.quit {
            break;
        }
        if event::poll(Duration::from_millis(80))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        match app.view {
                            View::Busy => app.note = "Poczekaj na zakończenie zapisu".into(),
                            View::Delivery(_) | View::CloseDelivery(_) => {
                                app.key(event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
                            }
                            _ => app.quit = true,
                        }
                    } else {
                        app.key(key);
                    }
                }
                Event::Paste(text) => app.paste(&text),
                _ => {}
            }
        }
    }
    Ok(())
}
