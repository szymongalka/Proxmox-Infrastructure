#!/usr/bin/env python3
"""Package an already built Linux binary, templates and installation instructions."""
import argparse
import hashlib
import pathlib
import shutil
import tarfile

root = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--target', choices=['x86_64-unknown-linux-musl', 'aarch64-unknown-linux-musl'])
parser.add_argument('--arch', choices=['amd64', 'arm64'])
parser.add_argument('--universal', action='store_true', help='Include Linux x86_64 and ARM64 in one installer package')
args = parser.parse_args()
if args.universal:
    if args.target or args.arch:
        parser.error('--universal nie łączy się z --target/--arch')
    targets = [('x86_64-unknown-linux-musl', 'amd64'), ('aarch64-unknown-linux-musl', 'arm64')]
    destination = root / 'dist' / 'wgtui-linux'
else:
    if not args.target or not args.arch:
        parser.error('Podaj --target i --arch albo --universal')
    expected = 'amd64' if args.target.startswith('x86_64') else 'arm64'
    if args.arch != expected:
        parser.error('Architektura nie pasuje do celu Rust')
    targets = [(args.target, args.arch)]
    destination = root / 'dist' / f'wgtui-linux-{args.arch}'
for target, arch in targets:
    binary = root / 'target' / target / 'release' / 'wgtui'
    if not binary.is_file():
        parser.error(f'Najpierw skompiluj {binary}')
# These are generated release directories, never application runtime data.
# Rebuild from explicit sources so old files cannot accidentally enter a release.
if destination.exists():
    shutil.rmtree(destination)
for target, arch in targets:
    binary = root / 'target' / target / 'release' / 'wgtui'
    bin_dir = destination / 'bin' / arch if args.universal else destination / 'bin'
    bin_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy2(binary, bin_dir / 'wgtui')
    (bin_dir / 'wgtui').chmod(0o755)
shutil.copytree(root / 'templates', destination / 'templates', dirs_exist_ok=True)
shutil.copytree(root / 'ansible', destination / 'ansible', dirs_exist_ok=True)
shutil.copytree(root / 'docs', destination / 'docs', dirs_exist_ok=True)
for name in ['README.md', 'LICENSE']:
    shutil.copy2(root / name, destination / name)
shutil.copy2(root / 'install.sh', destination / 'install.sh')
(destination / 'install.sh').chmod(0o755)
manifest = destination / 'SHA256SUMS'
manifest.write_text(''.join(
    f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.relative_to(destination).as_posix()}\n'
    for path in sorted(destination.rglob('*'))
    if path.is_file() and path != manifest
))
archive = destination.with_suffix('.tar.gz')
with tarfile.open(archive, 'w:gz') as tar:
    tar.add(destination, arcname=destination.name)
checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
archive.with_suffix(archive.suffix + '.sha256').write_text(f'{checksum}  {archive.name}\n')
print(archive)
