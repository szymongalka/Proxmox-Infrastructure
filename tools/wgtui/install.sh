#!/bin/bash
# Local installer for trusted, unpacked wgtui release packages.
set -euo pipefail
umask 077

wgtui_install_dir=/opt/custom-tools/wgtui
wgtui_link=/usr/local/bin/wgtui
wgtui_skip_packages=0
wgtui_stage=

fail() { printf 'wgtui: %s\n' "$*" >&2; exit 1; }
usage() {
    cat <<'HELP'
Instalacja lub aktualizacja wgtui na Debianie, Ubuntu i Proxmoxie.

  sudo ./install.sh
  sudo ./install.sh --skip-packages

  --skip-packages  Sprawdź zależności bez uruchamiania apt-get.
  -h, --help       Pokaż tę pomoc.

Instalator dobiera binarkę x86_64/ARM64 z lokalnej paczki, weryfikuje
SHA256SUMS i instaluje ją w /opt/custom-tools/wgtui/bin/wgtui.
Istniejące ustawienia, szablony i dane interfejsów są zachowywane.
Tunele, klucze, zapora, routing i autostart usług nie są zmieniane.
HELP
}
for wgtui_arg in "$@"; do
    case "$wgtui_arg" in
        --skip-packages) wgtui_skip_packages=1 ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; fail "Nieznana opcja: $wgtui_arg" ;;
    esac
done

[[ $(uname -s) == Linux ]] || fail 'Instalacja wymaga Linuksa. Na Macu możesz uruchomić demo ze źródeł.'
[[ $EUID -eq 0 ]] || fail 'Uruchom instalator przez sudo: sudo ./install.sh'
case $(uname -m) in
    x86_64|amd64) wgtui_arch=amd64; wgtui_machine=3e00 ;;
    aarch64|arm64) wgtui_arch=arm64; wgtui_machine=b700 ;;
    *) fail 'Obsługiwane architektury: x86_64 i ARM64.' ;;
esac
[[ -r /etc/os-release ]] || fail 'Nie można rozpoznać dystrybucji Linuksa.'
# This is the operating system's own root-controlled metadata.
# shellcheck disable=SC1091
. /etc/os-release
case " ${ID:-} ${ID_LIKE:-} " in
    *' debian '*|*' ubuntu '*) ;;
    *) fail 'Obsługiwane systemy: Debian, Ubuntu i Proxmox.' ;;
esac
for wgtui_command in sha256sum od tr awk stat install mkdir mktemp mv rm ln readlink flock dirname grep chmod dpkg-query; do
    command -v "$wgtui_command" >/dev/null || fail "Brakuje polecenia: $wgtui_command"
done
[[ -x /usr/bin/systemctl ]] || fail 'Aplikacja wymaga systemd (brak /usr/bin/systemctl).'

wgtui_script_dir=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
wgtui_payload=
for wgtui_candidate in "$wgtui_script_dir" "$wgtui_script_dir/dist/wgtui-linux" "$wgtui_script_dir/dist/wgtui-linux-$wgtui_arch"; do
    if [[ -f "$wgtui_candidate/bin/$wgtui_arch/wgtui" ]]; then
        wgtui_payload=$wgtui_candidate; wgtui_binary_rel=bin/$wgtui_arch/wgtui; break
    elif [[ -f "$wgtui_candidate/bin/wgtui" ]]; then
        wgtui_payload=$wgtui_candidate; wgtui_binary_rel=bin/wgtui; break
    fi
done
[[ -n "$wgtui_payload" ]] || fail 'Nie znaleziono lokalnej binarki. Rozpakuj pełną paczkę wgtui-linux.tar.gz i uruchom jej install.sh.'
[[ -f "$wgtui_payload/SHA256SUMS" && ! -L "$wgtui_payload/SHA256SUMS" ]] || fail 'Brak manifestu SHA256SUMS. Użyj nowej, kompletnej paczki instalacyjnej.'

checksum_for() {
    local wgtui_hash
    wgtui_hash=$(awk -v name="$1" '$2 == name { print $1; found++ } END { if (found != 1) exit 1 }' "$wgtui_payload/SHA256SUMS") || return 1
    [[ $wgtui_hash =~ ^[0-9a-f]{64}$ ]] || return 1
    printf '%s\n' "$wgtui_hash"
}
printf '[1/4] Weryfikacja paczki dla %s…\n' "$wgtui_arch"
for wgtui_file in "$wgtui_binary_rel" templates/settings.toml templates/profile.toml templates/client.conf.tpl install.sh; do
    [[ -f "$wgtui_payload/$wgtui_file" && ! -L "$wgtui_payload/$wgtui_file" ]] || fail "Brak zwykłego pliku w paczce: $wgtui_file"
    checksum_for "$wgtui_file" >/dev/null || fail "Niepełny manifest: $wgtui_file"
done
(cd -- "$wgtui_payload" && sha256sum --check --status SHA256SUMS) || fail 'Suma SHA-256 nie zgadza się z manifestem. Instalacja przerwana przed zmianami.'
wgtui_binary=$wgtui_payload/$wgtui_binary_rel
[[ $(od -An -v -tx1 -N6 "$wgtui_binary" | tr -d ' \n') == 7f454c460201 ]] || fail 'Binarka nie jest plikiem ELF64 little-endian dla Linuksa.'
[[ $(od -An -v -tx1 -j18 -N2 "$wgtui_binary" | tr -d ' \n') == "$wgtui_machine" ]] || fail "Binarka ma inną architekturę niż ten host ($wgtui_arch). Użyj paczki uniwersalnej."
wgtui_expected_binary_hash=$(checksum_for "$wgtui_binary_rel")

safe_dir() {
    local wgtui_path=$1 wgtui_mode
    [[ ! -L $wgtui_path ]] || fail "Katalog nie może być dowiązaniem: $wgtui_path"
    if [[ -e $wgtui_path ]]; then
        [[ -d $wgtui_path && $(stat -c %u -- "$wgtui_path") == 0 ]] || fail "Katalog musi należeć do root: $wgtui_path"
        wgtui_mode=$(stat -c %a -- "$wgtui_path")
        (( (8#$wgtui_mode & 0022) == 0 )) || fail "Katalog jest zapisywalny przez grupę lub innych: $wgtui_path"
    fi
}
safe_file() {
    local wgtui_path=$1
    [[ ! -L $wgtui_path ]] || fail "Plik nie może być dowiązaniem: $wgtui_path"
    if [[ -e $wgtui_path ]]; then
        [[ -f $wgtui_path && $(stat -c %u -- "$wgtui_path") == 0 && $(stat -c %h -- "$wgtui_path") == 1 ]] || fail "Oczekiwany zwykły plik root bez dodatkowych dowiązań: $wgtui_path"
    fi
}
check_link() {
    if [[ -L $wgtui_link ]]; then
        [[ $(readlink -- "$wgtui_link") == "$wgtui_install_dir/bin/wgtui" || $(readlink -f -- "$wgtui_link") == "$wgtui_install_dir/bin/wgtui" ]] || fail "Istnieje inne dowiązanie $wgtui_link. Nie zostało nadpisane."
    elif [[ -e $wgtui_link ]]; then
        fail "Istnieje inny plik $wgtui_link. Nie został nadpisany."
    fi
}
for wgtui_dir in /opt /opt/custom-tools "$wgtui_install_dir" "$wgtui_install_dir/bin" "$wgtui_install_dir/templates" "$wgtui_install_dir/interfaces" /usr /usr/local /usr/local/bin; do
    safe_dir "$wgtui_dir"
done
for wgtui_file in bin/wgtui settings.toml templates/profile.toml templates/client.conf.tpl .install.lock; do
    safe_file "$wgtui_install_dir/$wgtui_file"
done
check_link

for wgtui_dir in /opt/custom-tools "$wgtui_install_dir" "$wgtui_install_dir/bin"; do
    if [[ ! -d $wgtui_dir ]]; then install -d -o root -g root -m 0755 -- "$wgtui_dir"; fi
done
exec 9<>"$wgtui_install_dir/.install.lock"
chmod 0600 -- "$wgtui_install_dir/.install.lock"
flock --nonblock 9 || fail 'Inny instalator wgtui jest już uruchomiony.'
wgtui_stage=$(mktemp -d "$wgtui_install_dir/.install.XXXXXX")
cleanup() { if [[ -n $wgtui_stage && -d $wgtui_stage ]]; then rm -rf -- "$wgtui_stage"; fi; }
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Test exactly the staged bytes that will be published; never overwrite an
# existing executable with a bad or incompatible candidate.
install -o root -g root -m 0755 -- "$wgtui_binary" "$wgtui_stage/wgtui"
[[ $(sha256sum "$wgtui_stage/wgtui" | awk '{print $1}') == "$wgtui_expected_binary_hash" ]] || fail 'Binarka zmieniła się podczas kopiowania.'
wgtui_version=$("$wgtui_stage/wgtui" --version) || fail 'Binarka nie działa na tym systemie. Poprzednia wersja pozostała zachowana.'
[[ $wgtui_version == 'wgtui '* && $wgtui_version != *$'\n'* ]] || fail 'Nieoczekiwana odpowiedź binarki na --version.'

printf '[2/4] Zależności systemowe…\n'
wgtui_missing=()
for wgtui_package in wireguard-tools iproute2; do
    if ! dpkg-query -W -f='${Status}' "$wgtui_package" 2>/dev/null | grep -q ' ok installed$'; then
        wgtui_missing+=("$wgtui_package")
    fi
done
if (( ${#wgtui_missing[@]} > 0 )); then
    (( wgtui_skip_packages == 0 )) || fail "Brak pakietów: ${wgtui_missing[*]}. Uruchom ponownie bez --skip-packages."
    apt-get update
    DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends "${wgtui_missing[@]}"
else
    printf 'Pakiety są już zainstalowane; apt-get nie jest uruchamiany.\n'
fi
for wgtui_command in /usr/bin/wg /usr/bin/wg-quick /usr/sbin/ip; do
    [[ -x $wgtui_command ]] || fail "Brakuje narzędzia wymaganego przez wgtui: $wgtui_command"
done

printf '[3/4] Instalacja i zachowanie ustawień…\n'
install -d -o root -g root -m 0700 "$wgtui_install_dir/templates" "$wgtui_install_dir/interfaces"
for wgtui_entry in 'templates/settings.toml:settings.toml' 'templates/profile.toml:templates/profile.toml' 'templates/client.conf.tpl:templates/client.conf.tpl'; do
    wgtui_source=${wgtui_entry%%:*}
    wgtui_target=${wgtui_entry#*:}
    safe_file "$wgtui_install_dir/$wgtui_target"
    if [[ -e $wgtui_install_dir/$wgtui_target ]]; then
        chmod 0600 -- "$wgtui_install_dir/$wgtui_target"
        printf 'Zachowano: %s\n' "$wgtui_target"
    else
        install -o root -g root -m 0600 -- "$wgtui_payload/$wgtui_source" "$wgtui_stage/default"
        [[ $(sha256sum "$wgtui_stage/default" | awk '{print $1}') == "$(checksum_for "$wgtui_source")" ]] || fail 'Szablon zmienił się podczas kopiowania.'
        mv -T -- "$wgtui_stage/default" "$wgtui_install_dir/$wgtui_target"
        printf 'Dodano: %s\n' "$wgtui_target"
    fi
done
safe_file "$wgtui_install_dir/bin/wgtui"
check_link
mv -fT -- "$wgtui_stage/wgtui" "$wgtui_install_dir/bin/wgtui"
if [[ ! -d /usr/local/bin ]]; then install -d -o root -g root -m 0755 /usr/local/bin; fi
if [[ ! -L $wgtui_link ]]; then ln -s -- "$wgtui_install_dir/bin/wgtui" "$wgtui_link"; fi
printf '[4/4] Gotowe: %s (%s)\n' "$wgtui_version" "$wgtui_arch"
printf '\nUruchom: sudo wgtui\nUstawienia i szablony: %s\n' "$wgtui_install_dir"
