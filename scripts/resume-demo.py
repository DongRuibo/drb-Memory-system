#!/usr/bin/env python3
"""Run the isolated recovery demo using local retrieval and DeepSeek credentials."""
import argparse
import os
from pathlib import Path
import shlex
import signal
import socket
import subprocess
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def read_env(path, keys=None):
    values = {}
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith('#'):
            continue
        key, value = line.removeprefix('export ').split('=', 1)
        if keys is not None and key not in keys:
            continue
        words = shlex.split(value, comments=True)
        if key in values or len(words) > 1:
            raise ValueError('duplicate or invalid environment entry')
        values[key] = words[0] if words else ''
    return values


def runtime_env(credentials_file, tenant):
    local = read_env(ROOT / '.env')
    credentials = read_env(credentials_file, {'DEEPSEEK_API_KEY', 'DEEPSEEK_MODEL', 'apikey', 'model_name'})
    key = credentials.get('DEEPSEEK_API_KEY') or credentials.get('apikey')
    model = credentials.get('DEEPSEEK_MODEL') or credentials.get('model_name')
    if not key or not model:
        raise ValueError('DeepSeek key and model are required in the credentials file')
    env = os.environ | local
    env.update({
        'DATABASE_URL': f"postgres://memory:{local['MEMORY_POSTGRES_PASSWORD']}@127.0.0.1:56432/memory",
        'DEMO_DATABASE_URL': f"postgres://memory:{local['MEMORY_POSTGRES_PASSWORD']}@127.0.0.1:56432/demo_chat",
        'MEMORY_BIND': '127.0.0.1:19080', 'MEMORY_BASE_URL': 'http://127.0.0.1:19080',
        'MEM0_BASE_URL': 'http://127.0.0.1:19889',
        'MEMORY_AUTH_MODE': 'jwt_hs256', 'MEMORY_TENANT_IDS': tenant, 'DEMO_TENANT_ID': tenant,
        'MEMORY_JWT_ISSUER': 'lazytt-demo-web-api', 'MEMORY_JWT_AUDIENCE': 'lazytt-memory-service',
        'MEMORY_EMBEDDING_BASE_URL': 'http://127.0.0.1:19081',
        'MEMORY_EMBEDDING_PROVIDER': 'local-qwen', 'MEMORY_EMBEDDING_SEMANTIC': 'true',
        'MEMORY_EMBEDDING_MODEL': 'Qwen/Qwen3-Embedding-0.6B',
        'MEMORY_EMBEDDING_REVISION': '97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3',
        'MEMORY_EMBEDDING_DIMENSIONS': '1024', 'MEMORY_EMBEDDING_NORMALIZATION': 'l2',
        'MEMORY_EMBEDDING_INSTRUCTION_VERSION': 'mem0-symmetric-document-v1',
        'MEMORY_RERANK_MODE': 'dedicated',
        'MEMORY_DEDICATED_RERANK_BASE_URL': 'http://127.0.0.1:19081',
        'MEMORY_DEDICATED_RERANK_MODEL': 'Qwen/Qwen3-Reranker-0.6B',
        'MEMORY_DEDICATED_RERANK_REVISION': 'e61197ed45024b0ed8a2d74b80b4d909f1255473',
        'MEMORY_DEDICATED_RERANK_TIMEOUT_SECONDS': '60', 'MEMORY_INDEX_TIMEOUT_SECONDS': '30',
        'MEMORY_ALLOW_INSECURE_MODEL_HTTP': 'true', 'MEMORY_ALLOW_INSECURE_INDEX_HTTP': 'true',
        'MEMORY_BOOTSTRAP_EMPTY_PROJECTION': 'true',
        'DEMO_CHAT_API_KEY': key, 'DEMO_CHAT_BASE_URL': 'https://api.deepseek.com',
        'DEMO_CHAT_MODEL': model, 'DEMO_AGENT_ENV_FILE': str(ROOT / '.env'),
        'MEMORY_SUPPORT_ENABLED': 'true', 'MEMORY_SUPPORT_API_KEY': key, 'MEMORY_SUPPORT_MODEL': model,
        'MEMORY_EXTRACT_MODE': 'openai_compatible', 'MEMORY_EXTRACT_API_KEY': key,
        'MEMORY_EXTRACT_BASE_URL': 'https://api.deepseek.com', 'MEMORY_EXTRACT_MODEL': model,
        'MEMORY_EXTRACT_TIMEOUT_SECONDS': '90', 'MEMORY_TIMEOUT_SECONDS': '30',
        'KIMI_MAX_COMPLETION_TOKENS': '2048', 'KIMI_TIMEOUT_SECONDS': '90',
        'DEMO_WEB_API_BIND': '127.0.0.1:19090',
        'DEMO_WEB_ALLOWED_ORIGINS': 'http://127.0.0.1:13000,http://localhost:13000',
        'HF_HUB_OFFLINE': '1', 'TRANSFORMERS_OFFLINE': '1', 'NO_PROXY': '127.0.0.1,localhost',
    })
    return env


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--credentials-file', type=Path, required=True)
    parser.add_argument('--tenant', default='resume-deepseek-demo')
    args = parser.parse_args()
    env = runtime_env(args.credentials_file, args.tenant)
    for port in (19080, 19090, 13000):
        with socket.socket() as probe:
            probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            probe.bind(('127.0.0.1', port))
    processes = []
    logs = ROOT / '.run/resume-live'
    logs.mkdir(parents=True, exist_ok=True)
    frontend_env = os.environ | {
        'NEXT_PUBLIC_DEMO_API_URL': 'http://127.0.0.1:19090',
        'WRANGLER_SEND_METRICS': 'false', 'DO_NOT_TRACK': '1',
    }
    commands = [
        ('memory', [str(ROOT / 'target/debug/memory-service')], ROOT, env, 'http://127.0.0.1:19080/health/ready'),
        ('api', [str(ROOT / 'target/debug/demo-web-api')], ROOT, env, 'http://127.0.0.1:19090/api/health'),
        ('web', ['npm', 'run', 'dev', '--', '--hostname', '127.0.0.1', '--port', '13000'], ROOT / 'apps/demo-web', frontend_env, 'http://127.0.0.1:13000'),
    ]
    client = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        for name, command, cwd, child_env, health in commands:
            path = logs / f'{name}.log'
            with open(path, 'w', opener=lambda p, f: os.open(p, f, 0o600)) as output:
                process = subprocess.Popen(command, cwd=cwd, env=child_env, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
            processes.append(process)
            deadline = time.monotonic() + 120
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError(f'{name} exited; inspect the private local log')
                try:
                    with client.open(health, timeout=3):
                        break
                except (urllib.error.URLError, TimeoutError):
                    time.sleep(0.25)
            else:
                raise RuntimeError(f'{name} readiness timeout')
            print(f'{name} ready', flush=True)
        print('Open http://127.0.0.1:13000 ; Ctrl-C stops these three processes, preserving Docker data.', flush=True)
        while all(process.poll() is None for process in processes):
            time.sleep(1)
    finally:
        for process in reversed(processes):
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
        for process in processes:
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)


if __name__ == '__main__':
    try:
        main()
    except KeyboardInterrupt:
        pass
    except Exception as error:
        # Provider errors or config values must not accidentally expose a credential.
        raise SystemExit(f'Recovery demo failed ({type(error).__name__}); inspect local prerequisites.') from None
