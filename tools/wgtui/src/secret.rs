//! Secret values deliberately do not implement Debug or Serialize.
use anyhow::{Result, bail};
use zeroize::{Zeroize, Zeroizing};

pub struct Secret(Zeroizing<String>);
impl std::ops::Deref for Secret {
    type Target = String;
    fn deref(&self) -> &String {
        &self.0
    }
}
impl std::ops::DerefMut for Secret {
    fn deref_mut(&mut self) -> &mut String {
        &mut self.0
    }
}
impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for Secret {}

pub fn secret(value: impl Into<String>) -> Secret {
    Secret(Zeroizing::new(value.into()))
}

/// Never include raw child stderr, config lines or TOML parser errors in the UI.
pub fn redact(text: &str) -> String {
    text.lines()
        .map(|line| {
            let mut trimmed = line.trim_start();
            while let Some(rest) = trimmed.strip_prefix('#') {
                trimmed = rest.trim_start();
            }
            if let Some((key, _)) = trimmed.split_once('=')
                && [
                    "privatekey",
                    "presharedkey",
                    "preup",
                    "postup",
                    "predown",
                    "postdown",
                ]
                .contains(&key.trim().to_ascii_lowercase().as_str())
            {
                return format!("{} = <ukryty>", key.trim());
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn single_line(value: &str, label: &str) -> Result<()> {
    if value.chars().any(|c| c.is_control()) || value.contains('#') {
        bail!("{label}: niedozwolony znak sterujący lub #");
    }
    Ok(())
}

pub fn protect_process() -> Result<()> {
    #[cfg(target_os = "linux")]
    unsafe {
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::setrlimit(libc::RLIMIT_CORE, &limit) != 0
            || libc::prctl(libc::PR_SET_DUMPABLE, 0) != 0
        {
            bail!("Nie można wyłączyć zrzutów pamięci procesu");
        }
    }
    // Do not inherit group/world permissions for any newly created files.
    unsafe {
        libc::umask(0o077);
    }
    Ok(())
}

pub fn wipe_bytes(bytes: &mut Vec<u8>) {
    bytes.zeroize();
}
