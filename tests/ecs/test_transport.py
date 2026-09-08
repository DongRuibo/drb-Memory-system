"""Exercise remote orchestration with fake commands, never SSH or Docker."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class TransportTests(unittest.TestCase):
    def remote(self, action, failed_probes=0, wait='30'):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stage = root / 'stage'
            stage.mkdir()
            shutil.copyfile(ROOT / 'deploy/ecs/remote.sh', stage / 'remote.sh')
            commands = root / 'bin'
            commands.mkdir()
            log = root / 'calls.jsonl'
            helper = '''
import json, os, sys
from pathlib import Path
name = Path(sys.argv[0]).name
with Path(os.environ['TEST_LOG']).open('a') as output:
    output.write(json.dumps([name, *sys.argv[1:]]) + '\\n')
if name == 'docker':
    sys.stdin.read()
if name == 'python3' and sys.argv[2] == 'database-check':
    counter = Path(os.environ['TEST_COUNT'])
    count = int(counter.read_text()) if counter.exists() else 0
    counter.write_text(str(count+1))
    sys.exit(1 if count < int(os.environ['TEST_FAILURES']) else 0)
'''
            for name in ('python3', 'docker', 'sleep'):
                path = commands / name
                path.write_text('#!' + sys.executable + '\n' + helper)
                path.chmod(0o700)
            env = {**os.environ, 'PATH': str(commands) + ':' + os.environ['PATH'],
                   'TEST_LOG': str(log), 'TEST_COUNT': str(root / 'count'), 'TEST_FAILURES': str(failed_probes)}
            result = subprocess.run(['bash', str(stage / 'remote.sh'), action, str(root / 'deployment'),
                'a' * 40, 'memory-digest', 'mem0-digest', 'fixture', wait],
                input='fixture-registry-token', text=True, capture_output=True, env=env)
            self.assertFalse(stage.exists())
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            for call in calls:
                if call[0] == 'python3':
                    self.assertEqual(call[call.index('--revision') + 1], 'a' * 40)
                    self.assertEqual(call[call.index('--memory-image') + 1], 'memory-digest')
                    self.assertEqual(call[call.index('--mem0-image') + 1], 'mem0-digest')
            self.assertNotIn('fixture-registry-token', result.stdout + result.stderr + log.read_text())
            return result, [call[2] for call in calls if call[0] == 'python3']

    def test_wait_checks_schema_without_migrating_or_releasing(self):
        result, actions = self.remote('prepare-wait', failed_probes=2)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(actions, ['prepare', 'database-check', 'database-check', 'database-check'])

    def test_timeout_preserves_candidate_and_never_starts_apps(self):
        result, actions = self.remote('prepare-wait', failed_probes=2, wait='0')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(actions, ['prepare', 'database-check'])
        self.assertIn('Candidate and admin bundle remain', result.stderr)

    def test_release_does_not_rebuild_prepare_or_migrate(self):
        result, actions = self.remote('release')
        self.assertEqual(result.returncode, 0)
        self.assertEqual(actions, ['release'])


if __name__ == '__main__':
    unittest.main()
