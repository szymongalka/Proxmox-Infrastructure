#!/bin/bash
# Run ONLY in a fresh disposable Linux VM; this installs into real /opt paths.
set -euo pipefail
[[ ${WGTUI_DISPOSABLE_VM:-} == 1 && $EUID -eq 0 && $(uname -s) == Linux ]] || {
    printf 'Wymagana jednorazowa VM Linux, root i WGTUI_DISPOSABLE_VM=1.\n' >&2; exit 1;
}
[[ $# == 1 && -f $1 ]] || { printf 'Podaj uniwersalne archiwum wydania.\n' >&2; exit 1; }
[[ ! -e /opt/custom-tools/wgtui && ! -L /opt/custom-tools/wgtui && ! -e /usr/local/bin/wgtui && ! -L /usr/local/bin/wgtui ]] || {
    printf 'VM zawiera już wgtui; test odmawia zmiany istniejącej instalacji.\n' >&2; exit 1;
}
wgtui_fixture=/etc/wireguard/wgtui-install-fixture.conf
[[ ! -e $wgtui_fixture && ! -L $wgtui_fixture ]] || exit 1
wgtui_work=$(mktemp -d /tmp/wgtui-installer-test.XXXXXX)
trap 'rm -rf -- "$wgtui_work"; rm -f -- "$wgtui_fixture"' EXIT
tar -xzf "$1" --no-same-owner -C "$wgtui_work"
wgtui_release=$wgtui_work/wgtui-linux
wgtui_root=/opt/custom-tools/wgtui
install -d -m 0700 /etc/wireguard
printf '# Synthetic fixture; never started\n' > "$wgtui_fixture"
chmod 0600 "$wgtui_fixture"
wgtui_config_hash=$(sha256sum "$wgtui_fixture")
wgtui_links_before=$(ip -o link show | awk -F': ' '{print $2}')

"$wgtui_release/install.sh" > "$wgtui_work/first.log" 2>&1 || { cat "$wgtui_work/first.log"; exit 1; }
[[ $(/usr/local/bin/wgtui --version) == 'wgtui '* ]]
[[ $(readlink /usr/local/bin/wgtui) == "$wgtui_root/bin/wgtui" ]]
[[ $(stat -c %a "$wgtui_root/bin/wgtui") == 755 ]]
[[ $(stat -c %a "$wgtui_root/templates") == 700 ]]
[[ $(stat -c %a "$wgtui_root/settings.toml") == 600 ]]
[[ $(sha256sum "$wgtui_fixture") == "$wgtui_config_hash" ]]
[[ $(ip -o link show | awk -F': ' '{print $2}') == "$wgtui_links_before" ]]
printf 'PASS: instalacja, zależności, wersja, uprawnienia, brak zmian tuneli\n'

printf 'active_seconds = 777\n' > "$wgtui_root/settings.toml"
printf '\n# zachowaj ten szablon\n' >> "$wgtui_root/templates/client.conf.tpl"
printf 'endpoint = "vpn.example.org:51821"\n' > "$wgtui_root/interfaces/wg01admin02.toml"
chmod 0600 "$wgtui_root/interfaces/wg01admin02.toml"
wgtui_preserved=$(sha256sum "$wgtui_root/settings.toml" "$wgtui_root/templates/client.conf.tpl" "$wgtui_root/interfaces/wg01admin02.toml")
"$wgtui_release/install.sh" --skip-packages > "$wgtui_work/reinstall.log" 2>&1 || { cat "$wgtui_work/reinstall.log"; exit 1; }
[[ $(sha256sum "$wgtui_root/settings.toml" "$wgtui_root/templates/client.conf.tpl" "$wgtui_root/interfaces/wg01admin02.toml") == "$wgtui_preserved" ]]
grep -q 'apt-get nie jest uruchamiany' "$wgtui_work/reinstall.log"
printf 'PASS: aktualizacja zachowuje ustawienia, szablony i dane; pomija apt\n'
wgtui_installed_hash=$(sha256sum "$wgtui_root/bin/wgtui")

cp -a "$wgtui_release" "$wgtui_work/corrupt"
printf '\ncorrupt\n' >> "$wgtui_work/corrupt/templates/profile.toml"
if "$wgtui_work/corrupt/install.sh" > "$wgtui_work/corrupt.log" 2>&1; then exit 1; fi
grep -q 'Suma SHA-256' "$wgtui_work/corrupt.log"
[[ $(sha256sum "$wgtui_root/bin/wgtui") == "$wgtui_installed_hash" ]]
printf 'PASS: niezgodna suma SHA-256 odmawia przed instalacją\n'

case $(uname -m) in
    aarch64|arm64) wgtui_native=arm64; wgtui_other=amd64 ;;
    x86_64|amd64) wgtui_native=amd64; wgtui_other=arm64 ;;
esac
cp -a "$wgtui_release" "$wgtui_work/wrong-arch"
cp "$wgtui_work/wrong-arch/bin/$wgtui_other/wgtui" "$wgtui_work/wrong-arch/bin/$wgtui_native/wgtui"
wgtui_hash=$(sha256sum "$wgtui_work/wrong-arch/bin/$wgtui_native/wgtui" | awk '{print $1}')
awk -v name="bin/$wgtui_native/wgtui" -v hash="$wgtui_hash" '$2==name {print hash "  " name; next} {print}' "$wgtui_work/wrong-arch/SHA256SUMS" > "$wgtui_work/manifest"
mv "$wgtui_work/manifest" "$wgtui_work/wrong-arch/SHA256SUMS"
if "$wgtui_work/wrong-arch/install.sh" --skip-packages > "$wgtui_work/wrong-arch.log" 2>&1; then exit 1; fi
grep -q 'inną architekturę' "$wgtui_work/wrong-arch.log"
[[ $(sha256sum "$wgtui_root/bin/wgtui") == "$wgtui_installed_hash" ]]
printf 'PASS: niewłaściwa architektura odmawia także z poprawną sumą\n'

printf 'nie zmieniaj\n' > "$wgtui_work/sentinel"
mv "$wgtui_root/templates/profile.toml" "$wgtui_work/profile.toml"
ln -s "$wgtui_work/sentinel" "$wgtui_root/templates/profile.toml"
if "$wgtui_release/install.sh" --skip-packages > "$wgtui_work/symlink.log" 2>&1; then exit 1; fi
grep -q 'nie może być dowiązaniem' "$wgtui_work/symlink.log"
[[ $(cat "$wgtui_work/sentinel") == 'nie zmieniaj' ]]
rm "$wgtui_root/templates/profile.toml"
mv "$wgtui_work/profile.toml" "$wgtui_root/templates/profile.toml"
printf 'PASS: dowiązanie zamiast ustawień nie nadpisuje innego pliku\n'

rm /usr/local/bin/wgtui
printf 'inny program\n' > /usr/local/bin/wgtui
if "$wgtui_release/install.sh" --skip-packages > "$wgtui_work/collision.log" 2>&1; then exit 1; fi
[[ $(cat /usr/local/bin/wgtui) == 'inny program' ]]
rm /usr/local/bin/wgtui
ln -s "$wgtui_root/bin/wgtui" /usr/local/bin/wgtui
printf 'PASS: instalator nie nadpisuje innego programu w PATH\n'

exec 8<>"$wgtui_root/.install.lock"
flock --nonblock 8
if "$wgtui_release/install.sh" --skip-packages > "$wgtui_work/lock.log" 2>&1; then exit 1; fi
grep -q 'Inny instalator' "$wgtui_work/lock.log"
flock --unlock 8
exec 8>&-
[[ $(sha256sum "$wgtui_root/bin/wgtui") == "$wgtui_installed_hash" ]]
[[ $(sha256sum "$wgtui_fixture") == "$wgtui_config_hash" ]]
printf 'PASS: blokada drugiego instalatora; poprzednia instalacja zachowana\n'
