"""Offline behavior tests: real subprocesses, bounded scheduling, retained failures."""
import contextlib
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import native_workflows_smoke as runner


# These children exercise the scheduler, not the terminal acceptance suites. Each
# owns a real isolated workspace and emits output larger than a typical pipe buffer.
CHILD = r'''
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time

parser = argparse.ArgumentParser()
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--behavior', required=True)
parser.add_argument('--barrier', type=Path)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=False)
project = args.output / 'project'
project.mkdir()
(project / 'owned.txt').write_text(str(args.output))
started = time.monotonic_ns()
if args.barrier:
    (args.barrier / (args.output.name + '.started')).touch()
    deadline = time.monotonic() + 5
    while len(list(args.barrier.glob('*.started'))) < 2:
        if time.monotonic() > deadline:
            raise RuntimeError('bounded scheduler never started a second child')
        time.sleep(.01)
if args.behavior == 'timeout':
    child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'],
                             start_new_session=(sys.platform != 'win32'))
    (args.output / 'descendant.json').write_text(json.dumps({'pid': child.pid}))
    try:
        time.sleep(60)
    finally:
        child.kill()
        child.wait(timeout=5)
        (args.output / 'cleaned.json').write_text(json.dumps({'returncode': child.returncode}))
else:
    time.sleep(.15)
    if args.behavior == 'mutate':
        args.binary.write_bytes(b'changed executable')
    (args.output / 'evidence.json').write_text(json.dumps({
        'started': started, 'finished': time.monotonic_ns(),
        'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
        'workspace': str(project), 'behavior': args.behavior,
    }))
    sys.stdout.write('out-' * 32768)
    sys.stderr.write('err-' * 32768)
    raise SystemExit(7 if args.behavior == 'fail' else 0)
'''


class NativeRunnerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='ev-runner-tests-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.binary = self.root / 'already built binary'
        self.binary.write_bytes(b'immutable public fixture')
        self.child = self.root / 'behavior child.py'
        self.child.write_text(CHILD, encoding='utf-8')
        self.output = self.root / 'evidence'

    def plan(self, behaviors, *, jobs=2, barrier=False):
        if barrier:
            gate = self.root / 'barrier'
            gate.mkdir()
        commands = []
        for index, behavior in enumerate(behaviors):
            destination = self.output / ('child-' + str(index))
            argv = [sys.executable, str(self.child), '--binary', str(self.binary),
                    '--output', str(destination), '--behavior', behavior]
            if barrier:
                argv.extend(('--barrier', str(gate)))
            commands.append({'name': destination.name, 'output': str(destination),
                             'argv': argv, 'timeout_seconds': 10})
        return {'binary': str(self.binary), 'output': str(self.output),
                'profile': 'behavior-test', 'jobs': jobs,
                'counts': {'commands': len(commands)}, 'commands': commands}

    def test_profiles_select_distinct_scenarios_at_both_terminal_sizes(self):
        expected_tools = {suite for suite, _script, _extra in runner.TOOLS}
        for profile in runner.PROFILES:
            with self.subTest(profile=profile):
                plan = runner.make_plan(self.binary, self.output, profile)
                commands = plan['commands']
                suites = [command['coverage']['suite'] for command in commands]
                self.assertEqual(len(suites), len(set(suites)), 'each suite must run once')
                self.assertEqual(set(suites), {'files'} | (
                    set() if profile == 'architecture' else expected_tools))
                destinations = [command['output'] for command in commands]
                self.assertEqual(len(destinations), len(set(destinations)))
                for command in commands:
                    self.assertEqual(
                        {(size['width'], size['height'], size['ascii'])
                         for size in command['coverage']['sizes']},
                        {(120, 36, False), (60, 18, True)})
                    self.assertEqual(command['timeout_seconds'], runner.DRIVER_TIMEOUT)
                    self.assertEqual(command['argv'][command['argv'].index('--binary') + 1],
                                     str(self.binary))
                    self.assertEqual(command['argv'][command['argv'].index('--output') + 1],
                                     command['output'])
                self.assertFalse(self.output.exists(), 'planning must not create evidence')

    def test_full_profile_preserves_protocols_and_controlled_route_boundaries(self):
        for profile in runner.PROFILES:
            with self.subTest(profile=profile):
                plan = runner.make_plan(self.binary, self.output, profile)
                files = next(command for command in plan['commands']
                             if command['coverage']['suite'] == 'files')
                coverage = files['coverage']
                routes = {(route['provider'], route['protocol'])
                          for route in coverage['routes']}
                expected = {
                    ('openai', 'responses'), ('anthropic', 'messages'),
                    ('google', 'gemini'), ('compatible', 'chat_completions'),
                    ('compatible', 'responses'),
                } if profile == 'full' else {('compatible', 'chat_completions')}
                self.assertEqual(routes, expected)
                self.assertEqual(len(routes), len(coverage['routes']))
                self.assertEqual(coverage['controlled_codex'],
                                 profile == 'full' and os.name == 'posix')
                sizes = len(coverage['sizes'])
                self.assertEqual(plan['counts']['native_file_journeys'], sizes * len(routes))
                self.assertEqual(plan['counts']['controlled_codex_journeys'],
                                 sizes if coverage['controlled_codex'] else 0)
                self.assertEqual(plan['counts']['tool_journeys'],
                                 sum(len(command['coverage']['sizes'])
                                     for command in plan['commands']
                                     if command['coverage']['suite'] != 'files'))

    def test_failed_child_is_nonzero_without_suppressing_other_selected_children(self):
        plan = self.plan(('fail', 'pass', 'pass', 'pass'), barrier=True)
        stdout = io.StringIO()
        # Only selection is injected; main, its subprocess scheduler and every
        # child execute normally, including exit codes and file-backed streams.
        with patch.object(runner, 'make_plan', return_value=plan), contextlib.redirect_stdout(stdout):
            status = runner.main(['--binary', str(self.binary), '--output', str(self.output)])
        self.assertEqual(status, 1)
        report = json.loads(stdout.getvalue())
        self.assertEqual(report, json.loads((self.output / 'results.json').read_text()))
        self.assertEqual(report['status'], 'failed')
        self.assertEqual([row['returncode'] for row in report['results']], [7, 0, 0, 0])
        self.assertEqual([row['status'] for row in report['results']], ['failed', 'passed', 'passed', 'passed'])
        self.assertEqual(report['binary_sha256'], report['binary_sha256_after'])
        intervals = []
        workspaces = set()
        for command, result in zip(plan['commands'], report['results']):
            destination = Path(command['output'])
            evidence = json.loads((destination / 'evidence.json').read_text())
            workspaces.add(evidence['workspace'])
            self.assertEqual((Path(evidence['workspace']) / 'owned.txt').read_text(), str(destination))
            self.assertEqual(evidence['binary_sha256'], report['binary_sha256'])
            self.assertGreater(result['duration_seconds'], 0)
            self.assertEqual(Path(result['stdout']).read_bytes(), b'out-' * 32768)
            self.assertEqual(Path(result['stderr']).read_bytes(), b'err-' * 32768)
            intervals.extend(((evidence['started'], 1), (evidence['finished'], -1)))
        active = peak = 0
        for _, change in sorted(intervals):
            active += change
            peak = max(active, peak)
        self.assertEqual(active, 0)
        self.assertEqual(peak, 2, 'must execute concurrently without exceeding the configured bound')
        self.assertEqual(len(workspaces), len(plan['commands']))
        self.assertEqual(json.loads((self.output / 'coverage-manifest.json').read_text()), plan)

    def test_launch_failure_still_waits_for_remaining_child(self):
        plan = self.plan(('pass', 'pass'), jobs=1)
        plan['commands'][0]['argv'][0] = str(self.root / 'nonexistent interpreter')
        report = runner.run_plan(plan)
        self.assertEqual(report['status'], 'failed')
        self.assertEqual(len(report['results']), 2)
        self.assertEqual(report['results'][0]['status'], 'failed')
        self.assertEqual(report['results'][1]['returncode'], 0)
        self.assertTrue((self.output / 'child-1' / 'evidence.json').is_file())

    def test_binary_mutation_rejects_otherwise_successful_run(self):
        report = runner.run_plan(self.plan(('mutate',), jobs=1))
        self.assertEqual(report['results'][0]['returncode'], 0)
        self.assertEqual(report['status'], 'failed')
        self.assertIn('supplied binary changed', report['error'])
        self.assertNotEqual(report['binary_sha256'], report['binary_sha256_after'])

    def test_timeout_is_failure_and_later_selected_child_still_executes(self):
        plan = self.plan(('timeout', 'pass'), jobs=1)
        plan['commands'][0]['timeout_seconds'] = 2
        report = runner.run_plan(plan)
        self.assertEqual(report['status'], 'failed')
        self.assertTrue(report['results'][0]['timed_out'])
        self.assertEqual(report['results'][1]['returncode'], 0)
        self.assertTrue((self.output / 'child-1' / 'evidence.json').is_file())
        if os.name == 'posix':
            descendant = json.loads((self.output / 'child-0' / 'descendant.json').read_text())
            cleaned = json.loads((self.output / 'child-0' / 'cleaned.json').read_text())
            self.assertNotEqual(cleaned['returncode'], 0)
            with self.assertRaises(ProcessLookupError):
                os.kill(descendant['pid'], 0)


if __name__ == '__main__':
    unittest.main()
