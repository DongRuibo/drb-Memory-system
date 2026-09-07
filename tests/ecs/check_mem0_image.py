"""Run through stdin inside the built image, with --network none."""
import ast
import importlib.metadata
import importlib.util
from pathlib import Path
import site
import unittest
from unittest.mock import patch

import server_state
from mem0.utils.factory import VectorStoreFactory
from mem0.vector_stores.configs import VectorStoreConfig


class Mem0ImageTests(unittest.TestCase):
    def test_removed_backend_and_pool_are_absent_from_installed_image(self):
        names = {d.metadata["Name"].lower().replace("_", "-")
                 for d in importlib.metadata.distributions()}
        self.assertTrue({"pgvector", "psycopg-pool"}.isdisjoint(names))
        self.assertIn("psycopg", names)  # Still required by authentication/settings SQL.
        self.assertIsNone(importlib.util.find_spec("psycopg_pool"))
        for module in ("mem0.vector_stores.pgvector", "mem0.configs.vector_stores.pgvector"):
            self.assertIsNone(importlib.util.find_spec(module))
        self.assertNotIn("pgvector", VectorStoreFactory.provider_to_class)
        with self.assertRaises(ValueError):
            VectorStoreConfig(provider="pgvector", config={})
        roots = (Path("/app"), Path("/opt/lazytt"), *(Path(p) for p in site.getsitepackages()))
        for root in roots:
            for path in root.rglob("*"):
                if path.is_file():
                    self.assertNotIn("pgvector", path.name.lower(), str(path))
                    self.assertNotIn(b"pgvector", path.read_bytes().lower(), str(path))

    def test_first_start_waits_for_explicit_elasticsearch_configuration(self):
        tree = ast.parse(Path("/app/main.py").read_text())
        config = next(node.value for node in tree.body if isinstance(node, ast.Assign)
                      and any(isinstance(target, ast.Name) and target.id == "DEFAULT_CONFIG"
                              for target in node.targets))
        vector = next(ast.literal_eval(value) for key, value in zip(config.keys, config.values)
                      if ast.literal_eval(key) == "vector_store")
        self.assertEqual(vector, {"provider": "elasticsearch", "config": {}})
        with patch.object(server_state, "_load_overrides", return_value={}), \
             patch.object(server_state.Memory, "from_config") as factory:
            server_state.initialize_state({"vector_store": vector})
            factory.assert_not_called()
            with self.assertRaises(RuntimeError):
                server_state.get_memory_instance()

    def test_removed_backend_is_rejected_without_persisting_or_replacing_state(self):
        expected = {"vector_store": {"provider": "elasticsearch", "config": {}}}
        with patch.object(server_state, "_load_overrides", return_value={}), \
             patch.object(server_state.Memory, "from_config") as factory, \
             patch.object(server_state, "_save_overrides") as save:
            server_state.initialize_state(expected)
            with self.assertRaisesRegex(ValueError, "Only Elasticsearch"):
                server_state.update_config({"vector_store": {"provider": "pgvector"}})
            factory.assert_not_called()
            save.assert_not_called()
            self.assertEqual(server_state.get_current_config(), expected)
        with patch.object(server_state, "_load_overrides", return_value={
            "vector_store": {"provider": "pgvector", "config": {}}
        }), patch.object(server_state.Memory, "from_config") as factory:
            with self.assertRaisesRegex(RuntimeError, "Only Elasticsearch"):
                server_state.initialize_state(expected)
            factory.assert_not_called()


if __name__ == "__main__":
    unittest.main(verbosity=2)
