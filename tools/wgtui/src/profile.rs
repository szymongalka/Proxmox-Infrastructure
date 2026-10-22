use crate::{
    config::{Config, Peer, host_net, join_ips, validate_key, validate_routes},
    secret::{Secret, secret, single_line},
};
use anyhow::{Result, anyhow, ensure};
use ipnet::IpNet;
use qrcode::{Color, EcLevel, QrCode};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const CLIENT_TEMPLATE: &str = include_str!("../templates/client.conf.tpl");

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub active_seconds: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            active_seconds: 180,
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub endpoint: String,
    pub dns: Vec<String>,
    pub routes: Vec<IpNet>,
    pub keepalive: u16,
}

impl Profile {
    pub fn validate(&self) -> Result<()> {
        validate_endpoint(&self.endpoint)?;
        for dns in &self.dns {
            ensure!(
                dns.parse::<std::net::IpAddr>().is_ok(),
                "DNS musi być adresem IPv4 lub IPv6"
            );
        }
        validate_routes(&self.routes)?;
        Ok(())
    }

    pub fn render(
        &self,
        config: &Config,
        peer: &Peer,
        server_public: &str,
        private: Option<&str>,
        template: &str,
    ) -> Result<Secret> {
        self.validate()?;
        validate_key(server_public)?;
        ensure!(template.len() <= 8192, "Szablon jest zbyt duży");
        ensure!(
            !template
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')),
            "Szablon zawiera niedozwolone znaki sterujące"
        );
        let routes = if self.routes.is_empty() {
            config
                .addresses()?
                .into_iter()
                .map(|n| host_net(n.addr()))
                .collect()
        } else {
            self.routes.clone()
        };
        ensure!(!routes.is_empty(), "Brak tras do serwera w szablonie");
        let values = [
            (
                "private_key",
                secret(private.unwrap_or("<WKLEJ_KLUCZ_PRYWATNY_NA_URZADZENIU>")),
            ),
            ("address", secret(join_ips(&peer.allowed_ips))),
            (
                "dns_line",
                secret(if self.dns.is_empty() {
                    String::new()
                } else {
                    format!("DNS = {}", self.dns.join(", "))
                }),
            ),
            ("server_public_key", secret(server_public)),
            ("endpoint", secret(&self.endpoint)),
            ("allowed_ips", secret(join_ips(&routes))),
            (
                "keepalive_line",
                secret(format!("PersistentKeepalive = {}", self.keepalive)),
            ),
        ];
        ensure!(
            template.matches("{{private_key}}").count() == 1,
            "Szablon musi zawierać dokładnie jeden placeholder private_key"
        );
        let mut output = secret(template);
        for (key, val) in values {
            output = secret(output.replace(&format!("{{{{{key}}}}}"), &val));
        }
        ensure!(
            !output.contains("{{") && !output.contains("}}"),
            "Nieznany lub niezamknięty placeholder w szablonie"
        );
        // Parse structurally even in public-key mode, with a placeholder for PrivateKey.
        let rendered = Config::parse(&output)?;
        let peers = rendered.peers();
        ensure!(
            peers.len() == 1 && peers[0].public_key == server_public,
            "Szablon klienta musi zawierać dokładnie jednego peera serwera"
        );
        ensure!(
            rendered
                .secret_value("PrivateKey")
                .as_ref()
                .map(|s| s.as_str())
                == Some(private.unwrap_or("<WKLEJ_KLUCZ_PRYWATNY_NA_URZADZENIU>")),
            "Szablon zmienia klucz prywatny klienta"
        );
        ensure!(
            rendered.addresses()? == peer.allowed_ips && peers[0].allowed_ips == routes,
            "Szablon zmienia adresy lub trasy klienta"
        );
        Ok(output)
    }
}

pub fn validate_endpoint(endpoint: &str) -> Result<()> {
    single_line(endpoint, "Endpoint")?;
    let (host, port) = endpoint
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("Endpoint wymaga hosta i portu, np. vpn.example.org:51820"))?;
    let port: u16 = port
        .parse()
        .map_err(|_| anyhow!("Nieprawidłowy port endpointu"))?;
    ensure!(port > 0, "Port endpointu nie może być zerowy");
    let valid = if host.starts_with('[') && host.ends_with(']') {
        host[1..host.len() - 1]
            .parse::<std::net::Ipv6Addr>()
            .is_ok()
    } else {
        !host.is_empty()
            && host.len() <= 253
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
            && !host.starts_with('-')
            && !host.starts_with('.')
    };
    ensure!(
        valid,
        "Niepoprawny host endpointu; IPv6 zapisz w nawiasach []"
    );
    Ok(())
}

pub struct SecretQr {
    pub size: usize,
    modules: Zeroizing<Vec<u8>>,
}

impl SecretQr {
    pub fn new(payload: &str) -> Result<Self> {
        let code = QrCode::with_error_correction_level(payload.as_bytes(), EcLevel::M)
            .map_err(|_| anyhow!("Profil jest zbyt duży dla QR; skróć szablon lub listę tras"))?;
        let size = code.width();
        let mut colors = code.into_colors();
        let modules = Zeroizing::new(colors.iter().map(|c| u8::from(*c == Color::Dark)).collect());
        // qrcode does not implement Zeroize for Color. Consume and wipe its matrix.
        for color in &mut colors {
            unsafe {
                std::ptr::write_volatile(color, Color::Light);
            }
        }
        Ok(Self { size, modules })
    }
    pub fn dark(&self, x: usize, y: usize) -> bool {
        x < self.size && y < self.size && self.modules[y * self.size + x] != 0
    }
    pub fn terminal_size(&self) -> (u16, u16) {
        ((self.size + 8) as u16, (self.size + 8).div_ceil(2) as u16)
    }
}
