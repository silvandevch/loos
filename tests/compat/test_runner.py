#!/usr/bin/env python3
"""Regression tests for the compatibility corpus runner."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

RUNNER = Path(__file__).with_name("run.py")


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.manifest = self.root / "corpus.json"

    def tearDown(self):
        self.temp.cleanup()

    def invoke(self, *args):
        return subprocess.run([sys.executable, str(RUNNER), "--manifest", str(self.manifest), *args], text=True, capture_output=True)

    def write_manifest(self, cases):
        self.manifest.write_text(json.dumps({"version": 1, "cases": cases}), encoding="utf-8")

    def test_empty_corpus_lists_cleanly(self):
        self.write_manifest([])
        result = self.invoke("--list")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no cases registered", result.stdout)

    def test_selected_case_is_passed_as_argv_and_cwd(self):
        exe = self.root / "sample.exe"
        exe.write_bytes(b"MZ")
        subdir = self.root / "working"
        subdir.mkdir()
        capture = self.root / "captured.json"
        launcher = self.root / "launcher.py"
        launcher.write_text(
            "import json, os, sys\n"
            "print(json.dumps({'argv': sys.argv[1:], 'cwd': os.getcwd()}))\n",
            encoding="utf-8",
        )
        self.write_manifest([{"id": "sample10", "era": "10", "exe": "sample.exe", "argv": ["one arg"], "cwd": "working"}])
        result = self.invoke("--era", "10", "--launcher", f"{sys.executable} {launcher}")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("passed 1 launched case(s)", result.stdout)
        self.assertIn(str(exe), result.stdout)
        self.assertIn("one arg", result.stdout)
        self.assertIn(str(subdir), result.stdout)

    def test_missing_executable_is_rejected(self):
        self.write_manifest([{"id": "missing", "era": "7", "exe": "missing.exe"}])
        result = self.invoke("--list")
        self.assertEqual(result.returncode, 2)
        self.assertIn("does not exist", result.stderr)

    def test_nonzero_launcher_fails_case(self):
        exe = self.root / "sample.exe"
        exe.touch()
        self.write_manifest([{"id": "bad", "era": "11", "exe": str(exe)}])
        result = self.invoke("--launcher", f"{sys.executable} -c 'import sys; sys.exit(9)'")
        self.assertEqual(result.returncode, 9)
        self.assertIn("bad failed with status 9", result.stderr)


if __name__ == "__main__":
    unittest.main()
