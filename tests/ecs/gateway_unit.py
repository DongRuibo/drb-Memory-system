"""Failure-path tests run in the built image; no database or provider connection."""
import asyncio
from copy import deepcopy
import unittest
from unittest.mock import patch
from fastapi.testclient import TestClient
import gateway

CONFIG = {
    'vector_store': {'provider': 'elasticsearch', 'config': {'host': 'http://fixture', 'port': 9200,
        'user': 'fixture', 'password': 'secret-marker', 'collection_name': 'fixture', 'embedding_model_dims': 16}},
    'llm': {'provider': 'openai', 'config': {'model': 'fixture', 'api_key': 'secret-marker'}},
    'embedder': {'provider': 'openai', 'config': {'model': 'fixture', 'api_key': 'secret-marker', 'embedding_dims': 16}}}


class GatewayTests(unittest.TestCase):
    def setUp(self):
        gateway.CURRENT = {}
        gateway.MEMORY = None
        self.client = TestClient(gateway.app, raise_server_exceptions=False)
        self.headers = {'X-API-Key': gateway.KEY}

    def test_stored_es_failure_keeps_only_setup_available(self):
        async def start():
            async with gateway.lifespan(gateway.app):
                self.assertIsNone(gateway.MEMORY)
                self.assertEqual(gateway.CURRENT, CONFIG)
        with patch.object(gateway.DB, 'connect'), patch.object(gateway.database, 'identity'), \
             patch.object(gateway.database, 'verify'), patch.object(gateway.storage, 'load_config', return_value=CONFIG), \
             patch.object(gateway, 'new_memory', side_effect=RuntimeError('secret-marker')):
            asyncio.run(start())
        self.assertEqual(self.client.get('/health/ready').status_code, 503)
        self.assertEqual(self.client.get('/configure', headers=self.headers).status_code, 200)

    def test_failed_commit_does_not_activate_configuration(self):
        with patch.object(gateway, 'new_memory', return_value=object()), patch.object(gateway.storage, 'save_config', side_effect=RuntimeError('secret-marker')):
            response = self.client.post('/configure', json=CONFIG, headers=self.headers)
        self.assertEqual(response.status_code, 503)
        self.assertNotIn('secret-marker', response.text)
        self.assertEqual(gateway.CURRENT, {})
        self.assertIsNone(gateway.MEMORY)

    def test_reject_other_provider_before_constructing_it(self):
        config = deepcopy(CONFIG)
        config['vector_store']['provider'] = 'pgvector'
        with patch.object(gateway, 'new_memory') as factory:
            response = self.client.post('/configure', json=config, headers=self.headers)
        self.assertEqual(response.status_code, 422)
        factory.assert_not_called()

    def test_bad_key_cannot_read_or_write_config(self):
        with patch.object(gateway, 'new_memory') as factory:
            for method in ('get', 'post'):
                response = self.client.request(method, '/configure', json=CONFIG)
                self.assertEqual(response.status_code, 401)
        factory.assert_not_called()

    def test_validation_never_echoes_request_input(self):
        response = self.client.post('/memories', json={'messages': 'secret-marker'}, headers=self.headers)
        self.assertEqual(response.status_code, 422)
        self.assertNotIn('secret-marker', response.text)

    def test_persisted_config_is_activated_and_redacted(self):
        instance = object()
        with patch.object(gateway, 'new_memory', return_value=instance), patch.object(gateway.storage, 'save_config') as save:
            response = self.client.post('/configure', json=CONFIG, headers=self.headers)
        self.assertEqual(response.status_code, 200)
        self.assertIs(gateway.MEMORY, instance)
        save.assert_called_once()
        response = self.client.get('/configure', headers=self.headers)
        self.assertNotIn('secret-marker', response.text)


unittest.main()
