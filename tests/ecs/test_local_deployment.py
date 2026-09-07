import importlib.util
import json
import os
import re
import tempfile
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("local_deploy", ROOT / "deploy/local/deploy.py")
local = importlib.util.module_from_spec(spec)
spec.loader.exec_module(local)


class LocalTests(unittest.TestCase):
    def setUp(self):
        self.dep = local.LocalDeployment()
        self.events = []
        self.dep.compose = lambda *args, **kwargs: self.events.append(args) or ""
        self.dep.task = lambda program, action, **kwargs: self.events.append((program, action, kwargs))

    def test_up_checks_but_never_migrates_or_configures(self):
        self.dep.execute("up")
        self.assertEqual([e[:2] for e in self.events if e[0] in {"database", "configuration"}],
                         [("database", "check"), ("configuration", "check")])
        self.assertEqual([e[-1] for e in self.events if e[0] == "up"], ["mem0", "memory-service"])
        self.assertTrue(all("--no-build" in e and "never" in e for e in self.events if e[0] == "up"))

    def test_schema_failure_starts_nothing(self):
        with patch.object(self.dep, "task", side_effect=RuntimeError("schema mismatch")):
            with self.assertRaises(RuntimeError):
                self.dep.execute("up")
        self.assertEqual(self.events, [])

    def test_config_mismatch_prevents_memory_start(self):
        def task(program, action, **kwargs):
            if program == "configuration":
                raise RuntimeError("mismatch")
        self.dep.task = task
        with self.assertRaises(RuntimeError):
            self.dep.execute("up")
        self.assertEqual([e[-1] for e in self.events], ["mem0"])

    def test_configure_is_explicit_and_does_not_start_memory(self):
        self.dep.execute("configure")
        self.assertIn(("configuration", "configure", {}), self.events)
        self.assertEqual([e[-1] for e in self.events if e[0] == "up"], ["mem0"])

    def test_owner_password_only_enters_explicit_migration_task(self):
        with self.assertRaises(RuntimeError):
            self.dep.execute("migrate")
        self.dep.execute("migrate", {"MEM0_MIGRATION_PASSWORD": "fixture-owner"})
        self.assertEqual(self.events, [
            ("database", "migrate", {"extra": {"POSTGRES_USER": "lazytt_memory_mem0_owner", "POSTGRES_PASSWORD": "fixture-owner"}}),
            ("database", "check", {})])

    def test_local_compose_cannot_load_ambient_env_or_other_environment(self):
        with patch.dict(local.os.environ, {"COMPOSE_FILE": "compose.dev.yml", "MEM0_API_KEY": "ambient-secret"}):
            dep = local.LocalDeployment()
        with patch.object(local, "run", return_value="") as run:
            dep.compose("config", "--quiet")
        args, kwargs = run.call_args
        self.assertIn(str(ROOT / "compose.local.yml"), args[0])
        self.assertIn("/dev/null", args[0])
        self.assertNotIn("COMPOSE_FILE", kwargs["env"])
        self.assertNotIn("MEM0_API_KEY", kwargs["env"])

    def test_down_preserves_volumes(self):
        self.dep.execute("down")
        self.assertEqual(self.events, [("down",)])

    def test_environment_topologies_have_only_external_store(self):
        files = set(ROOT.rglob("*compose*.y*ml"))
        self.assertEqual(files, {ROOT / "compose.local.yml", ROOT / "compose.dev.yml"})
        documents = [json.loads(p.read_text()) for p in sorted(files)]
        for doc in documents:
            self.assertEqual(set(doc["services"]), {"mem0", "memory-service", "admin"})
            self.assertNotIn("include", doc)
            for name, service in doc["services"].items():
                self.assertNotIn("extends", service)
                self.assertNotIn("MEM0_MIGRATION_PASSWORD", service["environment"])
                if name != "memory-service":
                    self.assertNotIn("ports", service)
                if name != "admin":
                    self.assertEqual(service["restart"], "always")
            self.assertEqual(doc["services"]["admin"]["profiles"], ["admin"])
        self.assertNotEqual(documents[0]["name"], documents[1]["name"])

    def test_local_config_loads_owned_file_without_ambient_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "local.env"
            config.write_text("export MEM0_POSTGRES_HOST='fixture-pg'\n"
                              "export MEM0_POSTGRES_PORT='5432'\n"
                              "export MEM0_POSTGRES_RUNTIME_PASSWORD='fixture-runtime'\n")
            config.chmod(0o600)
            with patch.dict(os.environ, {"MEM0_MIGRATION_PASSWORD": "ambient-owner"}):
                values = local.read_config(config, database_only=True, owner=(os.getuid(), os.getgid()))
            self.assertEqual(set(values), {"MEM0_POSTGRES_HOST", "MEM0_POSTGRES_PORT", "MEM0_POSTGRES_RUNTIME_PASSWORD"})
            config.chmod(0o644)
            with self.assertRaisesRegex(RuntimeError, "0600"):
                local.read_config(config, database_only=True, owner=(os.getuid(), os.getgid()))

    def test_templates_cover_shared_required_config(self):
        from release import REQUIRED
        for filename in ("deploy/local/.env.local.example", "deploy/ecs/.env.dev.example"):
            keys = re.findall(r"^export ([A-Z0-9_]+)=", (ROOT / filename).read_text(), re.M)
            self.assertEqual(set(keys), set(REQUIRED))
            self.assertEqual(len(keys), len(set(keys)))

    def test_default_ecs_config_still_requires_root_ownership(self):
        # Local ownership support must not relax the ECS entry point's default.
        mode = SimpleNamespace(st_mode=0o100600, st_uid=501, st_gid=20)
        with patch.object(Path, "lstat", return_value=mode):
            with self.assertRaisesRegex(RuntimeError, "0:0"):
                local.read_config("/fixture", database_only=True)


if __name__ == "__main__":
    unittest.main()
