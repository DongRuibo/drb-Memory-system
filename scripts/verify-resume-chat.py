#!/usr/bin/env python3
"""Verify synthetic cross-session chat, correction, review, isolation and Forget."""
import argparse
import http.cookiejar
import importlib.util
import json
from pathlib import Path
import time
import urllib.request
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
    proof = {'synthetic_data_only': True, 'checks': {}, 'turns': []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    client = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
    revision = None
    memory = live.Memory(args.tenant, 'demo-alice', 'lazytt-memory-system-demo')

    def save():
        args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + '\n')

    def check(name, value):
        proof['checks'][name] = bool(value)
        save()
        print(name, 'PASS' if value else 'FAIL', flush=True)
        assert value, name

    def call(path, body=None):
        headers = {'Content-Type': 'application/json'}
        if revision:
            headers['x-demo-identity-revision'] = str(revision)
        request = urllib.request.Request('http://127.0.0.1:19090' + path,
            data=None if body is None else json.dumps(body, ensure_ascii=False).encode(), headers=headers)
        with client.open(request, timeout=120) as response:
            return json.load(response)

    def session(title):
        return call('/api/chat/sessions', {'title': '合成验收：' + title, 'mode': 'free'})['session']['id']

    def turn(sid, message, action=None):
        key = str(uuid.uuid4())
        body = {'session_id': sid, 'message': message, 'client_message_id': key, 'idempotency_key': key}
        if action:
            body['memory_action'] = action
        current = call('/api/chat/turns', body)['turn']
        started = time.monotonic()
        while time.monotonic() - started < 180:
            current = call('/api/chat/turns/' + current['id'])['turn']
            if current['status'] in ['completed', 'failed', 'cancelled']:
                break
            time.sleep(.5)
        view = call('/api/chat/sessions/' + sid)['session']
        answer = next((item['content'] for item in view['messages'] if item['id'] == current['assistant_message_id']), '')
        proof['turns'].append({'input': message, 'turn': current, 'answer': answer, 'latency_ms': round((time.monotonic() - started) * 1000)})
        save()
        assert current['status'] == 'completed', current.get('error_code')
        return current, answer

    for _ in range(60):
        try:
            bootstrap = call('/api/demo/bootstrap')
            break
        except urllib.error.URLError:
            time.sleep(.5)
    else:
        raise RuntimeError('Demo API did not become ready')
    revision = bootstrap['identity_revision']
    check('initial_user_empty', bootstrap['memories']['record_count'] == 0)
    turn(session('写入'), '我叫林舟。以后所有回答都请先给结论再讲细节。请简短确认。')
    exported = memory.call('/v1/memory/export')
    proof['initial_export'] = exported
    name = next(item for item in exported['records'] if item['memory_type'] == 'profile_fact')
    preference = next(item for item in exported['records'] if item['memory_type'] == 'preference')
    check('global_scope_not_rewritten', preference['versions'][-1]['structured_content']['payload']['applicability'] == 'global' and '技术' not in preference['versions'][-1]['content'])
    current, answer = turn(session('跨会话回忆'), '我叫什么？回答时我偏好的结构是什么？请简短回答。')
    check('cross_session_name_and_preference', '林舟' in answer and '结论' in answer and name['memory_record_id'] in current['used_memory_ids'])
    turn(session('更正'), '把我的姓名更正为陈星，请简短确认。', {'kind': 'correct', 'memory_record_id': name['memory_record_id'], 'new_content': '用户叫陈星。'})
    corrected = memory.call('/v1/memories/' + name['memory_record_id'])
    proof['corrected_record'] = corrected
    check('correction_appends_version', len(corrected['versions']) == 2 and corrected['current_version_id'] != name['current_version_id'])
    current, answer = turn(session('更正后回忆'), '我叫什么？请简短回答。')
    check('recall_uses_corrected_version', '陈星' in answer and '林舟' not in answer and corrected['current_version_id'] in current['used_memory_version_ids'])
    key = str(uuid.uuid4())
    conflict = memory.call('/v1/memory/events', {'memory_type': 'profile_fact', 'semantic_key': name['semantic_key'],
        'event_id': key, 'source_type': 'conversation_message', 'source_ref': 'synthetic-conflict:' + key,
        'content': '请记住：用户叫周越。', 'occurred_at': '2026-10-20T02:00:00Z',
        'scope_selector': {'kind': 'user', 'id': 'demo-alice'}}, key)
    proof['conflict'] = conflict
    contested = memory.call('/v1/memories/' + name['memory_record_id'])
    check('conflict_is_contested', contested['head_state'] == 'contested' and len(conflict['candidate_ids']) == 1)
    search = {'query': '我叫什么？', 'scope_selectors': [{'kind': 'user', 'id': 'demo-alice'}],
        'memory_types': ['profile_fact'], 'limit': 5, 'task_context': {'task_id': str(uuid.uuid4()), 'attempt_id': str(uuid.uuid4()), 'purpose': 'answer_user'}}
    check('contested_record_excluded', memory.call('/v1/memory/search', search)['hits'] == [])
    review = memory.call('/v1/memory-candidates/' + conflict['candidate_ids'][0] + '/decisions',
        {'expected_candidate_revision': 0, 'outcome': 'accept', 'reason': 'synthetic user confirmed new name'}, str(uuid.uuid4()))
    proof['review'] = review
    current, answer = turn(session('审核后回忆'), '我叫什么？请简短回答。')
    check('reviewed_value_is_recalled', '周越' in answer and '陈星' not in answer)
    switched = call('/api/demo/users/switch', {'user_id': 'demo-bob'})
    revision = switched['identity_revision']
    check('bob_cannot_read_alice_memories', switched['memories']['record_count'] == 0)
    switched = call('/api/demo/users/switch', {'user_id': 'demo-alice'})
    revision = switched['identity_revision']
    for item in memory.call('/v1/memory/export')['records']:
        call('/api/memories/' + item['memory_record_id'] + '/forget', {})
    current, answer = turn(session('遗忘后'), '我叫什么？请简短回答。')
    check('forget_removes_actual_prompt_memory', current['used_memory_ids'] == [] and current['prompt_context']['memory_context'] == [] and not any(value in answer for value in ['林舟', '陈星', '周越']))
    proof['all_passed'] = all(proof['checks'].values())
    save()


if __name__ == '__main__':
    main()
