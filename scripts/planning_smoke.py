#!/usr/bin/env python3
"""Opt-in real Codex planning acceptance; uses provider-owned credentials and billing."""
import argparse
from contextlib import ExitStack, closing
import json
import os
from pathlib import Path
import platform
import sqlite3
import subprocess
import tempfile
import time

from terminal_smoke import Session, confirm_exit, terminal_environment
from workbench_smoke import wait_for

HOME = '\x1b[H'
DOWN = '\x1b[1;3B'
DELETE = '\x1b[3~'
TITLE = 'Espeon / Codex planning'
IDLE = 'Ready ·'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--live', action='store_true', help='Authorize bounded real provider requests')
    parser.add_argument('--binary', default='target/release/ev')
    parser.add_argument('--output', default='target/planning-smoke')
    args = parser.parse_args()
    if not args.live:
        parser.error('--live is required; this suite makes real model requests')
    binary = Path(args.binary).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {'status': 'failed', 'host': platform.platform(), 'cases': []}
    sessions = []

    def passed(name):
        result['cases'].append(name)
        print('passed:', name, flush=True)

    def close_sessions():
        for session in sessions:
            session.close()

    try:
        with ExitStack() as stack:
            root = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix='ev-planning-空 格-'))).resolve()
            stack.callback(close_sessions)
            project = root / 'project'
            project.mkdir()
            source = project / 'note.txt'
            source.write_text('Selected design source, original content.\n')
            subprocess.run(['git', 'init', '--quiet'], cwd=project, check=True)
            for command in (['add', '--', 'note.txt'], ['-c', 'user.name=Smoke', '-c', 'user.email=smoke@example.invalid', 'commit', '-qm', 'fixture']):
                subprocess.run(['git', '-c', 'core.hooksPath=', *command], cwd=project, check=True)

            def launch(name, size=(120, 36), data_name=None, path_prefix=None):
                data = root / ('data-' + (data_name or name))
                environment = terminal_environment('mono')
                if path_prefix is not None:
                    environment['PATH'] = str(path_prefix) + os.pathsep + environment.get('PATH', '')
                session = Session([str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono'],
                                  project, environment, size, output / name)
                sessions.append(session)
                session.workbench_data = data
                wait_for(session, 'Storage: saved')
                session.command('verbose')  # Retain exact provider diagnostics and item identities.
                return session

            def task(session):
                with closing(sqlite3.connect((session.workbench_data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
                    return db.execute('SELECT goal,next_action FROM tasks ORDER BY id LIMIT 1').fetchone()

            def until(session, predicate, explanation, seconds=150):
                deadline = time.monotonic() + seconds
                while not predicate():
                    assert session.alive(), explanation + ': app exited'
                    assert time.monotonic() < deadline, explanation + '\n' + session.display()
                    session.pump(0.1)

            def goal(session, text):
                old = task(session)
                session.command('goal')
                wait_for(session, 'Task fact / local')
                assert not session.screen.cursor.hidden, 'goal command did not activate editing'
                if old:
                    session.send(HOME + DELETE * len(old[0]))
                session.paste(text)
                session.send('\r')
                until(session, lambda: task(session) is not None and task(session)[0] == text, 'goal committed', 15)
                if TITLE not in session.display():
                    wait_for(session, 'Storage: saved')

            def preview(session, resume=False):
                session.command('resume' if resume else 'new')
                wait_for(session, 'LOCAL PREVIEW', timeout=20)
                wait_for(session, 'y Run', timeout=20)
                session.capture('resume-preview' if resume else 'new-preview')

            def run_preview(session):
                session.send('y')

            def quit_app(session):
                session.send('\x1b')
                session.command('quit')
                confirm_exit(session)
                assert session.wait_exit() == 0
                session.close()

            def inspect_items(session, name):
                session.send('\x1b[1;3A' * 128)
                screens = []
                for index in range(128):
                    screen = session.display()
                    if screens and screen == screens[-1]:
                        break
                    screens.append(screen)
                    session.capture(f'{name}-item-{index}')
                    session.send(DOWN)
                return screens

            def has_probe_output(screen):
                return any(line.strip(' │') == 'EEVEE_APPROVAL_PROBE' for line in screen.splitlines())

            stale = launch('source-revalidation')
            goal(stale, 'Plan a two-step note workflow using only the selected source; do not run commands.')
            stale.command('files')
            wait_for(stale, 'eligible files /', timeout=35)
            stale.command('search')
            wait_for(stale, 'Filename filter')
            stale.paste('note.txt')
            stale.send('\r')
            wait_for(stale, '1 eligible files /')
            stale.send('\r')
            wait_for(stale, 'Review local capture /')
            stale.command('attach')
            wait_for(stale, '1 task-owned basket references / stored')
            stale.command('close')
            wait_for(stale, 'Storage: saved')
            stale.command('role-plan')
            wait_for(stale, TITLE)
            preview(stale)
            source.write_text('Changed after preview; must never be sent.\n')
            run_preview(stale)
            wait_for(stale, IDLE, timeout=30)
            stale.capture('changed-source-refused')
            assert any(word in stale.display().lower() for word in ('changed', 'stale', 'identity', 'mismatch')), stale.display()
            stale.command('resume')
            wait_for(stale, 'Resume requires an explicitly selected same-task thread')
            passed('changed source after preview refuses provider dispatch')
            if os.name == 'posix':
                original = 'Selected design source, original content.\n'
                source.write_text(original)
                preview(stale)
                outside = root / 'outside.txt'
                outside.write_text(original)
                source.unlink()
                source.symlink_to(outside)
                run_preview(stale)
                wait_for(stale, IDLE, timeout=30)
                stale.capture('substituted-source-link-refused')
                stale.command('resume')
                wait_for(stale, 'Resume requires an explicitly selected same-task thread')
                assert outside.read_text() == original
                source.unlink()
                source.write_text(original)
                passed('same-content escaping symlink after preview refuses provider dispatch')
            quit_app(stale)

            if os.name == 'posix':
                malformed_dir = root / 'malformed-transport'
                malformed_dir.mkdir()
                executable = malformed_dir / 'codex'
                executable.write_text('#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n'
                                      '  printf \"codex-cli 0.152.1\\\\n\"\n'
                                      'else\n  printf \"this is not JSON\\\\n\"\nfi\n')
                executable.chmod(0o700)
                malformed = launch('malformed-transport', path_prefix=malformed_dir)
                goal(malformed, 'A malformed transport must not produce a completed plan.')
                malformed.command('role-plan')
                preview(malformed)
                run_preview(malformed)
                wait_for(malformed, IDLE, timeout=30)
                malformed.capture('fatal-transport-refusal')
                assert 'Thread:' not in malformed.display()
                assert any(word in malformed.display().lower() for word in ('malformed', 'protocol', 'transport', 'json')), malformed.display()
                malformed.command('resume')
                wait_for(malformed, 'Resume requires an explicitly selected same-task thread')
                quit_app(malformed)
                passed('malformed owned transport fails visibly without completion or automatic resend')

            session = launch('live-planning')
            question_goal = ('Design a two-step local note-taking workflow. Before proposing a design, use request_user_input '
                             'to ask whether notes should be Markdown or plain text, with exactly those two options. '
                             'Do not read files or run commands. After my answer, give one brief design decision.')
            goal(session, question_goal)
            session.command('role-plan')
            wait_for(session, TITLE)
            preview(session)
            session.send('\r')
            wait_for(session, IDLE)
            session.command('resume')
            wait_for(session, 'Resume requires an explicitly selected same-task thread')
            preview(session)
            run_preview(session)
            wait_for(session, 'QUESTION 1/', timeout=150)
            session.capture('real-question')
            session.resize((60, 18))
            session.command('answer')
            wait_for(session, 'Literal answer')
            session.paste('Markdown — 한국어 메모')
            session.send('\r')
            session.command('answers-review')
            wait_for(session, 'y Send')
            session.capture('compact-answer-review')
            session.send('y')
            wait_for(session, IDLE, timeout=150)
            session.resize((120, 36))
            session.capture('completed-plan')
            for _ in range(128):
                session.command('decision')
                if 'Next action / local' in session.display() and not session.screen.cursor.hidden:
                    break
                session.send(DOWN)
            else:
                raise AssertionError('no complete selectable provider decision')
            session.send('\r')
            until(session, lambda: bool(task(session)[1]), 'selected decision saved', 15)
            decision = task(session)[1]
            assert 'Codex' in decision and ('Markdown' in decision or '마크다운' in decision), decision
            passed('real plan question/literal CJK answer/selected decision durably saved')

            if TITLE in session.display():
                session.send('\x1b')
            goal(session, 'Ask a NEW clarification via request_user_input: should the note title be Date or Topic? '
                          'Offer those two options, do not infer from previous answers, and wait for my response. '
                          'Do not read files or run commands.')
            session.command('role-plan')
            wait_for(session, TITLE)
            preview(session, resume=True)
            run_preview(session)
            wait_for(session, 'QUESTION 1/', timeout=150)
            session.capture('resumed-question')
            session.resize((39, 10))
            session.send('\t\r')
            session.capture('undersize-no-answer')
            session.command('cancel')
            session.resize((120, 36))
            wait_for(session, IDLE, timeout=30)
            session.send('\t\r')
            session.capture('cancelled-stale-answer')
            assert 'PROVIDER REQUEST' not in session.display(), 'cancelled prompt remained answerable'
            if '4096 bytes' in session.display():
                session.send('\x1b')
                wait_for(session, TITLE)
            passed('explicit same-thread resume and cancellation invalidate pending answer')

            session.send('\x1b')
            approval_goal = ('Perform a bounded approval-path diagnostic before giving a one-line plan: use exec_command '
                             'with command `printf EEVEE_APPROVAL_PROBE`, sandbox_permissions `require_escalated`, and a '
                             'justification asking permission for this harmless print. Do not substitute a sandboxed invocation '
                             'or write any file. If denied, stop and state that it was denied.')
            goal(session, approval_goal)
            session.command('role-plan')
            preview(session)
            run_preview(session)
            wait_for(session, 'a/y Allow', timeout=150)
            session.capture('real-command-deny')
            session.send('\r')
            wait_for(session, IDLE, timeout=150)
            session.capture('denied-command-outcome')
            denied_items = inspect_items(session, 'denied')
            assert not any(has_probe_output(screen) for screen in denied_items), 'denied command produced output'
            assert any('Status: declined' in screen or 'interrupted turn' in screen for screen in denied_items), denied_items
            passed('real provider command approval denied with offered option')

            preview(session)
            run_preview(session)
            wait_for(session, 'a/y Allow', timeout=150)
            session.send('a')
            wait_for(session, IDLE, timeout=150)
            session.capture('allowed-command-outcome')
            allowed_items = inspect_items(session, 'allowed')
            assert any('Status: completed' in screen and has_probe_output(screen) for screen in allowed_items), allowed_items
            passed('real provider command explicitly allowed once without persistent grant')
            quit_app(session)
            reopened = launch('reopened-planning', data_name='live-planning')
            assert task(reopened)[1] == decision, 'decision changed or disappeared after restart'
            reopened.command('role-plan')
            wait_for(reopened, TITLE)
            reopened.command('resume')
            wait_for(reopened, 'Resume requires an explicitly selected same-task thread')
            quit_app(reopened)
            passed('restart retains user-selected decision without provider replay')

            exiting = launch('active-safe-exit', size=(60, 18))
            goal(exiting, 'Use request_user_input to ask Date or Topic for note titles, then wait. '
                          'Do not run commands or read files.')
            exiting.command('role-plan')
            preview(exiting)
            run_preview(exiting)
            wait_for(exiting, 'QUESTION 1/', timeout=150)
            exiting.resize((39, 10))
            exiting.send('\x03')
            wait_for(exiting, 'Exit planning?')
            exiting.send('\r')
            assert exiting.alive(), 'neutral exit terminated the active provider turn'
            exiting.send('\x03')
            wait_for(exiting, 'Exit planning?')
            exiting.send('\t\r')
            exiting.resize((120, 36))
            deadline = time.monotonic() + 30
            while exiting.alive() and 'UNKNOWN OUTCOME' not in exiting.display() and 'Confirm exit' not in exiting.display():
                assert time.monotonic() < deadline, 'owned provider cancellation did not finish'
                exiting.pump(0.1)
            if exiting.alive() and 'UNKNOWN OUTCOME' in exiting.display():
                exiting.capture('informed-unknown-exit')
                exiting.send('\t\r')
            confirm_exit(exiting)
            assert exiting.wait_exit() == 0
            exiting.close()
            passed('neutral undersize safe exit cancels provider before storage-aware quit and restoration')
        result['status'] = 'passed'
    finally:
        for session in sessions:
            session.capture('final-surface')
            session.close()
        (output / 'results.json').write_text(json.dumps(result, indent=2, ensure_ascii=False) + '\n')


if __name__ == '__main__':
    main()
