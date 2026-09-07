"""Executed inside the isolated gateway container against its real HTTP server."""
import json
import os
import urllib.error
import urllib.request


def request(path, method='GET', body=None, key=None, status=200):
    headers = {'Content-Type': 'application/json', 'X-API-Key': key or os.environ['ADMIN_API_KEY']}
    req = urllib.request.Request('http://127.0.0.1:8000' + path, method=method, headers=headers,
                                 data=json.dumps(body).encode() if body is not None else None)
    try:
        response = urllib.request.urlopen(req, timeout=90)
    except urllib.error.HTTPError as error:
        response = error
    data = response.read()
    assert response.status == status, 'Unexpected HTTP status for ' + path
    return json.loads(data)


request('/health/ready')
request('/configure', key='invalid', status=401)
config = request('/configure')
assert config['vector_store']['provider'] == 'elasticsearch'
assert config['vector_store']['config']['password'] == '[redacted]'
assert config['llm']['config']['api_key'] == '[redacted]'
for provider in ('pgvector', 'qdrant'):
    altered = json.loads(json.dumps(config))
    altered['vector_store']['provider'] = provider
    request('/configure', 'POST', altered, status=422)
altered = json.loads(json.dumps(config))
altered['vector_store']['config']['collection_name'] = 'unapproved-index-change'
request('/configure', 'POST', altered, status=409)
request('/search', 'POST', {'query': 'test', 'filters': {}, 'top_k': 2}, status=422)
body = {'messages': [{'role': 'user', 'content': 'prefers brief answers'}], 'user_id': 'http-user', 'infer': True}
request('/memories', 'POST', body, key='invalid', status=401)
created = request('/memories', 'POST', body)
assert created['results'], 'Expected inferred memory from test model'
memory_id = created['results'][0]['id']
assert request('/memories/' + memory_id)['user_id'] == 'http-user'
# Elasticsearch refresh is separate from the provider's indexing call.
from elasticsearch import Elasticsearch
es = Elasticsearch('http://' + os.environ['POSTGRES_HOST'].removesuffix('-pg') + '-es:9200')
es.indices.refresh(index='test-memories')
found = request('/search', 'POST', {'query': 'answer preference', 'filters': {'user_id': 'http-user'}, 'top_k': 10})
assert any(item['id'] == memory_id for item in found['results'])
other = request('/search', 'POST', {'query': 'answer preference', 'filters': {'user_id': 'other-user'}, 'top_k': 10})
assert not other['results']
request('/memories/' + memory_id, 'DELETE')
request('/memories/' + memory_id, status=404)
print('GATEWAY_HTTP_OK: inferred write/search/get/delete, auth rejection, ES-only config and user filtering; deterministic model fixture')
