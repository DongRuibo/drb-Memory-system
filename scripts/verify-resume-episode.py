#!/usr/bin/env python3
"""Exercise completed-turn idempotency and real-model Episode consolidation."""
import argparse
import importlib.util
import json
from pathlib import Path
import time
import uuid
import urllib.error

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('live_eval', ROOT / 'scripts/eval-resume-live.py')
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tenant', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    episode = str(uuid.uuid4())
    memory = live.Memory(args.tenant, 'episode-' + episode, 'episode-project-' + episode)
    proof = {'synthetic_data_only': True, 'episode_id': episode, 'user_id': memory.user, 'turns': [], 'checks': {}}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    initial_records = {r['memory_record_id'] for r in memory.call('/v1/memory/export')['records']}
    if initial_records:
        raise RuntimeError('Episode validation requires fresh user and project scopes')

    def save():
        args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + '\n')

    try:
        for number in range(1, 11):
            turn = str(uuid.uuid4())
            payload = {'turn_id': turn, 'task_id': turn, 'attempt_id': turn,
                'user_content': f'2026年10月19日下午，我验证了 task-{number} 后台任务。',
                'assistant_content': '已确认。助手另行猜测用户是公司CEO，此猜测没有用户来源支持。',
                'verified_tool_results': [], 'outcome': 'succeeded', 'completed_at': '2026-10-20T02:00:00Z', 'timezone': 'Asia/Shanghai'}
            response = memory.call('/v1/memory/episodes/' + episode + '/turns', payload, turn)
            proof['turns'].append(response)
            assert response['sequence_no'] == number
            if number == 1:
                replay = memory.call('/v1/memory/episodes/' + episode + '/turns', payload, turn)
                proof['checks']['turn_replay_stable'] = replay == response
                assert proof['checks']['turn_replay_stable']
            save()
            print('completed turn', number, flush=True)
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            status = memory.call('/v1/memory/episodes/' + episode + '/status')
            proof['status'] = status
            save()
            if status['consolidations'] and all(item['status'] in ['completed', 'failed'] for item in status['consolidations']):
                break
            time.sleep(.5)
        jobs = status['consolidations']
        proof['checks']['one_completed_job_for_ten_turns'] = len(jobs) == 1 and jobs[0]['status'] == 'completed' and jobs[0]['from_turn_sequence'] == 1 and jobs[0]['through_turn_sequence'] == 10
        proof['checks']['bounded_real_model_proposals'] = len(jobs) == 1 and 0 <= (jobs[0]['proposal_count'] or 0) <= 4
        exported = memory.call('/v1/memory/export')
        proof['export'] = exported
        proof['checks']['assistant_guess_not_promoted'] = all('CEO' not in (version['content'] or '') for row in exported['records'] for version in row['versions'])
        save()
        try:
            memory.call('/v1/memory/episodes/' + episode + '/consolidations', {'trigger': 'manual'}, str(uuid.uuid4()))
            proof['checks']['no_uncovered_turns_rejected'] = False
        except urllib.error.HTTPError as error:
            proof['checks']['no_uncovered_turns_rejected'] = error.code == 409
        after = memory.call('/v1/memory/episodes/' + episode + '/status')
        proof['checks']['covered_range_not_duplicated'] = len(after['consolidations']) == 1
        proof['all_passed'] = all(proof['checks'].values())
        save()
        print(json.dumps(proof['checks']), flush=True)
        assert proof['all_passed']
    finally:
        created = {r['memory_record_id'] for r in memory.call('/v1/memory/export')['records']} - initial_records
        proof['cleanup_record_count'] = memory.cleanup(created)
        save()


if __name__ == '__main__':
    main()
