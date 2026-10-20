#!/usr/bin/env python3
"""Evaluate synthetic extraction through the running recovery Memory API."""
import argparse
import base64
import hashlib
import hmac
import json
from pathlib import Path
import shlex
import time
import urllib.error
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]


def jwt(tenant, user, project):
    values = {}
    for line in (ROOT / '.env').read_text().splitlines():
        if '=' in line and not line.lstrip().startswith('#'):
            k, v = line.split('=', 1)
            words = shlex.split(v, comments=True)
            values[k] = words[0] if words else ''
    now = int(time.time())
    encode = lambda value: base64.urlsafe_b64encode(value).rstrip(b'=')
    claims = {'iss': 'lazytt-demo-web-api', 'aud': 'lazytt-memory-service', 'sub': user,
              'tenant_id': tenant, 'user_id': user, 'tenant_scope': False,
              'project_ids': [project], 'agent_ids': [], 'permissions': ['memory:*'],
              'iat': now, 'nbf': now - 1, 'exp': now + 3600}
    message = encode(b'{"alg":"HS256","typ":"JWT"}') + b'.' + encode(json.dumps(claims).encode())
    return (message + b'.' + encode(hmac.new(values['MEMORY_JWT_HS256_SECRET'].encode(), message, hashlib.sha256).digest())).decode()


class Memory:
    def __init__(self, tenant, user, project):
        self.user, self.project = user, project
        self.token = jwt(tenant, user, project)
        self.client = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def call(self, path, body=None, key=None):
        headers = {'Content-Type': 'application/json', 'Authorization': 'Bearer ' + self.token}
        if key:
            headers['Idempotency-Key'] = key
        request = urllib.request.Request('http://127.0.0.1:19080' + path,
            data=None if body is None else json.dumps(body, ensure_ascii=False).encode(), headers=headers)
        with self.client.open(request, timeout=120) as response:
            raw = response.read()
            return json.loads(raw) if raw else None

    def forget(self, record, revision):
        return self.call('/v1/memories/' + record + '/forget',
            {'expected_revision': revision, 'reason': 'synthetic validation cleanup'}, str(uuid.uuid4()))


    def cleanup(self):
        exported = self.call('/v1/memory/export')
        for record in exported['records']:
            if record['head_state'] == 'contested':
                event = next(event for event in reversed(record['events']) if event['event_type'] == 'memory_contested')
                self.call('/v1/memory-candidates/' + event['payload']['candidate_id'] + '/decisions',
                    {'expected_candidate_revision': 0, 'outcome': 'reject', 'reason': 'synthetic validation cleanup retains original value'}, str(uuid.uuid4()))
                record = self.call('/v1/memories/' + record['memory_record_id'])
            if record['head_state'] == 'active':
                self.forget(record['memory_record_id'], record['revision'])
        return exported['record_count']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tenant', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    data = json.loads((ROOT / 'evals/extraction-resume/dataset.json').read_text())
    run = str(uuid.uuid4())
    results = []
    args.output.parent.mkdir(parents=True, exist_ok=True)
    for case in data['cases']:
        memory = Memory(args.tenant, 'extract-' + run + '-' + case['id'], 'project-' + run + '-' + case['id'])
        payload = {'event_id': run + case['id'], 'source_type': 'conversation_message',
                   'source_ref': 'synthetic:' + run + ':' + case['id'], 'content': case['source'],
                   'occurred_at': data['source_event_time'], 'timezone': data['timezone'], 'max_candidates': 8,
                   'scope_selector': {'kind': 'user', 'id': memory.user},
                   'project_scope_selector': {'kind': 'project', 'id': memory.project}}
        started = time.monotonic()
        result = {'case_id': case['id'], 'source': case['source'], 'expected_types': case['expected_types']}
        try:
            response = memory.call('/v1/memory/extractions', payload, run + case['id'])
            proposals = [item['proposal'] for item in response['candidates']]
            result.update({'response': response, 'latency_ms': round((time.monotonic() - started) * 1000)})
            passed = sorted(item['memory_type'] for item in proposals) == sorted(case['expected_types'])
            if case['check'] in ('global', 'contextual'):
                passed &= all((item.get('structured_content') or {}).get('payload', {}).get('applicability', 'contextual') == case['check'] for item in proposals)
            if case['check'] == 'event_time':
                passed &= bool(proposals) and all(item.get('event_time') is not None for item in proposals)
            if case['check'] == 'no_event_time':
                passed &= all(item.get('event_time') is None for item in proposals)
            if case['id'] == 'global-language':
                replay = memory.call('/v1/memory/extractions', payload, run + case['id'])
                result['idempotent_replay'] = replay == response
                passed &= result['idempotent_replay']
            result['passed'] = passed
            cleaned = set()
            for item in response['candidates']:
                op = item['operation']
                if op.get('memory_record_id') and op['memory_record_id'] not in cleaned:
                    memory.forget(op['memory_record_id'], op['head_revision'])
                    cleaned.add(op['memory_record_id'])
            result['cleaned_record_count'] = len(cleaned)
        except urllib.error.HTTPError as error:
            result.update({'passed': error.code == case.get('expected_http_status'), 'http_status': error.code})
        except Exception as error:
            result.update({'passed': False, 'error_type': type(error).__name__})
        results.append(result)
        args.output.write_text(json.dumps({'version': data['version'], 'run_id': run, 'synthetic_data_only': True,
            'passed': sum(item['passed'] for item in results), 'total': len(results), 'results': results}, ensure_ascii=False, indent=2) + '\n')
        print(case['id'], 'PASS' if result['passed'] else 'FAIL', flush=True)
    raise SystemExit(0 if all(item['passed'] for item in results) else 2)


if __name__ == '__main__':
    main()
