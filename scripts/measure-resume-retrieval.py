#!/usr/bin/env python3
"""Measure warm Memory search latency with synthetic records (not chat latency)."""
import argparse
import importlib.util
import json
import math
from pathlib import Path
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('live_eval', ROOT / 'scripts/eval-resume-live.py')
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tenant', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    run = str(uuid.uuid4())
    memory = live.Memory(args.tenant, 'latency-' + run, 'latency-project-' + run)
    proof = {'synthetic_data_only': True, 'sample_count': 30, 'warmup_count': 3,
             'query_concurrency': 1, 'record_count': 3, 'endpoint': 'POST /v1/memory/search', 'samples_ms': []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    try:
        for number, content in enumerate(['当前项目的权威数据库是 PostgreSQL。', '当前项目使用事务型 Outbox 投递消息。', '当前项目部署在本机 Docker 中。']):
            key = run + str(number)
            operation = memory.call('/v1/memory/events', {'memory_type': 'semantic_fact', 'semantic_key': 'latency.' + str(number),
                'event_id': key, 'source_type': 'conversation_message', 'source_ref': 'synthetic-latency:' + key,
                'content': '请记住：' + content, 'occurred_at': '2026-10-20T02:00:00Z',
                'scope_selector': {'kind': 'project', 'id': memory.project}}, key)
            for _ in range(240):
                state = memory.call('/v1/memory/operations/' + operation['operation_id'])['projection_status']
                if state == 'ready':
                    break
                assert state != 'failed'
                time.sleep(.25)
            else:
                raise RuntimeError('projection readiness timeout')
        for index in range(33):
            body = {'query': '当前项目的权威数据库是什么？', 'scope_selectors': [{'kind': 'project', 'id': memory.project}],
                'memory_types': ['semantic_fact'], 'limit': 3,
                'task_context': {'task_id': str(uuid.uuid4()), 'attempt_id': str(uuid.uuid4()), 'purpose': 'answer_user'}}
            started = time.perf_counter()
            response = memory.call('/v1/memory/search', body)
            latency = round((time.perf_counter() - started) * 1000, 2)
            assert response['hits'] and 'PostgreSQL' in response['hits'][0]['content'] and not response['degraded']
            proof['ranking_version'] = response['ranking_version']
            if index >= 3:
                proof['samples_ms'].append(latency)
            args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + '\n')
            print('request', index + 1, latency, 'ms', flush=True)
        ordered = sorted(proof['samples_ms'])
        proof.update({'p50_ms': ordered[math.ceil(len(ordered) * .50) - 1],
                      'p95_ms': ordered[math.ceil(len(ordered) * .95) - 1], 'all_requests_succeeded': True,
                      'boundary': 'single repeated query, 3 records, loopback client; includes local retrieval and DeepSeek verification, excludes chat generation and extraction'})
    finally:
        proof['cleanup_record_count'] = memory.cleanup()
        args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + '\n')


if __name__ == '__main__':
    main()
