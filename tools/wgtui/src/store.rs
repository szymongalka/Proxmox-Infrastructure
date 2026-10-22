use crate::{
    config::{Config, valid_interface, validate_routes},
    profile::{CLIENT_TEMPLATE, Profile, Settings},
    secret::{Secret, redact, secret},
    system::{Backend, ServiceAction},
};
use anyhow::{Result, anyhow, bail, ensure};
use fs2::FileExt;
use ipnet::IpNet;
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};
use tempfile::NamedTempFile;

pub struct Change {
    pub interface: String,
    pub before: Option<Secret>,
    pub after: Secret,
    pub profile: Option<Profile>,
    pub start: bool,
    pub autostart: bool,
    pub service: Option<ServiceAction>,
}

impl Change {
    pub fn preview(&self) -> String {
        if let Some(action) = self.service {
            let action = match action {
                ServiceAction::Start => "Uruchom tunel",
                ServiceAction::Stop => "Zatrzymaj tunel",
                ServiceAction::Enable => "Włącz autostart",
                ServiceAction::Disable => "Wyłącz autostart",
            };
            return format!(
                "Interfejs: {}\nOperacja: {action}\n\nZatrzymanie tunelu może przerwać sesję SSH prowadzoną przez ten tunel.",
                self.interface
            );
        }
        let before = self
            .before
            .as_ref()
            .map(|s| redact(s))
            .unwrap_or_else(|| "(nowy tunel)".into());
        format!(
            "PRZED\n{before}\n\nPO ZMIANIE\n{}\n\nUruchomienie: {}   Autostart nowego tunelu: {}",
            redact(&self.after),
            self.start,
            self.autostart
        )
    }
}

#[derive(Clone)]
pub struct Store {
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    pub backend: Arc<dyn Backend>,
}

pub struct TunnelSpec {
    pub name: String,
    pub addresses: Vec<IpNet>,
    pub port: u16,
    pub profile: Profile,
    pub start: bool,
    pub autostart: bool,
}

impl Store {
    pub fn open(data: PathBuf, configs: PathBuf, backend: Arc<dyn Backend>) -> Result<Self> {
        secure_dir(&data)?;
        secure_dir(&configs)?;
        for sub in ["templates", "interfaces"] {
            secure_dir(&data.join(sub))?;
        }
        secure_dir(&configs.join(".wgtui"))?;
        for sub in ["locks", "transactions"] {
            secure_dir(&configs.join(".wgtui").join(sub))?;
        }
        let store = Self {
            data_dir: fs::canonicalize(data)?,
            config_dir: fs::canonicalize(configs)?,
            backend,
        };
        for (name, contents) in [
            ("settings.toml", include_str!("../templates/settings.toml")),
            (
                "templates/profile.toml",
                include_str!("../templates/profile.toml"),
            ),
            ("templates/client.conf.tpl", CLIENT_TEMPLATE),
        ] {
            let path = store.data_dir.join(name);
            if !path.try_exists()? {
                atomic_write(&path, contents)?;
            }
        }
        Ok(store)
    }

    pub fn settings(&self) -> Result<Settings> {
        let settings: Settings = toml::from_str(&read_protected(
            &self.data_dir.join("settings.toml"),
        )?)
        .map_err(|_| anyhow!("Niepoprawny settings.toml; szczegóły ukryto, aby chronić sekrety"))?;
        ensure!(
            (1..=86400).contains(&settings.active_seconds),
            "active_seconds musi mieścić się w zakresie 1–86400"
        );
        Ok(settings)
    }

    pub fn template(&self) -> Result<Secret> {
        read_protected(&self.data_dir.join("templates/client.conf.tpl"))
    }

    pub fn names(&self) -> Result<Vec<String>> {
        let mut names = vec![];
        for entry in fs::read_dir(&self.config_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(name) = name.strip_suffix(".conf").filter(|n| valid_interface(n)) {
                names.push(name.to_string());
            }
        }
        names.sort();
        Ok(names)
    }

    fn config_path(&self, name: &str) -> Result<PathBuf> {
        ensure!(
            valid_interface(name),
            "Edycja wymaga nazwy wgXXadminXX z numerami 01–99"
        );
        Ok(self.config_dir.join(format!("{name}.conf")))
    }
    fn profile_path(&self, name: &str) -> PathBuf {
        self.data_dir
            .join("interfaces")
            .join(format!("{name}.toml"))
    }
    fn journal_path(&self, name: &str) -> PathBuf {
        self.config_dir
            .join(".wgtui/transactions")
            .join(format!("{name}.pending"))
    }

    pub fn read(&self, name: &str) -> Result<Secret> {
        read_protected(&self.config_path(name)?)
    }
    pub fn profile(&self, name: &str) -> Result<Profile> {
        ensure!(valid_interface(name), "Niedozwolona nazwa interfejsu");
        let path = self.profile_path(name);
        let path = if path.try_exists()? {
            path
        } else {
            self.data_dir.join("templates/profile.toml")
        };
        toml::from_str(&read_protected(&path)?)
            .map_err(|_| anyhow!("Niepoprawny profil TOML; szczegóły ukryto, aby chronić sekrety"))
    }

    pub fn change(&self, name: &str, before: Secret, after: Secret) -> Result<Change> {
        self.config_path(name)?;
        Config::parse(&before)?.editable()?;
        Config::parse(&after)?.editable()?;
        Ok(Change {
            interface: name.into(),
            before: Some(before),
            after,
            profile: None,
            start: false,
            autostart: false,
            service: None,
        })
    }

    pub fn service_change(&self, name: &str, action: ServiceAction) -> Result<Change> {
        let before = self.read(name)?;
        Config::parse(&before)?.editable()?;
        Ok(Change {
            interface: name.into(),
            after: secret(before.as_str()),
            before: Some(before),
            profile: None,
            start: false,
            autostart: false,
            service: Some(action),
        })
    }

    pub fn new_tunnel(&self, spec: TunnelSpec) -> Result<Change> {
        ensure!(
            !self.config_path(&spec.name)?.try_exists()?,
            "Konfiguracja o tej nazwie już istnieje"
        );
        ensure!(
            !self.backend.observe()?.contains_key(&spec.name),
            "Interfejs o tej nazwie już istnieje"
        );
        ensure!(
            !spec.addresses.is_empty() && spec.addresses.iter().any(|n| n.addr().is_ipv4()),
            "Podaj adres serwera IPv4 z prefiksem puli"
        );
        ensure!(spec.port != 0, "Port nie może być zerowy");
        let mut families = BTreeSet::new();
        for addr in &spec.addresses {
            ensure!(
                families.insert(addr.addr().is_ipv4()),
                "Podaj najwyżej jeden adres IPv4 i jeden IPv6"
            );
            ensure!(
                addr.prefix_len() < if addr.addr().is_ipv4() { 32 } else { 128 },
                "Pula musi pomieścić serwer i klientów"
            );
            if let IpNet::V4(n) = addr {
                ensure!(
                    n.addr() != n.network() && n.addr() != n.broadcast(),
                    "Adres serwera nie może być adresem sieci ani broadcast"
                );
            }
        }
        spec.profile.validate()?;
        validate_routes(&spec.profile.routes)?;
        for name in self.names()? {
            let other = Config::parse(&self.read(&name)?)?;
            ensure!(
                !other.addresses()?.iter().any(|a| spec
                    .addresses
                    .iter()
                    .any(|b| crate::config::overlaps(*a, *b))),
                "Pula nakłada się na konfigurację innego tunelu (także zatrzymanego)"
            );
        }
        let live = self.backend.observe()?;
        ensure!(
            !live.values().any(|i| i.listen_port == spec.port),
            "Port jest używany przez inny tunel WireGuard"
        );
        // Check all local interface routes before offering a new connected network.
        #[cfg(target_os = "linux")]
        {
            let routes = crate::system::run(
                "/usr/sbin/ip",
                &["-o", "route", "show", "table", "all"],
                None,
            )?;
            let routes6 = crate::system::run(
                "/usr/sbin/ip",
                &["-6", "-o", "route", "show", "table", "all"],
                None,
            )?;
            for line in routes.lines().chain(routes6.lines()) {
                for token in line.split_whitespace().take(2) {
                    if let Ok(route) = token.parse::<IpNet>() {
                        ensure!(
                            route.prefix_len() == 0
                                || !spec
                                    .addresses
                                    .iter()
                                    .any(|a| crate::config::overlaps(*a, route)),
                            "Pula tunelu nakłada się na istniejącą trasę systemu"
                        );
                    }
                }
            }
        }
        let (private, _) = self.backend.keypair()?;
        let after = secret(format!(
            "# Zarządzanie peerami: wgtui\n[Interface]\nPrivateKey = {}\nAddress = {}\nListenPort = {}\nSaveConfig = false\n",
            private.as_str(),
            crate::config::join_ips(&spec.addresses),
            spec.port
        ));
        Ok(Change {
            interface: spec.name,
            before: None,
            after,
            profile: Some(spec.profile),
            start: spec.start,
            autostart: spec.autostart,
            service: None,
        })
    }

    fn lock(&self, name: &str) -> Result<File> {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(
                self.config_dir
                    .join(".wgtui/locks")
                    .join(format!("{name}.lock")),
            )?;
        ensure!(
            lock.metadata()?.uid() == unsafe { libc::geteuid() }
                && lock.metadata()?.mode() & 0o077 == 0,
            "Niebezpieczne uprawnienia blokady"
        );
        lock.try_lock_exclusive()
            .map_err(|_| anyhow!("Inna instancja wgtui zmienia ten interfejs"))?;
        Ok(lock)
    }

    pub fn apply(&self, change: &Change) -> Result<()> {
        let path = self.config_path(&change.interface)?;
        let _lock = self.lock(&change.interface)?;
        let _allocation_lock = if change.before.is_none() {
            Some(self.lock("new-tunnel")?)
        } else {
            None
        };
        ensure!(
            !self.journal_path(&change.interface).try_exists()?,
            "Poprzednia operacja została przerwana. Uzgodnij plik i stan interfejsu, a następnie usuń znacznik transactions/*.pending (instrukcja w README)"
        );
        let current = if path.try_exists()? {
            Some(read_protected(&path)?)
        } else {
            None
        };
        ensure!(
            current == change.before,
            "Konfiguracja zmieniła się od czasu podglądu. Otwórz formularz ponownie"
        );
        let after = Config::parse(&change.after)?;
        after.editable()?;
        if change.before.is_none() {
            let networks = after.addresses()?;
            for name in self.names()? {
                let other = Config::parse(&self.read(&name)?)?;
                ensure!(
                    !other
                        .addresses()?
                        .iter()
                        .any(|a| networks.iter().any(|b| crate::config::overlaps(*a, *b))),
                    "Inny tunel zajął pulę od czasu podglądu. Otwórz kreator ponownie"
                );
            }
        }
        let running = self.backend.observe()?.contains_key(&change.interface);
        let runtime = if running {
            let runtime = self.backend.runtime_config(&change.interface)?;
            let before = change
                .before
                .as_ref()
                .ok_or_else(|| anyhow!("Nazwa interfejsu została zajęta"))?;
            ensure!(
                equivalent(&Config::parse(before)?, &Config::parse(&runtime)?),
                "Plik i aktywny interfejs różnią się. Uzgodnij je poza wgtui przed edycją; learned endpoint nie jest traktowany jako konflikt"
            );
            Some(runtime)
        } else {
            None
        };
        if let Some(action) = change.service {
            let was_enabled = self.backend.autostart(&change.interface)?;
            atomic_write(
                &self.journal_path(&change.interface),
                "Przerwana operacja systemd. Sprawdź stan usługi przed usunięciem znacznika.\n",
            )?;
            match self.backend.service(&change.interface, action) {
                Ok(()) => {
                    remove_sync(&self.journal_path(&change.interface))?;
                    return Ok(());
                }
                Err(_) => {
                    let restore_action = match action {
                        ServiceAction::Start | ServiceAction::Stop => {
                            if running {
                                ServiceAction::Start
                            } else {
                                ServiceAction::Stop
                            }
                        }
                        ServiceAction::Enable | ServiceAction::Disable => {
                            if was_enabled {
                                ServiceAction::Enable
                            } else {
                                ServiceAction::Disable
                            }
                        }
                    };
                    let mut restored = self
                        .backend
                        .service(&change.interface, restore_action)
                        .is_ok();
                    if matches!(action, ServiceAction::Start | ServiceAction::Stop)
                        && let Some(runtime) = &runtime
                    {
                        restored &= self.backend.sync(&change.interface, runtime).is_ok();
                    }
                    if restored {
                        remove_sync(&self.journal_path(&change.interface))?;
                        bail!("Operacja systemd nie powiodła się; przywrócono poprzedni stan");
                    }
                    bail!(
                        "Operacja systemd i przywrócenie nie powiodły się. Znacznik pozostawiono: sprawdź stan usługi przed kolejną zmianą"
                    );
                }
            }
        }
        let profile_path = self.profile_path(&change.interface);
        let old_profile = if profile_path.try_exists()? {
            Some(read_protected(&profile_path)?)
        } else {
            None
        };
        let new_profile = change
            .profile
            .as_ref()
            .map(toml::to_string)
            .transpose()
            .map_err(|_| anyhow!("Nie można zapisać profilu"))?;
        atomic_write(
            &self.journal_path(&change.interface),
            "Przerwana zmiana konfiguracji. Sprawdź plik i stan WireGuarda przed usunięciem znacznika. Ten plik nie zawiera kluczy.\n",
        )?;
        let mut started = false;
        let mut enabled = false;
        let mut wrote_config = false;
        let result = (|| -> Result<()> {
            wrote_config = true;
            atomic_write(&path, &change.after)?;
            if let Some(profile) = &new_profile {
                atomic_write(&profile_path, profile)?;
            }
            if running {
                self.backend.sync(&change.interface, &after.stripped())?;
                ensure!(
                    equivalent(
                        &after,
                        &Config::parse(&self.backend.runtime_config(&change.interface)?)?
                    ),
                    "Weryfikacja aktywnej konfiguracji nie powiodła się"
                );
            } else if change.start {
                started = true;
                self.backend
                    .service(&change.interface, ServiceAction::Start)?;
            }
            if change.before.is_none() && change.autostart {
                enabled = true;
                self.backend
                    .service(&change.interface, ServiceAction::Enable)?;
            }
            Ok(())
        })();
        if result.is_err() {
            let mut rollback_ok = true;
            if enabled {
                rollback_ok &= self
                    .backend
                    .service(&change.interface, ServiceAction::Disable)
                    .is_ok();
            }
            if started {
                // A queued start job must be cancelled even if no interface is visible.
                rollback_ok &= self
                    .backend
                    .service(&change.interface, ServiceAction::Stop)
                    .is_ok();
            }
            if wrote_config {
                rollback_ok &= restore(&path, change.before.as_deref().map(|v| v.as_str())).is_ok();
                rollback_ok &=
                    restore(&profile_path, old_profile.as_deref().map(|v| v.as_str())).is_ok();
                if let Some(runtime) = &runtime {
                    rollback_ok &= self.backend.sync(&change.interface, runtime).is_ok();
                }
            }
            if rollback_ok {
                remove_sync(&self.journal_path(&change.interface))?;
                bail!(
                    "Zmiana nie powiodła się; przywrócono poprzednią konfigurację i stan. Sprawdź uprawnienia, endpoint i dostępność narzędzi"
                );
            }
            bail!(
                "Zmiana i przywrócenie stanu nie powiodły się. Edycja zablokowana przez znacznik transactions/*.pending; wymagane ręczne uzgodnienie stanu"
            );
        }
        remove_sync(&self.journal_path(&change.interface))?;
        Ok(())
    }
}

/// Compare configured peer identity/address/keepalive and listen port. A roaming
/// endpoint and handshake/traffic counters are deliberately not part of equality.
pub fn equivalent(a: &Config, b: &Config) -> bool {
    a.same_runtime_as(b)
}

pub fn secure_dir(path: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        for ancestor in absolute.ancestors() {
            if let Ok(meta) = fs::symlink_metadata(ancestor) {
                let sticky_root = meta.uid() == 0 && meta.mode() & libc::S_ISVTX != 0;
                ensure!(
                    meta.is_dir()
                        && !meta.file_type().is_symlink()
                        && (meta.uid() == 0 || meta.uid() == unsafe { libc::geteuid() })
                        && (meta.mode() & 0o022 == 0 || sticky_root),
                    "Katalog nadrzędny jest niezaufany: {}",
                    ancestor.display()
                );
            }
        }
    }
    if !path.try_exists()? {
        fs::create_dir_all(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir()
            && !meta.file_type().is_symlink()
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o022 == 0,
        "Katalog musi należeć do bieżącego użytkownika, bez dowiązań i uprawnień zapisu grupy/innych"
    );
    Ok(())
}

pub fn read_protected(path: &Path) -> Result<Secret> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| anyhow!("Nie można odczytać chronionego pliku {}", path.display()))?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0
            && meta.nlink() == 1,
        "Plik {} musi być zwykłym plikiem właściciela z uprawnieniami 0600, bez dowiązań",
        path.display()
    );
    ensure!(meta.len() <= 1024 * 1024, "Plik jest zbyt duży");
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    (&mut file).take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "Plik jest zbyt duży");
    Ok(secret(
        std::str::from_utf8(&bytes).map_err(|_| anyhow!("Plik nie jest tekstem UTF-8"))?,
    ))
}

pub fn atomic_write(path: &Path, contents: &str) -> Result<()> {
    if path.symlink_metadata().is_ok() {
        let _ = read_protected(path)?;
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("Nieprawidłowa ścieżka pliku"))?;
    secure_dir(parent)?;
    let mut temp = NamedTempFile::new_in(parent)?;
    temp.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    temp.write_all(contents.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|_| anyhow!("Nie można atomowo zapisać konfiguracji"))?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn remove_sync(path: &Path) -> Result<()> {
    if path.try_exists()? {
        fs::remove_file(path)?;
        File::open(path.parent().unwrap())?.sync_all()?;
    }
    Ok(())
}

fn restore(path: &Path, contents: Option<&str>) -> Result<()> {
    if let Some(contents) = contents {
        atomic_write(path, contents)
    } else {
        remove_sync(path)
    }
}
