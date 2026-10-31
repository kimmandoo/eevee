#!/usr/bin/env python3
"""Opt-in PK09 actual PTY acceptance. Requires an already-built binary.

Creates real captures, approved local commands, committed review generations,
renames/releases and explicit cross-task navigation through the TUI. SQLite is
read-only observation, never fixture seeding. No provider request or paid call.
Fixtures and all actual PTY frames remain under --output for integration proof.
"""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import re
import traceback

from records_smoke import Journey, Peer, save, digest, wait_for, FORMS
from workflow_smoke import ESC, DOWN, LEFT, RIGHT, HOME, PAGE


def normalized(text):
    return ''.join(text.split())


class ContributionsJourney(Journey):
    def open_contributions(self, partner, context='partner-summary'):
        self.open_records(context)
        roster = self.state()['partners']
        index = next(i for i, row in enumerate(roster) if row['id'] == partner)
        self.session.send(DOWN * index)
        self.session.send('c')
        wait_for(self.session, 'Task/form contributions')
        wait_for(self.session, 'p Producer')

    def inspect_all(self, partner, label, expected=None):
        if expected is None:
            expected = [row for row in self.records('local_artifacts')
                        if row['producer'] and row['producer']['partner'] == partner]
        assert expected, 'actual retained producer fixture is empty'
        before = self.effects()
        for index, artifact in enumerate(expected):
            if index:
                self.session.send(DOWN)
            text = self.pages_text(label + '-item-' + str(index))
            flat = normalized(text)
            assert normalized(artifact['id']) in flat, (artifact, text)
            assert normalized(artifact['producer']['name']) in flat, (artifact, text)
            assert FORMS[artifact['producer']['form']] in text, text
            assert 'No visit foreign key' in text or 'Novisitforeignkey' in flat, text
            assert 'notroute-visitoutputs' in flat, text
        assert self.effects() == before, 'contribution inspection performed work'
        return expected

    def review(self, label):
        self.form('glaceon')
        before = {row['id'] for row in self.records('local_artifacts')}
        self.session.command('workflows')
        wait_for(self.session, 'Specialist desk')
        self.workflow_edit(1, 'Finding detail', label + ' unverified public failure 한글')
        self.workflow_edit(2, 'Actionable recommendation', 'Inspect actual retained support; do not infer correctness.')
        self.workflow_menu(4)
        self.workflow_menu(11)
        wait_for(self.session, 'RequestChanges')
        self.until(lambda: any(row['kind'] == 'Review' and row['id'] not in before
                               for row in self.records('local_artifacts')), 'new actual review generation')
        self.workspace()

    def saved_draft(self, task, partner):
        row = next(row for row in self.state()['tasks'] if row['id'] == task)
        return row['partner_drafts'].get(str(partner))

    def choose_task(self, task):
        revision = self.task()['revision']
        self.session.command('tasks')
        wait_for(self.session, 'Saved sessions / committed tasks')
        rows = re.findall(r'([ >])([ *]) #(\d+) (?:Open|Complete) /', self.session.display())
        index = next(index for index, row in enumerate(rows) if int(row[2]) == task)
        self.session.send(HOME + DOWN * index)
        self.until(lambda: self.task()['revision'] > revision and
                   'Storage: saved' in self.session.display(),
                   'task-picker navigation did not settle before explicit selection')
        self.session.send('\r')
        self.until(lambda: self.task()['id'] == task, 'explicit ordinary task selection')
        self.workspace()


def run(binary, output, size):
    report = {'status': 'failed', 'size': list(size), 'scope': 'Actual PTY/local commands and store; no provider calls'}
    journey = None
    try:
        with Peer('chat_completions', 'unused-keyless-public-fixture', True, retain_body_bytes=True) as peer:
            journey = j = ContributionsJourney(binary, output / 'journey', output / 'fixture', size,
                                                size[0] == 60, {}, peer)
            first = j.owner
            j.nickname('Bolt')
            j.form('vaporeon')
            first_capture = j.pin_range('資料.txt')
            j.execute_check('partner-one', 'Failed')
            j.review('First partner')
            j.form('flareon')  # Explicit form-only visit: no retained output.
            before_visit = j.records('local_artifacts')
            j.form('eevee')
            assert j.records('local_artifacts') == before_visit, 'form-only visit manufactured evidence'
            j.session.command('party')
            wait_for(j.session, 'Party roster')
            j.session.send('a')
            j.until(lambda: len(j.state()['partners']) == 2, 'second actual partner committed')
            j.workspace()
            second = next(row['id'] for row in j.state()['partners'] if row['id'] != first)
            j.lead(second)
            j.nickname('Mizu 한글')
            j.form('vaporeon')
            second_capture = j.pin_range('beta.txt')
            j.execute_check('partner-two', 'Failed')
            j.review('Second partner')
            before_visit = j.records('local_artifacts')
            j.form('flareon')
            j.form('glaceon')
            assert j.records('local_artifacts') == before_visit, 'second form-only visit manufactured evidence'
            originals = j.records('local_artifacts')
            assert {row['producer']['partner'] for row in originals if row['producer']} == {first, second}
            assert {'Capture', 'Run', 'Review'} <= {row['kind'] for row in originals}
            j.open_contributions(first)
            j.inspect_all(first, 'first-partner')
            j.session.send(ESC)
            wait_for(j.session, 'Partner summary')
            j.workspace()
            j.open_contributions(second, 'routes')
            j.inspect_all(second, 'route-context-second-partner')
            # Cycle to Flareon: all/0/1/2/3. This visit never has evidence.
            j.session.send('f' * 4)
            wait_for(j.session, 'No retained contributions')
            j.capture('form-only-no-evidence')
            j.session.send(ESC)
            wait_for(j.session, 'Form routes')
            j.workspace()
            j.nickname('Renamed')
            j.open_contributions(second)
            text = j.pages_text('renamed-frozen-attribution')
            assert 'Mizu' in text and 'Renamed' in text, text
            j.workspace()
            # Exact source/run inspectors return to the same filtered list.
            j.open_contributions(first)
            first_items = [row for row in j.records('local_artifacts') if row['producer'] and row['producer']['partner'] == first]
            capture_index = next(i for i, row in enumerate(first_items) if row['kind'] == 'Capture')
            j.session.send(DOWN * capture_index + '\r')
            wait_for(j.session, 'Sources /')
            j.capture('existing-exact-source-inspector')
            j.session.send(ESC)
            wait_for(j.session, 'Task/form contributions')
            j.workspace()
            j.open_contributions(first)
            run_index = next(i for i, row in enumerate(first_items) if row['kind'] == 'Run')
            j.session.send(DOWN * run_index + '\r')
            wait_for(j.session, 'Local command evidence')
            j.capture('existing-exact-run-inspector')
            j.session.send(ESC)
            wait_for(j.session, 'Task/form contributions')
            j.workspace()
            j.open_contributions(second)
            second_items = [row for row in j.records('local_artifacts') if row['producer'] and row['producer']['partner'] == second]
            review_index = next(i for i, row in enumerate(second_items) if row['kind'] == 'Review')
            j.session.send(DOWN * review_index + '\r')
            wait_for(j.session, 'Specialist desk')
            j.capture('existing-current-generation-review-inspector')
            j.session.send(ESC)
            wait_for(j.session, 'Task/form contributions')
            j.workspace()
            old_task = j.task()['id']
            j.session.command('task-new')
            j.session.command('goal')
            j.session.paste('Other retained task 한글')
            j.session.send('\r')
            j.until(lambda: len(j.state()['tasks']) == 2, 'second retained task committed')
            current_task = j.task()['id']
            owner = j.task()['lead_partner']
            draft = 'Exact other-task Request 한글 é suffix'
            j.write_request(draft)
            j.session.send(LEFT * 3 + '\x13')
            j.until(lambda: any(row['field'] == 'Request' and row['text'] == draft
                               for row in (j.saved_draft(current_task, owner) or {}).get('drafts', [])),
                    'exact current owner draft saved')
            saved = j.saved_draft(current_task, owner)
            j.open_contributions(first)
            before = j.effects()
            j.session.send('g')
            wait_for(j.session, 'Saved sessions / committed tasks')
            assert j.task()['id'] == current_task and j.task()['lead_partner'] == owner
            j.capture('cross-task-highlight-only')
            j.session.send(ESC)
            wait_for(j.session, 'Task/form contributions')
            assert j.saved_draft(current_task, owner) == saved
            assert j.effects() == before
            j.session.send('g\r')
            j.until(lambda: j.task()['id'] == old_task, 'second deliberate Enter selects retained task')
            j.workspace()
            assert j.saved_draft(current_task, owner) == saved, 'cross-task selection lost owned draft/caret'
            j.lead(first)
            j.session.command('release-partner ' + str(second))
            wait_for(j.session, 'Release')
            j.capture('release-neutral')
            j.session.send('\r')
            assert len(j.state()['partners']) == 2, 'neutral Enter released producer'
            j.session.command('release-partner ' + str(second))
            wait_for(j.session, 'Release')
            j.session.send('y')
            j.until(lambda: all(row['id'] != second for row in j.state()['partners']), 'explicit producer release')
            j.workspace()
            j.open_contributions(first)
            j.session.send('p')  # retained released producer follows first ID
            text = j.pages_text('released-producer-historical-identity')
            assert 'Mizu' in text and 'Producerreleased' in normalized(text), text
            before_unknown = j.records('local_artifacts')
            j.session.send('p')
            wait_for(j.session, 'partner unknown')
            j.capture('legacy-unknown-scope-never-assigned-to-lead')
            assert j.records('local_artifacts') == before_unknown
            j.workspace()
            # Purge actual capture through existing guarded source review.
            j.session.command('basket')
            wait_for(j.session, 'Sources /')
            basket = j.state()['excerpts']
            index = next(i for i, row in enumerate(basket) if row['record']['id'] == second_capture['id'])
            j.session.send(HOME + DOWN * index)
            j.session.command('purge')
            wait_for(j.session, 'Review local excerpt removal')
            j.capture('source-purge-neutral')
            j.session.send('\r')
            assert any(row['reference'] == str(second_capture['id']) and row['kind'] == 'Capture' for row in j.records('local_artifacts'))
            j.session.command('purge')
            j.session.send(PAGE * 16 + RIGHT + '\r')
            j.until(lambda: not any(row['reference'] == str(second_capture['id']) and row['kind'] == 'Capture'
                                   for row in j.records('local_artifacts')), 'purged capture contribution removed')
            j.workspace()
            j.open_contributions(first)
            j.session.send('p')
            retained = [row for row in j.records('local_artifacts') if row['producer'] and row['producer']['partner'] == second]
            j.inspect_all(second, 'released-after-purge', retained)
            j.workspace()
            j.choose_task(current_task)
            assert j.saved_draft(current_task, owner) == saved
            j.restart()
            assert j.saved_draft(current_task, owner) == saved
            assert not peer.snapshot(), 'inspection unexpectedly made network calls'
            j.capture('restart-exact-draft-no-replay')
            report.update(status='passed', project=str(j.project), data_dir=str(j.data),
                          producers=[first, second], captures=[first_capture['id'], second_capture['id']],
                          artifacts_before_release=originals, preserved_draft=saved,
                          effects=j.effects(), network=peer.snapshot())
            j.quit()
            peer.assert_clean()
    except Exception:
        report['error'] = traceback.format_exc()
        if journey and journey.session:
            journey.capture('failure')
    finally:
        if journey:
            for session in journey.sessions:
                session.close()
        output.mkdir(parents=True, exist_ok=True)
        save(output / 'result.json', report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True, help='new retained directory; never overwritten')
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    before = digest(binary)
    results = [run(binary, output / ('%dx%d' % size), size) for size in ((60, 18), (120, 36))]
    assert digest(binary) == before, 'supplied binary changed'
    report = {'status': 'passed' if all(row['status'] == 'passed' for row in results) else 'failed',
              'binary_sha256': before, 'journeys': results}
    save(output / 'report.json', report)
    print(json.dumps(report, ensure_ascii=False, indent=2))
    raise SystemExit(0 if report['status'] == 'passed' else 1)


if __name__ == '__main__':
    main()
