#!/usr/bin/env python3
"""Evaluate a frozen synthetic dataset against the running recovery service.

Each invocation uses a separate synthetic user/project. Credentials stay in
process memory; the existing retrieval driver forgets its ingested records.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import uuid

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tenant', required=True)
    parser.add_argument('--dataset', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error('output must be a new directory')
    dataset_hash = hashlib.sha256(args.dataset.read_bytes()).hexdigest()
    freeze = args.dataset.parent / 'freeze.json'
    if freeze.exists():
        for path, expected in json.loads(freeze.read_text())['hashes'].items():
            if hashlib.sha256((ROOT / path).read_bytes()).hexdigest() != expected:
                parser.error('frozen dataset or retrieval policy changed: ' + path)
    runner = load('resume_runner', ROOT / 'scripts/resume-demo.py')
    live = load('resume_live', ROOT / 'scripts/eval-resume-live.py')
    run = str(uuid.uuid4())
    user, project = 'final-eval-' + run, 'final-project-' + run
    memory = live.Memory(args.tenant, user, project)
    initial_records = {r['memory_record_id'] for r in memory.call('/v1/memory/export')['records']}
    if initial_records:
        raise RuntimeError('evaluation requires fresh user and project scopes')
    search = memory.call('/v1/memory/search', {'query': '合成评测初始化', 'limit': 1,
        'scope_selectors': [{'kind': 'user', 'id': user}, {'kind': 'project', 'id': project}],
        'task_context': {'task_id': str(uuid.uuid4()), 'attempt_id': str(uuid.uuid4()), 'purpose': 'answer_user'}})
    env = os.environ | runner.read_env(ROOT / '.env') | {
        'MEMORY_BASE_URL': 'http://127.0.0.1:19080', 'MEM0_BASE_URL': 'http://127.0.0.1:19889',
        'MEMORY_BEARER_TOKEN': memory.token, 'EVAL_TENANT_ID': args.tenant, 'EVAL_USER_ID': user,
        'EVAL_PROJECT_ID': project, 'EVAL_GENERATION': search['projection_generation'],
        'EVAL_DATASET': str(args.dataset.resolve()), 'EVAL_OUTPUT_DIR': str(args.output.resolve()),
        'EVAL_PROVIDER_PROFILE': 'qwen3-cpu-plus-deepseek-event-time-v2', 'EVAL_QUERY_CONCURRENCY': '1',
        'EVAL_ENFORCE_GATE': 'false', 'NO_PROXY': '127.0.0.1,localhost',
    }
    try:
        result = subprocess.run([str(ROOT / 'target/debug/retrieval-eval')], cwd=ROOT, env=env)
        if result.returncode:
            raise RuntimeError('retrieval evaluation failed; inspect its local output')
        assert hashlib.sha256(args.dataset.read_bytes()).hexdigest() == dataset_hash
        (args.output / 'dataset-sha256.json').write_text(json.dumps({'dataset': str(args.dataset),
            'sha256': dataset_hash, 'user': user, 'project': project}, indent=2) + '\n')
    finally:
        created = {r['memory_record_id'] for r in memory.call('/v1/memory/export')['records']} - initial_records
        memory.cleanup(created)


if __name__ == '__main__':
    main()
