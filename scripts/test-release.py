"""Validate publication ordering and retries without contacting GitHub."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
ASSET = "boomux-desktop-x86_64-unknown-linux-gnu.tar.gz"


class PublicationTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.dist = self.root / "dist"
        self.dist.mkdir()
        (self.dist / ASSET).write_bytes(b"release fixture")
        digest = hashlib.sha256((self.dist / ASSET).read_bytes()).hexdigest()
        (self.dist / (ASSET + ".sha256")).write_text(f"{digest}  {ASSET}\n")
        self.state = self.root / "state.json"
        self.save(dict(draft=True, assets={}, uploads=[], published=False))
        mock = self.root / "gh"
        mock.write_text('''#!/usr/bin/python3
import hashlib, json, os, pathlib, sys
path = pathlib.Path(os.environ['MOCK_STATE'])
state = json.loads(path.read_text())
args = sys.argv[1:]
if args[:2] == ['release', 'upload']:
    if os.environ.get('FAIL_UPLOAD'): sys.exit(1)
    asset = pathlib.Path(args[3])
    state['assets'][asset.name] = 'sha256:' + hashlib.sha256(asset.read_bytes()).hexdigest()
    state['uploads'].append(asset.name)
elif 'PATCH' in args:
    assert len(state['assets']) == 2, 'published before all assets were uploaded'
    state['draft'] = False
    state['published'] = True
elif any('/assets?' in a for a in args):
    if os.environ.get('FAIL_LIST'): sys.exit(1)
    for i, (name, digest) in enumerate(state['assets'].items()):
        print(name + chr(31) + digest + chr(31) + str(i + 1))
elif args[-1] == '.id':
    print(7)
elif args[-1] == '.draft':
    print(str(state['draft']).lower())
elif 'join(' in args[-1]:
    print('v0.1.0' + chr(9) + str(state['draft']).lower())
else:
    raise AssertionError(args)
path.write_text(json.dumps(state))
''')
        mock.chmod(0o755)
        self.env = dict(os.environ, PATH=f"{self.root}:/usr/bin:/bin",
                        GH_REPO="fixture/desktop", MOCK_STATE=str(self.state))

    def save(self, value):
        self.state.write_text(json.dumps(value))

    def run_publish(self, success=True):
        result = subprocess.run(["bash", ROOT / "scripts/publish-release.sh", "v0.1.0"],
                                cwd=self.root, env=self.env, text=True, capture_output=True)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        return json.loads(self.state.read_text())

    def test_uploads_complete_bundle_before_publication(self):
        state = self.run_publish()
        self.assertEqual(set(state["uploads"]), {ASSET, ASSET + ".sha256"})
        self.assertTrue(state["published"])

    def test_retry_skips_identical_assets(self):
        state = json.loads(self.state.read_text())
        state["assets"] = {file.name: "sha256:" + hashlib.sha256(file.read_bytes()).hexdigest()
                           for file in self.dist.iterdir()}
        self.save(state)
        state = self.run_publish()
        self.assertEqual(state["uploads"], [])
        self.assertTrue(state["published"])

    def test_conflicting_asset_does_not_publish(self):
        state = json.loads(self.state.read_text())
        state["assets"][ASSET] = "sha256:" + "0" * 64
        self.save(state)
        self.assertFalse(self.run_publish(False)["published"])

    def test_network_errors_do_not_publish(self):
        for variable in ["FAIL_UPLOAD", "FAIL_LIST"]:
            with self.subTest(variable=variable):
                self.env[variable] = "1"
                self.assertFalse(self.run_publish(False)["published"])
                del self.env[variable]

    def test_published_release_is_unchanged(self):
        state = json.loads(self.state.read_text())
        state["draft"] = False
        self.save(state)
        self.assertEqual(self.run_publish(False), state)

    def test_corrupt_bundle_does_not_upload(self):
        (self.dist / ASSET).write_bytes(b"corrupt")
        state = self.run_publish(False)
        self.assertEqual(state["uploads"], [])
        self.assertFalse(state["published"])


if __name__ == "__main__":
    unittest.main()
