use crate::secret::{Secret, secret, single_line};
use anyhow::{Result, anyhow, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ipnet::IpNet;
use std::{collections::BTreeSet, net::IpAddr};

const DISABLED: &str = "# wgtui:disabled\n";
const END_DISABLED: &str = "# wgtui:end-disabled\n";

pub fn valid_interface(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 11
        && &b[..2] == b"wg"
        && &b[4..9] == b"admin"
        && b[2..4].iter().all(u8::is_ascii_digit)
        && b[9..11].iter().all(u8::is_ascii_digit)
        && &b[2..4] != b"00"
        && &b[9..11] != b"00"
}

pub fn valid_runtime_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 15
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_=+.-".contains(&b))
}

pub fn validate_key(key: &str) -> Result<()> {
    let decoded = STANDARD
        .decode(key)
        .map_err(|_| anyhow!("Klucz publiczny musi być poprawnym Base64"))?;
    ensure!(
        decoded.len() == 32 && STANDARD.encode(&decoded) == key && decoded.iter().any(|b| *b != 0),
        "Klucz publiczny musi zawierać 32 bajty i nie może być zerowy"
    );
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    pub name: String,
    pub public_key: String,
    pub allowed_ips: Vec<IpNet>,
    pub enabled: bool,
    pub keepalive: u16,
}

impl Peer {
    pub fn label(&self) -> String {
        if self.name.is_empty() {
            self.public_key.chars().take(12).collect()
        } else {
            self.name.clone()
        }
    }
    pub fn block(&self) -> String {
        format!(
            "[Peer]\n# wgtui:name = {}\nPublicKey = {}\nAllowedIPs = {}\nPersistentKeepalive = {}\n",
            self.name,
            self.public_key,
            join_ips(&self.allowed_ips),
            self.keepalive
        )
    }
}

pub fn join_ips(ips: &[IpNet]) -> String {
    ips.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn parse_ips(value: &str) -> Result<Vec<IpNet>> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse()
                .map_err(|_| anyhow!("Niepoprawny adres lub prefiks CIDR"))
        })
        .collect()
}

pub fn overlaps(a: IpNet, b: IpNet) -> bool {
    a.contains(&b.addr()) || b.contains(&a.addr())
}

pub fn host_net(addr: IpAddr) -> IpNet {
    IpNet::new(addr, if addr.is_ipv4() { 32 } else { 128 }).expect("host prefix")
}

struct Section {
    raw: Secret,
    peer: Option<Peer>,
}

/// Lossless sections: untouched bytes (including hooks and comments) survive edits.
pub struct Config {
    header: Secret,
    sections: Vec<Section>,
}

fn value<'a>(line: &'a str, expected: &str) -> Option<&'a str> {
    let (key, val) = line.split('#').next()?.split_once('=')?;
    key.trim()
        .eq_ignore_ascii_case(expected)
        .then_some(val.trim())
}

impl Config {
    pub fn parse(input: &str) -> Result<Self> {
        let mut header = secret("");
        let mut sections = Vec::new();
        let mut current = secret("");
        let mut disabled = false;
        let mut interface_count = 0;
        for line in input.split_inclusive('\n') {
            let t = line.trim();
            if t == "# wgtui:disabled" {
                ensure!(!disabled, "Zagnieżdżony wyłączony peer");
                if !current.is_empty() {
                    sections.push(Self::section(std::mem::replace(&mut current, secret("")))?);
                }
                disabled = true;
            }
            if !disabled
                && (t.eq_ignore_ascii_case("[Peer]") || t.eq_ignore_ascii_case("[Interface]"))
            {
                if t.eq_ignore_ascii_case("[Interface]") {
                    interface_count += 1;
                }
                if !current.is_empty() {
                    sections.push(Self::section(std::mem::replace(&mut current, secret("")))?);
                }
                current.push_str(line);
            } else if current.is_empty() && !disabled {
                if sections.is_empty() {
                    header.push_str(line);
                } else {
                    sections.push(Section {
                        raw: secret(line),
                        peer: None,
                    });
                }
            } else {
                current.push_str(line);
            }
            if t == "# wgtui:end-disabled" {
                ensure!(disabled, "Niepoprawny koniec wyłączonego peera");
                disabled = false;
                sections.push(Self::section(std::mem::replace(&mut current, secret("")))?);
            }
        }
        ensure!(
            !disabled && interface_count == 1,
            "Konfiguracja musi zawierać jedną sekcję [Interface]"
        );
        if !current.is_empty() {
            sections.push(Self::section(current)?);
        }
        let config = Self { header, sections };
        for field in ["PrivateKey", "ListenPort", "SaveConfig", "FwMark"] {
            ensure!(
                config
                    .sections
                    .iter()
                    .filter(|s| s.peer.is_none())
                    .flat_map(|s| s.raw.lines())
                    .filter(|line| value(line, field).is_some())
                    .count()
                    <= 1,
                "Powtórzone jednoznaczne pole w sekcji [Interface]"
            );
        }
        let mut seen = BTreeSet::new();
        for peer in config.peers() {
            ensure!(
                seen.insert(peer.public_key.clone()),
                "Powtórzony klucz publiczny w konfiguracji"
            );
        }
        Ok(config)
    }

    fn section(raw: Secret) -> Result<Section> {
        let enabled = raw.lines().next().unwrap_or("").trim() != "# wgtui:disabled";
        let content = if enabled {
            secret(raw.as_str())
        } else {
            decode_disabled(&raw)?
        };
        if !content
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("[Peer]")
        {
            return Ok(Section { raw, peer: None });
        }
        let mut peer = Peer {
            name: String::new(),
            public_key: String::new(),
            allowed_ips: vec![],
            enabled,
            keepalive: 0,
        };
        let mut key_count = 0;
        for line in content.lines() {
            if let Some(v) = line.trim().strip_prefix("# wgtui:name = ") {
                peer.name = v.to_string();
            }
            if let Some(v) = value(line, "PublicKey") {
                key_count += 1;
                peer.public_key = v.to_string();
            }
            if let Some(v) = value(line, "AllowedIPs") {
                peer.allowed_ips.extend(parse_ips(v)?);
            }
            if let Some(v) = value(line, "PersistentKeepalive") {
                peer.keepalive = if v == "off" {
                    0
                } else {
                    v.parse().map_err(|_| anyhow!("Błędny keepalive"))?
                };
            }
        }
        ensure!(
            key_count == 1,
            "Peer musi mieć dokładnie jeden klucz publiczny"
        );
        single_line(&peer.name, "Nazwa peera")?;
        validate_key(&peer.public_key)?;
        Ok(Section {
            raw,
            peer: Some(peer),
        })
    }

    pub fn peers(&self) -> Vec<Peer> {
        self.sections
            .iter()
            .filter_map(|s| s.peer.clone())
            .collect()
    }

    pub fn addresses(&self) -> Result<Vec<IpNet>> {
        let mut result = vec![];
        for section in self.sections.iter().filter(|s| s.peer.is_none()) {
            for line in section.raw.lines() {
                if let Some(v) = value(line, "Address") {
                    result.extend(parse_ips(v)?);
                }
            }
        }
        Ok(result)
    }

    pub fn interface_value(&self, key: &str) -> Option<String> {
        self.sections
            .iter()
            .filter(|s| s.peer.is_none())
            .flat_map(|s| s.raw.lines())
            .find_map(|l| value(l, key).map(str::to_string))
    }

    pub fn secret_value(&self, key: &str) -> Option<Secret> {
        self.sections
            .iter()
            .filter(|s| s.peer.is_none())
            .flat_map(|s| s.raw.lines())
            .find_map(|l| value(l, key).map(secret))
    }

    pub fn same_runtime_as(&self, other: &Self) -> bool {
        fn signature(config: &Config) -> Vec<(String, Vec<IpNet>, u16, Option<Secret>)> {
            let mut result = config
                .sections
                .iter()
                .filter_map(|s| {
                    let peer = s.peer.as_ref().filter(|p| p.enabled)?;
                    let mut ips = peer.allowed_ips.clone();
                    ips.sort();
                    let psk = s
                        .raw
                        .lines()
                        .find_map(|l| value(l, "PresharedKey").map(secret));
                    Some((peer.public_key.clone(), ips, peer.keepalive, psk))
                })
                .collect::<Vec<_>>();
            result.sort_by(|a, b| a.0.cmp(&b.0));
            result
        }
        fn number(v: Option<String>) -> Option<u32> {
            let v = v.unwrap_or_else(|| "0".into());
            if v == "off" {
                Some(0)
            } else if let Some(hex) = v.strip_prefix("0x") {
                u32::from_str_radix(hex, 16).ok()
            } else {
                v.parse().ok()
            }
        }
        signature(self) == signature(other)
            && self.secret_value("PrivateKey") == other.secret_value("PrivateKey")
            && number(self.interface_value("ListenPort"))
                == number(other.interface_value("ListenPort"))
            && number(self.interface_value("FwMark")) == number(other.interface_value("FwMark"))
    }

    pub fn editable(&self) -> Result<()> {
        ensure!(
            self.interface_value("SaveConfig")
                .is_none_or(|v| !v.eq_ignore_ascii_case("true")),
            "SaveConfig=true nadpisuje plik przy zatrzymaniu. Zmień to ustawienie poza wgtui przed edycją"
        );
        let private = self.secret_value("PrivateKey").ok_or_else(|| {
            anyhow!("Zarządzanie wymaga klucza serwera PrivateKey w pliku konfiguracji")
        })?;
        let decoded = zeroize::Zeroizing::new(
            STANDARD
                .decode(private.as_str())
                .map_err(|_| anyhow!("Nieprawidłowy klucz serwera PrivateKey"))?,
        );
        ensure!(
            decoded.len() == 32,
            "Nieprawidłowa długość klucza serwera PrivateKey"
        );
        Ok(())
    }

    pub fn render(&self) -> Secret {
        let mut output = secret(self.header.as_str());
        for s in &self.sections {
            output.push_str(&s.raw);
        }
        output
    }

    pub fn validate_peer(&self, peer: &Peer, replacing: Option<&str>) -> Result<()> {
        validate_key(&peer.public_key)?;
        single_line(&peer.name, "Nazwa peera")?;
        ensure!(
            !peer.name.trim().is_empty() && peer.name.chars().count() <= 64,
            "Nazwa musi mieć od 1 do 64 znaków"
        );
        ensure!(!peer.allowed_ips.is_empty(), "Podaj adres klienta");
        let pools = self.addresses()?;
        ensure!(!pools.is_empty(), "Konfiguracja nie zawiera puli Address");
        let mut families = BTreeSet::new();
        for ip in &peer.allowed_ips {
            ensure!(
                ip.prefix_len() == if ip.addr().is_ipv4() { 32 } else { 128 },
                "Adres klienta musi być /32 lub /128"
            );
            ensure!(
                families.insert(ip.addr().is_ipv4()),
                "Dozwolony jest jeden adres każdej rodziny IP"
            );
            ensure!(
                pools.iter().any(|p| usable_in_pool(*p, ip.addr())),
                "Adres klienta nie należy do dostępnej puli lub jest adresem serwera/sieci/broadcast"
            );
            for other in self
                .peers()
                .iter()
                .filter(|p| Some(p.public_key.as_str()) != replacing)
            {
                ensure!(
                    other.public_key != peer.public_key,
                    "Peer z tym kluczem już istnieje"
                );
                ensure!(
                    !other.allowed_ips.iter().any(|a| overlaps(*a, *ip)),
                    "Adres jest zajęty przez innego peera (także wyłączonego)"
                );
            }
        }
        Ok(())
    }

    pub fn next_addresses(&self) -> Result<Vec<IpNet>> {
        let peers = self.peers();
        let mut result = vec![];
        for pool in self.addresses()? {
            if result
                .iter()
                .any(|n: &IpNet| n.addr().is_ipv4() == pool.addr().is_ipv4())
            {
                continue;
            }
            // At most N occupied hosts can precede a free host. Avoid scanning IPv6 networks.
            let max = peers.len() + 4;
            let mut addr = pool.network();
            for _ in 0..max {
                if usable_in_pool(pool, addr)
                    && !peers
                        .iter()
                        .any(|p| p.allowed_ips.iter().any(|n| n.contains(&addr)))
                {
                    result.push(host_net(addr));
                    break;
                }
                addr = match addr {
                    IpAddr::V4(v) => u32::from(v).checked_add(1).map(|v| IpAddr::V4(v.into())),
                    IpAddr::V6(v) => u128::from(v).checked_add(1).map(|v| IpAddr::V6(v.into())),
                }
                .ok_or_else(|| anyhow!("Brak wolnych adresów"))?;
                if !pool.contains(&addr) {
                    break;
                }
            }
        }
        ensure!(!result.is_empty(), "Brak wolnego adresu w puli");
        Ok(result)
    }

    pub fn add_peer(&mut self, peer: Peer) -> Result<()> {
        self.editable()?;
        self.validate_peer(&peer, None)?;
        if let Some(last) = self.sections.last_mut() {
            if !last.raw.ends_with('\n') {
                last.raw.push('\n');
            }
            last.raw.push('\n');
        }
        self.sections.push(Self::section(secret(peer.block()))?);
        Ok(())
    }

    pub fn update_peer(&mut self, old_key: &str, mut peer: Peer) -> Result<()> {
        self.editable()?;
        if peer.name.is_empty() {
            peer.name = peer.label();
        }
        self.validate_peer(&peer, Some(old_key))?;
        let section = self
            .sections
            .iter_mut()
            .find(|s| s.peer.as_ref().is_some_and(|p| p.public_key == old_key))
            .ok_or_else(|| anyhow!("Nie znaleziono peera"))?;
        let old_content = if section.peer.as_ref().unwrap().enabled {
            secret(section.raw.as_str())
        } else {
            decode_disabled(&section.raw)?
        };
        let mut updated = secret("");
        for line in old_content.split_inclusive('\n') {
            if line.trim_start().starts_with("# wgtui:name = ")
                || ["PublicKey", "AllowedIPs", "PersistentKeepalive"]
                    .iter()
                    .any(|k| value(line, k).is_some())
            {
                continue;
            }
            updated.push_str(line);
        }
        if !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push_str(&format!(
            "# wgtui:name = {}\nPublicKey = {}\nAllowedIPs = {}\nPersistentKeepalive = {}\n",
            peer.name,
            peer.public_key,
            join_ips(&peer.allowed_ips),
            peer.keepalive
        ));
        section.raw = if peer.enabled {
            updated
        } else {
            encode_disabled(&updated)
        };
        section.peer = Some(peer);
        Ok(())
    }

    pub fn remove_peer(&mut self, key: &str) -> Result<()> {
        self.editable()?;
        let old_len = self.sections.len();
        self.sections
            .retain(|s| !s.peer.as_ref().is_some_and(|p| p.public_key == key));
        ensure!(old_len != self.sections.len(), "Nie znaleziono peera");
        Ok(())
    }

    /// Equivalent to wg-quick strip, without a shell or an on-disk secret copy.
    pub fn stripped(&self) -> Secret {
        let quick = [
            "Address",
            "DNS",
            "MTU",
            "Table",
            "PreUp",
            "PostUp",
            "PreDown",
            "PostDown",
            "SaveConfig",
        ];
        let mut result = secret("");
        for s in &self.sections {
            if s.peer.as_ref().is_some_and(|p| !p.enabled) {
                continue;
            }
            for line in s.raw.split_inclusive('\n') {
                if s.peer.is_none() && quick.iter().any(|k| value(line, k).is_some()) {
                    continue;
                }
                if !result.is_empty() && !result.ends_with('\n') {
                    result.push('\n');
                }
                result.push_str(line);
            }
        }
        result
    }
}

fn usable_in_pool(pool: IpNet, addr: IpAddr) -> bool {
    pool.contains(&addr)
        && addr != pool.addr()
        && match pool {
            IpNet::V4(n) if n.prefix_len() < 31 => {
                addr != IpAddr::V4(n.network()) && addr != IpAddr::V4(n.broadcast())
            }
            _ => true,
        }
}

fn encode_disabled(content: &str) -> Secret {
    let mut result = secret(DISABLED);
    for line in content.split_inclusive('\n') {
        result.push_str("# ");
        result.push_str(line);
    }
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result.push_str(END_DISABLED);
    result
}

fn decode_disabled(raw: &str) -> Result<Secret> {
    let mut result = secret("");
    let mut lines = raw.split_inclusive('\n');
    ensure!(
        lines.next().is_some_and(|l| l.trim() == "# wgtui:disabled"),
        "Uszkodzony blok wyłączonego peera"
    );
    while let Some(line) = lines.next() {
        if line.trim() == "# wgtui:end-disabled" {
            ensure!(lines.next().is_none(), "Dane po końcu wyłączonego peera");
            return Ok(result);
        }
        result.push_str(
            line.strip_prefix("# ")
                .ok_or_else(|| anyhow!("Uszkodzony komentarz wyłączonego peera"))?,
        );
    }
    bail!("Brak końca wyłączonego peera")
}

pub fn next_interface(names: &[String]) -> Result<String> {
    (1..=99)
        .map(|n| format!("wg01admin{n:02}"))
        .find(|n| !names.contains(n))
        .ok_or_else(|| anyhow!("Brak wolnej nazwy wg01adminXX"))
}

pub fn validate_routes(routes: &[IpNet]) -> Result<()> {
    if routes.iter().any(|n| n.prefix_len() == 0) {
        bail!("V1 obsługuje administrację i wskazane podsieci, bez trasy domyślnej /0");
    }
    Ok(())
}
