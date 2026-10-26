#!/usr/bin/env python3
"""Opt-in PK07 acceptance against an already-built real application.

Uses the existing Records journey and genuine approved local executions. SQLite
is read-only observation, never a fixture writer. No model request, paid service,
credential, build, or alternate executor is used. Runs compact or
wide terminals as selected by the caller; retains actual screen/store evidence.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import json
import platform
import tempfile
import traceback

from records_smoke import Journey, BAD
from moves_smoke import save, digest
from native_connection_smoke import Peer
from workbench_smoke import wait_for
from workflow_smoke import UP, DOWN, RIGHT, PAGE, ESC


def qualify(journey):
    j = journey
    # The broad Records fixture also contains a source-boundary symlink case;
    # this targeted journey does not exercise it or silently follow that link.
    (j.project / 'outside.py').unlink()
    j.session.command('diff')
    wait_for(j.session, 'Repository')
    j.session.command('refresh')
    wait_for(j.session, 'Repository: refreshed')
    j.workspace()
    j.write_request('Retained unsent PK07 Request 漢字')
    # Existing flow records two distinct commands with one exact signature,
    # explicitly links supported diagnosis to the matching pass, then fails the
    # same argv/cwd again. Its screen assertions distinguish all three states.
    j.case('failure', j.failures)
    j.case('resolution', j.resolution)
    j.current_case = 'PK07'
    before_runs = len(j.state()['executions'])
    before_invocations = j.invocations()
    before_posts = j.peer.snapshot('POST')
    j.source.write_text(BAD + '# distinct source after recurrence\n', encoding='utf-8')
    j.session.command('workflows')
    j.wait_caption('Specialist desk', )
    j.session.command('refresh')
    latest = j.state()['executions'][-1]['record']['prepared']
    stale_id = 'stale:run:{}:{}'.format(latest['task_id'], latest['run_id'])
    j.until(lambda: any(row['id'] == stale_id for row in j.records('local_activity')),
            'fresh source worker invalidated the recurring failure observation')
    j.workspace()
    j.open_records('dex')
    text = j.pages_text('pk07-source-changed-never-current')
    compact = ''.join(text.split())
    assert 'stale' in text.lower(), text
    assert j.pass_id in compact and j.support['id'] in compact, text
    j.workspace()

    # Both real owners retain nonempty Request bytes/carets during neutral
    # recipient navigation and cancellation. The ordinary recipient guard also
    # refuses affirmative replacement; no send or run can be inferred.
    j.session.command('party')
    j.session.send('a')
    j.until(lambda: len(j.state()['partners']) == 2, 'second actual partner committed')
    j.workspace()
    recipient = next(row['id'] for row in j.state()['partners'] if row['id'] != j.owner)
    j.lead(recipient)
    j.form('umbreon')
    j.write_request('recipient retained 한글 draft')
    j.lead(j.owner)
    j.write_request('sender retained 漢字 draft')
    drafts = j.task()['partner_drafts']
    j.open_records('dex')
    j.session.send('r')
    wait_for(j.session, 'Send to')
    j.session.send(DOWN + '\r')
    j.capture('pk07-neutral-recipient-review')
    j.session.send('\r')  # neutral review returns to recipients, never stages
    assert j.task()['partner_drafts'] == drafts
    j.session.send(ESC)
    j.workspace()
    assert j.task()['partner_drafts'] == drafts
    j.open_records('dex')
    j.session.send('r')
    wait_for(j.session, 'Send to')
    j.session.send(DOWN + '\r' + PAGE * 24 + 'y')
    wait_for(j.session, 'nonempty')
    assert j.task()['partner_drafts'] == drafts
    j.capture('pk07-nonempty-recipient-refused')
    j.session.send(ESC)
    j.workspace()

    # Explicitly clear only recipient Request, then stage references through the
    # same full review. Sender stays lead with its exact retained draft/caret.
    j.lead(recipient)
    j.write_request('')
    j.lead(j.owner)
    sender = j.task()['partner_drafts'][str(j.owner)]
    j.open_records('dex')
    j.session.send('r')
    wait_for(j.session, 'Send to')
    j.session.send(DOWN + '\r' + PAGE * 24 + 'y')
    def staged():
        owner = j.task()['partner_drafts'].get(str(recipient), {})
        return any('Historical/stale' in draft['text'] and j.signature in draft['text']
                   for draft in owner.get('drafts', []) if draft['field'] == 'Request')
    j.until(staged, 'reviewed evidence references staged only in recipient Request')
    assert j.task()['partner_drafts'][str(j.owner)] == sender
    assert j.task()['lead_partner'] == j.owner
    j.capture('pk07-reviewed-reference-only-relay')
    j.workspace()

    # Select the actual cited pass through the existing execution inspector and
    # exercise neutral and affirmative output purge. No SQL effects are seeded.
    j.session.command('run')
    j.session.command('runs')
    rows = [row for row in j.state()['executions'] if row['task_id'] == j.record_task]
    ordered = list(reversed(rows))  # actual store history is descending rowid
    index = next(index for index, row in enumerate(ordered)
                 if row['record']['prepared']['run_id'] == j.pass_id)
    j.session.send(UP * (len(rows) + 1) + DOWN * index + '\r')
    wait_for(j.session, 'test records::total ... ok')
    j.session.command('purge')
    j.wait_caption('Purge retained local output', )
    j.session.send('\r')
    assert not next(row['record'] for row in j.state()['executions']
                    if row['record']['prepared']['run_id'] == j.pass_id)['output_unavailable']
    j.session.command('purge')
    j.session.send(PAGE * 24 + RIGHT + '\r')
    j.until(lambda: next(row['record'] for row in j.state()['executions']
                        if row['record']['prepared']['run_id'] == j.pass_id)['output_unavailable'],
            'actual referenced passing output purged through ordinary writer')
    assert any(row['passing_run'] == j.pass_id for row in j.records('failure_resolutions'))
    j.workspace()
    j.open_records('dex')
    text = j.pages_text('pk07-purged-pass-honestly-unavailable')
    compact = ''.join(text.split())
    assert j.pass_id in compact
    assert 'purged/missing/replaced' in compact, text
    assert 'Unresolvedrecurringfailure' in compact, text
    assert len(j.state()['executions']) == before_runs
    assert j.invocations() == before_invocations
    assert j.peer.snapshot('POST') == before_posts == []
    j.workspace()
    j.restart()
    j.open_records('dex')
    text = j.pages_text('pk07-restart-keeps-historical-unavailable')
    assert j.pass_id in ''.join(text.split())
    assert len(j.state()['executions']) == before_runs and j.invocations() == before_invocations
    j.workspace()
    j.results['PK07']['status'] = 'passed'
    j.current_case = None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--compact', action='store_true')
    args = parser.parse_args()
    binary = args.binary.resolve()
    assert binary.is_file(), 'supply an already-built real binary'
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    identity = digest(binary)
    results = {key: {'status': 'not-run', 'evidence': []}
               for key in ('failure', 'resolution', 'PK07')}
    report = {'status': 'failed', 'binary': str(binary), 'sha256': identity,
              'host': platform.platform(), 'size': [60, 18] if args.compact else [120, 36], 'cases': results,
              'scope': 'Real app + local commands + reviewed relay/purge; no provider requests'}
    journey = None
    try:
        with tempfile.TemporaryDirectory(prefix='ev-pk07-') as temporary:
            with Peer('chat_completions', 'unused-keyless-public-fixture', True) as peer:
                try:
                    journey = Journey.__new__(Journey)
                    journey.__init__(binary, output / 'journey', Path(temporary).resolve(),
                                     tuple(report['size']), args.compact, results, peer, )
                    qualify(journey)
                    journey.quit()
                    peer.assert_clean()
                    assert peer.snapshot('POST') == []
                    report['status'] = 'passed'
                finally:
                    if journey:
                        journey.close()
        assert digest(binary) == identity, 'supplied binary changed during acceptance'
    except Exception as error:
        report.update(status='failed', error=str(error), traceback=traceback.format_exc())
    finally:
        save(output / 'results.json', report)
    print(json.dumps({'status': report['status'], 'output': str(output)}))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
