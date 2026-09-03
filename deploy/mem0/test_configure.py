from __future__ import annotations

import os
import unittest
from unittest.mock import patch

import configure


class VectorStoreConfigTests(unittest.TestCase):
    def test_elasticsearch_uses_basic_auth_and_requested_dimensions(self) -> None:
        environment = {
            "MEM0_VECTOR_STORE_PROVIDER": "elasticsearch",
            "MEM0_ELASTICSEARCH_HOST": "https://elasticsearch.example.com",
            "MEM0_ELASTICSEARCH_PORT": "9200",
            "MEM0_ELASTICSEARCH_USER": "memory-user",
            "MEM0_ELASTICSEARCH_PASSWORD": "test-password-not-secret",
            "MEM0_ELASTICSEARCH_VERIFY_CERTS": "true",
            "MEM0_COLLECTION_NAME": "mem0-test",
        }
        with patch.dict(os.environ, environment, clear=True):
            vector_store = configure.vector_store_config(1024)

        self.assertEqual(vector_store["provider"], "elasticsearch")
        self.assertEqual(vector_store["config"]["embedding_model_dims"], 1024)
        self.assertEqual(vector_store["config"]["collection_name"], "mem0-test")
        self.assertTrue(vector_store["config"]["verify_certs"])

    def test_elasticsearch_rejects_a_host_path(self) -> None:
        environment = {
            "MEM0_VECTOR_STORE_PROVIDER": "elasticsearch",
            "MEM0_ELASTICSEARCH_HOST": "https://elasticsearch.example.com/private",
            "MEM0_ELASTICSEARCH_PORT": "9200",
            "MEM0_ELASTICSEARCH_USER": "memory-user",
            "MEM0_ELASTICSEARCH_PASSWORD": "test-password-not-secret",
            "MEM0_COLLECTION_NAME": "mem0-test",
        }
        with patch.dict(os.environ, environment, clear=True):
            with self.assertRaisesRegex(RuntimeError, "without credentials or a path"):
                configure.vector_store_config(1024)


if __name__ == "__main__":
    unittest.main()
