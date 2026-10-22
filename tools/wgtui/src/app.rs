use crate::{
    config::{Config, Peer, join_ips, next_interface, parse_ips, valid_interface},
    profile::{CLIENT_TEMPLATE, Profile, SecretQr, Settings},
    secret::{Secret, secret},
    store::{Change, Store, TunnelSpec},
    system::{InterfaceStats, PeerStats, ServiceAction},
};
use anyhow::{Result, anyhow, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{
    collections::BTreeMap,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub struct InterfaceView {
    pub name: String,
    pub raw: Option<Secret>,
    pub peers: Vec<Peer>,
    pub stats: Option<InterfaceStats>,
    pub autostart: bool,
    pub error: Option<String>,
}

pub struct Snapshot {
    pub interfaces: Vec<InterfaceView>,
    pub observed: Instant,
}

pub fn snapshot(store: &Store) -> Result<Snapshot> {
    let mut live = store.backend.observe()?;
    let mut names = store.names()?;
    for name in live.keys() {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    names.sort();
    let interfaces = names
        .into_iter()
        .map(|name| {
            let stats = live.remove(&name);
            let mut error = None;
            let mut raw = None;
            let mut peers = vec![];
            let mut autostart = false;
            if valid_interface(&name) {
                match store.read(&name).and_then(|r| {
                    let c = Config::parse(&r)?;
                    peers = c.peers();
                    if let Err(e) = c.editable() {
                        error = Some(e.to_string());
                    }
                    Ok(r)
                }) {
                    Ok(r) => {
                        raw = Some(r);
                        autostart = store.backend.autostart(&name).unwrap_or(false);
                    }
                    Err(e) => error = Some(e.to_string()),
                }
            } else {
                error = Some("Interfejs poza zakresem wgXXadminXX — tylko podgląd".into());
            }
            if let Some(stats) = &stats {
                for (key, stat) in &stats.peers {
                    if !peers.iter().any(|p| p.public_key == *key) {
                        peers.push(Peer {
                            name: String::new(),
                            public_key: key.clone(),
                            allowed_ips: stat.allowed_ips.clone(),
                            enabled: true,
                            keepalive: stat.keepalive,
                        });
                    }
                }
            }
            InterfaceView {
                name,
                raw,
                peers,
                stats,
                autostart,
                error,
            }
        })
        .collect();
    Ok(Snapshot {
        interfaces,
        observed: Instant::now(),
    })
}

pub fn monitor(store: Store) -> Receiver<Result<Snapshot>> {
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        loop {
            match tx.try_send(snapshot(&store)) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => break,
            }
            thread::sleep(Duration::from_secs(1));
        }
    });
    rx
}

#[derive(Clone, Copy, PartialEq)]
pub enum FormKind {
    AddPeer { generated: bool },
    EditPeer,
    Tunnel,
    Profile,
}
#[derive(Clone, Copy, PartialEq)]
pub enum FieldKind {
    Text,
    Bool,
    ReadOnly,
}
pub struct Field {
    pub label: &'static str,
    pub value: String,
    pub hint: &'static str,
    pub kind: FieldKind,
    pub cursor: usize,
}
impl Field {
    fn text(label: &'static str, value: impl Into<String>, hint: &'static str) -> Self {
        let value = value.into();
        let cursor = value.len();
        Self {
            label,
            value,
            hint,
            kind: FieldKind::Text,
            cursor,
        }
    }
    fn boolean(label: &'static str, hint: &'static str) -> Self {
        Self {
            label,
            value: "nie".into(),
            hint,
            kind: FieldKind::Bool,
            cursor: 0,
        }
    }
    pub fn is_true(&self) -> bool {
        self.value == "tak"
    }
}
pub struct Form {
    pub kind: FormKind,
    pub fields: Vec<Field>,
    pub focus: usize,
    pub old_key: Option<String>,
    pub error: Option<String>,
    pub interface: String,
    pub before: Option<Secret>,
}

impl Form {
    pub fn title(&self) -> &'static str {
        match self.kind {
            FormKind::AddPeer { generated: true } => "Nowy peer · jednorazowy QR",
            FormKind::AddPeer { generated: false } => "Nowy peer · klucz publiczny",
            FormKind::EditPeer => "Edytuj peera",
            FormKind::Tunnel => "Nowy tunel administracyjny",
            FormKind::Profile => "Ustawienia profilu klientów",
        }
    }
    pub fn advance(&mut self, delta: isize) {
        loop {
            self.focus =
                (self.focus as isize + delta).rem_euclid(self.fields.len() as isize) as usize;
            if self.fields[self.focus].kind != FieldKind::ReadOnly {
                break;
            }
        }
    }
    pub fn input(&mut self, key: KeyEvent) {
        self.error = None;
        if key.code == KeyCode::F(2) {
            if let FormKind::AddPeer { generated } = &mut self.kind {
                *generated = !*generated;
                self.fields[1].kind = if *generated {
                    FieldKind::ReadOnly
                } else {
                    FieldKind::Text
                };
                self.fields[1].value = if *generated {
                    "(generowany jednorazowo w pamięci)".into()
                } else {
                    String::new()
                };
                self.fields[1].cursor = self.fields[1].value.len();
                if self.fields[self.focus].kind == FieldKind::ReadOnly {
                    self.advance(1);
                }
            }
            return;
        }
        match key.code {
            KeyCode::Tab | KeyCode::Down | KeyCode::Enter => {
                self.advance(1);
                return;
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.advance(-1);
                return;
            }
            _ => {}
        }
        let f = &mut self.fields[self.focus];
        if f.kind == FieldKind::Bool {
            if matches!(
                key.code,
                KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right
            ) {
                f.value = if f.is_true() { "nie" } else { "tak" }.into();
            }
            return;
        }
        if f.kind != FieldKind::Text {
            return;
        }
        match key.code {
            KeyCode::Home => f.cursor = 0,
            KeyCode::End => f.cursor = f.value.len(),
            KeyCode::Left => {
                if f.cursor > 0 {
                    f.cursor = f.value[..f.cursor]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
            }
            KeyCode::Right => {
                if f.cursor < f.value.len() {
                    f.cursor += f.value[f.cursor..].chars().next().unwrap().len_utf8();
                }
            }
            KeyCode::Backspace => {
                if f.cursor > 0 {
                    let prev = f.value[..f.cursor].char_indices().last().unwrap().0;
                    f.value.replace_range(prev..f.cursor, "");
                    f.cursor = prev;
                }
            }
            KeyCode::Delete => {
                if f.cursor < f.value.len() {
                    let end = f.cursor + f.value[f.cursor..].chars().next().unwrap().len_utf8();
                    f.value.replace_range(f.cursor..end, "");
                }
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                f.value.clear();
                f.cursor = 0;
            }
            KeyCode::Char(c)
                if !c.is_control()
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && f.value.len() < 4096 =>
            {
                f.value.insert(f.cursor, c);
                f.cursor += c.len_utf8();
            }
            _ => {}
        }
    }
    pub fn paste(&mut self, text: &str) {
        if text.chars().any(char::is_control) {
            self.error = Some("Wklej jedną linię; wieloliniowe wartości są odrzucane".into());
            return;
        }
        let f = &mut self.fields[self.focus];
        if f.kind == FieldKind::Text && f.value.len() + text.len() <= 4096 {
            f.value.insert_str(f.cursor, text);
            f.cursor += text.len();
        }
    }
}

pub struct Delivery {
    pub content: Secret,
    pub qr: Option<SecretQr>,
    pub show_text: bool,
    pub scroll: u16,
}
pub struct Pending {
    pub change: Change,
    pub delivery: Option<Delivery>,
    pub preview: String,
    pub scroll: u16,
}
pub enum View {
    Main,
    Form(Form),
    Review(Pending),
    Busy,
    Delivery(Delivery),
    CloseDelivery(Delivery),
    Help,
    Details,
    Search,
    Message(String),
}

pub struct Completion {
    pub pending: Pending,
    pub error: Option<String>,
}

pub struct App {
    pub store: Option<Store>,
    pub settings: Settings,
    pub data: Snapshot,
    pub interface: usize,
    pub selected: usize,
    pub view: View,
    pub query: String,
    pub note: String,
    pub monitor_error: Option<String>,
    pub rates: BTreeMap<(String, String), (u64, u64)>,
    pub quit: bool,
    pub focus_interfaces: bool,
    pub clear_secret_screen: bool,
    pub completion: Option<Receiver<Completion>>,
    pub requested_interface: Option<String>,
    demo_counter: u8,
}

impl App {
    pub fn new(store: Option<Store>, settings: Settings) -> Self {
        let demo = store.is_none();
        Self {
            store,
            settings,
            data: if demo {
                demo_snapshot()
            } else {
                Snapshot {
                    interfaces: vec![],
                    observed: Instant::now(),
                }
            },
            interface: 0,
            selected: 0,
            view: View::Main,
            query: String::new(),
            note: if demo {
                "DEMO · bez zapisu i połączeń · profile służą tylko do prezentacji".into()
            } else {
                "Odczytywanie konfiguracji…".into()
            },
            monitor_error: None,
            rates: BTreeMap::new(),
            quit: false,
            focus_interfaces: false,
            clear_secret_screen: false,
            completion: None,
            requested_interface: None,
            demo_counter: 60,
        }
    }
    pub fn iface(&self) -> Option<&InterfaceView> {
        self.data.interfaces.get(self.interface)
    }
    pub fn filtered(&self) -> Vec<&Peer> {
        self.iface()
            .map(|i| {
                i.peers
                    .iter()
                    .filter(|p| {
                        self.query.is_empty()
                            || format!(
                                "{} {} {}",
                                p.label(),
                                p.public_key,
                                join_ips(&p.allowed_ips)
                            )
                            .to_lowercase()
                            .contains(&self.query.to_lowercase())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn peer(&self) -> Option<&Peer> {
        self.filtered().get(self.selected).copied()
    }
    pub fn update(&mut self, data: Snapshot) {
        let name = self.iface().map(|i| i.name.clone());
        let selected = self.peer().map(|p| p.public_key.clone());
        let elapsed = data
            .observed
            .duration_since(self.data.observed)
            .as_secs_f64()
            .max(0.1);
        self.rates.clear();
        for iface in &data.interfaces {
            if let Some(old) = self.data.interfaces.iter().find(|i| i.name == iface.name)
                && let (Some(a), Some(b)) = (&iface.stats, &old.stats)
            {
                for (key, stat) in &a.peers {
                    if let Some(prev) = b.peers.get(key) {
                        self.rates.insert(
                            (iface.name.clone(), key.clone()),
                            (
                                (stat.rx.saturating_sub(prev.rx) as f64 / elapsed) as u64,
                                (stat.tx.saturating_sub(prev.tx) as f64 / elapsed) as u64,
                            ),
                        );
                    }
                }
            }
        }
        self.data = data;
        let requested = self.requested_interface.take();
        if let Some(name) = requested.or(name) {
            self.interface = self
                .data
                .interfaces
                .iter()
                .position(|i| i.name == name)
                .unwrap_or(0);
        }
        if let Some(key) = selected {
            self.selected = self
                .filtered()
                .iter()
                .position(|p| p.public_key == key)
                .unwrap_or(0);
        }
        self.selected = self.selected.min(self.filtered().len().saturating_sub(1));
        self.monitor_error = None;
        if self.note == "Odczytywanie konfiguracji…" {
            self.note = "Odświeżanie co 1 s · wybierz peera lub utwórz tunel".into();
        }
    }
    fn editable(&self) -> Result<(&InterfaceView, Secret)> {
        ensure!(
            self.monitor_error.is_none(),
            "Monitoring jest nieaktualny; poczekaj na udany odczyt"
        );
        let iface = self
            .iface()
            .ok_or_else(|| anyhow!("Najpierw utwórz tunel (n)"))?;
        ensure!(
            iface.error.is_none(),
            "{}",
            iface.error.as_deref().unwrap_or("")
        );
        let raw = iface
            .raw
            .as_ref()
            .ok_or_else(|| anyhow!("Brak konfiguracji do edycji"))?;
        Config::parse(raw)?.editable()?;
        Ok((iface, secret(raw.as_str())))
    }
    fn current_profile(&self, name: &str) -> Result<Profile> {
        if let Some(store) = &self.store {
            store.profile(name)
        } else {
            Ok(Profile {
                endpoint: "vpn.example.org:51820".into(),
                dns: vec![],
                routes: vec![],
                keepalive: 25,
            })
        }
    }
    fn profile_fields(profile: &Profile) -> Vec<Field> {
        vec![
            Field::text(
                "Endpoint serwera",
                &profile.endpoint,
                "Publiczny adres DNS lub IP i port; IPv6 w []",
            ),
            Field::text(
                "DNS klientów",
                profile.dns.join(", "),
                "Opcjonalne adresy DNS, oddzielone przecinkami",
            ),
            Field::text(
                "Trasy klientów",
                join_ips(&profile.routes),
                "Puste: tylko adres serwera w tunelu; bez trasy /0",
            ),
            Field::text(
                "Keepalive klienta",
                profile.keepalive.to_string(),
                "0 = wyłączony; dla urządzeń za NAT zwykle 25 s",
            ),
        ]
    }

    fn peer_form(&self, edit: bool) -> Result<Form> {
        let (iface, raw) = self.editable()?;
        let config = Config::parse(&raw)?;
        let profile = self.current_profile(&iface.name)?;
        let (kind, fields, old_key) = if edit {
            let p = self.peer().ok_or_else(|| anyhow!("Wybierz peera"))?;
            (
                FormKind::EditPeer,
                vec![
                    Field::text("Nazwa", p.label(), "Opis urządzenia, do 64 znaków"),
                    Field::text(
                        "Adres klienta",
                        join_ips(&p.allowed_ips),
                        "Host /32, opcjonalnie IPv6 /128",
                    ),
                ],
                Some(p.public_key.clone()),
            )
        } else {
            let mut key = Field::text(
                "Klucz publiczny",
                "(generowany jednorazowo w pamięci)",
                "F2 przełącza między QR a wklejeniem klucza publicznego",
            );
            key.kind = FieldKind::ReadOnly;
            let mut fields = vec![
                Field::text("Nazwa", "", "Np. macbook-01 lub telefon-01"),
                key,
                Field::text(
                    "Adres klienta",
                    join_ips(&config.next_addresses()?),
                    "Proponowany wolny adres; także wyłączone peery rezerwują adres",
                ),
            ];
            fields.extend(Self::profile_fields(&profile));
            (FormKind::AddPeer { generated: true }, fields, None)
        };
        Ok(Form {
            kind,
            fields,
            focus: 0,
            old_key,
            error: None,
            interface: iface.name.clone(),
            before: Some(raw),
        })
    }
    fn tunnel_form(&self) -> Result<Form> {
        let name = next_interface(
            &self
                .data
                .interfaces
                .iter()
                .map(|i| i.name.clone())
                .collect::<Vec<_>>(),
        )?;
        let profile = if let Some(store) = &self.store {
            store.profile(&name)?
        } else {
            Profile {
                keepalive: 25,
                ..Profile::default()
            }
        };
        let mut fields = vec![
            Field::text(
                "Nazwa interfejsu",
                name,
                "wgXXadminXX; oba numery od 01 do 99",
            ),
            Field::text(
                "Adres serwera IPv4",
                "",
                "Adres z prefiksem puli, np. 10.90.0.1/24 — wybierz wolną sieć",
            ),
            Field::text(
                "Adres serwera IPv6",
                "",
                "Opcjonalny adres z prefiksem puli",
            ),
            Field::text("Port UDP", "51820", "Port nasłuchu tunelu"),
        ];
        fields.extend(Self::profile_fields(&profile));
        fields.push(Field::boolean(
            "Uruchom po zapisie",
            "Spacja przełącza; domyślnie tylko zapis",
        ));
        fields.push(Field::boolean(
            "Autostart systemd",
            "Spacja przełącza; domyślnie wyłączony",
        ));
        Ok(Form {
            kind: FormKind::Tunnel,
            fields,
            focus: 0,
            old_key: None,
            error: None,
            interface: String::new(),
            before: None,
        })
    }
    fn profile_form(&self) -> Result<Form> {
        let (iface, raw) = self.editable()?;
        Ok(Form {
            kind: FormKind::Profile,
            fields: Self::profile_fields(&self.current_profile(&iface.name)?),
            focus: 0,
            old_key: None,
            error: None,
            interface: iface.name.clone(),
            before: Some(raw),
        })
    }
    fn form_profile(form: &Form, offset: usize) -> Result<Profile> {
        let f = &form.fields;
        let p = Profile {
            endpoint: f[offset].value.trim().into(),
            dns: f[offset + 1]
                .value
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            routes: parse_ips(&f[offset + 2].value)?,
            keepalive: f[offset + 3]
                .value
                .trim()
                .parse()
                .map_err(|_| anyhow!("Keepalive musi być liczbą 0–65535"))?,
        };
        p.validate()?;
        Ok(p)
    }
    fn keypair(&mut self) -> Result<(Secret, String)> {
        if let Some(store) = &self.store {
            store.backend.keypair()
        } else {
            self.demo_counter = self.demo_counter.wrapping_add(1).max(60);
            Ok((
                secret(STANDARD.encode([7u8; 32])),
                STANDARD.encode([self.demo_counter; 32]),
            ))
        }
    }
    fn prepare(&mut self, form: &Form) -> Result<Pending> {
        let f = &form.fields;
        let mut delivery = None;
        let change = if form.kind == FormKind::Tunnel {
            let addresses = parse_ips(
                &[f[1].value.clone(), f[2].value.clone()]
                    .into_iter()
                    .filter(|s| !s.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join(","),
            )?;
            let spec = TunnelSpec {
                name: f[0].value.trim().into(),
                addresses,
                port: f[3]
                    .value
                    .trim()
                    .parse()
                    .map_err(|_| anyhow!("Port musi być liczbą 1–65535"))?,
                profile: Self::form_profile(form, 4)?,
                start: f[8].is_true(),
                autostart: f[9].is_true(),
            };
            if let Some(store) = &self.store {
                store.new_tunnel(spec)?
            } else {
                ensure!(
                    valid_interface(&spec.name),
                    "Nazwa musi odpowiadać wgXXadminXX (01–99)"
                );
                ensure!(
                    !self.data.interfaces.iter().any(|i| i.name == spec.name),
                    "Nazwa jest już zajęta"
                );
                ensure!(
                    !spec.addresses.is_empty() && spec.port != 0,
                    "Podaj adres serwera oraz port"
                );
                let (private, _) = self.keypair()?;
                Change {
                    interface: spec.name,
                    before: None,
                    after: secret(format!(
                        "[Interface]\nPrivateKey = {}\nAddress = {}\nListenPort = {}\nSaveConfig = false\n",
                        private.as_str(),
                        join_ips(&spec.addresses),
                        spec.port
                    )),
                    profile: Some(spec.profile),
                    start: spec.start,
                    autostart: spec.autostart,
                    service: None,
                }
            }
        } else {
            let before = secret(
                form.before
                    .as_ref()
                    .ok_or_else(|| anyhow!("Brak konfiguracji"))?
                    .as_str(),
            );
            let mut config = Config::parse(&before)?;
            let mut profile = None;
            match form.kind {
                FormKind::AddPeer { generated } => {
                    let p = Self::form_profile(form, 3)?;
                    let (private, public) = if generated {
                        let (k, pubkey) = self.keypair()?;
                        (Some(k), pubkey)
                    } else {
                        (None, f[1].value.trim().into())
                    };
                    let peer = Peer {
                        name: f[0].value.trim().into(),
                        public_key: public,
                        allowed_ips: parse_ips(&f[2].value)?,
                        enabled: true,
                        keepalive: 0,
                    };
                    config.validate_peer(&peer, None)?;
                    let server_public = if let Some(store) = &self.store {
                        let key = config.secret_value("PrivateKey").ok_or_else(|| {
                            anyhow!("Konfiguracja musi zawierać klucz serwera PrivateKey")
                        })?;
                        store.backend.public_key(&key)?
                    } else {
                        STANDARD.encode([42u8; 32])
                    };
                    let template = if let Some(store) = &self.store {
                        store.template()?
                    } else {
                        secret(CLIENT_TEMPLATE)
                    };
                    ensure!(
                        peer.public_key != server_public,
                        "Klucz klienta nie może być kluczem publicznym serwera"
                    );
                    let content = p.render(
                        &config,
                        &peer,
                        &server_public,
                        private.as_ref().map(|s| s.as_str()),
                        &template,
                    )?;
                    let qr = if generated {
                        Some(SecretQr::new(&content)?)
                    } else {
                        None
                    };
                    delivery = Some(Delivery {
                        content,
                        qr,
                        show_text: !generated,
                        scroll: 0,
                    });
                    config.add_peer(peer)?;
                    profile = Some(p);
                }
                FormKind::EditPeer => {
                    let key = form
                        .old_key
                        .as_deref()
                        .ok_or_else(|| anyhow!("Brak klucza peera"))?;
                    let mut peer = config
                        .peers()
                        .into_iter()
                        .find(|p| p.public_key == key)
                        .ok_or_else(|| anyhow!("Peer został usunięty"))?;
                    peer.name = f[0].value.trim().into();
                    peer.allowed_ips = parse_ips(&f[1].value)?;
                    config.update_peer(key, peer)?;
                }
                FormKind::Profile => profile = Some(Self::form_profile(form, 0)?),
                _ => unreachable!(),
            }
            Change {
                interface: form.interface.clone(),
                before: Some(before),
                after: config.render(),
                profile,
                start: false,
                autostart: false,
                service: None,
            }
        };
        let mut preview = change.preview();
        if let Some(p) = &change.profile {
            preview.push_str(&format!("\n\nPROFIL NASTĘPNYCH KLIENTÓW\nEndpoint: {}\nDNS: {}\nTrasy: {}\nKeepalive: {}\n\nDostęp do podsieci wymaga osobno routingu powrotnego, forwardingu i reguł zapory. Wgtui ich nie zmienia.",p.endpoint,p.dns.join(", "), if p.routes.is_empty() {"adres serwera w tunelu".into()} else {join_ips(&p.routes)},p.keepalive));
        }
        if let Some(d) = &delivery {
            preview.push_str(&format!(
                "\n\nPROFIL KLIENTA (sekrety ukryte)\n{}",
                crate::secret::redact(&d.content)
            ));
        }
        Ok(Pending {
            change,
            delivery,
            preview,
            scroll: 0,
        })
    }
    fn peer_change(&self, remove: bool) -> Result<Pending> {
        let (iface, before) = self.editable()?;
        let mut config = Config::parse(&before)?;
        let mut peer = self.peer().ok_or_else(|| anyhow!("Wybierz peera"))?.clone();
        if remove {
            config.remove_peer(&peer.public_key)?;
        } else {
            peer.enabled = !peer.enabled;
            config.update_peer(&peer.public_key, peer.clone())?;
        }
        let change = Change {
            interface: iface.name.clone(),
            before: Some(before),
            after: config.render(),
            profile: None,
            start: false,
            autostart: false,
            service: None,
        };
        let preview = format!(
            "{}: {}\n\n{}",
            if remove {
                "Usuń peera"
            } else {
                "Zmień stan peera"
            },
            peer.label(),
            change.preview()
        );
        Ok(Pending {
            change,
            delivery: None,
            preview,
            scroll: 0,
        })
    }
    fn service_pending(&self, autostart: bool) -> Result<Pending> {
        let (iface, raw) = self.editable()?;
        let action = if autostart {
            if iface.autostart {
                ServiceAction::Disable
            } else {
                ServiceAction::Enable
            }
        } else if iface.stats.is_some() {
            ServiceAction::Stop
        } else {
            ServiceAction::Start
        };
        let change = Change {
            interface: iface.name.clone(),
            after: secret(raw.as_str()),
            before: Some(raw),
            profile: None,
            start: false,
            autostart: false,
            service: Some(action),
        };
        let preview = change.preview();
        Ok(Pending {
            change,
            delivery: None,
            preview,
            scroll: 0,
        })
    }
    fn begin_apply(&mut self, pending: Pending) -> View {
        if let Some(store) = &self.store {
            let store = store.clone();
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let error = store.apply(&pending.change).err().map(|e| e.to_string());
                let _ = tx.send(Completion { pending, error });
            });
            self.completion = Some(rx);
            View::Busy
        } else {
            self.apply_demo(&pending.change);
            self.finish(Completion {
                pending,
                error: None,
            })
        }
    }
    fn apply_demo(&mut self, change: &Change) {
        if let Some(action) = change.service {
            if let Some(iface) = self
                .data
                .interfaces
                .iter_mut()
                .find(|i| i.name == change.interface)
            {
                match action {
                    ServiceAction::Start => {
                        iface.stats = Some(InterfaceStats {
                            public_key: STANDARD.encode([42u8; 32]),
                            listen_port: 51820,
                            peers: BTreeMap::new(),
                        })
                    }
                    ServiceAction::Stop => iface.stats = None,
                    ServiceAction::Enable => iface.autostart = true,
                    ServiceAction::Disable => iface.autostart = false,
                }
            }
            return;
        }
        let config = Config::parse(&change.after).expect("validated demo config");
        if let Some(iface) = self
            .data
            .interfaces
            .iter_mut()
            .find(|i| i.name == change.interface)
        {
            iface.raw = Some(secret(change.after.as_str()));
            iface.peers = config.peers();
        } else {
            self.data.interfaces.push(InterfaceView {
                name: change.interface.clone(),
                raw: Some(secret(change.after.as_str())),
                peers: config.peers(),
                stats: if change.start {
                    Some(InterfaceStats::default())
                } else {
                    None
                },
                autostart: change.autostart,
                error: None,
            });
        }
    }
    fn finish(&mut self, mut completion: Completion) -> View {
        if let Some(error) = completion.error {
            self.note = error;
            View::Review(completion.pending)
        } else {
            self.requested_interface = Some(completion.pending.change.interface.clone());
            self.note = if self.store.is_some() {
                "Zmiana zapisana. Monitoring odświeży stan za chwilę."
            } else {
                "DEMO · zmiana zasymulowana bez zapisu"
            }
            .into();
            completion
                .pending
                .delivery
                .take()
                .map(View::Delivery)
                .unwrap_or(View::Main)
        }
    }
    pub fn poll_completion(&mut self) {
        if let Some(rx) = &self.completion {
            match rx.try_recv() {
                Ok(result) => {
                    self.completion = None;
                    self.view = self.finish(result);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.completion = None;
                    self.view=View::Message("Wątek operacji zakończył się niespodziewanie. Sprawdź znacznik transakcji przed ponowną zmianą.".into());
                }
                _ => {}
            }
        }
    }
    pub fn key(&mut self, key: KeyEvent) {
        let view = std::mem::replace(&mut self.view, View::Main);
        self.view = match view {
            View::Form(mut form) => {
                if key.code == KeyCode::Esc {
                    View::Main
                } else if key.code == KeyCode::F(10)
                    || (key.code == KeyCode::Char('s')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    match self.prepare(&form) {
                        Ok(p) => View::Review(p),
                        Err(e) => {
                            form.error = Some(e.to_string());
                            View::Form(form)
                        }
                    }
                } else {
                    form.input(key);
                    View::Form(form)
                }
            }
            View::Review(mut pending) => match key.code {
                KeyCode::Enter => self.begin_apply(pending),
                KeyCode::Esc => View::Main,
                KeyCode::Down | KeyCode::Char('j') => {
                    pending.scroll = pending.scroll.saturating_add(1);
                    View::Review(pending)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    pending.scroll = pending.scroll.saturating_sub(1);
                    View::Review(pending)
                }
                KeyCode::PageDown => {
                    pending.scroll = pending.scroll.saturating_add(12);
                    View::Review(pending)
                }
                KeyCode::PageUp => {
                    pending.scroll = pending.scroll.saturating_sub(12);
                    View::Review(pending)
                }
                _ => View::Review(pending),
            },
            View::Delivery(mut d) => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    if d.qr.is_some() {
                        View::CloseDelivery(d)
                    } else {
                        self.clear_secret_screen = true;
                        View::Main
                    }
                }
                KeyCode::Char('t') => {
                    d.show_text = !d.show_text;
                    d.scroll = 0;
                    View::Delivery(d)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    d.scroll = d.scroll.saturating_add(1);
                    View::Delivery(d)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    d.scroll = d.scroll.saturating_sub(1);
                    View::Delivery(d)
                }
                _ => View::Delivery(d),
            },
            View::CloseDelivery(d) => match key.code {
                KeyCode::Enter => {
                    drop(d);
                    self.clear_secret_screen = true;
                    self.note="Profil zamknięty. Klucz klienta nie został zapisany; QR nie będzie dostępny ponownie.".into();
                    View::Main
                }
                KeyCode::Esc => View::Delivery(d),
                _ => View::CloseDelivery(d),
            },
            View::Busy => View::Busy,
            View::Help | View::Details | View::Message(_) => View::Main,
            View::Search => match key.code {
                KeyCode::Esc => {
                    self.query.clear();
                    self.selected = 0;
                    View::Main
                }
                KeyCode::Enter => View::Main,
                KeyCode::Backspace => {
                    self.query.pop();
                    self.selected = 0;
                    View::Search
                }
                KeyCode::Char(c) if !c.is_control() => {
                    self.query.push(c);
                    self.selected = 0;
                    View::Search
                }
                _ => View::Search,
            },
            View::Main => self.main_key(key),
        };
    }
    fn main_key(&mut self, key: KeyEvent) -> View {
        let result: Result<View> = (|| {
            match key.code {
                KeyCode::Char('q') => self.quit = true,
                KeyCode::Char('?') => return Ok(View::Help),
                KeyCode::Char('/') => return Ok(View::Search),
                KeyCode::Esc => {
                    self.query.clear();
                    self.selected = 0;
                }
                KeyCode::Tab => self.focus_interfaces = !self.focus_interfaces,
                KeyCode::Left | KeyCode::Char('[') => self.switch_interface(-1),
                KeyCode::Right | KeyCode::Char(']') => self.switch_interface(1),
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.focus_interfaces {
                        self.switch_interface(-1)
                    } else {
                        self.selected = self.selected.saturating_sub(1)
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.focus_interfaces {
                        self.switch_interface(1)
                    } else {
                        self.selected =
                            (self.selected + 1).min(self.filtered().len().saturating_sub(1))
                    }
                }
                KeyCode::Enter => return Ok(View::Details),
                KeyCode::Char('a') => return Ok(View::Form(self.peer_form(false)?)),
                KeyCode::Char('e') => return Ok(View::Form(self.peer_form(true)?)),
                KeyCode::Char('n') => return Ok(View::Form(self.tunnel_form()?)),
                KeyCode::Char('p') => return Ok(View::Form(self.profile_form()?)),
                KeyCode::Char(' ') => return Ok(View::Review(self.peer_change(false)?)),
                KeyCode::Char('d') => return Ok(View::Review(self.peer_change(true)?)),
                KeyCode::Char('s') => return Ok(View::Review(self.service_pending(false)?)),
                KeyCode::Char('b') => return Ok(View::Review(self.service_pending(true)?)),
                _ => {}
            }
            Ok(View::Main)
        })();
        result.unwrap_or_else(|e| View::Message(e.to_string()))
    }
    fn switch_interface(&mut self, delta: isize) {
        if !self.data.interfaces.is_empty() {
            self.interface = (self.interface as isize + delta)
                .rem_euclid(self.data.interfaces.len() as isize)
                as usize;
            self.selected = 0;
            self.query.clear();
        }
    }
    pub fn paste(&mut self, text: &str) {
        if let View::Form(form) = &mut self.view {
            form.paste(text);
        }
    }
    pub fn tick_demo(&mut self) {
        if self.store.is_some() {
            return;
        }
        if let Some(i) = self.data.interfaces.first_mut()
            && let Some(s) = &mut i.stats
            && let Some(p) = s.peers.values_mut().next()
        {
            p.rx += 1536;
            p.tx += 768;
            p.handshake = now().saturating_sub(17);
        }
    }
}

fn demo_snapshot() -> Snapshot {
    let key = |n: u8| STANDARD.encode([n; 32]);
    let raw = secret(format!(
        "# DEMO: fikcyjne klucze, nie używaj do połączeń\n[Interface]\nPrivateKey = {}\nAddress = 10.90.0.1/24\nListenPort = 51820\nSaveConfig = false\n\n[Peer]\n# wgtui:name = macbook-01\nPublicKey = {}\nAllowedIPs = 10.90.0.2/32\n\n[Peer]\n# wgtui:name = telefon-01\nPublicKey = {}\nAllowedIPs = 10.90.0.3/32\n\n[Peer]\n# wgtui:name = laptop-02\nPublicKey = {}\nAllowedIPs = 10.90.0.4/32\n",
        key(7),
        key(10),
        key(11),
        key(12)
    ));
    let mut config = Config::parse(&raw).unwrap();
    let mut disabled = config.peers()[2].clone();
    disabled.enabled = false;
    config
        .update_peer(&disabled.public_key.clone(), disabled)
        .unwrap();
    let mut stats = InterfaceStats {
        public_key: key(42),
        listen_port: 51820,
        peers: BTreeMap::new(),
    };
    stats.peers.insert(
        key(10),
        PeerStats {
            endpoint: "198.51.100.24:41289".into(),
            allowed_ips: parse_ips("10.90.0.2/32").unwrap(),
            handshake: now().saturating_sub(17),
            rx: 172_900_123,
            tx: 48_830_334,
            keepalive: 0,
        },
    );
    stats.peers.insert(
        key(11),
        PeerStats {
            endpoint: "203.0.113.72:60122".into(),
            allowed_ips: parse_ips("10.90.0.3/32").unwrap(),
            handshake: now().saturating_sub(842),
            rx: 19_330_201,
            tx: 8_200_411,
            keepalive: 0,
        },
    );
    Snapshot {
        interfaces: vec![InterfaceView {
            name: "wg01admin01".into(),
            raw: Some(config.render()),
            peers: config.peers(),
            stats: Some(stats),
            autostart: true,
            error: None,
        }],
        observed: Instant::now(),
    }
}
