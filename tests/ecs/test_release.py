import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("release", ROOT / "deploy/ecs/release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
SHA = "a" * 40
OLD = "ghcr.io/example/memory@sha256:" + "1" * 64
NEW = "ghcr.io/example/memory@sha256:" + "2" * 64


class FakeDeployment(release.Deployment):
    def __init__(self, directory):
        super().__init__(directory, {"test": "value"})
        self.events = []
        self.failure = None

    def preflight(self):
        self.events.append("preflight")

    def ownership(self):
        self.events.append("ownership")

    def check_image(self, image, revision):
        release.require(release.IMAGE.fullmatch(image) and release.SHA.fullmatch(revision), "Invalid candidate")
        self.events.append(("pull", image))

    def task(self, program, action, service="admin", extra=None):
        event = (program, action, service)
        self.events.append(event)
        if self.failure == event:
            self.failure = None
            raise RuntimeError("injected failure")

    def compose(self, *args, extra=None):
        self.events.append(args)
        if self.failure == "start" and args[-1] == "memory-service" and args[0] == "up":
            self.failure = None
            raise RuntimeError("injected start failure")
        return ""

    def verify_running(self, document):
        self.events.append("verified")


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.dep = FakeDeployment(self.temp.name)
        self.template = ROOT / "compose.dev.yml"
        self.dep.prepare(self.template, SHA, OLD, OLD)
        prior = self.dep.read()
        prior["x-lazytt"].update(released=True, config_hash=release.fingerprint(self.dep.values))
        self.dep.write(prior)
        self.dep.events.clear()

    def test_prepare_live_candidate_preserves_application_images_and_never_starts(self):
        self.dep.prepare(self.template, SHA, NEW, NEW)
        doc = self.dep.read()
        self.assertEqual(doc["services"]["mem0"]["image"], OLD)
        self.assertEqual(doc["services"]["admin"]["image"], NEW)
        self.assertFalse(any(e[0] in {"up", "database", "configuration"} for e in self.dep.events if isinstance(e, tuple)))
        self.assertEqual(list(Path(self.temp.name).glob("*.yml")), [self.dep.path])

    def test_changed_topology_does_not_overwrite_running_compose(self):
        before = self.dep.path.read_bytes()
        template = json.loads(self.template.read_text())
        template["services"]["mem0"]["mem_limit"] = "4g"
        source = Path(self.temp.name) / "new.json"
        source.write_text(json.dumps(template))
        with self.assertRaisesRegex(RuntimeError, "topology changed"):
            self.dep.prepare(source, SHA, NEW, NEW)
        self.assertEqual(self.dep.path.read_bytes(), before)

    def test_release_never_migrates_and_reconciles_configuration(self):
        self.dep.prepare(self.template, SHA, NEW, NEW)
        self.dep.release()
        self.assertEqual(self.dep.read()["services"]["memory-service"]["image"], NEW)
        self.assertNotIn(("database", "migrate", "admin"), self.dep.events)
        self.assertIn(("configuration", "ensure", "admin"), self.dep.events)

    def test_candidate_schema_failure_leaves_existing_services_untouched(self):
        self.dep.prepare(self.template, SHA, NEW, NEW)
        before = self.dep.path.read_bytes()
        self.dep.failure = ("database", "check", "admin")
        with self.assertRaises(RuntimeError):
            self.dep.release()
        self.assertEqual(self.dep.path.read_bytes(), before)
        self.assertFalse(any(e[0] in {"up", "stop"} for e in self.dep.events if isinstance(e, tuple)))

    def test_failed_start_restores_compatible_prior_images(self):
        self.dep.prepare(self.template, SHA, NEW, NEW)
        self.dep.failure = "start"
        with self.assertRaises(RuntimeError):
            self.dep.release()
        self.assertEqual(self.dep.read()["services"]["mem0"]["image"], OLD)
        self.assertIn(("stop", "memory-service", "mem0"), self.dep.events)
        self.assertFalse(any("down" in e for e in self.dep.events))

    def test_config_change_blocks_automatic_rollback(self):
        self.dep.prepare(self.template, SHA, NEW, NEW)
        self.dep.values["test"] = "changed"
        self.dep.failure = "start"
        with self.assertRaises(RuntimeError):
            self.dep.release()
        self.assertEqual(self.dep.read()["services"]["mem0"]["image"], NEW)
        self.assertFalse(self.dep.read()["x-lazytt"]["released"])

    def test_first_failed_release_stops_without_dropping_data(self):
        doc = self.dep.read()
        doc["x-lazytt"].pop("released")
        self.dep.write(doc)
        self.dep.failure = "start"
        with self.assertRaises(RuntimeError):
            self.dep.release()
        self.assertFalse(self.dep.read()["x-lazytt"]["released"])
        self.assertNotIn(("down", "-v"), self.dep.events)

    def test_task_secrets_are_environment_only_and_stdin_closed(self):
        dep = release.Deployment(self.temp.name)
        with patch.object(release, "run", return_value="") as call:
            dep.task("database", "migrate", extra={"POSTGRES_PASSWORD": "fixture-not-a-real-secret"})
        args, kwargs = call.call_args
        self.assertNotIn("fixture-not-a-real-secret", " ".join(args[0]))
        self.assertEqual(kwargs["env"]["POSTGRES_PASSWORD"], "fixture-not-a-real-secret")

    def test_child_failure_does_not_print_secrets(self):
        result = subprocess.CompletedProcess(["docker"], 1, "fixture-secret", "fixture-secret")
        with patch.object(release.subprocess, "run", return_value=result) as call:
            with self.assertRaises(RuntimeError) as error:
                release.run(["docker", "compose"])
        self.assertNotIn("fixture-secret", str(error.exception))
        self.assertEqual(call.call_args.kwargs["stdin"], subprocess.DEVNULL)

    def test_mutable_image_rejected(self):
        with self.assertRaises(RuntimeError):
            self.dep.prepare(self.template, SHA, "ghcr.io/example/memory:dev", NEW)

    def test_migration_does_not_require_provider_or_jwt_materials(self):
        db = {"MEM0_POSTGRES_HOST": "fixture-pg", "MEM0_POSTGRES_PORT": "5432", "MEM0_POSTGRES_RUNTIME_PASSWORD": "fixture"}
        mode = SimpleNamespace(st_mode=0o100600, st_uid=0, st_gid=0)
        with patch.object(Path, "lstat", return_value=mode), patch.object(release, "run", return_value="\0".join(k + "=" + v for k, v in db.items())):
            self.assertEqual(release.read_config("/fixture", database_only=True), db)

    def test_application_file_rejects_migration_password(self):
        db = "MEM0_POSTGRES_HOST=pg\0MEM0_POSTGRES_PORT=5432\0MEM0_POSTGRES_RUNTIME_PASSWORD=fixture\0MEM0_MIGRATION_PASSWORD=fixture"
        mode = SimpleNamespace(st_mode=0o100600, st_uid=0, st_gid=0)
        with patch.object(Path, "lstat", return_value=mode), patch.object(release, "run", return_value=db):
            with self.assertRaisesRegex(RuntimeError, "Administrator/publishing"):
                release.read_config("/fixture", database_only=True)

    def test_stale_migration_revision_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "candidate changed"):
            self.dep.assert_candidate("b" * 40)
        self.dep.assert_candidate(SHA)

    def test_admin_bundle_survives_transport_cleanup_and_uses_one_compose(self):
        self.dep.install_admin_bundle(SHA)
        root = Path(self.temp.name)
        self.assertTrue((root / "admin" / SHA / "database/03_verify.sql").is_file())
        wrapper = (root / "database.sh").read_text()
        self.assertIn("--revision " + SHA, wrapper)
        self.assertNotIn("POSTGRES_PASSWORD", wrapper)
        self.assertEqual(list(root.rglob("*.yml")), [self.dep.path])
        self.dep.install_admin_bundle(SHA)
        (root / "admin" / SHA / "release.py").write_text("changed")
        with self.assertRaisesRegex(RuntimeError, "bundle changed"):
            self.dep.install_admin_bundle(SHA)

    def test_world_readable_config_is_rejected_before_loading(self):
        mode = SimpleNamespace(st_mode=0o100644, st_uid=0, st_gid=0)
        with patch.object(Path, "lstat", return_value=mode), patch.object(release, "run") as command:
            with self.assertRaisesRegex(RuntimeError, "0600"):
                release.read_config("/fixture")
            command.assert_not_called()


if __name__ == "__main__":
    unittest.main()
