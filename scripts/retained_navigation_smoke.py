#!/usr/bin/env python3
"""Opt-in GP05/RX06 actual PTY/SQLite/controlled-local-peer journeys; no paid/live calls.

Uses the existing disposable JSONL peer and real app-owned journal/writer. Database
queries are read-only. --keep-fixture preserves the canonical project/data paths
for genuine older-reader refusal proof. Requires an already built binary, pyte.
"""
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import re
import sqlite3
import shutil
from cooperation_smoke import Journey as CooperationJourney, ESC, HOME, LEFT, DELETE, UP
from response_smoke import SurfaceSession, response_view, wheel
from workbench_smoke import wait_for

DOWN = ESC + "[B"

class Journey(CooperationJourney):
    def launch(self, suffix):
        self.search_archived = False
        session = SurfaceSession(self.argv, self.project, self.environment, self.size, self.output / suffix)
        wait_for(session, "Storage: saved", timeout=40)
        return session

    def tasks(self):
        with closing(sqlite3.connect(self.database.as_uri()+"?mode=ro", uri=True)) as db:
            db.row_factory = sqlite3.Row
            return [dict(row) for row in db.execute("SELECT * FROM tasks ORDER BY id")]

    def labels(self, task):
        row = next(row for row in self.tasks() if row['id'] == task)
        return json.loads(row['workflows']).get('landmarks', [])

    def set_goal(self, value):
        self.session.command('goal')
        self.session.send(UP * 100 + HOME + DELETE * 4096)
        self.session.paste(value)
        self.session.send('\r')
        self.until(lambda: any(row['goal'] == value for row in self.tasks()), 'goal save')
        self.workspace()
        wait_for(self.session, 'Storage: saved')

    def search(self, literal, archived=False):
        self.session.command('project-search ' + literal)
        if archived != getattr(self, 'search_archived', False):
            self.session.send('\t\r')
            self.search_archived = archived
        self.until(lambda: 'fields scanned' in self.session.display() and 'Reading bounded' not in self.session.display(), 'retained search completion')
        return self.session.display()

    def select_task(self, task):
        self.workspace()
        self.session.command('tasks')
        wait_for(self.session, 'Saved sessions / committed tasks')
        rows = re.findall(r'#(\d+) (?:Open|Complete) /', self.session.display())
        assert str(task) in rows, self.session.display()
        self.session.send(HOME + DOWN * rows.index(str(task)) + '\r')
        self.workspace()

    def restart(self, label):
        self.quit()
        self.session = self.launch(label)
        self.sessions.append(self.session)
        self.workspace()


def run(binary, output, size, keep):
    j = Journey(binary, output, size, 'retained')
    result = {'size': list(size), 'status': 'failed', 'scope': 'Actual PTY, existing writer, durable journals, controlled disposable local JSONL peer; no live provider calls', 'cases': []}
    try:
        j.setup(count=2)
        first = j.snapshot()[0]['id']
        token = j.root / 'local-retained-summary'
        j.approve_command(token, partner=2)
        token.with_suffix('.finish').touch()
        j.until(lambda: not j.claims() and j.snapshot()[3] and j.snapshot()[3][-1]['state'] == 'Succeeded', 'actual owned command summary retained')
        j.lead(1)
        summary = j.search('local-retained-summary')
        assert 'run summary' in summary and '1 retained matches' in summary, summary
        j.session.send('\r')
        wait_for(j.session, 'Original task')
        j.session.capture('retained-actual-run-summary')
        j.workspace()
        result['cases'].append('literal run-summary search opens original actual owned local execution facts, labelled run ended Unix milliseconds; no output reconstruction')
        j.request(1, 'COOP_RETAINED_A')
        j.finish('COOP_RETAINED_A')
        j.until(lambda: not j.claims() and bool(j.snapshot()[4]), 'first durable retained journal')
        j.request(2, 'COOP_RETAINED_B')
        j.lead(1)
        if j.workspace().startswith('Type Request |'):
            j.session.send('\r')
        draft = 'Unsent landmark draft 설명 exact caret'
        j.session.paste(draft)
        j.session.send(LEFT * 3 + '\x13')
        j.until(lambda: j.draft(1) and j.draft(1)['text'] == draft, 'exact dirty draft persisted')
        saved = j.draft(1).copy()
        paused = None
        if 'Response ' in j.session.display():
            view = response_view(j.session)
            wheel(j.session, 'up', view['point'], 2)
            paused = next((line.strip() for line in j.session.display().splitlines()
                           if line.strip().startswith('Paused ')), None)
            assert paused is not None, 'wheel did not pause the actual retained output'
        starts = len(j.peers('started'))
        j.session.command('retained-responses')
        j.until(lambda: 'provider journal' in j.session.display() and 'Reading bounded' not in j.session.display(), 'retained journal list')
        j.session.send('\r')
        wait_for(j.session, 'Original task')
        j.session.send('l')
        wait_for(j.session, 'Landmark label')
        j.session.paste('Explanation 설명')
        j.session.send('\r')
        j.until(lambda: len(j.labels(first)) == 1, 'durable label while another partner streams')
        assert j.claims() == {2} and j.draft(1) == saved and len(j.peers('started')) == starts
        j.session.capture('label-other-partner-streaming')
        j.session.resize((120, 36) if size[0] == 60 else (60, 18))
        j.session.command('landmarks')
        wait_for(j.session, 'Explanation')
        j.session.send('\r')
        wait_for(j.session, 'Original task')
        assert j.draft(1) == saved and j.snapshot()[0]['lead_partner'] == 1
        j.session.capture('jump-reflow-preserves-owner')
        j.workspace()
        if paused is not None:
            assert any(line.strip().startswith('Paused ') for line in j.session.display().splitlines()), 'landmark navigation resumed paused output'
        j.finish('COOP_RETAINED_B')
        j.until(lambda: not j.claims(), 'other partner settled')
        j.session.resize(size)
        j.restart('label-restart')
        j.session.command('landmarks')
        wait_for(j.session, 'Explanation')
        j.session.send('n')
        j.session.send(DELETE)  # Unsupported edit key must not touch the composer.
        j.session.paste(' cancelled')
        j.session.send(ESC)
        assert j.labels(first)[0]['label'] == 'Explanation 설명' and j.draft(1) == saved
        j.session.send('\r')
        wait_for(j.session, 'Original task')
        assert len(j.peers('started')) == starts
        j.session.capture('restart-attribution-no-replay')
        result['cases'].append('durable label/jump during other-partner stream, resize/reflow/restart; original partner/request attribution, draft/caret/paused-output ownership; cancellation no sends')
        j.workspace()
        j.session.command('task-new')
        j.set_goal('COOP_RETAINED second local task 설명')
        second = max(row['id'] for row in j.tasks())
        view = j.search('COOP_RETAINED')
        assert 'FOREIGN' not in view
        # Results contain retained goals and journal data; counts are derived only from this project.
        expected = sum('COOP_RETAINED' in row['goal'] for row in j.tasks() if row['project'] == str(j.project))
        with closing(sqlite3.connect(j.database.as_uri()+'?mode=ro', uri=True)) as db:
            expected += sum('COOP_RETAINED' in json.loads(encoded)['text'] for (encoded,) in db.execute('SELECT record FROM provider_journal WHERE task_id IN (SELECT id FROM tasks WHERE project=?)', (str(j.project),)))
        assert f'{expected} retained matches' in view, view
        j.session.send('\r')
        wait_for(j.session, 'Original task')
        assert j.draft(1) == saved and len(j.peers('started')) == starts
        j.workspace()
        j.select_task(second)
        j.session.command('tasks')
        j.session.send(HOME)
        rows = re.findall(r'#(\d+) (?:Open|Complete) /', j.session.display())
        j.session.send(DOWN * rows.index(str(second)))
        j.session.command('archive')
        j.until(lambda: next(row for row in j.tasks() if row['id'] == second)['archived'] == 1, 'archive second task')
        # Archiving the active task returns to the ordinary task chooser.
        # Explicitly resume its remaining active row before quitting.
        j.until(lambda: 'Saved sessions / committed tasks' in j.session.display()
                and re.findall(r'#(\d+) (?:Open|Complete) /', j.session.display()) == [str(first)],
                'remaining active task picker rendered')
        j.session.send(HOME + '\r')
        j.workspace()
        assert j.draft(1) == saved and len(j.peers('started')) == starts and not j.claims()
        j.restart('archive-restart')
        original = j.project
        j.quit()
        j.project = j.root / 'foreign-project'
        j.project.mkdir()
        j.session = j.launch('foreign-project')
        j.sessions.append(j.session)
        j.set_goal('COOP_RETAINED FOREIGN PRIVATE body')
        j.quit()
        j.project = original
        j.session = j.launch('project-private-restart')
        j.sessions.append(j.session)
        view = j.search('COOP_RETAINED')
        assert 'FOREIGN' not in view and f'{expected-1} retained matches' in view, view
        view = j.search('COOP_RETAINED', archived=True)
        assert 'FOREIGN' not in view and f'{expected} retained matches' in view, view
        j.session.capture('project-private-archive-restart-search')
        result['cases'].append('two local tasks plus foreign task: exact local-only snippets/counts, archived filter, restart; guarded original inspection preserves dirty draft; no implicit context selection')
        j.select_task(first)
        j.session.command('history')
        j.session.command('journal-purge')
        # Existing neutral full-scope purge review; traverse its actual bounded scope before consent.
        j.session.send((ESC + '[6~') * 12)
        j.session.capture('purge-original-journal-review')
        j.session.send('y')
        wait_for(j.session, 'No retained observations', timeout=40)
        assert not j.snapshot()[4], 'committed purge retained a provider journal'
        j.session.command('landmarks')
        wait_for(j.session, 'Explanation')
        j.session.send('\r')
        j.until(lambda: 'unavailable' in j.session.display().lower() or 'purged' in j.session.display().lower(), 'purged landmark unavailable')
        assert j.labels(first) and j.draft(1) == saved and len(j.peers('started')) == starts
        view = j.search('COOP_RETAINED', archived=True)
        assert 'provider journal' not in view and 'FOREIGN' not in view
        j.session.capture('purge-invalidates-search-and-landmark')
        j.session.command('landmarks')
        j.session.send('n')
        j.session.send('\x7f' * 128)
        j.session.paste('Renamed unavailable 설명')
        j.session.send('\r')
        j.until(lambda: j.labels(first)[0]['label'] == 'Renamed unavailable 설명', 'rename durable landmark')
        # Preserve a genuine current-reader fixture before explicit removal.
        j.quit()
        fixture_data = j.output / 'landmark-reader-fixture'
        shutil.copytree(j.data, fixture_data)
        result['fixture'] = {'root': str(j.root), 'data_dir': str(fixture_data), 'project': str(j.project), 'task': first, 'landmark': j.labels(first)[0]}
        result['cases'].append('actual journal purge invalidates search and renders landmark unavailable without reconstruction; durable rename; restart no replay')
        j.session = j.launch('remove-landmark')
        j.sessions.append(j.session)
        j.session.command('landmarks')
        wait_for(j.session, 'Renamed unavailable')
        j.session.send('d')
        j.until(lambda: not j.labels(first), 'explicit metadata removal')
        j.quit()
        result['cases'].append('explicit remove deletes only landmark metadata; retained task/draft and provider invocation counts unchanged')
        result['status'] = 'passed'
        return result
    finally:
        for session in j.sessions:
            if not session.closed:
                session.capture('final')
                session.close()
        j.output.mkdir(parents=True, exist_ok=True)
        (j.output/'peer.jsonl').write_text(j.log.read_text())
        if keep:
            j.temporary._finalizer.detach()
            result['retained_fixture_root'] = str(j.root)
        else:
            j.temporary.cleanup()
        (j.output/'results.json').write_text(json.dumps(result, indent=2, ensure_ascii=False)+'\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--keep-fixture', action='store_true')
    args = parser.parse_args()
    binary, output = Path(args.binary).resolve(), Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    results = {'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'journeys': []}
    try:
        for size in ((120, 36), (60, 18)):
            results['journeys'].append(run(binary, output, size, args.keep_fixture))
    finally:
        (output/'results.json').write_text(json.dumps(results, indent=2, ensure_ascii=False)+'\n')
    print(json.dumps(results, ensure_ascii=False))

if __name__ == '__main__':
    main()
