# Proxmox Infrastructure

Skrypty konfiguracji infrastruktury Proxmox VE.

## Konfiguracja storage

`disk-configuration.sh` konfiguruje dwa storage:

- `local`: `/var/lib/vz`, na ISO, kopie zapasowe, szablony i snippets.
- `local-zfs`: pula ZFS `data`, na dyski VM i systemy plików kontenerów.

Skrypt wymaga istniejącej, zdrowej puli `data` i zamontowanego `/var/lib/vz`.
Zastępuje cały `/etc/pve/storage.cfg`, wcześniej zapisując jego kopię, jeśli plik istnieje.
Przed ponownym użyciem należy sprawdzić, czy host nie ma innych storage wymagających zachowania.

Uruchomienie na właściwym hoście jako root:

```bash
bash disk-configuration.sh
```

Na hoście `pve-01` konfigurację sprawdzono po restarcie 22 października 2026 r.:
oba storage były aktywne, pula ZFS miała stan ONLINE, a macierze mdraid były zdrowe.

Oba storage współdzielą tę samą pulę. Kopie na `local` nie chronią przed utratą całego hosta.
