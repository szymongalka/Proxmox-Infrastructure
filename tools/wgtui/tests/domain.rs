use base64::{Engine, engine::general_purpose::STANDARD};
use wgtui::{
    config::*,
    profile::{CLIENT_TEMPLATE, Profile, SecretQr},
    secret::redact,
};

fn key(n: u8) -> String {
    STANDARD.encode([n; 32])
}
fn source() -> String {
    format!(
        "# zachowaj komentarz\r\n[Interface]\r\nPrivateKey = {}\r\nAddress = 10.90.0.1/24, fd90::1/64\r\nListenPort = 51820\r\nPostUp = echo przykład\r\nSaveConfig = false\r\n\r\n[Peer]\r\n# istniejący komentarz\r\nPublicKey = {}\r\nAllowedIPs = 10.90.0.2/32, fd90::2/128\r\nEndpoint = example.org:1234\r\n",
        key(7),
        key(10)
    )
}
fn peer(n: u8, ip: &str) -> Peer {
    Peer {
        name: format!("telefon-{n:02}"),
        public_key: key(n),
        allowed_ips: parse_ips(ip).unwrap(),
        enabled: true,
        keepalive: 0,
    }
}

#[test]
fn lossless_round_trip_and_edit_preserve_hooks_comments_endpoint() {
    let input = source();
    let mut c = Config::parse(&input).unwrap();
    assert_eq!(*c.render(), input);
    let mut p = c.peers()[0].clone();
    p.name = "macbook-01".into();
    p.allowed_ips = parse_ips("10.90.0.3/32, fd90::3/128").unwrap();
    c.update_peer(&p.public_key.clone(), p).unwrap();
    let result = c.render();
    assert!(result.contains("# istniejący komentarz\r\n"));
    assert!(result.contains("PostUp = echo przykład\r\n"));
    assert!(result.contains("Endpoint = example.org:1234\r\n"));
    assert_eq!(
        Config::parse(&result).unwrap().peers()[0].name,
        "macbook-01"
    );
}

#[test]
fn disabled_peer_round_trip_reserves_ip_and_restores_unknown_options() {
    let mut c = Config::parse(&source()).unwrap();
    let p = c.peers()[0].clone();
    let mut disabled = p.clone();
    disabled.enabled = false;
    c.update_peer(&p.public_key, disabled).unwrap();
    let rendered = c.render();
    let mut c = Config::parse(&rendered).unwrap();
    assert_eq!(*c.render(), *rendered);
    assert!(!c.peers()[0].enabled);
    let crlf = rendered.replace("\r\n", "\n").replace('\n', "\r\n");
    assert!(!Config::parse(&crlf).unwrap().peers()[0].enabled);
    assert!(!c.stripped().contains(&key(10)));
    assert!(c.validate_peer(&peer(11, "10.90.0.2/32"), None).is_err());
    assert_eq!(
        join_ips(&c.next_addresses().unwrap()),
        "10.90.0.3/32, fd90::/128"
    );
    c.update_peer(&p.public_key, p.clone()).unwrap();
    assert!(c.render().contains("Endpoint = example.org:1234"));
    assert!(c.peers()[0].enabled);
}

#[test]
fn strip_removes_only_interface_quick_options() {
    let c = Config::parse(&source()).unwrap();
    let stripped = c.stripped();
    assert!(!stripped.contains("Address ="));
    assert!(!stripped.contains("PostUp ="));
    assert!(!stripped.contains("SaveConfig ="));
    assert!(stripped.contains("PrivateKey ="));
    assert!(stripped.contains("AllowedIPs ="));
}

#[test]
fn rejects_invalid_keys_duplicate_keys_ips_network_broadcast_server_and_subnets() {
    let c = Config::parse(&source()).unwrap();
    for bad in [
        "bad",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "not-a-key\n",
    ] {
        assert!(validate_key(bad).is_err());
    }
    for ip in [
        "10.90.0.2/32",
        "10.90.0.1/32",
        "10.90.0.0/32",
        "10.90.0.255/32",
        "10.91.0.1/32",
        "10.90.0.4/24",
        "10.90.0.3/32, 10.90.0.4/32",
    ] {
        assert!(
            c.validate_peer(&peer(11, ip), None).is_err(),
            "accepted {ip}"
        );
    }
    assert!(c.validate_peer(&peer(10, "10.90.0.3/32"), None).is_err());
    assert!(
        Config::parse(&format!(
            "{}\n{}",
            source(),
            peer(10, "10.90.0.4/32").block()
        ))
        .is_err()
    );
    assert!(valid_interface("wg01admin01"));
    assert!(valid_interface("wg99admin99"));
    for name in [
        "wg00admin01",
        "wg01admin00",
        "wg1admin1",
        "../wg01admin01",
        "wg01admin01\n",
    ] {
        assert!(!valid_interface(name));
    }
}

#[test]
fn saveconfig_blocks_changes_and_secret_redaction_covers_disabled_psk() {
    let mut c =
        Config::parse(&source().replace("SaveConfig = false", "SaveConfig = true")).unwrap();
    assert!(c.add_peer(peer(11, "10.90.0.3/32")).is_err());
    assert!(c.remove_peer(&key(10)).is_err());
    let text = redact(&format!(
        "PrivateKey = {}\n# PresharedKey = {}\nPublicKey = {}",
        key(7),
        key(8),
        key(10)
    ));
    assert!(!text.contains(&key(7)));
    assert!(!text.contains(&key(8)));
    assert!(text.contains(&key(10)));
}

#[test]
fn runtime_comparison_ignores_roaming_but_detects_key_and_psk_drift() {
    let c = Config::parse(&source()).unwrap();
    let runtime = Config::parse(
        &c.stripped()
            .replace("example.org:1234", "198.51.100.4:44000"),
    )
    .unwrap();
    assert!(c.same_runtime_as(&runtime));
    let changed = Config::parse(&c.stripped().replace(&key(7), &key(8))).unwrap();
    assert!(!c.same_runtime_as(&changed));
    let changed = Config::parse(&format!(
        "{}PresharedKey = {}\n",
        c.stripped().as_str(),
        key(8)
    ))
    .unwrap();
    assert!(!c.same_runtime_as(&changed));
}

#[test]
fn templates_have_no_execution_and_enforce_address_key_and_route() {
    let c = Config::parse(&source()).unwrap();
    let p = peer(11, "10.90.0.3/32");
    let profile = Profile {
        endpoint: "vpn.example.org:51820".into(),
        keepalive: 25,
        ..Profile::default()
    };
    let public = profile
        .render(&c, &p, &key(42), None, CLIENT_TEMPLATE)
        .unwrap();
    assert!(public.contains("<WKLEJ_KLUCZ_PRYWATNY_NA_URZADZENIU>"));
    assert!(public.contains("AllowedIPs = 10.90.0.1/32, fd90::1/128"));
    assert!(!public.contains(&key(7)));
    for template in [
        CLIENT_TEMPLATE.replace("{{endpoint}}", "{{shell}}"),
        CLIENT_TEMPLATE.replace("{{private_key}}", "literal"),
        CLIENT_TEMPLATE.replace("{{address}}", "10.0.0.1/32"),
    ] {
        assert!(
            profile
                .render(&c, &p, &key(42), Some(&key(9)), &template)
                .is_err()
        );
    }
    let full = Profile {
        routes: parse_ips("0.0.0.0/0").unwrap(),
        ..profile.clone()
    };
    assert!(full.validate().is_err());
    let bad = Profile {
        endpoint: "vpn.example.org:51820\nPostUp = command".into(),
        ..profile
    };
    assert!(bad.validate().is_err());
}

#[test]
fn qr_decodes_with_independent_decoder_including_quiet_zone() {
    let payload = format!(
        "[Interface]\nPrivateKey = {}\nAddress = 10.90.0.2/32\n\n[Peer]\nPublicKey = {}\nEndpoint = vpn.example.org:51820\nAllowedIPs = 10.90.0.1/32\n",
        key(7),
        key(42)
    );
    let qr = SecretQr::new(&payload).unwrap();
    let size = (qr.size + 8) * 5;
    let image = image::GrayImage::from_fn(size as u32, size as u32, |x, y| {
        let x = x as usize / 5;
        let y = y as usize / 5;
        image::Luma([if x >= 4 && y >= 4 && qr.dark(x - 4, y - 4) {
            0
        } else {
            255
        }])
    });
    let mut prepared = rqrr::PreparedImage::prepare(image);
    let grids = prepared.detect_grids();
    assert_eq!(grids.len(), 1);
    let (_, decoded) = grids[0].decode().unwrap();
    assert_eq!(decoded, payload);
}
