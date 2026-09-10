import json
from pathlib import Path
import tempfile
import unittest
from run import browser_counts, classify, inventory, report, ROOT


def pw(*statuses):
    return {'suites': [{'specs': [{'tests': [{'status': status} for status in statuses]}]}]}


class VerdictTests(unittest.TestCase):
    def test_empty_browser_run_cannot_pass(self):
        self.assertEqual(classify(0, '', pw()), 'INCOMPLETE')

    def test_skipped_or_retried_coverage_is_not_green(self):
        self.assertEqual(classify(0, '', pw('expected', 'skipped')), 'INCOMPLETE')
        self.assertEqual(classify(0, '', pw('expected', 'flaky')), 'FAIL')

    def test_real_provider_runtime_skip_overrides_success_exit(self):
        self.assertEqual(classify(0, 'SKIPPED: herdr unavailable', live=True), 'INCOMPLETE')

    def test_nonzero_exit_and_nested_failures_win(self):
        self.assertEqual(classify(1, '', pw('expected')), 'FAIL')
        self.assertEqual(classify(0, '', pw('unexpected')), 'FAIL')
        self.assertEqual(browser_counts(pw('expected', 'unexpected'))['failed'], 1)

    def test_successful_nonempty_scope_passes(self):
        self.assertEqual(classify(0, '', pw('expected')), 'PASS')

    def test_new_specs_automatically_enter_inventory(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'e2e/nested').mkdir(parents=True)
            (root / 'e2e/nested/new.spec.ts').write_text('test("new", () => {})')
            found = inventory(root)
            self.assertEqual([row['path'] for row in found], ['e2e/nested/new.spec.ts'])
            self.assertEqual(len(found[0]['sha256']), 64)

    def test_every_case_has_real_sources_and_an_outcome(self):
        cases = json.loads((ROOT / 'e2e/lifecycle/cases.json').read_text())
        self.assertEqual(len(cases), len({c['id'] for c in cases}))
        for case in cases:
            self.assertTrue(case['actions'])
            self.assertTrue(case['expected'])
            for source in case['sources']:
                self.assertTrue((ROOT / source).is_file(), (case['id'], source))

    def test_report_preserves_failed_and_incomplete_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            state = {'status': 'FAIL', 'mode': 'full', 'commit': '<unsafe>', 'sources': [],
                     'stages': [{'name': 'browser', 'status': 'FAIL'},
                                {'name': 'live', 'status': 'INCOMPLETE', 'note': '<no provider>'}]}
            report(out, state)
            self.assertEqual(json.loads((out / 'summary.json').read_text()), state)
            self.assertIn('&lt;no provider&gt;', (out / 'index.html').read_text())


if __name__ == '__main__': unittest.main()
