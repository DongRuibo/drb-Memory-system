import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('cleanup_live', ROOT / 'scripts/eval-resume-live.py')
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)


class CleanupBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.client = live.Memory.__new__(live.Memory)
        self.client.user, self.client.project = 'probe-user', 'shared-project'
        self.records = [
            {'memory_record_id': 'owned', 'head_state': 'active', 'revision': 0,
             'scope': {'kind': 'user', 'id': 'probe-user'}},
            {'memory_record_id': 'shared', 'head_state': 'active', 'revision': 4,
             'scope': {'kind': 'project', 'id': 'shared-project'}},
        ]
        self.writes = []
        self.client.call = lambda path: {'record_count': len(self.records), 'records': self.records}
        self.client.forget = lambda record, revision: self.writes.append((record, revision))

    def test_visible_shared_fact_is_not_owned_by_the_probe(self):
        self.assertEqual(self.client.cleanup({'owned'}), 1)
        self.assertEqual(self.writes, [('owned', 0)])
        self.assertEqual(self.records[1]['head_state'], 'active')

    def test_cleanup_requires_an_explicit_allowlist(self):
        with self.assertRaises(TypeError):
            self.client.cleanup()
        self.assertEqual(self.writes, [])

    def test_unknown_id_rejects_the_entire_cleanup_before_any_write(self):
        with self.assertRaises(ValueError):
            self.client.cleanup({'owned', 'unknown'})
        self.assertEqual(self.writes, [])

    def test_mismatched_scope_rejects_the_entire_cleanup_before_any_write(self):
        self.records[1]['scope']['id'] = 'other-project'
        with self.assertRaises(ValueError):
            self.client.cleanup({'owned', 'shared'})
        self.assertEqual(self.writes, [])


if __name__ == '__main__':
    unittest.main()
