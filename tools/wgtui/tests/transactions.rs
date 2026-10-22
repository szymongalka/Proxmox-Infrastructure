use anyhow::{Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    sync::{Arc, Mutex},
};
use tempfile::TempDir;
use wgtui::{
    config::{Config, Peer, parse_ips},
    secret::{Secret, secret},
    store::{Store, atomic_write},
    system::{Backend, InterfaceStats, Observation, ServiceAction},
};

#[derive(Default)]
struct Mock {
    runtime: Mutex<Option<Secret>>,
    fail_sync: Mutex<usize>,
    sync_calls: Mutex<usize>,
    fail_service: Mutex<usize>,
    service_calls: Mutex<Vec<ServiceAction>>,
}
impl Backend for Mock {
    fn observe(&self) -> Result<Observation> {
        let mut o = Observation::new();
        if self.runtime.lock().unwrap().is_some() {
            o.insert("wg01admin01".into(), InterfaceStats::default());
        }
        Ok(o)
    }
    fn runtime_config(&self, _: &str) -> Result<Secret> {
        Ok(secret(
            self.runtime.lock().unwrap().as_ref().unwrap().as_str(),
        ))
    }
    fn sync(&self, _: &str, c: &str) -> Result<()> {
        *self.sync_calls.lock().unwrap() += 1;
        let mut fail = self.fail_sync.lock().unwrap();
        if *fail > 0 {
            *fail -= 1;
            bail!("injected");
        }
        *self.runtime.lock().unwrap() = Some(secret(c));
        Ok(())
    }
    fn service(&self, _: &str, action: ServiceAction) -> Result<()> {
        self.service_calls.lock().unwrap().push(action);
        let mut fail = self.fail_service.lock().unwrap();
        if *fail > 0 {
            *fail -= 1;
            bail!("injected service error");
        }
        Ok(())
    }
    fn autostart(&self, _: &str) -> Result<bool> {
        Ok(false)
    }
    fn keypair(&self) -> Result<(Secret, String)> {
        Ok((secret(key(99)), key(12)))
    }
    fn public_key(&self, _: &str) -> Result<String> {
        Ok(key(42))
    }
}

#[test]
fn failed_service_restores_previous_state_and_clears_marker() {
    let (_dir, store, backend) = setup(true);
    *backend.fail_service.lock().unwrap() = 1;
    let change = store
        .service_change("wg01admin01", ServiceAction::Stop)
        .unwrap();
    assert!(
        store
            .apply(&change)
            .unwrap_err()
            .to_string()
            .contains("przywrócono")
    );
    assert_eq!(
        *backend.service_calls.lock().unwrap(),
        vec![ServiceAction::Stop, ServiceAction::Start]
    );
    assert!(
        !store
            .config_dir
            .join(".wgtui/transactions/wg01admin01.pending")
            .exists()
    );
}

#[test]
fn failed_start_cancels_job_even_without_a_visible_interface() {
    use wgtui::{profile::Profile, store::TunnelSpec};
    let (_dir, store, backend) = setup(false);
    let change = store
        .new_tunnel(TunnelSpec {
            name: "wg01admin02".into(),
            addresses: parse_ips("10.91.0.1/24").unwrap(),
            port: 51821,
            profile: Profile {
                endpoint: "vpn.example.org:51821".into(),
                keepalive: 25,
                ..Profile::default()
            },
            start: true,
            autostart: false,
        })
        .unwrap();
    *backend.fail_service.lock().unwrap() = 1;
    assert!(
        store
            .apply(&change)
            .unwrap_err()
            .to_string()
            .contains("przywrócono")
    );
    assert_eq!(
        *backend.service_calls.lock().unwrap(),
        vec![ServiceAction::Start, ServiceAction::Stop]
    );
    assert!(!store.config_dir.join("wg01admin02.conf").exists());
}
fn key(n: u8) -> String {
    STANDARD.encode([n; 32])
}
fn setup(running: bool) -> (TempDir, Store, Arc<Mock>) {
    let dir = TempDir::new().unwrap();
    let backend = Arc::new(Mock::default());
    let store = Store::open(
        dir.path().join("data"),
        dir.path().join("wg"),
        backend.clone(),
    )
    .unwrap();
    let text = format!(
        "[Interface]\nPrivateKey = {}\nAddress = 10.90.0.1/24\nListenPort = 51820\nSaveConfig = false\n",
        key(7)
    );
    atomic_write(&store.config_dir.join("wg01admin01.conf"), &text).unwrap();
    if running {
        *backend.runtime.lock().unwrap() = Some(Config::parse(&text).unwrap().stripped());
    }
    (dir, store, backend)
}
fn change(store: &Store) -> wgtui::store::Change {
    let before = store.read("wg01admin01").unwrap();
    let mut c = Config::parse(&before).unwrap();
    c.add_peer(Peer {
        name: "telefon-01".into(),
        public_key: key(11),
        allowed_ips: parse_ips("10.90.0.2/32").unwrap(),
        enabled: true,
        keepalive: 0,
    })
    .unwrap();
    store.change("wg01admin01", before, c.render()).unwrap()
}
#[test]
fn running_apply_updates_disk_and_runtime_with_private_permissions() {
    let (_dir, store, backend) = setup(true);
    let change = change(&store);
    store.apply(&change).unwrap();
    assert_eq!(*store.read("wg01admin01").unwrap(), *change.after);
    assert!(
        backend
            .runtime
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains(&key(11))
    );
    assert_eq!(
        fs::metadata(store.config_dir.join("wg01admin01.conf"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
#[test]
fn stopped_interface_only_writes_config() {
    let (_dir, store, backend) = setup(false);
    store.apply(&change(&store)).unwrap();
    assert_eq!(*backend.sync_calls.lock().unwrap(), 0);
}
#[test]
fn external_file_change_rejects_stale_review() {
    let (_dir, store, backend) = setup(true);
    let change = change(&store);
    let updated = format!(
        "{}\n# zmiana administratora\n",
        store.read("wg01admin01").unwrap().as_str()
    );
    atomic_write(&store.config_dir.join("wg01admin01.conf"), &updated).unwrap();
    assert!(
        store
            .apply(&change)
            .unwrap_err()
            .to_string()
            .contains("zmieniła")
    );
    assert_eq!(*backend.sync_calls.lock().unwrap(), 0);
}
#[test]
fn sync_error_rolls_back_disk_and_runtime() {
    let (_dir, store, backend) = setup(true);
    let before = store.read("wg01admin01").unwrap();
    *backend.fail_sync.lock().unwrap() = 1;
    assert!(
        store
            .apply(&change(&store))
            .unwrap_err()
            .to_string()
            .contains("przywrócono")
    );
    assert_eq!(*store.read("wg01admin01").unwrap(), *before);
    assert_eq!(*backend.sync_calls.lock().unwrap(), 2);
    assert!(
        !store
            .config_dir
            .join(".wgtui/transactions/wg01admin01.pending")
            .exists()
    );
}
#[test]
fn failed_rollback_leaves_marker_and_blocks_next_change() {
    let (_dir, store, backend) = setup(true);
    *backend.fail_sync.lock().unwrap() = 2;
    assert!(store.apply(&change(&store)).is_err());
    assert!(
        store
            .config_dir
            .join(".wgtui/transactions/wg01admin01.pending")
            .exists()
    );
    assert!(
        store
            .apply(&change(&store))
            .unwrap_err()
            .to_string()
            .contains("przerwana")
    );
}
#[test]
fn runtime_drift_blocks_write() {
    let (_dir, store, backend) = setup(true);
    let original = store.read("wg01admin01").unwrap();
    *backend.runtime.lock().unwrap() = Some(secret(original.replace("51820", "51821")));
    assert!(
        store
            .apply(&change(&store))
            .unwrap_err()
            .to_string()
            .contains("różnią")
    );
    assert_eq!(*store.read("wg01admin01").unwrap(), *original);
}
#[test]
fn second_instance_cannot_bypass_lock_using_different_data_directory() {
    let (dir, store, backend) = setup(false);
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(store.config_dir.join(".wgtui/locks/wg01admin01.lock"))
        .unwrap();
    lock.set_permissions(fs::Permissions::from_mode(0o600))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let other = Store::open(
        dir.path().join("other-data"),
        store.config_dir.clone(),
        backend,
    )
    .unwrap();
    assert!(
        other
            .apply(&change(&other))
            .unwrap_err()
            .to_string()
            .contains("Inna instancja")
    );
}
#[test]
fn unsafe_files_and_symlinks_are_rejected_without_overwriting_target() {
    let (dir, store, _) = setup(false);
    let path = store.config_dir.join("wg01admin01.conf");
    let target = dir.path().join("target");
    fs::write(&target, "zachowaj").unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&target, &path).unwrap();
    assert!(store.read("wg01admin01").is_err());
    assert!(atomic_write(&path, "nowe").is_err());
    assert_eq!(fs::read_to_string(target).unwrap(), "zachowaj");
}
#[test]
fn failed_atomic_write_does_not_change_configuration_or_runtime() {
    let (_dir, store, backend) = setup(true);
    let pending = change(&store);
    let original = store.read("wg01admin01").unwrap();
    // An unsafe journal destination causes failure before any configuration write.
    let journal = store
        .config_dir
        .join(".wgtui/transactions/wg01admin01.pending");
    symlink("/nonexistent-wgtui-target", &journal).unwrap();
    assert!(store.apply(&pending).is_err());
    assert_eq!(*store.read("wg01admin01").unwrap(), *original);
    assert_eq!(*backend.sync_calls.lock().unwrap(), 0);
}
#[test]
fn client_secret_never_occurs_in_data_files_or_preview() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use wgtui::{
        app::{App, View, snapshot},
        profile::Settings,
    };
    let (_dir, store, _) = setup(false);
    let secret_client = key(99);
    let mut app = App::new(Some(store.clone()), Settings::default());
    app.update(snapshot(&store).unwrap());
    let press = |app: &mut App, code| app.key(KeyEvent::new(code, KeyModifiers::NONE));
    press(&mut app, KeyCode::Char('a'));
    if let View::Form(f) = &mut app.view {
        f.fields[0].value = "telefon-01".into();
        f.fields[3].value = "vpn.example.org:51820".into();
    } else {
        panic!("missing form");
    }
    press(&mut app, KeyCode::F(10));
    if let View::Review(p) = &app.view {
        assert!(!p.preview.contains(&secret_client));
        assert!(!p.preview.contains(&key(7)));
    } else {
        panic!("missing review");
    }
    press(&mut app, KeyCode::Enter);
    for _ in 0..100 {
        app.poll_completion();
        if !matches!(app.view, View::Busy) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if let View::Delivery(d) = &app.view {
        assert!(d.content.contains(&secret_client));
    } else {
        panic!("missing delivery: {}", app.note);
    }
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.view, View::Main));
    fn scan(path: &std::path::Path, needle: &str) {
        for entry in fs::read_dir(path).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                scan(&p, needle)
            } else {
                assert!(!fs::read_to_string(p).unwrap().contains(needle));
            }
        }
    }
    scan(&store.data_dir, &secret_client);
    scan(&store.config_dir, &secret_client);
}
