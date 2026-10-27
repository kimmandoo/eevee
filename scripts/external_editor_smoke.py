#!/usr/bin/env python3
"""Opt-in RX05 real disposable-editor journeys against one prebuilt binary.

No builds, provider calls or SQL writes. Fixtures/data/screens remain under --output.
  python scripts/external_editor_smoke.py --binary /absolute/ev --output /tmp/rx05
"""
from __future__ import annotations
import argparse
from contextlib import closing
import json
import os
from pathlib import Path
import shutil
import sqlite3
import sys
import time

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import wait_for

ESC = '\x1b'
ORIGINAL = 'Original 한글😀 draft kept\nlast line\n'
CANDIDATE = 'External 한글😀\r\nlast line\r\n\n'
LITERAL = '$HOME; not a shell argument'

FIXTURE = r'''import json, os, signal, sys, time
from pathlib import Path
root = Path(sys.argv[1])
assert sys.argv[2] == '$HOME; not a shell argument'
p = Path(sys.argv[-1])
mode = (root / 'mode').read_text()
evidence = {'mode': mode, 'path': str(p), 'argv': sys.argv, 'input_hex': p.read_bytes().hex(), 'stdin_isatty': sys.stdin.isatty()}
if os.name != 'nt':
    import termios
    attrs = termios.tcgetattr(0)
    evidence['canonical'] = bool(attrs[3] & termios.ICANON)
    evidence['echo'] = bool(attrs[3] & termios.ECHO)
    evidence['process_group'] = os.getpgrp()
    evidence['parent_process_group'] = os.getpgid(os.getppid())
(root / ('editor-' + mode + '.json')).write_text(json.dumps(evidence))
if mode == 'cancel':
    print('DISPOSABLE_EDITOR_WAITING_FOR_CTRL_C', flush=True)
    while True: time.sleep(1)
if mode == 'error': sys.exit(7)
if mode == 'invalid': p.write_bytes(b'\xff')
elif mode == 'oversize': p.write_bytes(b'x' * 4097)
elif mode == 'unsafe':
    p.unlink()
    p.symlink_to(root / 'mode')
elif mode == 'replace':
    other = p.with_suffix('.replacement')
    other.write_text('replacement')
    other.replace(p)
elif mode == 'cleanup':
    (p.parent / 'editor-owned.backup').write_bytes(p.read_bytes())
    p.write_bytes(b'candidate not imported')
elif mode == 'link':
    os.link(p, p.with_suffix('.hardlink'))
elif mode == 'permission':
    p.chmod(0o644)
elif mode == 'success': p.write_bytes('External 한글😀\r\nlast line\r\n\n'.encode())
'''


def until(session, predicate, why, timeout=30):
    deadline = time.monotonic() + timeout
    while not predicate():
        assert session.alive(), 'application exited: ' + why
        assert time.monotonic() < deadline, why + '\n' + session.display()
        session.pump(.05)


def expose(session):
    # Never skip scope with End: each next page must overlap the exposure frontier.
    for _ in range(160):
        if 'y approves' in session.display():
            return
        session.send(ESC + '[6~')
        session.pump(.03)
    raise AssertionError('review never fully exposed\n' + session.display())


def approve(session):
    # Full exposure is not global idle: an unrelated records load can become
    # inflight after rendering. Tab is non-approving and acknowledges the same
    # complete lifecycle/owner gate used by launch and import.
    deadline = time.monotonic() + 30
    while '[Proceed]' not in session.display():
        assert session.alive(), 'application exited while selecting Proceed'
        assert time.monotonic() < deadline, 'review not ready to proceed\n' + session.display()
        session.send('\t')
    session.send('\r')


def journey(binary, output, size):
    output.mkdir(parents=True, exist_ok=False)
    project, home, data = (output / name for name in ('project 空 格', 'home', 'data'))
    project.mkdir(); home.mkdir()
    fixture = output / 'disposable editor.py'
    fixture.write_text(FIXTURE, encoding='utf-8')
    env = terminal_environment('mono')
    for key in list(env):
        if key.startswith(('EV_', 'EEVEE_', 'CODEX_', 'OPENAI_', 'ANTHROPIC_', 'CLAUDE_', 'GEMINI_', 'GOOGLE_', 'AWS_', 'OPENCODE_', 'DEVIN_', 'WINDSURF_')) or key.lower().endswith('_proxy'):
            env.pop(key, None)
    env.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / 'config'),
               XDG_DATA_HOME=str(home / 'data'), XDG_CACHE_HOME=str(home / 'cache'),
               VISUAL=json.dumps([sys.executable, str(fixture), str(output), LITERAL]), EDITOR='missing-editor-must-not-be-used')
    session = None
    report = {'size': list(size), 'binary': str(binary), 'data': str(data), 'project': str(project), 'journeys': []}

    def state():
        with closing(sqlite3.connect((data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
            db.row_factory = sqlite3.Row
            row = dict(db.execute('SELECT * FROM tasks ORDER BY id LIMIT 1').fetchone())
            drafts = json.loads(row['partner_drafts']).get(str(row['lead_partner']), {}).get('drafts', [])
            draft = next((draft for draft in drafts if draft['field'] == 'Request'), {'text': '', 'cursor': 0})
            effects = {table: db.execute('SELECT count(*) FROM ' + table).fetchone()[0]
                       for table in ('provider_journal', 'executions', 'active_work', 'excerpts')}
            # selections is the project's active-task pointer, not selected context.
            active = [tuple(item) for item in db.execute('SELECT project,active_id FROM selections ORDER BY project')]
            return {'draft': draft, 'effects': effects, 'lead': row['lead_partner'],
                    'active': active, 'approved_memories': json.loads(row['approved_memories'])}

    def launch():
        nonlocal session
        session = Session([str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono'],
                          project, env, size, output / ('terminal-' + str(len(report['journeys']))), controlling_tty=True)
        wait_for(session, 'Storage: saved', timeout=30)

    def finish():
        session.command('quit'); confirm_exit(session)
        assert session.wait_exit() == 0
        session.close()

    def open_review():
        session.command('external-edit')
        wait_for(session, 'launch review', timeout=30)
        expose(session)

    try:
        launch()
        session.paste(ORIGINAL)
        session.send(ESC + '[H' + (ESC + '[C') * 3)
        session.send('\x13')
        until(session, lambda: (data / 'workbench.sqlite3').exists() and state()['draft']['text'] == ORIGINAL, 'original persisted')
        baseline = state()
        # Launch cancellation is neutral even after complete disclosure.
        open_review(); session.capture('launch-reviewed-neutral')
        count = len(list(output.glob('editor-*.json')))
        session.send('\r')
        assert len(list(output.glob('editor-*.json'))) == count
        assert state() == baseline
        # Return cancellation: real process, then no import.
        (output / 'mode').write_text('success')
        open_review(); approve(session)
        wait_for(session, 'returned candidate', timeout=30)
        expose(session); session.capture('candidate-reviewed-neutral')
        session.send(ESC)
        assert state() == baseline
        evidence = json.loads((output / 'editor-success.json').read_text())
        assert bytes.fromhex(evidence['input_hex']) == ORIGINAL.encode()
        assert not Path(evidence['path']).parent.exists()
        # Error and unsafe-output journeys must restore the original owning text/caret.
        modes = ['error', 'invalid', 'oversize', 'replace', 'cleanup', 'cancel']
        if os.name != 'nt': modes += ['unsafe', 'link', 'permission']
        for mode in modes:
            (output / 'mode').write_text(mode)
            open_review(); approve(session)
            if mode == 'cancel':
                wait_for(session, 'DISPOSABLE_EDITOR_WAITING_FOR_CTRL_C', timeout=20)
                session.send('\x03')
            wait_for(session, 'External editing failed', timeout=30)
            # Every real child sees restored cooked modes and the exact original bytes.
            evidence = json.loads((output / ('editor-' + mode + '.json')).read_text())
            assert evidence['stdin_isatty']
            if os.name != 'nt': assert evidence['canonical'] and evidence['echo']
            if os.name != 'nt':
                assert evidence['process_group'] != evidence['parent_process_group']
            assert bytes.fromhex(evidence['input_hex']) == ORIGINAL.encode()
            if mode in ('cleanup', 'link', 'permission'):
                # Permission refusal can still clean safely; only backups/hardlinks must warn.
                if mode != 'permission':
                    expose_error = session.display()
                    for _ in range(50):
                        if 'cleanup FAILED' in expose_error: break
                        session.send(ESC + '[6~'); expose_error += session.display()
                    assert 'cleanup FAILED' in expose_error
            session.capture('error-' + mode)
            session.send(ESC)
            assert state() == baseline
            temp_parent = Path(evidence['path']).parent
            if mode not in ('cleanup', 'link'):
                assert not temp_parent.exists(), mode + ' left its temporary directory'
            if temp_parent.exists(): shutil.rmtree(temp_parent)  # Fixture-owned cleanup only after warning proof.
            report['journeys'].append(mode)
        # Final approved import must preserve CRLF and trailing newlines in SQLite.
        (output / 'mode').write_text('success')
        open_review(); approve(session)
        wait_for(session, 'returned candidate', timeout=30)
        expose(session); approve(session)
        until(session, lambda: state()['draft']['text'].encode() == CANDIDATE.encode(), 'exact imported CRLF draft persisted')
        imported = state()
        assert imported['draft']['cursor'] == len(CANDIDATE.encode())
        assert imported['effects'] == baseline['effects']
        assert not any(imported['effects'].values())
        assert imported['active'] == baseline['active']
        assert imported['approved_memories'] == baseline['approved_memories'] == []
        report['effects'] = imported['effects']
        session.capture('exact-import-unsent')
        finish()
        report['journeys'].append('success-exact-utf8-crlf-trailing-newlines')
        # Missing executable review is separately exercised with a real restart and no send.
        env['VISUAL'] = str(output / 'missing-editor')
        launch(); session.command('external-edit')
        wait_for(session, 'Executable not found', timeout=30)
        assert state() == imported
        session.capture('missing-executable-original-kept'); session.send(ESC)
        finish(); report['journeys'].append('missing-executable')
        report['status'] = 'passed'
    except BaseException:
        report['status'] = 'failed'
        if session is not None: session.capture('failure')
        raise
    finally:
        if session is not None: session.close()
        (output / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--size', action='append', default=[])
    args = parser.parse_args()
    args.output = args.output.resolve()
    binary = args.binary.resolve(strict=True)
    args.output.mkdir(parents=True, exist_ok=False)
    results = []
    for value in args.size or ['120x36', '60x18']:
        size = tuple(map(int, value.split('x')))
        results.append(journey(binary, args.output / value, size))
    (args.output / 'report.json').write_text(json.dumps(results, indent=2, ensure_ascii=False) + '\n')

if __name__ == '__main__': main()
