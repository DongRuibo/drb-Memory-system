"""Read-only release preflight must work before any Mem0 HTTP server starts."""
import copy
import importlib.util
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "deploy/mem0"))
spec = importlib.util.spec_from_file_location("configuration", ROOT / "deploy/ecs/configuration.py")
configuration = importlib.util.module_from_spec(spec)
spec.loader.exec_module(configuration)


class ConfigurationTests(unittest.TestCase):
    def setUp(self):
        self.expected = {
            "vector_store": {"provider": "elasticsearch", "config": {
                "collection_name": "fixture", "embedding_model_dims": 16, "host": "https://es.example", "port": 9200}},
            "embedder": {"provider": "openai", "config": {
                "model": "fixture", "embedding_dims": 16, "openai_base_url": "https://model.example/v1", "api_key": "fixture"}},
            "llm": {"provider": "openai", "config": {"model": "fixture", "api_key": "fixture"}},
        }

    def preflight(self, current):
        storage = SimpleNamespace(engine=Mock(), load_config=Mock(return_value=current), save_config=Mock())
        with patch.dict(sys.modules, {"storage": storage}), \
             patch.object(configuration, "desired", return_value=self.expected), \
             patch.object(configuration.configure, "request_json") as http:
            try:
                configuration.main("preflight")
            finally:
                http.assert_not_called()
                storage.save_config.assert_not_called()

    def test_first_configuration_needs_only_database_read(self):
        self.preflight({})

    def test_unchanged_configuration_needs_no_http(self):
        self.preflight(copy.deepcopy(self.expected))

    def test_incompatible_index_and_embedding_are_rejected_before_http(self):
        for section, field in (("vector_store", "collection_name"), ("vector_store", "host"),
                               ("vector_store", "port"), ("vector_store", "embedding_model_dims"),
                               ("embedder", "model"), ("embedder", "embedding_dims"), ("embedder", "openai_base_url")):
            with self.subTest(field=field):
                current = copy.deepcopy(self.expected)
                current[section]["config"][field] = "changed"
                with self.assertRaisesRegex(RuntimeError, "migration"):
                    self.preflight(current)

    def test_unknown_stored_provider_is_rejected(self):
        current = copy.deepcopy(self.expected)
        current["vector_store"]["provider"] = "unsupported"
        with self.assertRaisesRegex(RuntimeError, "migration"):
            self.preflight(current)

    def test_credential_rotation_does_not_require_index_migration(self):
        current = copy.deepcopy(self.expected)
        current["embedder"]["config"]["api_key"] = "old-fixture"
        self.preflight(current)
