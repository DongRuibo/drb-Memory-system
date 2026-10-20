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
import urllib.parse

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
    parser.add_argument('--memory-base', default='http://127.0.0.1:19080')
    parser.add_argument('--regression', action='store_true',
        help='reuse an inspected dataset with a changed policy; dataset hash remains mandatory')
    args = parser.parse_args()
    url = urllib.parse.urlsplit(args.memory_base)
    if (url.scheme != 'http' or url.hostname not in ('127.0.0.1', 'localhost', '::1')
            or url.username or url.password or url.query or url.fragment or url.path not in ('', '/')):
        parser.error('memory-base must be a local HTTP origin')
    args.memory_base = args.memory_base.rstrip('/')
    if args.output.exists():
        parser.error('output must be a new directory')
    dataset_hash = hashlib.sha256(args.dataset.read_bytes()).hexdigest()
    freeze = args.dataset.parent / 'freeze.json'
    frozen = {}
    if freeze.exists():
        frozen = json.loads(freeze.read_text())
        hashes = dict(frozen['hashes'])
        if args.dataset.resolve() not in {(ROOT / p).resolve() for p in hashes}:
            parser.error('dataset path is not covered by its freeze manifest')
        if frozen.get('policy_freeze'):
            hashes.update(json.loads((args.dataset.parent / frozen['policy_freeze']).read_text())['hashes'])
        for path, expected in hashes.items():
            if args.regression and (ROOT / path).resolve() != args.dataset.resolve():
                continue
            if hashlib.sha256((ROOT / path).read_bytes()).hexdigest() != expected:
                parser.error('frozen dataset or retrieval policy changed: ' + path)
    runner = load('resume_runner', ROOT / 'scripts/resume-demo.py')
    live = load('resume_live', ROOT / 'scripts/eval-resume-live.py')
    run = str(uuid.uuid4())
    user, project = 'final-eval-' + run, 'final-project-' + run
    memory = live.Memory(args.tenant, user, project, args.memory_base)
    initial_records = {r['memory_record_id'] for r in memory.call('/v1/memory/export')['records']}
    if initial_records:
        raise RuntimeError('evaluation requires fresh user and project scopes')
    search = memory.call('/v1/memory/search', {'query': '合成评测初始化', 'limit': 1,
        'scope_selectors': [{'kind': 'user', 'id': user}, {'kind': 'project', 'id': project}],
        'task_context': {'task_id': str(uuid.uuid4()), 'attempt_id': str(uuid.uuid4()), 'purpose': 'answer_user'}})
    env = os.environ | runner.read_env(ROOT / '.env') | {
        'MEMORY_BASE_URL': args.memory_base, 'MEM0_BASE_URL': 'http://127.0.0.1:19889',
        'MEMORY_BEARER_TOKEN': memory.token, 'EVAL_TENANT_ID': args.tenant, 'EVAL_USER_ID': user,
        'EVAL_PROJECT_ID': project, 'EVAL_GENERATION': search['projection_generation'],
        'EVAL_DATASET': str(args.dataset.resolve()), 'EVAL_OUTPUT_DIR': str(args.output.resolve()),
        'EVAL_PROVIDER_PROFILE': 'qwen3-cpu-plus-deepseek-query-time-v6', 'EVAL_QUERY_CONCURRENCY': '1',
        'EVAL_ENFORCE_GATE': 'false', 'NO_PROXY': '127.0.0.1,localhost',
    }
    args.output.mkdir(parents=True)
    (args.output / 'evaluation-design.json').write_text(json.dumps({
        'dataset_sha256': dataset_hash, 'dataset_use': 'regression' if args.regression else 'frozen evaluation',
        'policy_hashes': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in [
            'apps/memory-service/src/support.rs', 'apps/memory-service/src/api.rs',
            'crates/memory-application/src/reranker.rs', 'crates/memory-application/src/temporal.rs',
            'apps/retrieval-eval/src/main.rs']},
        'initial_ranking_version': search.get('ranking_version'),
        'additional_precision_gate': frozen.get('additional_precision_gate', {'irrelevant_injection_rate_max': 0.1}),
    }, indent=2) + '\n')
    try:
        result = subprocess.run([str(ROOT / 'target/debug/retrieval-eval')], cwd=ROOT, env=env)
        if result.returncode:
            raise RuntimeError('retrieval evaluation failed; inspect its local output')
        assert hashlib.sha256(args.dataset.read_bytes()).hexdigest() == dataset_hash
        (args.output / 'dataset-sha256.json').write_text(json.dumps({'dataset': str(args.dataset),
            'sha256': dataset_hash, 'user': user, 'project': project}, indent=2) + '\n')
        metrics = json.loads((args.output / 'metric-results.json').read_text())
        maximum = frozen.get('additional_precision_gate', {}).get('irrelevant_injection_rate_max', 0.1)
        precision = metrics['methods']['hybrid_service']['irrelevant_injection_rate']
        passed = metrics['release_gate_passed'] and precision <= maximum
        (args.output / 'acceptance.json').write_text(json.dumps({
            'original_release_gate_passed': metrics['release_gate_passed'],
            'irrelevant_injection_rate': precision, 'irrelevant_injection_rate_max': maximum,
            'precision_gate_passed': precision <= maximum, 'all_gates_passed': passed,
        }, indent=2) + '\n')
    finally:
        created = {r['memory_record_id'] for r in memory.call('/v1/memory/export')['records']} - initial_records
        memory.cleanup(created)
    if not passed:
        raise SystemExit('Evaluation recorded; one or more acceptance gates failed')


if __name__ == '__main__':
    main()
