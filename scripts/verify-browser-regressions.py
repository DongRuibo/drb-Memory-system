#!/usr/bin/env python3
"""Verify the original browser failure cases through real chat and context snapshots.

Use a fresh isolated tenant. This driver calls DeepSeek through the backend and
does not claim to test browser buttons or visual layout.
"""
import argparse
import http.cookiejar
import importlib.util
import json
from pathlib import Path
import time
import urllib.parse
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('live', ROOT / 'scripts/eval-resume-live.py')
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)
SOURCE = ('我是 Agent 研发工程师，主要使用 Rust。我们正在开发企业级 Agent 记忆系统，'
          '当前项目使用 PostgreSQL 作为权威数据库。2026年7月15日下午做 Outbox 时遇到过重复消费，'
          '后来用 event_id 去重解决了。我喜欢回答技术问题时先给一句结论，再列出要点。'
          '请根据这些情况，简要说说上线前最需要关注什么。')
DATE_QUERY = '我在2026年7月15日下午处理过什么工程问题，最后是怎么解决的？请根据已有记忆回答，不知道的不要猜。'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tenant', required=True)
    parser.add_argument('--memory-base', default='http://127.0.0.1:19080')
    parser.add_argument('--demo-base', default='http://127.0.0.1:19090')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    for base in (args.memory_base, args.demo_base):
        url = urllib.parse.urlsplit(base)
        if (url.scheme != 'http' or url.hostname not in ('127.0.0.1', 'localhost', '::1')
                or url.username or url.password or url.query or url.fragment or url.path not in ('', '/')):
            parser.error('regression API URLs must be local HTTP origins')
    proof = {'synthetic_data_only': True, 'checks': {}, 'turns': [], 'extractions': []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    client = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
    revision = None

    def save():
        args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + '\n')

    def check(name, condition):
        proof['checks'][name] = bool(condition)
        save()
        print(name, 'PASS' if condition else 'FAIL', flush=True)
        assert condition, name

    def call(base, path, body=None, headers=None):
        request = urllib.request.Request(base + path, headers={'Content-Type': 'application/json'} | (headers or {}),
            data=None if body is None else json.dumps(body, ensure_ascii=False).encode())
        with client.open(request, timeout=120) as response:
            return json.load(response)

    def demo(path, body=None):
        return call(args.demo_base, path, body, {'x-demo-identity-revision': str(revision)} if revision else {})

    def memory(user, project, path, body=None):
        return call(args.memory_base, path, body, {'Authorization': 'Bearer ' + live.jwt(args.tenant, user, project),
            'Idempotency-Key': str(uuid.uuid4())})

    project = 'lazytt-memory-system-demo'

    def alice(path, body=None):
        return memory('demo-alice', project, path, body)

    def turn(message):
        sid = demo('/api/chat/sessions', {'title': '合成回归：' + message[:20], 'mode': 'free'})['session']['id']
        key = str(uuid.uuid4())
        current = demo('/api/chat/turns', {'session_id': sid, 'message': message, 'client_message_id': key, 'idempotency_key': key})['turn']
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            current = demo('/api/chat/turns/' + current['id'])['turn']
            if current['status'] in ('completed', 'failed', 'cancelled'):
                break
            time.sleep(.25)
        view = demo('/api/chat/sessions/' + sid)['session']
        answer = next((m['content'] for m in view['messages'] if m['id'] == current['assistant_message_id']), '')
        proof['turns'].append({'input': message, 'turn': current, 'answer': answer})
        save()
        assert current['status'] == 'completed', current.get('error_code')
        assert current['context_pack_id'], 'must not pass using an empty-context fallback'
        # Canonical acceptance precedes asynchronous Outbox projection readiness.
        # Check recall after convergence; do not silently assert read-after-write consistency.
        for effect in current.get('memory_effects', []):
            operation_id = effect.get('operation_id')
            if not operation_id:
                continue
            started = time.monotonic()
            for _ in range(360):
                operation = alice('/v1/memory/operations/' + operation_id)
                if operation['projection_status'] in ('ready', 'deleted'):
                    break
                assert operation['projection_status'] not in ('failed', 'dead_letter', 'stale')
                time.sleep(.25)
            else:
                raise AssertionError('projection did not converge within 90 seconds')
            proof.setdefault('projection_waits', []).append({'operation_id': operation_id,
                'projection_status': operation['projection_status'],
                'wait_seconds': round(time.monotonic() - started, 3)})
            save()
        return current, answer

    if alice('/v1/memory/export')['records']:
        raise RuntimeError('regression requires a fresh tenant before any write or cleanup')
    proof['fresh_scope_export_empty'] = True
    save()

    # Repeat the whole source unchanged in separate identities, never split clauses.
    for _ in range(3):
        user = 'browser-regression-' + str(uuid.uuid4())
        response = memory(user, project, '/v1/memory/extractions', {'event_id': str(uuid.uuid4()),
            'source_type': 'conversation_message', 'source_ref': 'synthetic:browser-regression:' + user,
            'content': SOURCE, 'occurred_at': '2026-10-20T04:00:00Z', 'timezone': 'Asia/Shanghai', 'max_candidates': 8,
            'scope_selector': {'kind': 'user', 'id': user}})
        proof['extractions'].append(response)
        check('whole_source_types_' + str(len(proof['extractions'])),
            {'profile_fact', 'preference', 'episodic_summary'} <= {c['proposal']['memory_type'] for c in response['candidates']})
        check('whole_source_language_' + str(len(proof['extractions'])),
            any(c['proposal']['memory_type'] == 'profile_fact' and 'Rust' in c['proposal']['content'] for c in response['candidates']))
        owned = {c['operation']['memory_record_id'] for c in response['candidates']
            if c.get('operation') and c['operation'].get('memory_record_id')}
        live.Memory(args.tenant, user, project, args.memory_base).cleanup(owned)

    bootstrap = demo('/api/demo/bootstrap')
    revision = bootstrap['identity_revision']
    check('fresh_demo_user', bootstrap['memories']['record_count'] == 0)
    turn(SOURCE)
    records = alice('/v1/memory/export')['records']
    proof['initial_records'] = records
    check('whole_chat_four_types', {'profile_fact', 'preference', 'semantic_fact', 'episodic_summary'} <= {r['memory_type'] for r in records})
    episode = next(r for r in records if r['memory_type'] == 'episodic_summary')
    episode_id = episode['memory_record_id']
    database = next(r for r in records if r['memory_type'] == 'semantic_fact')
    current, answer = turn('我的职业、常用编程语言、当前项目权威数据库和回答格式偏好是什么？')
    check('cross_session_recall', all(word in answer for word in ['Rust', 'PostgreSQL', '结论']) and len(current['used_memory_ids']) >= 4)
    for name, question in [('afternoon', DATE_QUERY), ('day', '我在2026年7月15日处理过什么工程问题？'),
                           ('month', '我在2026年7月处理过什么工程问题？')]:
        current, answer = turn(question)
        check('date_recall_' + name, episode_id in current['used_memory_ids'] and 'Outbox' in answer and 'event_id' in answer)
        if name == 'afternoon':
            check('answer_preserves_afternoon', '下午' in answer and '上午' not in answer)
            context = next(m for m in current['prompt_context']['memory_context'] if m['memory_record_id'] == episode_id)
            check('local_time_evidence', context['event_time_local'] == '[2026-07-15 12:00:00, 2026-07-15 18:00:00) Asia/Shanghai')
    for name, question in [('other_day', '我在2026年7月16日处理过什么Outbox问题？'),
                           ('morning', '我在2026年7月15日上午处理过什么Outbox问题？')]:
        current, _ = turn(question)
        check('wrong_time_excludes_episode_' + name, episode_id not in current['used_memory_ids'])
    turn('项目的权威数据库现在改用 CockroachDB，不再使用 PostgreSQL。请按新情况更新建议。')
    updated = alice('/v1/memories/' + database['memory_record_id'])
    current, answer = turn('当前项目的权威数据库是什么？')
    check('database_update_keeps_history', len(updated['versions']) >= 2 and 'CockroachDB' in answer
        and updated['current_version_id'] in current['used_memory_version_ids'])
    demo('/api/memories/' + episode_id + '/forget', {})
    for name, question in [('date', DATE_QUERY), ('keyword', '我以前怎么解决Outbox重复消费的？')]:
        current, _ = turn(question)
        check('forgotten_episode_excluded_' + name, episode_id not in current['used_memory_ids'])
    proof['all_passed'] = all(proof['checks'].values())
    save()


if __name__ == '__main__':
    main()
