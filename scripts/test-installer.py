"""Exercise the real installer and launcher using local release fixtures."""

import hashlib
import io
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
ASSET = "boomux-desktop-x86_64-unknown-linux-gnu.tar.gz"


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="boomux installer ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.mock = self.root / "mock"
        self.mock.mkdir()
        self.releases = self.root / "downloads"
        self.releases.mkdir()
        self.install = self.root / "installed"
        self.bin = self.root / "bin"
        self.env = dict(
            os.environ,
            PATH=f"{self.mock}:/usr/bin:/bin",
            BOOMUX_DESKTOP_INSTALL_DIR=str(self.install),
            BOOMUX_DESKTOP_BIN_DIR=str(self.bin),
            FIXTURES=str(self.releases),
            BOOMUX_DESKTOP_VERSION="",
            TRACE=str(self.root / "trace"),
            MOCK_OS="Linux",
            MOCK_ARCH="x86_64",
        )
        self.executable("uname", '#!/bin/sh\ncase "$1" in -s) echo "$MOCK_OS";; -m) echo "$MOCK_ARCH";; esac\n')
        self.executable("curl", '''#!/usr/bin/python3
import os, pathlib, shutil, sys
args = sys.argv[1:]
url = next(a for a in args if a.startswith('https://'))
if url.endswith('/latest'):
    print('https://github.com/gardnmi/boomux-desktop/releases/tag/v0.1.0', end='')
else:
    version, name = url.rsplit('/', 2)[-2:]
    shutil.copyfile(pathlib.Path(os.environ['FIXTURES']) / version / name, args[args.index('-o') + 1])
''')
        self.fixture("v0.1.0")

    def executable(self, name, content):
        path = self.mock / name
        path.write_text(content)
        path.chmod(0o755)

    def fixture(self, version):
        directory = self.releases / version
        directory.mkdir()
        archive = directory / ASSET
        files = {
            "bin/boomux-desktop": (ROOT / "packaging/boomux-desktop").read_bytes(),
            "bin/boomux": b'#!/bin/sh\nprintf "boomux:%s\\n" "$*" >> "$TRACE"\n',
            "libexec/boomux-desktop": f'#!/bin/sh\nprintf "desktop:{version}:%s\\n" "$*" >> "$TRACE"\ncommand -v boomux >> "$TRACE"\n'.encode(),
            "LICENSE": b"fixture",
            "LICENSE.boomux": b"fixture",
            "release.txt": version.encode(),
        }
        with tarfile.open(archive, "w:gz") as tar:
            for name, content in files.items():
                info = tarfile.TarInfo(name)
                info.size = len(content)
                info.mode = 0o755 if "/" in name else 0o644
                tar.addfile(info, io.BytesIO(content))
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        (directory / (ASSET + ".sha256")).write_text(f"{digest}  {ASSET}\n")

    def run_installer(self, success=True):
        # Match curl | sh, including a script read from stdin.
        result = subprocess.run(["sh"], input=(ROOT / "install.sh").read_text(),
                                env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        return result

    def test_clean_install_launch_and_reinstall(self):
        self.run_installer()
        first = (self.install / "current").resolve()
        subprocess.run([self.bin / "boomux-desktop", "argument with spaces"], env=self.env, check=True)
        trace = Path(self.env["TRACE"]).read_text().splitlines()
        self.assertEqual(trace[:2], ["boomux:daemon start", "desktop:v0.1.0:argument with spaces"])
        self.assertEqual(trace[2], str(first / "bin/boomux"))
        self.assertEqual((self.bin / "boomux").resolve(), first / "bin/boomux")
        self.run_installer()
        self.assertEqual((self.install / "current").resolve(), first)
        self.assertFalse(list(self.install.glob(".install.*")))

    def test_update_retains_previous_release_and_bad_download_preserves_current(self):
        self.run_installer()
        previous = (self.install / "current").resolve()
        self.fixture("v0.2.0")
        self.env["BOOMUX_DESKTOP_VERSION"] = "v0.2.0"
        checksum = self.releases / "v0.2.0" / (ASSET + ".sha256")
        valid = checksum.read_text()
        checksum.write_text("0" * 64 + "  " + ASSET + "\n")
        self.run_installer(success=False)
        self.assertEqual((self.install / "current").resolve(), previous)
        checksum.write_text(valid)
        self.run_installer()
        self.assertNotEqual((self.install / "current").resolve(), previous)
        self.assertTrue((previous / "bin/boomux").is_file())

    def test_daemon_start_failure_prevents_desktop_launch(self):
        self.run_installer()
        (self.install / "current/bin/boomux").write_text("#!/bin/sh\nexit 17\n")
        result = subprocess.run([self.bin / "boomux-desktop"], env=self.env)
        self.assertEqual(result.returncode, 17)
        self.assertFalse(Path(self.env["TRACE"]).exists())

    def test_existing_cli_is_preserved_and_unowned_desktop_is_rejected(self):
        self.bin.mkdir()
        cli = self.bin / "boomux"
        cli.write_text("existing CLI")
        self.run_installer()
        self.assertEqual(cli.read_text(), "existing CLI")
        desktop = self.bin / "boomux-desktop"
        desktop.unlink()
        desktop.write_text("existing desktop")
        self.run_installer(success=False)
        self.assertEqual(desktop.read_text(), "existing desktop")

    def test_unsupported_platform_and_invalid_version(self):
        for key, value in [("MOCK_OS", "Darwin"), ("MOCK_ARCH", "aarch64"),
                           ("BOOMUX_DESKTOP_VERSION", "v1/../../escape")]:
            with self.subTest(value=value):
                old = self.env[key]
                self.env[key] = value
                self.run_installer(success=False)
                self.assertFalse(self.install.exists())
                self.env[key] = old

    def test_concurrent_install_is_rejected(self):
        self.install.mkdir()
        lock = self.install / ".install-lock"
        lock.mkdir()
        self.run_installer(success=False)
        self.assertFalse((self.install / "current").exists())
        lock.rmdir()
        self.run_installer()
        self.assertFalse(lock.exists())


if __name__ == "__main__":
    unittest.main()
