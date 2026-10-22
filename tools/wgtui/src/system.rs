use crate::{
    config::{parse_ips, valid_runtime_name},
    secret::{Secret, secret},
};
use anyhow::{Result, anyhow, bail, ensure};
use ipnet::IpNet;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[derive(Clone, Debug, Default)]
pub struct PeerStats {
    pub endpoint: String,
    pub allowed_ips: Vec<IpNet>,
    pub handshake: u64,
    pub rx: u64,
    pub tx: u64,
    pub keepalive: u16,
}

#[derive(Clone, Debug, Default)]
pub struct InterfaceStats {
    pub public_key: String,
    pub listen_port: u16,
    pub peers: BTreeMap<String, PeerStats>,
}

pub type Observation = BTreeMap<String, InterfaceStats>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Stop,
    Enable,
    Disable,
}

pub trait Backend: Send + Sync {
    fn observe(&self) -> Result<Observation>;
    fn runtime_config(&self, interface: &str) -> Result<Secret>;
    fn sync(&self, interface: &str, config: &str) -> Result<()>;
    fn service(&self, interface: &str, action: ServiceAction) -> Result<()>;
    fn autostart(&self, interface: &str) -> Result<bool>;
    fn keypair(&self) -> Result<(Secret, String)>;
    fn public_key(&self, private: &str) -> Result<String>;
}

#[derive(Default)]
pub struct RealBackend;

/// Commands are invoked directly. No shell, key arguments or raw stderr in errors.
pub fn run(program: &str, args: &[&str], input: Option<&str>) -> Result<Secret> {
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .env("WG_HIDE_KEYS", "always")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| anyhow!("Nie można uruchomić {program}; sprawdź instalację i uprawnienia"))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let reader = thread::spawn(move || {
        let mut data = Zeroizing::new(Vec::new());
        let result = stdout.take(4 * 1024 * 1024 + 1).read_to_end(&mut data);
        (result, data)
    });
    if let Some(text) = input {
        let mut stdin = child.stdin.take().expect("piped stdin");
        // Configurations are bounded by Store, comfortably below the pipe limit.
        // Write on a worker so an unresponsive child cannot freeze the TUI.
        let bytes = Zeroizing::new(text.as_bytes().to_vec());
        thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        });
    }
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < Duration::from_secs(10) => {
                thread::sleep(Duration::from_millis(15))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                bail!(
                    "Przekroczony czas lub błąd polecenia {program}; szczegóły mogą zawierać sekrety i nie są wyświetlane"
                );
            }
        }
    };
    let (read_result, bytes) = reader
        .join()
        .map_err(|_| anyhow!("Błąd odczytu wyniku polecenia"))?;
    ensure!(
        status.success(),
        "Polecenie {program} nie powiodło się (kod {}). Szczegóły ukryto, aby chronić klucze",
        status.code().unwrap_or(-1)
    );
    read_result.map_err(|_| anyhow!("Błąd odczytu wyniku polecenia"))?;
    ensure!(
        bytes.len() <= 4 * 1024 * 1024,
        "Wynik polecenia jest zbyt duży"
    );
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| anyhow!("Wynik polecenia nie jest tekstem UTF-8"))?;
    Ok(secret(text))
}

fn wg(args: &[&str], input: Option<&str>) -> Result<Secret> {
    #[cfg(target_os = "linux")]
    let program = "/usr/bin/wg";
    #[cfg(not(target_os = "linux"))]
    let program = "wg";
    run(program, args, input)
}

impl Backend for RealBackend {
    fn observe(&self) -> Result<Observation> {
        let mut result = Observation::new();
        for name in wg(&["show", "interfaces"], None)?.split_whitespace() {
            ensure!(
                valid_runtime_name(name),
                "Nieprawidłowa nazwa interfejsu otrzymana z wg"
            );
            result.insert(name.to_string(), InterfaceStats::default());
        }
        if result.is_empty() {
            return Ok(result);
        }
        for field in [
            "public-key",
            "listen-port",
            "endpoints",
            "allowed-ips",
            "latest-handshakes",
            "transfer",
            "persistent-keepalive",
        ] {
            let output = wg(&["show", "all", field], None)?;
            for line in output.lines() {
                let parts: Vec<_> = line.split_whitespace().collect();
                if parts.len() < 2 {
                    continue;
                }
                let Some(iface) = result.get_mut(parts[0]) else {
                    continue;
                };
                match field {
                    "public-key" => iface.public_key = parts[1].to_string(),
                    "listen-port" => iface.listen_port = parts[1].parse()?,
                    _ if parts.len() >= 3 => {
                        let peer = iface.peers.entry(parts[1].to_string()).or_default();
                        match field {
                            "endpoints" => peer.endpoint = parts[2].to_string(),
                            "allowed-ips" => {
                                peer.allowed_ips = if parts[2] == "(none)" {
                                    vec![]
                                } else {
                                    parse_ips(&parts[2..].join(""))?
                                }
                            }
                            "latest-handshakes" => peer.handshake = parts[2].parse()?,
                            "transfer" if parts.len() == 4 => {
                                peer.rx = parts[2].parse()?;
                                peer.tx = parts[3].parse()?;
                            }
                            "persistent-keepalive" => {
                                peer.keepalive = if parts[2] == "off" {
                                    0
                                } else {
                                    parts[2].parse()?
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(result)
    }
    fn runtime_config(&self, interface: &str) -> Result<Secret> {
        wg(&["showconf", interface], None)
    }
    fn sync(&self, interface: &str, config: &str) -> Result<()> {
        wg(&["syncconf", interface, "/dev/stdin"], Some(config))?;
        Ok(())
    }
    fn service(&self, interface: &str, action: ServiceAction) -> Result<()> {
        ensure!(
            crate::config::valid_interface(interface),
            "Niedozwolona nazwa interfejsu"
        );
        let verb = match action {
            ServiceAction::Start => "start",
            ServiceAction::Stop => "stop",
            ServiceAction::Enable => "enable",
            ServiceAction::Disable => "disable",
        };
        run(
            "/usr/bin/systemctl",
            &[verb, &format!("wg-quick@{interface}.service")],
            None,
        )?;
        Ok(())
    }
    fn autostart(&self, interface: &str) -> Result<bool> {
        ensure!(
            crate::config::valid_interface(interface),
            "Niedozwolona nazwa interfejsu"
        );
        let state = run(
            "/usr/bin/systemctl",
            &[
                "show",
                "--property=UnitFileState",
                "--value",
                &format!("wg-quick@{interface}.service"),
            ],
            None,
        )?;
        ensure!(
            !state.trim().is_empty(),
            "Nie można ustalić stanu autostartu systemd"
        );
        Ok(matches!(state.trim(), "enabled" | "enabled-runtime"))
    }
    fn keypair(&self) -> Result<(Secret, String)> {
        let raw = wg(&["genkey"], None)?;
        let private = secret(raw.trim());
        let public = self.public_key(&private)?;
        Ok((private, public))
    }
    fn public_key(&self, private: &str) -> Result<String> {
        let public = wg(&["pubkey"], Some(private))?;
        crate::config::validate_key(public.trim())?;
        Ok(public.trim().to_string())
    }
}
