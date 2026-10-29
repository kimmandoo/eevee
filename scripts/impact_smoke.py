#!/usr/bin/env python3
"""Actual inert impact review, caller-test failure and shared-config broadening.

Uses a built ev in isolated PTYs, a real disposable Rust project and its actual
Cargo checks. No provider peer, paid calls, SQL writes or test substitute.
"""
from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
import time
import traceback

from response_smoke import SurfaceSession
from terminal_smoke import confirm_exit, terminal_environment
from workbench_smoke import wait_for
from tool_plans_smoke import review_plan

ESC = '\x1b'
PAGE = ESC + '[6~'


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')


def rows(database):
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True, timeout=10)) as connection:
        connection.row_factory = sqlite3.Row
        task = dict(connection.execute('SELECT * FROM tasks ORDER BY id DESC LIMIT 1').fetchone())
        task['workflows'] = json.loads(task['workflows'])
        runs = [json.loads(row['record']) for row in connection.execute('SELECT record FROM executions ORDER BY rowid')]
        return task, runs


def until(session, predicate, message, timeout=60):
    deadline = time.monotonic() + timeout
    while not predicate():
        assert session.alive(), message + ': application exited'
        assert time.monotonic() < deadline, message + '\n' + session.display()
        session.pump(0.05)


def pages(session, output, label):
    observations = []
    previous = None
    for index in range(100):
        display = session.display()
        session.capture(label + '-' + str(index))
        if display == previous:
            break
        observations.append(display)
        previous = display
        session.send(PAGE)
    else:
        raise AssertionError('Impact review paging did not terminate')
    text = '\n'.join(observations)
    (output / (label + '.txt')).write_text(text, encoding='utf-8')
    return ''.join(text.split())


def journey(binary, output, size):
    with tempfile.TemporaryDirectory(prefix='ev-impact-') as temporary:
        root = Path(temporary).resolve()
        project, home, data = root / 'project', root / 'home', root / 'data'
        project.mkdir(); home.mkdir()
        member = project / 'member'
        other = project / 'other'
        (member / 'src').mkdir(parents=True); (other / 'src').mkdir(parents=True)
        (project / 'Cargo.toml').write_text('[workspace]\nmembers=["member","other"]\ndefault-members=["other"]\nresolver="2"\n', encoding='utf-8')
        (member / 'Cargo.toml').write_text('[package]\nname="impact_fixture"\nversion="0.0.1"\nedition="2021"\n', encoding='utf-8')
        (other / 'Cargo.toml').write_text('[package]\nname="default_other"\nversion="0.0.1"\nedition="2021"\n', encoding='utf-8')
        (other / 'src/lib.rs').write_text('#[test] fn unrelated_contract() { assert_eq!(2+2, 4); }\n', encoding='utf-8')
        (project / '.gitignore').write_text('/target/\n', encoding='utf-8')
        (member / 'src/lib.rs').write_text('pub mod callee;\npub mod caller;\n', encoding='utf-8')
        callee = member / 'src/callee.rs'
        callee.write_text('pub fn value() -> i32 { 2 }\n', encoding='utf-8')
        (member / 'src/caller.rs').write_text('pub fn caller() -> i32 { crate::callee::value() }\n#[cfg(test)] mod tests {\n#[test] #[ignore = "explicit impact opt-in"] fn caller_contract() { assert_eq!(super::caller(), 2); }\n}\n', encoding='utf-8')
        environment = terminal_environment('mono')
        for key in list(environment):
            if key.startswith(('EV_', 'OPENAI_', 'CODEX_', 'ANTHROPIC_', 'CLAUDE_', 'GOOGLE_', 'GEMINI_')) or key.lower().endswith('_proxy'):
                environment.pop(key, None)
        environment.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / 'config'),
                           XDG_DATA_HOME=str(home / 'data'), XDG_CACHE_HOME=str(home / 'cache'),
                           EV_PROVIDER='compatible', EV_API_BASE_URL='http://127.0.0.1:9/v1', EV_API_MODEL='unused', EV_API_KEYLESS='1')
        # Keep the installed toolchain location, not a disposable empty rustup home.
        for name in ('CARGO_HOME', 'RUSTUP_HOME'):
            environment[name] = os.environ.get(name, str(Path.home() / ('.cargo' if name == 'CARGO_HOME' else '.rustup')))
        environment['RUSTUP_TOOLCHAIN'] = '1.90.0'
        subprocess.run([shutil.which('cargo'), 'generate-lockfile', '--offline'], cwd=project, env=environment, check=True, capture_output=True, timeout=30)
        git = shutil.which('git')
        def command(*args):
            return subprocess.run([git, '-c', 'core.hooksPath=', *args], cwd=project, check=True, capture_output=True, timeout=30)
        command('init', '-q'); command('add', '.')
        command('-c', 'user.name=Local acceptance', '-c', 'user.email=local@example.invalid', 'commit', '-qm', 'impact baseline')
        database = data / 'workbench.sqlite3'
        session = SurfaceSession([str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono', '--no-animation', *(['--ascii'] if size[0] == 60 else [])], project, environment, size, output, )
        try:
            wait_for(session, 'Storage: saved', timeout=35)
            session.command('goal'); session.paste('Impact-directed caller check acceptance'); session.send('\r')
            wait_for(session, 'Storage: saved', timeout=35)
            session.command('jolteon')
            wait_for(session, 'Storage: saved', timeout=35)
            before = rows(database)[1]
            callee.write_text('pub fn value() -> i32 { 3 }\n', encoding='utf-8')
            session.command('refresh')
            session.command('diff')
            wait_for(session, 'Repository: refreshed', timeout=35)
            session.send(ESC)
            session.command('impact-checks')
            until(session, lambda: 'Targeted recommendation' in session.display(), 'Targeted proposal did not finish')
            text = pages(session, output, 'targeted-proposal')
            assert 'caller_contract' in text and 'unrelated_contract' not in text, text
            assert 'TARGETED' in text, text
            assert rows(database)[1] == before, 'Inspection started a command'
            session.send(ESC)
            session.command('stage-impact-plan')
            assert rows(database)[1] == before, 'Staging without active preview ran a command'
            session.command('impact-checks')
            until(session, lambda: 'Targeted recommendation' in session.display(), 'Refreshed proposal did not finish')
            session.command('stage-impact-plan')
            until(session, lambda: rows(database)[0]['workflows'].get('tool_plans') is not None, 'Impact plan did not persist')
            task, runs = rows(database)
            assert runs == before, 'Staging granted execution'
            plan = task['workflows']['tool_plans']
            save(output / 'staged-targeted-plan.json', plan)
            review_plan(session)
            session.command('approvals')
            until(session, lambda: 'Review one-shot execution permission' in session.display(), 'Each proposed command needs exact Jolteon review')
            session.capture('exact-jolteon-review')
            session.send(ESC + '[C' + chr(13))
            until(session, lambda: len(rows(database)[1]) == len(before) + 1 and rows(database)[1][-1]['state'] == 'Failed', 'Caller test did not catch changed callee', timeout=120)
            failure = rows(database)[1][-1]
            assert failure['exit_code'] == 101, failure
            assert 'caller_contract' in json.dumps(failure['prepared']['spec']), failure
            save(output / 'actual-targeted-failure.json', failure)
            # Existing executed plans cannot be replaced as if history vanished.
            # Broader inspection remains available without staging over that plan.
            manifest = project / 'Cargo.toml'
            manifest.write_text(manifest.read_text() + '\n[profile.test]\nopt-level=1\n', encoding='utf-8')
            session.command('refresh')
            session.command('diff')
            wait_for(session, 'Repository: refreshed', timeout=35)
            session.send(ESC)
            session.command('impact-checks')
            until(session, lambda: 'Broader recommendation' in session.display(), 'Shared-config proposal did not broaden')
            broader = pages(session, output, 'shared-config-proposal')
            assert '--workspace' in broader and '--all-targets' in broader, broader
            assert len(rows(database)[1]) == len(before) + 1, 'Broader recommendation ran automatically'
            session.send(ESC)
            session.command('quit'); confirm_exit(session)
            assert session.wait_exit() == 0
            return {'status': 'passed', 'size': size, 'targeted_exit': 101,
                    'actual_runs': 1, 'inert_preview_and_stage': True, 'shared_config_broadened': True}
        finally:
            session.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    binary, output = Path(args.binary).resolve(), Path(args.output).resolve()
    assert not output.exists() or not any(output.iterdir()), 'Use a new evidence directory'
    output.mkdir(parents=True, exist_ok=True)
    report = {'status': 'failed', 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'journeys': []}
    try:
        for size in ((120, 36), (60, 18)):
            report['journeys'].append(journey(binary, output / ('wide' if size[0] == 120 else 'compact'), size))
        report['status'] = 'passed'
    except Exception:
        report['error'] = traceback.format_exc()
        raise
    finally:
        save(output / 'results.json', report)
    print(json.dumps(report, ensure_ascii=False))

if __name__ == '__main__':
    main()
