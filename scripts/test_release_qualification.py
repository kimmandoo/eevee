"""Offline regression cases for the release qualification trust boundary."""
from copy import deepcopy
import unittest
from unittest.mock import patch

import release_qualification as gate


class QualificationTests(unittest.TestCase):
    def setUp(self):
        self.sha = 'a' * 40
        self.workflow = {'id': 7, 'path': gate.WORKFLOW, 'state': 'active'}
        self.run = {
            'id': 42, 'workflow_id': 7, 'path': gate.WORKFLOW, 'run_attempt': 1,
            'event': 'push', 'head_branch': 'main', 'head_sha': self.sha,
            'repository': {'id': 9}, 'head_repository': {'id': 9},
            'status': 'completed', 'conclusion': 'success',
        }
        self.runs = [{'id': 41}, {'id': 42}]
        self.jobs = []
        for kind in ('Build', 'Smoke'):
            for row in gate.ROWS:
                self.jobs.append({
                    'name': f'{kind} / {row}', 'run_id': 42, 'run_attempt': 1,
                    'head_sha': self.sha, 'status': 'completed', 'conclusion': 'success',
                    'started_at': '2026-10-13T01:00:00Z' if kind == 'Build' else '2026-10-13T02:00:00Z',
                    'completed_at': '2026-10-13T02:00:00Z' if kind == 'Build' else '2026-10-13T03:00:00Z',
                    'steps': [{'status': 'completed', 'conclusion': 'success'}],
                })
        self.jobs.append({**deepcopy(self.jobs[0]), 'name': gate.SDK_JOB})
        self.artifacts = [{
            'id': index + 100, 'name': 'ev-' + target, 'expired': False,
            'expires_at': '2099-10-27T00:00:00Z', 'created_at': '2026-10-13T01:30:00Z',
            'size_in_bytes': 1024, 'digest': 'sha256:' + 'b' * 64,
            'workflow_run': {'id': 42, 'head_sha': self.sha, 'head_branch': 'main',
                             'repository_id': 9, 'head_repository_id': 9},
        } for index, target in enumerate(gate.TARGETS)]
        self.final_run = None
        self.run_reads = 0

    def api(self, endpoint):
        if endpoint.endswith('/workflows/native.yml'):
            return deepcopy(self.workflow)
        if '/workflows/7/runs?' in endpoint:
            self.assertIn('head_sha=' + self.sha, endpoint)
            self.assertIn('event=push', endpoint)
            self.assertIn('branch=main', endpoint)
            key, values = 'workflow_runs', self.runs
        elif endpoint.endswith('/runs/42'):
            self.run_reads += 1
            return deepcopy(self.final_run if self.run_reads > 1 and self.final_run else self.run)
        elif '/runs/42/attempts/1/jobs?' in endpoint:
            key, values = 'jobs', self.jobs
        elif '/runs/42/artifacts?' in endpoint:
            key, values = 'artifacts', self.artifacts
        else:
            self.fail(f'Unexpected API endpoint: {endpoint}')
        return {key: deepcopy(values), 'total_count': len(values)}

    def qualify(self):
        with patch.object(gate, 'api', side_effect=self.api):
            return gate.qualify('owner/repo', 9, self.sha)

    def test_exact_qualified_packages(self):
        self.assertEqual(self.qualify(), {
            'run-id': '42', 'run-attempt': '1', 'artifact-ids': '100,101,102,103,104',
        })

    def test_no_same_commit_run(self):
        self.runs = []
        with self.assertRaisesRegex(ValueError, 'No native main push'):
            self.qualify()

    def test_newest_failure_does_not_fall_back(self):
        self.run['conclusion'] = 'failure'
        with self.assertRaisesRegex(ValueError, 'Newest same-commit'):
            self.qualify()

    def test_pending_cancelled_or_skipped_run(self):
        for status, conclusion in (('in_progress', None), ('completed', 'cancelled'),
                                   ('completed', 'skipped')):
            with self.subTest(status=status, conclusion=conclusion):
                self.run.update(status=status, conclusion=conclusion)
                with self.assertRaises(ValueError):
                    self.qualify()

    def test_untrusted_run_metadata(self):
        original = deepcopy(self.run)
        for key, value in (('event', 'pull_request'), ('event', 'workflow_dispatch'),
                           ('head_branch', 'topic'), ('head_sha', 'c' * 40),
                           ('repository', {'id': 8}), ('head_repository', {'id': 8}),
                           ('workflow_id', 8), ('path', '.github/workflows/other.yml')):
            with self.subTest(key=key, value=value):
                self.run = {**deepcopy(original), key: value}
                with self.assertRaises(ValueError):
                    self.qualify()

    def test_every_native_row_and_sdk_are_required(self):
        original = deepcopy(self.jobs)
        for missing in range(len(original)):
            with self.subTest(missing=original[missing]['name']):
                self.jobs = original[:missing] + original[missing + 1:]
                with self.assertRaisesRegex(ValueError, 'all five build and smoke'):
                    self.qualify()

    def test_failed_skipped_old_attempt_or_wrong_commit_job(self):
        original = deepcopy(self.jobs[0])
        for key, value in (('conclusion', 'failure'), ('conclusion', 'skipped'),
                           ('status', 'queued'), ('run_attempt', 0),
                           ('head_sha', 'c' * 40), ('run_id', 41)):
            with self.subTest(key=key, value=value):
                self.jobs[0] = {**deepcopy(original), key: value}
                with self.assertRaisesRegex(ValueError, 'Unqualified native job'):
                    self.qualify()

    def test_skipped_acceptance_step(self):
        self.jobs[5]['steps'][0]['conclusion'] = 'skipped'
        with self.assertRaisesRegex(ValueError, 'Incomplete or skipped steps'):
            self.qualify()

    def test_missing_duplicate_or_unexpected_package(self):
        original = deepcopy(self.artifacts)
        for packages in (original[:-1], original + [original[0]],
                         original + [{**original[0], 'name': 'ev-unexpected'}]):
            with self.subTest(names=[item['name'] for item in packages]):
                self.artifacts = packages
                with self.assertRaisesRegex(ValueError, 'exactly one immutable package'):
                    self.qualify()

    def test_untrusted_artifact_provenance(self):
        original = deepcopy(self.artifacts[0]['workflow_run'])
        for key, value in (('id', 41), ('head_sha', 'c' * 40), ('head_branch', 'topic'),
                           ('repository_id', 8), ('head_repository_id', 8)):
            with self.subTest(key=key, value=value):
                self.artifacts[0]['workflow_run'] = {**original, key: value}
                with self.assertRaisesRegex(ValueError, 'Unexpected artifact provenance'):
                    self.qualify()

    def test_unusable_or_old_attempt_artifact(self):
        original = deepcopy(self.artifacts[0])
        for key, value in (('expired', True), ('expires_at', '2000-01-01T00:00:00Z'),
                           ('size_in_bytes', 0), ('size_in_bytes', 129 * 1024 * 1024),
                           ('digest', None), ('digest', 'sha256:invalid'),
                           ('created_at', '2026-10-13T00:00:00Z'),
                           ('created_at', '2026-10-13T04:00:00Z')):
            with self.subTest(key=key, value=value):
                self.artifacts[0] = {**deepcopy(original), key: value}
                with self.assertRaises(ValueError):
                    self.qualify()

    def test_rerun_during_qualification(self):
        self.final_run = {**self.run, 'run_attempt': 2}
        with self.assertRaisesRegex(ValueError, 'changed during qualification'):
            self.qualify()

    def test_paginated_listing(self):
        with patch.object(gate, 'api', side_effect=[
            {'jobs': list(range(100)), 'total_count': 101},
            {'jobs': [100], 'total_count': 101},
        ]) as api:
            self.assertEqual(gate.pages('jobs?filter=latest', 'jobs'), list(range(101)))
            self.assertTrue(api.call_args_list[1].args[0].endswith('&per_page=100&page=2'))

    def test_incomplete_listing_fails_closed(self):
        with patch.object(gate, 'api', return_value={'jobs': [], 'total_count': 1}):
            with self.assertRaisesRegex(ValueError, 'Incomplete or changing'):
                gate.pages('jobs', 'jobs')


if __name__ == '__main__':
    unittest.main()
