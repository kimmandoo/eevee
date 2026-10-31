#!/usr/bin/env python3
"""Actual fresh/explicit-resume sessions and black-canvas acceptance; no provider calls.

Uses disposable projects/homes and read-only SQLite observations. Requires an
already-built ev plus the existing pyte/wcwidth terminal smoke dependencies.
"""
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3
import tempfile
import traceback

from native_connection_smoke import until
from response_smoke import SurfaceSession
from terminal_smoke import confirm_exit, terminal_environment
from workbench_smoke import wait_for

ESC = '\x1b'
GOAL = 'Retained session facts 731'
DRAFT = 'Retained 요청界e\u0301 draft 941'
PICKER = 'Saved sessions / committed tasks'


class Fixture:
    def __init__(self, binary, output, root, size):
        self.binary, self.output, self.root, self.size = binary, output, root, size
        self.project, self.home, self.data = (root / name for name in ('project', 'home', 'data'))
        self.project.mkdir(); self.home.mkdir()
        self.environment = terminal_environment('truecolor')
        for key in list(self.environment):
            if key.startswith(('EV_', 'OPENAI_', 'CODEX_', 'ANTHROPIC_', 'CLAUDE_', 'GEMINI_', 'GOOGLE_', 'AWS_', 'OPENCODE_')) or key.lower().endswith('_proxy'):
                self.environment.pop(key, None)
        self.environment.update(HOME=str(self.home), USERPROFILE=str(self.home),
            XDG_CONFIG_HOME=str(self.home / 'config'), XDG_DATA_HOME=str(self.home / 'data'),
            XDG_CACHE_HOME=str(self.home / 'cache'), APPDATA=str(self.home / 'config'),
            LOCALAPPDATA=str(self.home / 'cache'), CODEX_HOME=str(self.home / 'codex'))
        self.session = None
        self.sessions = []

    def launch(self, label, *flags, project=None):
        self.session = SurfaceSession([str(self.binary), '--data-dir', str(self.data),
            '--images', 'off', '--no-animation', '--theme', 'light' if self.size[0] == 60 else 'dark',
            '--color', '256' if self.size[0] == 60 else 'auto', *flags],
            project or self.project, self.environment, self.size, self.output / label)
        self.sessions.append(self.session)
        wait_for(self.session, 'Storage: saved', timeout=40)
        return self.session

    def state(self):
        database = self.data / 'workbench.sqlite3'
        with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True, timeout=10)) as db:
            db.row_factory = sqlite3.Row
            return {
                'tasks': [dict(row) for row in db.execute('SELECT id,goal,lead_partner,partner_drafts FROM tasks ORDER BY id')],
                'effects': {name: db.execute('SELECT count(*) FROM ' + name).fetchone()[0]
                            for name in ('provider_journal', 'executions', 'active_work')},
            }

    def draft(self):
        task = self.state()['tasks'][0]
        drafts = json.loads(task['partner_drafts'])[str(task['lead_partner'])].get('drafts', [])
        return next((row for row in drafts if row['field'] == 'Request'), None)

    def quit(self):
        self.session.command('quit')
        confirm_exit(self.session)
        assert self.session.wait_exit() == 0
        self.session.close()

    def close(self):
        for session in self.sessions:
            session.close()

    def canvas(self):
        # Check blank outer corners, not intentional selected-choice fills.
        screen = self.session.screen
        for y, x in ((0, 0), (screen.lines - 1, screen.columns - 1)):
            cell = screen.buffer[y][x]
            assert cell.bg in ('000000', 'black'), (x, y, cell)

    def fresh(self):
        view = self.session.display()
        assert GOAL not in view and DRAFT not in view, view
        wait_for(self.session, 'What would you like to do?')
        assert 'AUTO-APPROVE' not in view
        self.canvas()

    def seed(self):
        session = self.launch('seed')
        session.command('goal'); session.paste(GOAL); session.send('\r')
        until(session, lambda: any(row['goal'] == GOAL for row in self.state()['tasks']), 'saved goal')
        session.paste(DRAFT); session.send(ESC + '[D' + ESC + '[D' + '\x13')
        until(session, lambda: self.draft() is not None and self.draft()['text'] == DRAFT
              and self.draft()['cursor'] < len(DRAFT.encode()), 'saved Unicode draft and interior caret')
        saved = self.draft()
        session.capture('saved-draft-before-exit')
        self.quit()
        return saved

    def run(self):
        saved = self.seed()
        task_id = self.state()['tasks'][0]['id']
        session = self.launch('plain-fresh')
        self.fresh()
        assert self.draft() == saved
        session.command('resume'); wait_for(session, PICKER)
        session.capture('explicit-resume-picker-no-send'); self.canvas()
        session.send(ESC); self.fresh()
        self.quit()
        assert len(self.state()['tasks']) == 1, 'blank navigation created an empty saved session'

        session = self.launch('continue', '--continue')
        wait_for(session, 'draft 941')
        assert self.draft() == saved
        session.command('auto-approve-session'); wait_for(session, 'AUTO-APPROVE')
        wait_for(session, 'Storage: saved')
        session.command('new'); self.fresh()
        assert self.draft() == saved
        session.capture('new-clears-old-work-and-session-grant')
        session.command('resume'); wait_for(session, PICKER)
        session.send(ESC + '[H' + '\r')
        wait_for(session, 'draft 941')
        assert 'AUTO-APPROVE' not in session.display(), 'resuming restored an old grant'
        assert self.draft() == saved
        self.quit()

        session = self.launch('resume-id', '--resume', str(task_id))
        wait_for(session, 'draft 941')
        session.capture('exact-saved-id-restored-without-send')
        self.quit()
        session = self.launch('launch-picker', '--resume')
        wait_for(session, PICKER); session.send(ESC); self.fresh(); self.quit()
        session = self.launch('missing-id', '--resume', '999999')
        wait_for(session, '#999999 is unavailable')
        wait_for(session, PICKER)
        session.capture('missing-id-actionable-not-substituted')
        session.send(ESC); self.fresh(); self.quit()
        foreign = self.root / 'foreign-project'; foreign.mkdir()
        session = self.launch('foreign-id', '--resume', str(task_id), project=foreign)
        wait_for(session, f'#{task_id} is unavailable')
        assert GOAL not in session.display() and 'draft 941' not in session.display()
        self.quit()
        assert self.draft() == saved
        session = self.launch('new-with-pending-draft')
        pending = 'Keep pending 요청 without Send 517'
        session.paste(pending, settle=False)
        session.command('new', settle=False)
        until(session, lambda: any(
            draft['text'] == pending
            for task in self.state()['tasks']
            for state in json.loads(task['partner_drafts']).values()
            for draft in state.get('drafts', [])), 'old pending draft committed before fresh session')
        until(session, lambda: pending not in session.display(), 'new session waited for the draft save')
        self.fresh()
        session.capture('pending-draft-saved-before-new-no-send')
        self.quit()
        assert all(value == 0 for value in self.state()['effects'].values()), self.state()
        return {'status': 'passed', 'size': list(self.size), 'task_id': task_id,
                'cases': ['fresh-launch-preserved-retained-draft', 'neutral-picker-no-materialization',
                    'explicit-continue-and-id', 'new-and-resume-no-send-no-inherited-approval',
                    'missing-and-foreign-id-no-substitution', 'new-waits-for-pending-draft-save',
                    'black-canvas', 'zero-provider-or-command-effects']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    report = {'status': 'failed', 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'journeys': []}
    try:
        for size in ((120, 36), (60, 18)):
            directory = output / str(size[0]); directory.mkdir()
            with tempfile.TemporaryDirectory(prefix='ev-session-') as temporary:
                fixture = Fixture(binary, directory, Path(temporary), size)
                try:
                    report['journeys'].append(fixture.run())
                except Exception:
                    fixture.session.capture('failure')
                    report['journeys'].append({'status': 'failed', 'size': list(size), 'error': traceback.format_exc()})
                finally:
                    fixture.close()
        if all(row['status'] == 'passed' for row in report['journeys']):
            report['status'] = 'passed'
    finally:
        (output / 'results.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({'status': report['status'], 'results': str(output / 'results.json')}))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
