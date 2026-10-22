//! Real kernel test. Requires an isolated Linux VM, root, iproute2,
//! wireguard-tools and iputils-ping. Never touches production interfaces.
#![cfg(target_os = "linux")]
use anyhow::{Result, ensure};
use std::{fs::File, os::fd::AsRawFd, sync::Arc};
use tempfile::TempDir;
use wgtui::{
    config::{Config, Peer, parse_ips},
    profile::{CLIENT_TEMPLATE, Profile},
    secret::secret,
    store::{Store, TunnelSpec},
    system::{Backend, RealBackend, run},
};

fn ip(args: &[&str]) -> Result<()> {
    run("/usr/sbin/ip", args, None)?;
    Ok(())
}
fn ns(
    namespace: &str,
    program: &str,
    args: &[&str],
    input: Option<&str>,
) -> Result<wgtui::secret::Secret> {
    let mut all = vec!["netns", "exec", namespace, program];
    all.extend_from_slice(args);
    run("/usr/sbin/ip", &all, input)
}
struct Namespaces {
    names: Vec<String>,
    original: File,
}
impl Drop for Namespaces {
    fn drop(&mut self) {
        unsafe {
            libc::setns(self.original.as_raw_fd(), libc::CLONE_NEWNET);
        }
        for name in &self.names {
            let _ = ip(&["netns", "del", name]);
        }
    }
}
#[test]
#[ignore = "root + Linux network namespaces; run only in a disposable VM"]
fn handshake_add_without_reset_disable_and_enable() -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "Test wymaga root w izolowanej VM"
    );
    let pid = std::process::id();
    let names = vec![
        format!("wgtui-s-{pid}"),
        format!("wgtui-a-{pid}"),
        format!("wgtui-b-{pid}"),
    ];
    let guard = Namespaces {
        names: names.clone(),
        original: File::open("/proc/thread-self/ns/net")?,
    };
    for name in &names {
        ip(&["netns", "add", name])?;
        ns(name, "/usr/sbin/ip", &["link", "set", "lo", "up"], None)?;
    }
    for index in 1..=2 {
        let a = format!("ws{index}");
        let b = format!("wc{index}");
        ip(&["link", "add", &a, "type", "veth", "peer", "name", &b])?;
        ip(&["link", "set", &a, "netns", &names[0]])?;
        ip(&["link", "set", &b, "netns", &names[index]])?;
        ns(
            &names[0],
            "/usr/sbin/ip",
            &[
                "address",
                "add",
                &format!("192.0.{}.1/24", index + 1),
                "dev",
                &a,
            ],
            None,
        )?;
        ns(
            &names[index],
            "/usr/sbin/ip",
            &[
                "address",
                "add",
                &format!("192.0.{}.2/24", index + 1),
                "dev",
                &b,
            ],
            None,
        )?;
        ns(&names[0], "/usr/sbin/ip", &["link", "set", &a, "up"], None)?;
        ns(
            &names[index],
            "/usr/sbin/ip",
            &["link", "set", &b, "up"],
            None,
        )?;
    }
    let server_ns = File::open(format!("/var/run/netns/{}", names[0]))?;
    ensure!(
        unsafe { libc::setns(server_ns.as_raw_fd(), libc::CLONE_NEWNET) } == 0,
        "Nie można wejść do testowej przestrzeni sieciowej"
    );
    let temp = TempDir::new()?;
    let backend = Arc::new(RealBackend);
    let store = Store::open(
        temp.path().join("data"),
        temp.path().join("wireguard"),
        backend.clone(),
    )?;
    let name = "wg01admin01";
    let change = store.new_tunnel(TunnelSpec {
        name: name.into(),
        addresses: parse_ips("10.90.0.1/24")?,
        port: 51820,
        profile: Profile {
            endpoint: "192.0.2.1:51820".into(),
            keepalive: 25,
            ..Profile::default()
        },
        start: false,
        autostart: false,
    })?;
    store.apply(&change)?;
    let server_key = backend.public_key(
        &Config::parse(&store.read(name)?)?
            .secret_value("PrivateKey")
            .unwrap(),
    )?;
    let path = store.config_dir.join(format!("{name}.conf"));
    run("/usr/bin/wg-quick", &["up", path.to_str().unwrap()], None)?;
    let mut peers = vec![];
    let mut private_clients = vec![];
    for index in 1..=2 {
        let (private, public) = backend.keypair()?;
        let peer = Peer {
            name: format!("test-{index:02}"),
            public_key: public,
            allowed_ips: parse_ips(&format!("10.90.0.{}/32", index + 1))?,
            enabled: true,
            keepalive: 0,
        };
        let before = store.read(name)?;
        let mut c = Config::parse(&before)?;
        c.add_peer(peer.clone())?;
        store.apply(&store.change(name, before, c.render())?)?;
        ns(
            &names[index],
            "/usr/sbin/ip",
            &["link", "add", "wgclient", "type", "wireguard"],
            None,
        )?;
        let profile = Profile {
            endpoint: format!("192.0.{}.1:51820", index + 1),
            keepalive: 1,
            ..Profile::default()
        };
        let client = profile.render(&c, &peer, &server_key, Some(&private), CLIENT_TEMPLATE)?;
        ns(
            &names[index],
            "/usr/bin/wg",
            &["setconf", "wgclient", "/dev/stdin"],
            Some(&Config::parse(&client)?.stripped()),
        )?;
        ns(
            &names[index],
            "/usr/sbin/ip",
            &[
                "address",
                "add",
                &format!("10.90.0.{}/32", index + 1),
                "dev",
                "wgclient",
            ],
            None,
        )?;
        ns(
            &names[index],
            "/usr/sbin/ip",
            &["link", "set", "wgclient", "up"],
            None,
        )?;
        ns(
            &names[index],
            "/usr/sbin/ip",
            &["route", "add", "10.90.0.1/32", "dev", "wgclient"],
            None,
        )?;
        ns(
            &names[index],
            "/usr/bin/ping",
            &["-c", "1", "-W", "3", "10.90.0.1"],
            None,
        )?;
        peers.push(peer);
        private_clients.push(private);
        if index == 1 {
            let handshake = backend.observe()?[name].peers[&peers[0].public_key].handshake;
            ensure!(handshake > 0, "Brak handshake’u pierwszego klienta");
            // Save only a public timestamp for the assertion after adding client B.
            atomic_timestamp(temp.path(), handshake)?;
        }
    }
    let original: u64 = std::fs::read_to_string(temp.path().join("handshake"))?.parse()?;
    ensure!(
        backend.observe()?[name].peers[&peers[0].public_key].handshake == original,
        "Dodanie peera zresetowało sesję pierwszego"
    );
    ns(
        &names[1],
        "/usr/bin/ping",
        &["-c", "1", "-W", "3", "10.90.0.1"],
        None,
    )?;
    for enabled in [false, true] {
        let before = store.read(name)?;
        let mut c = Config::parse(&before)?;
        let mut peer = peers[0].clone();
        peer.enabled = enabled;
        c.update_peer(&peer.public_key.clone(), peer)?;
        store.apply(&store.change(name, before, c.render())?)?;
        ensure!(
            backend.observe()?[name]
                .peers
                .contains_key(&peers[0].public_key)
                == enabled,
            "Niepoprawny stan peera"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let ping = loop {
            let result = ns(
                &names[1],
                "/usr/bin/ping",
                &["-c", "1", "-W", "1", "10.90.0.1"],
                None,
            );
            if !enabled || result.is_ok() || std::time::Instant::now() > deadline {
                break result;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        };
        ensure!(
            ping.is_ok() == enabled,
            "Niepoprawna dostępność po zmianie stanu enabled={enabled}"
        );
    }
    let server_file = store.read(name)?;
    for key in &private_clients {
        ensure!(
            !server_file.contains(key.as_str()),
            "Klucz klienta trafił do pliku serwera"
        );
    }
    // Public-key-only configuration is sufficient for both real clients.
    let _ = secret("no client keys persisted");
    drop(guard);
    Ok(())
}
fn atomic_timestamp(path: &std::path::Path, timestamp: u64) -> Result<()> {
    std::fs::write(path.join("handshake"), timestamp.to_string())?;
    Ok(())
}
