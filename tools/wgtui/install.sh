#!/bin/bash
# Installer for a local release package or a checked-out source tree.
set -euo pipefail
umask 077

wgtui_install_dir=/opt/custom-tools/wgtui
wgtui_link=/usr/local/bin/wgtui
wgtui_skip_packages=0
wgtui_stage=
wgtui_work=
wgtui_from_source=0
wgtui_jobs=2

cleanup() {
    if [[ -n $wgtui_stage && -d $wgtui_stage ]]; then rm -rf -- "$wgtui_stage"; fi
    if [[ -n $wgtui_work && -d $wgtui_work ]]; then rm -rf -- "$wgtui_work"; fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

fail() { printf 'wgtui: %s\n' "$*" >&2; exit 1; }
usage() {
    cat <<'HELP'
Instalacja lub aktualizacja wgtui na Debianie, Ubuntu i Proxmoxie.

  sudo ./install.sh
  sudo ./install.sh --skip-packages
  sudo ./install.sh --from-source --jobs 2

  --skip-packages  Nie uruchamiaj apt-get ani instalatora Rust.
  --from-source    Wymuś kompilację ze źródeł zamiast lokalnej paczki.
  --jobs N         Liczba równoległych zadań kompilacji (domyślnie 2).
  -h, --help       Pokaż tę pomoc.

Instalator dobiera binarkę x86_64/ARM64 z lokalnej paczki, weryfikuje
SHA256SUMS i instaluje ją w /opt/custom-tools/wgtui/bin/wgtui.
W repozytorium bez binarki automatycznie kompiluje źródła z Cargo.lock.
W razie potrzeby pobiera Rust stable z oficjalnego serwera do katalogu
tymczasowego. Kompilacja wymaga internetu, czasu i wolnego miejsca.
Opcja --skip-packages nadal pozwala Cargo pobierać zależności.
Istniejące ustawienia, szablony i dane interfejsów są zachowywane.
Tunele, klucze, zapora, routing i autostart usług nie są zmieniane.
HELP
}
while (( $# > 0 )); do
    case "$1" in
        --skip-packages) wgtui_skip_packages=1; shift ;;
        --from-source) wgtui_from_source=1; shift ;;
        --jobs)
            [[ $# -ge 2 && $2 =~ ^[1-9][0-9]*$ ]] || fail '--jobs wymaga dodatniej liczby całkowitej.'
            wgtui_jobs=$2; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; fail "Nieznana opcja: $1" ;;
    esac
done

[[ $(uname -s) == Linux ]] || fail 'Instalacja wymaga Linuksa. Na Macu możesz uruchomić demo ze źródeł.'
[[ $EUID -eq 0 ]] || fail 'Uruchom instalator przez sudo: sudo ./install.sh'
case $(uname -m) in
    x86_64|amd64) wgtui_arch=amd64; wgtui_machine=3e00; wgtui_rust_host=x86_64-unknown-linux-gnu ;;
    aarch64|arm64) wgtui_arch=arm64; wgtui_machine=b700; wgtui_rust_host=aarch64-unknown-linux-gnu ;;
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
for wgtui_command in sha256sum od tr awk stat install mkdir mktemp mv rm ln readlink flock dirname grep chmod dpkg-query cp sort head; do
    command -v "$wgtui_command" >/dev/null || fail "Brakuje polecenia: $wgtui_command"
done
[[ -x /usr/bin/systemctl ]] || fail 'Aplikacja wymaga systemd (brak /usr/bin/systemctl).'

wgtui_script_dir=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
wgtui_payload=
for wgtui_candidate in "$wgtui_script_dir" "$wgtui_script_dir/dist/wgtui-linux" "$wgtui_script_dir/dist/wgtui-linux-$wgtui_arch"; do
    (( wgtui_from_source == 0 )) || break
    if [[ -f "$wgtui_candidate/bin/$wgtui_arch/wgtui" ]]; then
        wgtui_payload=$wgtui_candidate; wgtui_binary_rel=bin/$wgtui_arch/wgtui; break
    elif [[ -f "$wgtui_candidate/bin/wgtui" ]]; then
        wgtui_payload=$wgtui_candidate; wgtui_binary_rel=bin/wgtui; break
    fi
done
# A checkout does not contain release binaries. Build in a private temporary
# directory so Cargo never writes build artifacts into the source repository.
if [[ -z $wgtui_payload ]]; then
    [[ -f "$wgtui_script_dir/Cargo.toml" && -f "$wgtui_script_dir/Cargo.lock" && -d "$wgtui_script_dir/src" ]] ||
        fail 'Brak binarki i źródeł. Sklonuj całe repozytorium lub rozpakuj pełną paczkę wydania.'
    for wgtui_file in templates/settings.toml templates/profile.toml templates/client.conf.tpl; do
        [[ -f "$wgtui_script_dir/$wgtui_file" && ! -L "$wgtui_script_dir/$wgtui_file" ]] || fail "Brak pliku źródłowego: $wgtui_file"
    done
    printf '[Kompilacja] Przygotowanie zależności i źródeł (%s zadania)…\n' "$wgtui_jobs"
    wgtui_missing=()
    for wgtui_package in build-essential ca-certificates curl wireguard-tools iproute2; do
        if ! dpkg-query -W -f='${Status}' "$wgtui_package" 2>/dev/null | grep -q ' ok installed$'; then
            wgtui_missing+=("$wgtui_package")
        fi
    done
    if (( ${#wgtui_missing[@]} > 0 )); then
        (( wgtui_skip_packages == 0 )) || fail "Brak pakietów: ${wgtui_missing[*]}. Uruchom bez --skip-packages."
        apt-get update
        DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends "${wgtui_missing[@]}"
    fi
    command -v cc >/dev/null || fail 'Brakuje kompilatora C (build-essential).'
    wgtui_work=$(mktemp -d /var/tmp/wgtui-build.XXXXXX)
    wgtui_build=$wgtui_work/source
    mkdir -p "$wgtui_build" "$wgtui_work/payload/bin" "$wgtui_work/payload/templates"
    cp -- "$wgtui_script_dir/Cargo.toml" "$wgtui_script_dir/Cargo.lock" "$wgtui_build/"
    cp -R -- "$wgtui_script_dir/src" "$wgtui_build/src"
    cp -R -- "$wgtui_script_dir/templates" "$wgtui_build/templates"
    # Determine the minimum compiler version from the project itself.
    wgtui_min_rust=$(awk -F '"' '/^rust-version[[:space:]]*=/ {print $2}' "$wgtui_build/Cargo.toml")
    [[ $wgtui_min_rust =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?$ ]] || fail 'Brak poprawnego rust-version w Cargo.toml.'
    wgtui_rust_version=
    if command -v rustc >/dev/null && command -v cargo >/dev/null; then
        wgtui_rust_version=$(rustc --version 2>/dev/null | awk '{print $2}') || wgtui_rust_version=
    fi
    if [[ -z $wgtui_rust_version || $(printf '%s\n' "$wgtui_min_rust" "$wgtui_rust_version" | sort -V | head -1) != "$wgtui_min_rust" ]]; then
        (( wgtui_skip_packages == 0 )) || fail "Wymagany Rust >= $wgtui_min_rust i Cargo; zainstaluj je lub uruchom bez --skip-packages."
        wgtui_rust_url=https://static.rust-lang.org/rustup/dist/$wgtui_rust_host/rustup-init
        printf '[Kompilacja] Pobieranie tymczasowego środowiska Rust z static.rust-lang.org…\n'
        curl --proto '=https' --tlsv1.2 --fail --show-error --location --retry 3 "$wgtui_rust_url" -o "$wgtui_work/rustup-init"
        wgtui_rust_hash=$(curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location --retry 3 "$wgtui_rust_url.sha256" | awk '{print $1}')
        [[ $wgtui_rust_hash =~ ^[0-9a-f]{64}$ ]] || fail 'Niepoprawna suma SHA-256 instalatora Rust.'
        [[ $(sha256sum "$wgtui_work/rustup-init" | awk '{print $1}') == "$wgtui_rust_hash" ]] || fail 'Błędna suma SHA-256 instalatora Rust.'
        chmod 0700 "$wgtui_work/rustup-init"
        export CARGO_HOME=$wgtui_work/cargo RUSTUP_HOME=$wgtui_work/rustup
        # Keep the user's Rust installation and shell profile untouched.
        RUSTUP_INIT_SKIP_PATH_CHECK=yes "$wgtui_work/rustup-init" -y --no-modify-path --profile minimal --default-host "$wgtui_rust_host" --default-toolchain stable
        export PATH="$CARGO_HOME/bin:$PATH"
    fi
    printf '[Kompilacja] Budowanie wgtui; pierwsze uruchomienie może potrwać kilka minut…\n'
    (cd -- "$wgtui_build" && CARGO_TARGET_DIR="$wgtui_build/target" cargo build --release --locked --jobs "$wgtui_jobs" --target "$wgtui_rust_host" --bin wgtui)
    wgtui_payload=$wgtui_work/payload
    wgtui_binary_rel=bin/wgtui
    cp -- "$wgtui_build/target/$wgtui_rust_host/release/wgtui" "$wgtui_payload/bin/wgtui"
    cp -- "$wgtui_script_dir/templates/"* "$wgtui_payload/templates/"
    cp -- "$wgtui_script_dir/install.sh" "$wgtui_payload/install.sh"
    (cd -- "$wgtui_payload" && sha256sum bin/wgtui templates/settings.toml templates/profile.toml templates/client.conf.tpl install.sh > SHA256SUMS)
fi
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
