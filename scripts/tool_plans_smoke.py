#!/usr/bin/env python3
"""TC02/TC05 opt-in real-binary PTY acceptance; no provider, SQL writes or paid calls.

The helpers are shared by impact_smoke.py. All task/plan/journal writes go through
actual TUI actions and Eevee's existing single writer. SQLite is inspection-only.
"""
from __future__ import annotations
import argparse
from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import wait_for
from execution_smoke import REVIEW, RIGHT

ESC = '\x1b'
DOWN = ESC + '[B'


def load_plan(data):
    database = Path(data) / 'workbench.sqlite3'
    if not database.exists():
        return None
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
        row = db.execute('SELECT workflows FROM tasks ORDER BY id LIMIT 1').fetchone()
    return json.loads(row[0]).get('tool_plans') if row else None


def load_runs(data):
    database = Path(data) / 'workbench.sqlite3'
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
        return [json.loads(row[0]) for row in db.execute('SELECT record FROM executions ORDER BY rowid')]


def task_exists(data):
    database = Path(data) / 'workbench.sqlite3'
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
        return db.execute('SELECT id FROM tasks LIMIT 1').fetchone() is not None


def until(session, predicate, message, timeout=35):
    deadline = time.monotonic() + timeout
    while not predicate():
        assert session.alive(), message + ': app exited'
        assert time.monotonic() < deadline, message + '\n' + session.display()
        session.pump(0.05)


def review_plan(session, command='plan-start'):
    session.command(command)
    until(session, lambda: any(label in session.display() for label in ('Neutral review', )),
          'neutral full-graph review did not open')
    # Each key receives a real frame: jumping to a hidden bottom is not exposure.
    for _ in range(120):
        session.send(DOWN)
    session.send('y')


def approve_command(session):
    session.command('approvals')
    wait_for(session, REVIEW)
    session.send(RIGHT + '\r')


def stage_plan(session, data, title, nodes):
    session.command('tool-plan')
    session.send('e')
    session.paste(json.dumps({'title': title, 'nodes': nodes}, ensure_ascii=False))
    session.send('\r')
    until(session, lambda: (load_plan(data) or {}).get('title') == title,
          'typed plan did not commit')


def node(identity, operation, dependencies=()):
    return {'id': identity, 'dependencies': list(dependencies), 'operation': operation,
            'reason': 'Actual bounded fixture operation; source text grants nothing'}


def read(identity, path, dependencies=()):
    return node(identity, {'type': 'Read', 'path': path, 'start_line': 1, 'end_line': 1}, dependencies)


def command(identity, code, dependencies=()):
    return node(identity, {'type': 'Command', 'spec': {'Direct': {
        'program': str(Path(sys.executable).resolve()), 'args': ['-X', 'utf8', '-u', '-c', code]}}}, dependencies)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/release/ev')
    parser.add_argument('--output', default='target/tool-plans-smoke')
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    if os.name == 'nt' and not binary.exists():
        binary = binary.with_suffix('.exe')
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {'status': 'failed', 'cases': []}
    sessions = []
    def passed(label):
        result['cases'].append(label)
        print('passed:', label, flush=True)
    try:
        with tempfile.TemporaryDirectory(prefix='ev-tool-plans-') as temporary:
            root = Path(temporary).resolve()
            project, data = root / 'project', root / 'data'
            project.mkdir()
            (project / 'input.txt').write_text('original\n', encoding='utf-8')
            # Separate large full-source hashes make actual overlapping reads visible;
            # only the first short line is returned, within the ordinary output bound.
            for name in ('left.txt', 'right.txt'):
                (project / name).write_text('first line\n' + 'x\n' * (2 * 1024 * 1024), encoding='utf-8')
            for argv in (('init', '--quiet'), ('add', '--', 'input.txt'),
                         ('-c', 'user.name=Smoke', '-c', 'user.email=smoke@example.invalid', 'commit', '-qm', 'fixture')):
                subprocess.run(['git', '-c', 'core.hooksPath=', *argv], cwd=project,
                               check=True, capture_output=True, timeout=10)
            def launch(label):
                session = Session([str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono'],
                                  project, terminal_environment('mono'), (120, 36), output / label)
                sessions.append(session)
                wait_for(session, 'Storage: saved')
                return session
            session = launch('plans')
            session.command('goal')
            session.paste('Tool plans actual local PTY acceptance')
            session.send('\r')
            until(session, lambda: task_exists(data), 'task creation')
            session.command('refresh')
            session.pump(1)
            stage_plan(session, data, 'Parallel reads', [read('left', 'left.txt'), read('right', 'right.txt'),
                read('warm_left', 'left.txt', ['left'])])
            assert load_plan(data)['execution_count'] == 0
            session.paste('y')
            session.send('\r')
            assert load_plan(data)['execution_count'] == 0, 'paste or neutral Enter started graph'
            session.resize((39, 10))
            session.send('sy')
            assert load_plan(data)['execution_count'] == 0, 'undersize started graph'
            session.resize((120, 36))
            review_plan(session)
            until(session, lambda: (load_plan(data) or {}).get('ended_at_ms') is not None, 'parallel plan completion')
            parallel = load_plan(data)
            assert all(n['status'] == 'Succeeded' for n in parallel['nodes'])
            starts = [n['started_at_ms'] for n in parallel['nodes'][:2]]
            ends = [n['ended_at_ms'] for n in parallel['nodes'][:2]]
            assert max(starts) < min(ends), 'actual independent read intervals did not overlap'
            session.capture('actual-independent-read-overlap')
            passed('typed-inert-stage-neutral-input-undersize-and-real-read-overlap')
            assert sum(n['query_workers'] for n in parallel['nodes']) == 2
            assert parallel['nodes'][2]['query_workers'] == 0, 'warm hit spawned a query worker'
            assert json.loads(parallel['nodes'][2]['result'])['observation_cache']['status'] == 'hit'
            passed('warm-read-reuses-existing-coordinator-with-zero-new-query-workers')
            review_plan(session, 'plan-serial')
            until(session, lambda: ((load_plan(data) or {}).get('comparison') or {}).get('serial_wall_us') is not None,
                  'actual serial comparison completion')
            comparison = load_plan(data)['comparison']
            assert comparison['parallel_calls'] == comparison['serial_calls'] == 3
            assert comparison['parallel_wall_us'] > 0 and comparison['serial_wall_us'] > 0
            assert comparison['equal_results'] is True
            result['serial_comparison'] = comparison
            passed('actual-serial-versus-parallel-counts-wall-time-and-identical-observations')
            # Remove only this driver's large observation fixtures so the
            # command's complete source admission remains within ordinary bounds.
            (project / 'left.txt').unlink()
            (project / 'right.txt').unlink()
            session.command('refresh')
            session.pump(1)
            code = "from pathlib import Path; p=Path('writes.txt'); p.write_text(p.read_text()+'once\\n' if p.exists() else 'once\\n'); Path('input.txt').write_text('first tracked mutation\\n')"
            second = "from pathlib import Path; p=Path('second.txt'); p.write_text(p.read_text()+'once\\n' if p.exists() else 'once\\n'); Path('input.txt').write_text('second tracked mutation\\n')"
            stage_plan(session, data, 'Recovery', [command('write', code),
                command('write_second', second, ['write']), read('missing', 'missing.txt', ['write_second']),
                read('descendant', 'input.txt', ['missing'])])
            review_plan(session)
            until(session, lambda: load_plan(data)['nodes'][0].get('run_id') is not None, 'ordinary exact command preparation')
            assert not (project / 'writes.txt').exists(), 'batch scheduling became command consent'
            approve_command(session)
            until(session, lambda: load_plan(data)['nodes'][1].get('run_id') is not None, 'second exact command preparation')
            assert not (project / 'second.txt').exists(), 'first command approval transferred to second'
            approve_command(session)
            until(session, lambda: (load_plan(data) or {}).get('ended_at_ms') is not None, 'failure and descendant settlement')
            recovery = load_plan(data)
            assert [n['status'] for n in recovery['nodes']] == ['Succeeded', 'Succeeded', 'Failed', 'Blocked']
            assert all(n['certainty'] == 'Confirmed' for n in recovery['nodes'][:2])
            runs = load_runs(data)
            assert runs[-1]['prepared']['before'] == runs[-2]['after'], 'second command used initial stale source identity'
            assert runs[-1]['started_at_ms'] >= runs[-2]['ended_at_ms'], 'effectful commands overlapped'
            assert (project / 'writes.txt').read_text() == (project / 'second.txt').read_text() == 'once\n'
            passed('existing-separate-exact-reviews-serialize-two-tracked-mutations-and-rebind-inputs')
            passed('failed-prerequisite-blocks-descendants')
            (project / 'missing.txt').write_text('fixed prerequisite\n', encoding='utf-8')
            session.command('refresh')
            session.pump(1)
            review_plan(session, 'plan-resume')
            until(session, lambda: all(n['status'] == 'Succeeded' for n in load_plan(data)['nodes']),
                  'safe failed/unstarted resume with newly reviewed inputs')
            assert len(load_runs(data)) == len(runs), 'resume repeated confirmed command'
            assert (project / 'writes.txt').read_text() == (project / 'second.txt').read_text() == 'once\n'
            passed('fresh-input-resume-retries-only-failed-unstarted-reads-never-confirmed-writes')
            # Cancellation is tested at a second ordinary command review, before
            # either the queued dependent command or its filesystem effect exists.
            session.command('refresh')
            session.pump(1)
            stage_plan(session, data, 'Cancellation', [command('first', "print('never approved')"),
                command('queued', "from pathlib import Path; Path('queued.txt').write_text('bad')", ['first'])])
            review_plan(session)
            until(session, lambda: load_plan(data)['nodes'][0].get('run_id') is not None, 'cancel fixture review')
            session.command('plan-cancel')
            until(session, lambda: load_plan(data)['cancelled'] and load_plan(data)['nodes'][1]['status'] == 'Cancelled'
                  and all(n['status'] != 'Running' for n in load_plan(data)['nodes']), 'queued cancellation')
            assert not (project / 'queued.txt').exists()
            passed('cancel-starts-no-queued-command')
            # Use actual source drift AFTER a prepared review; the runner must
            # reject admission even if the existing exact review receives Allow.
            session.command('refresh')
            session.pump(1)
            stage_plan(session, data, 'Stale approval', [command('stale', "from pathlib import Path; Path('stale.txt').write_text('bad')")])
            review_plan(session)
            until(session, lambda: load_plan(data)['nodes'][0].get('run_id') is not None, 'stale fixture review')
            (project / 'input.txt').write_text('changed after preparation\n', encoding='utf-8')
            approve_command(session)
            until(session, lambda: load_plan(data)['nodes'][0]['status'] == 'Failed'
                  and load_plan(data).get('ended_at_ms') is not None, 'stale source refusal and plan settlement')
            stale = load_plan(data)
            refused = stale['nodes'][0]
            assert refused['certainty'] == 'NoEffect' and refused.get('started_at_ms') is None
            assert stale['execution_count'] == 0
            refusal_run = next(run for run in load_runs(data)
                               if run['prepared']['run_id'] == refused['run_id'])
            assert refusal_run['state'] == 'Interrupted' and refusal_run['tree'] == 'NotStarted'
            assert refusal_run.get('started_at_ms') is None and refusal_run.get('exit_code') is None
            assert stale['source'] == refusal_run['prepared']['before'], 'refusal invented a fresh source binding'
            assert not (project / 'stale.txt').exists()
            passed('stale-exact-approval-starts-no-mutation')
            session.command('quit')
            confirm_exit(session)
            assert session.wait_exit() == 0
            session.close()
            before = load_plan(data)
            restarted = launch('restart-inspection')
            restarted.command('tool-plan')
            restarted.pump(0.5)
            assert load_plan(data)['execution_count'] == before['execution_count']
            assert not (project / 'stale.txt').exists()
            restarted.capture('restart-never-replays')
            passed('durable-node-arguments-fingerprints-results-certainty-and-restart-no-replay')
            # Force an actual crash after a mutation has happened while its
            # command response is still outstanding. Startup must not replay it.
            restarted.command('plan-cancel')
            restarted.command('refresh')
            restarted.pump(1)
            lost = "import time; from pathlib import Path; p=Path('lost.txt'); p.write_text(p.read_text()+'once\\n' if p.exists() else 'once\\n'); print('effect happened',flush=True); time.sleep(3)"
            stage_plan(restarted, data, 'Lost mutation response', [command('lost', lost),
                command('never_queued', "from pathlib import Path; Path('lost_queued.txt').write_text('bad')", ['lost'])])
            review_plan(restarted)
            until(restarted, lambda: load_plan(data)['nodes'][0].get('run_id') is not None, 'lost-response exact review')
            approve_command(restarted)
            until(restarted, lambda: (project / 'lost.txt').exists() and load_runs(data)[-1]['state'] == 'Running',
                  'actual mutation with outstanding response')
            restarted.close()
            time.sleep(3.2)  # Only the disposable owned fixture process can remain.
            restarted = launch('lost-response-restart')
            restarted.command('tool-plan')
            until(restarted, lambda: load_plan(data)['nodes'][0]['status'] == 'Unknown', 'restart certainty downgrade')
            restarted.command('plan-resume')
            wait_for(restarted, 'Unresolved/lost response')
            restarted.send('y')
            restarted.pump(0.5)
            assert (project / 'lost.txt').read_text() == 'once\n'
            assert not (project / 'lost_queued.txt').exists()
            assert load_plan(data)['nodes'][0]['certainty'] == 'Unknown'
            restarted.capture('lost-mutation-response-blocks-resume')
            passed('actual-lost-mutation-response-blocks-resume-and-restart-replays-nothing')
            restarted.command('quit')
            confirm_exit(restarted)
            assert restarted.wait_exit() == 0
            result['final_plan'] = load_plan(data)
        result['status'] = 'passed'
    finally:
        for session in sessions:
            session.close()
        (output / 'results.json').write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
