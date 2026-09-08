from __future__ import annotations

import os
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import unittest
from unittest.mock import patch

import configure


class VectorStoreConfigTests(unittest.TestCase):
    def test_defaults_to_elasticsearch_without_postgres_vector_settings(self) -> None:
        environment = {
            "MEM0_ELASTICSEARCH_HOST": "https://elasticsearch.example.com",
            "MEM0_ELASTICSEARCH_PORT": "9200",
            "MEM0_ELASTICSEARCH_USER": "memory-user",
            "MEM0_ELASTICSEARCH_PASSWORD": "test-password-not-secret",
            "MEM0_COLLECTION_NAME": "mem0-test",
        }
        with patch.dict(os.environ, environment, clear=True):
            result = configure.vector_store_config(1024)
        self.assertEqual(result["provider"], "elasticsearch")
        self.assertNotIn("dbname", result["config"])

    def test_rejects_removed_or_unexpected_backend_without_echoing_input(self) -> None:
        for provider in ("pgvector", "qdrant", "", "credential-like-input"):
            with self.subTest(provider=provider), patch.dict(
                os.environ, {"MEM0_VECTOR_STORE_PROVIDER": provider}, clear=True
            ):
                with self.assertRaisesRegex(RuntimeError, "^MEM0_VECTOR_STORE_PROVIDER must be elasticsearch$"):
                    configure.vector_store_config(1024)

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


class ConfigurationRequestTests(unittest.TestCase):
    def test_configuration_can_finish_after_the_read_request_deadline(self):
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                self.rfile.read(int(self.headers['Content-Length']))
                # Deliberately exceed the former 10-second read timeout.
                time.sleep(11)
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b'{}')

        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            url = f'http://127.0.0.1:{server.server_port}/configure'
            self.assertEqual(configure.request_json(url, 'POST', 'unit-fixture', {}), {})
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)
        self.assertFalse(thread.is_alive())

    def test_unknown_configuration_write_is_not_retried(self):
        with patch.object(configure.urllib.request, 'urlopen', side_effect=TimeoutError) as request:
            with self.assertRaises(TimeoutError):
                configure.request_json('http://unit-fixture/configure', 'POST', 'unit-fixture', {})
        self.assertEqual(request.call_count, 1)


if __name__ == "__main__":
    unittest.main()
