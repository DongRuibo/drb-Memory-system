#!/usr/bin/env python3
"""Run the frozen 16 applicability regressions through real retrieval and authority checks."""
import argparse
import importlib.util
import json
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
    data = json.loads((ROOT / 'evals/retrieval-v2/reranker-answer-support.json').read_text())
    results = []
    args.output.parent.mkdir(parents=True, exist_ok=True)
    for case in data['cases']:
        run = str(uuid.uuid4())
        memory = live.Memory(args.tenant, 'applicability-' + run, 'applicability-project-' + run)
        result = {'case_id': case['id'], 'query': case['query'], 'expected_relevant': case['relevant']}
        try:
            project = case['memory_type'] == 'semantic_fact'
            operation = memory.call('/v1/memory/events', {'memory_type': case['memory_type'], 'semantic_key': 'applicability.' + case['id'],
                'event_id': run, 'source_type': 'conversation_message', 'source_ref': 'synthetic-applicability:' + run,
                'content': '请记住：' + case['content'], 'occurred_at': '2026-10-20T02:00:00Z',
                'scope_selector': {'kind': 'project', 'id': memory.project} if project else {'kind': 'user', 'id': memory.user}}, run)
            for _ in range(240):
                status = memory.call('/v1/memory/operations/' + operation['operation_id'])['projection_status']
                if status == 'ready':
                    break
                assert status != 'failed'
                time.sleep(.25)
            else:
                raise RuntimeError('projection timeout')
            response = memory.call('/v1/memory/search', {'query': case['query'],
                'scope_selectors': [{'kind': 'project', 'id': memory.project}, {'kind': 'user', 'id': memory.user}],
                'memory_types': [case['memory_type']], 'limit': 3,
                'task_context': {'task_id': str(uuid.uuid4()), 'attempt_id': str(uuid.uuid4()), 'purpose': 'answer_user'}})
            result.update({'response': response, 'passed': bool(response['hits']) == case['relevant'] and not response['degraded']})
        except Exception as error:
            result.update({'passed': False, 'error_type': type(error).__name__})
        finally:
            result['cleanup_record_count'] = memory.cleanup()
        results.append(result)
        args.output.write_text(json.dumps({'version': data['version'], 'method': 'full Memory API with Qwen plus DeepSeek support',
            'synthetic_data_only': True, 'passed': sum(item['passed'] for item in results), 'total': len(results), 'results': results}, ensure_ascii=False, indent=2) + '\n')
        print(case['id'], 'PASS' if result['passed'] else 'FAIL', flush=True)
    raise SystemExit(0 if all(item['passed'] for item in results) else 2)


if __name__ == '__main__':
    main()
