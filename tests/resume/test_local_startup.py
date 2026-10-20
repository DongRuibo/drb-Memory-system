"""Exercise launch orchestration with process fixtures; no network or model calls."""

import contextlib
import io
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import MagicMock, patch


ROOT = Path(__file__).resolve().parents[2]
FIXTURE = """#!/usr/bin/env python3
import json, os, sys, time
from pathlib import Path
name = Path(sys.argv[0]).name
out = Path(os.environ['TEST_RECEIPTS'])
if name == 'lsof':
    sys.exit(1)
if name == 'curl':
    package = 'demo-web-api' if ':19090/' in sys.argv[-1] else 'memory-service'
    sys.exit(0 if (out / package).exists() else 1)
if name == 'cargo':
    name = sys.argv[-1]
keys = ['MEMORY_BIND', 'DEMO_WEB_API_BIND', 'NEXT_PUBLIC_DEMO_API_URL',
        'DEMO_WEB_ALLOWED_ORIGINS', 'MEMORY_BOOTSTRAP_EMPTY_PROJECTION']
(out / name).write_text(json.dumps({'args': sys.argv[1:],
    'environment': {key: os.environ.get(key) for key in keys}}))
if name != 'npm':
    time.sleep(5)
"""


class LocalStartupTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "scripts").mkdir()
        (self.root / "apps/demo-web/node_modules").mkdir(parents=True)
        for name in ("demo-web-stack.sh", "demo-web.sh", "run-memory-service-semantic.sh"):
            shutil.copy2(ROOT / "scripts" / name, self.root / "scripts" / name)
        commands = self.root / "bin"
        commands.mkdir()
        fixture = commands / "fixture"
        fixture.write_text(FIXTURE)
        fixture.chmod(0o755)
        for name in ("cargo", "npm", "curl", "lsof"):
            (commands / name).symlink_to(fixture)
        self.environment = {
            "PATH": str(commands) + os.pathsep + os.environ["PATH"],
            "TEST_RECEIPTS": str(self.root),
            "KIMI_API_KEY": "unit-fixture-only",
            "MEMORY_JWT_HS256_SECRET": "unit-fixture-with-at-least-32-bytes",
            "MEMORY_BIND": "127.0.0.1:19080",
            "DEMO_WEB_API_BIND": "127.0.0.1:19090",
            "DEMO_WEB_PORT": "13000",
        }

    def launch(self):
        return subprocess.run(
            ["bash", str(self.root / "scripts/demo-web-stack.sh")],
            env=self.environment, capture_output=True, text=True, timeout=10,
        )

    def test_isolated_ports_reach_children_browser_and_cors(self):
        result = self.launch()
        self.assertEqual(result.returncode, 0, result.stderr)
        memory = json.loads((self.root / "memory-service").read_text())["environment"]
        self.assertEqual(memory["MEMORY_BIND"], "127.0.0.1:19080")
        self.assertEqual(memory["MEMORY_BOOTSTRAP_EMPTY_PROJECTION"], "true")
        api = json.loads((self.root / "demo-web-api").read_text())["environment"]
        self.assertEqual(api["DEMO_WEB_API_BIND"], "127.0.0.1:19090")
        self.assertIn("http://127.0.0.1:13000", api["DEMO_WEB_ALLOWED_ORIGINS"])
        web = json.loads((self.root / "npm").read_text())
        self.assertEqual(web["environment"]["NEXT_PUBLIC_DEMO_API_URL"], "http://127.0.0.1:19090")
        self.assertEqual(web["args"], ["run", "dev", "--", "--hostname", "127.0.0.1", "--port", "13000"])
        self.assertIn("http://localhost:13000", result.stdout)

    def test_explicitly_disabled_bootstrap_is_preserved(self):
        self.environment["MEMORY_BOOTSTRAP_EMPTY_PROJECTION"] = "false"
        result = self.launch()
        self.assertEqual(result.returncode, 0, result.stderr)
        memory = json.loads((self.root / "memory-service").read_text())["environment"]
        self.assertEqual(memory["MEMORY_BOOTSTRAP_EMPTY_PROJECTION"], "false")

    def test_non_loopback_or_invalid_ports_start_nothing(self):
        for bind in ("0.0.0.0:19080", "127.0.0.1:0", "127.0.0.1:65536", "127.0.0.1:bad"):
            with self.subTest(bind=bind):
                self.environment["MEMORY_BIND"] = bind
                self.assertNotEqual(self.launch().returncode, 0)
                self.assertFalse((self.root / "memory-service").exists())


class ConfigurationFailureTests(unittest.TestCase):
    def test_configuration_errors_do_not_echo_provider_details(self):
        environment = {
            "MEM0_BASE_URL": "http://127.0.0.1:19888",
            "MEM0_API_KEY": "unit-fixture",
            "EMBEDDING_API_KEY": "unit-fixture",
            "EMBEDDING_DIMENSIONS": "1024",
            "EMBEDDING_BASE_URL": "http://host.docker.internal:19082",
            "EMBEDDING_MODEL": "unit-fixture",
            "MEM0_POSTGRES_HOST": "unit-fixture",
            "MEM0_POSTGRES_DB": "unit-fixture",
            "MEM0_POSTGRES_USER": "unit-fixture",
            "MEM0_POSTGRES_PASSWORD": "unit-fixture",
            "MEM0_COLLECTION_NAME": "unit-fixture",
        }
        response = MagicMock()
        response.__enter__.return_value.read.return_value = b"{}"
        output = io.StringIO()
        with patch.dict(os.environ, environment, clear=True), \
             patch("urllib.request.urlopen", side_effect=[response, RuntimeError("DO_NOT_LOG_FIXTURE")]), \
             contextlib.redirect_stderr(output), contextlib.redirect_stdout(output):
            with self.assertRaises(SystemExit) as stopped:
                runpy.run_path(str(ROOT / "deploy/mem0/configure.py"), run_name="__main__")
        self.assertEqual(stopped.exception.code, 1)
        self.assertEqual(json.loads(output.getvalue()), {"status": "failed", "error_type": "RuntimeError"})


if __name__ == "__main__":
    unittest.main()
