#!/usr/bin/env bash
set -Eeuo pipefail

# Konfiguruje:
#   local     — pliki w /var/lib/vz (ISO, kopie zapasowe, szablony itp.)
#   local-zfs — obrazy VM i systemy plików kontenerów w puli ZFS data
#
# Nie formatuje dysków, nie tworzy puli i nie zmienia partycji.

CONFIG=/etc/pve/storage.cfg
POOL=data
MOUNTPOINT=/var/lib/vz

die() {
  printf 'Błąd: %s\n' "$*" >&2
  exit 1
}

[[ $EUID -eq 0 ]] || die "uruchom skrypt jako root"
command -v pvesm >/dev/null || die "nie znaleziono polecenia pvesm"
command -v zpool >/dev/null || die "nie znaleziono polecenia zpool"
command -v zfs >/dev/null || die "nie znaleziono polecenia zfs"

[[ -d /etc/pve ]] || die "katalog /etc/pve jest niedostępny"
mountpoint -q /etc/pve || die "system plików /etc/pve nie jest zamontowany"
[[ -d "$MOUNTPOINT" ]] || die "brakuje katalogu $MOUNTPOINT"
mountpoint -q "$MOUNTPOINT" || die "$MOUNTPOINT nie jest osobnym punktem montowania"

zpool list -H -o name "$POOL" >/dev/null 2>&1 ||
  die "nie znaleziono puli ZFS $POOL"

POOL_STATE=$(zpool list -H -o health "$POOL")
[[ "$POOL_STATE" == "ONLINE" ]] ||
  die "pula $POOL ma stan $POOL_STATE; oczekiwano ONLINE"

printf '%s\n' "Bieżący stan puli:"
zpool status "$POOL"

printf '\n%s\n' "Bieżące storage Proxmox:"
pvesm status || true

if [[ -e "$CONFIG" ]]; then
  BACKUP="${CONFIG}.backup.$(date +%Y%m%d-%H%M%S)"
  cp -a "$CONFIG" "$BACKUP"
  printf '\nKopia zapasowa zapisana w: %s\n' "$BACKUP"
fi

TMP=$(mktemp /etc/pve/storage.cfg.tmp.XXXXXX)
trap 'rm -f "$TMP"' EXIT

cat >"$TMP" <<'EOF'
dir: local
        path /var/lib/vz
        content iso,backup,vztmpl,snippets
        prune-backups keep-all=1

zfspool: local-zfs
        pool data
        content images,rootdir
        sparse 1
EOF

# /etc/pve jest klastrowym systemem plików.
# Podmieniamy plik atomowo.
mv "$TMP" "$CONFIG"
trap - EXIT

printf '\nNowa konfiguracja:\n'
cat "$CONFIG"

printf '\nWeryfikacja storage Proxmox:\n'
pvesm status

printf '\nWeryfikacja puli ZFS:\n'
zpool status "$POOL"
