"""Release layout tests; synthetic ELF bytes are never executed."""
import hashlib
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[1]


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'scripts').mkdir()
        shutil.copy2(SOURCE / 'scripts/package.py', self.root / 'scripts/package.py')
        shutil.copy2(SOURCE / 'install.sh', self.root / 'install.sh')
        for directory in ['templates', 'docs', 'ansible']:
            (self.root / directory).mkdir()
            (self.root / directory / 'fixture.txt').write_text('fixture\n')
        for name in ['README.md', 'LICENSE']:
            (self.root / name).write_text('fixture\n')
        for target, machine in [('x86_64-unknown-linux-musl', 62), ('aarch64-unknown-linux-musl', 183)]:
            path = self.root / 'target' / target / 'release' / 'wgtui'
            path.parent.mkdir(parents=True)
            header = bytearray(20)
            header[:6] = b'\x7fELF\x02\x01'
            header[18:20] = machine.to_bytes(2, 'little')
            path.write_bytes(header)

    def package(self, *args, success=True):
        result = subprocess.run([sys.executable, str(self.root / 'scripts/package.py'), *args], capture_output=True, text=True)
        self.assertEqual(result.returncode == 0, success, result.stderr)
        return result

    def test_universal_manifest_and_external_checksum(self):
        self.package('--universal')
        archive = self.root / 'dist/wgtui-linux.tar.gz'
        expected = archive.with_suffix('.gz.sha256').read_text().split()[0]
        self.assertEqual(hashlib.sha256(archive.read_bytes()).hexdigest(), expected)
        with tarfile.open(archive) as tar:
            self.assertEqual(tar.getmember('wgtui-linux/install.sh').mode & 0o777, 0o755)
            for arch in ['amd64', 'arm64']:
                self.assertEqual(tar.getmember(f'wgtui-linux/bin/{arch}/wgtui').mode & 0o777, 0o755)
            manifest = tar.extractfile('wgtui-linux/SHA256SUMS').read().decode()
            for line in manifest.splitlines():
                checksum, name = line.split('  ', 1)
                self.assertEqual(hashlib.sha256(tar.extractfile(f'wgtui-linux/{name}').read()).hexdigest(), checksum)

    def test_single_architecture_remains_compatible(self):
        self.package('--target', 'x86_64-unknown-linux-musl', '--arch', 'amd64')
        with tarfile.open(self.root / 'dist/wgtui-linux-amd64.tar.gz') as tar:
            self.assertIn('wgtui-linux-amd64/bin/wgtui', tar.getnames())
            self.assertIn('wgtui-linux-amd64/install.sh', tar.getnames())
            self.assertIn('wgtui-linux-amd64/SHA256SUMS', tar.getnames())

    def test_old_generated_files_cannot_enter_new_release(self):
        self.package('--universal')
        (self.root / 'dist/wgtui-linux/old.conf').write_text('stale fixture')
        self.package('--universal')
        with tarfile.open(self.root / 'dist/wgtui-linux.tar.gz') as tar:
            self.assertFalse(any(name.endswith('old.conf') for name in tar.getnames()))

    def test_invalid_inputs_preserve_previous_release(self):
        self.package('--universal')
        archive = self.root / 'dist/wgtui-linux.tar.gz'
        before = archive.read_bytes()
        self.package('--target', 'x86_64-unknown-linux-musl', '--arch', 'arm64', success=False)
        self.package('--universal', '--arch', 'amd64', success=False)
        (self.root / 'target/aarch64-unknown-linux-musl/release/wgtui').unlink()
        self.package('--universal', success=False)
        self.assertEqual(archive.read_bytes(), before)


if __name__ == '__main__':
    unittest.main()
