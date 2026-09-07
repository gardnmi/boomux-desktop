"""Exercise the retired URL without network access or installation changes."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

class ForwardingTests(unittest.TestCase):
    def run_forwarder(self, fail=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "bin"
            binary.mkdir()
            curl = binary / "curl"
            curl.write_text("""#!/usr/bin/env python3
import json, os, pathlib, sys
args = sys.argv[1:]
assert 'https://github.com/gardnmi/boomux/releases/latest/download/boomux-desktop-installer.sh' in args
assert args[args.index('--proto') + 1] == '=https'
assert args[args.index('--proto-redir') + 1] == '=https'
dest = pathlib.Path(args[args.index('-o') + 1])
dest.write_text('printf executed > "$MARKER"\\nprintf "%s\\n" "$BOOMUX_DESKTOP_VERSION" "$@" > "$RECEIPT"\\n')
sys.exit(22 if os.environ.get('FAIL_DOWNLOAD') else 0)
""")
            curl.chmod(0o755)
            env = dict(os.environ, PATH=str(binary) + os.pathsep + os.environ['PATH'],
                       TMPDIR=str(root), MARKER=str(root/'marker'), RECEIPT=str(root/'receipt'),
                       BOOMUX_DESKTOP_VERSION='v1.10.0')
            if fail:
                env['FAIL_DOWNLOAD'] = '1'
            result = subprocess.run(['sh', str(ROOT/'install.sh'), '--prepare'], env=env, capture_output=True)
            self.assertEqual(list(root.glob('boomux-desktop-forward.*')), [])
            if fail:
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((root/'marker').exists())
            else:
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual((root/'receipt').read_text().splitlines(), ['v1.10.0', '--prepare'])

    def test_preserves_version_and_arguments(self):
        self.run_forwarder()

    def test_failed_download_never_executes_partial_installer(self):
        self.run_forwarder(fail=True)

if __name__ == '__main__':
    unittest.main()
