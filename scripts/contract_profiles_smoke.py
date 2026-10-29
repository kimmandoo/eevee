#!/usr/bin/env python3
"""XC06 credential-free actual-binary profile journeys, including reviewed external edits.
No live provider/account calls; the loopback peer supplies deterministic responses.
"""
from __future__ import annotations
import smoke_sizes
import argparse
from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import sys
import tempfile
import traceback
from native_connection_smoke import Journey, Peer, until, decoded_payload, ESC
from external_editor_smoke import expose, approve
from workbench_smoke import wait_for
from terminal_smoke import confirm_exit

TEXT = 'Explain the current context.'
EDITED = 'Inspect exact changed bytes first. Report observed results, including 한글 boundaries; preferred checks are suggestions only.'
FORMS = ('Eevee', 'Vaporeon', 'Jolteon', 'Flareon', 'Espeon', 'Umbreon', 'Leafeon', 'Glaceon', 'Sylveon')


def context(protocol, body):
    policy, _ = decoded_payload(protocol, body)
    return json.loads(policy.split('UNTRUSTED CURRENT TASK/SOURCE DATA (JSON):\n', 1)[1])


def receipt(journey):
    with closing(sqlite3.connect(journey.data / 'workbench.sqlite3', timeout=10)) as db:
        row = db.execute('SELECT record FROM outgoing_receipts ORDER BY rowid DESC LIMIT 1').fetchone()
        return json.loads(row[0]) if row else None


def run(binary, output, size, route, protocol, edit):
    path = output / f'{size[0]}x{size[1]}-{protocol}'
    path.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='ev-profile-') as temporary:
        root = Path(temporary)
        editor = root / 'editor.py'
        candidate = 'schema_version = 1\ninstructions = ' + json.dumps(EDITED, ensure_ascii=False) + '\npreferred_checks = [["cargo", "test", "--lib"]]\n'
        editor.write_text('import sys\nfrom pathlib import Path\nPath(sys.argv[-1]).write_text(' + repr(candidate) + ', encoding="utf-8")\n')
        original_visual = os.environ.get('VISUAL')
        os.environ['VISUAL'] = json.dumps([sys.executable, str(editor)])
        journey = None
        try:
            keyless = route == 'compatible'
            with Peer(protocol, 'sk-fixture-profile-2a73dd491c', keyless) as peer:
                journey = Journey(binary, path, root, size, size[0] < 90, route, protocol, keyless, False, peer, controlling_tty=True)
                session = journey.session
                journey.setup(secret=None if keyless else peer.secret, initial=True)
                history = journey.send(TEXT, [], 'Observed profile baseline.', 'default-profile')
                packet = context(protocol, peer.snapshot('POST')[-1]['body'])['behavior_profile']
                assert packet['instructions'] and packet['schema_version'] == 1
                assert not any(name.lower() in json.dumps(packet).lower() for name in FORMS)
                expected = len(packet['instructions'].encode())
                until(session, lambda: receipt(journey).get('profile_instruction_bytes') == expected,
                      'exact durable neutral instruction byte accounting')
                journey.capture('profile-chip')
                session.command('form-profile off')
                history = journey.send(TEXT, history, 'Profile explicitly disabled.', 'profile-off')
                assert context(protocol, peer.snapshot('POST')[-1]['body']).get('behavior_profile') is None
                until(session, lambda: receipt(journey).get('profile_instruction_bytes') is None, 'off receipt has no profile bytes')
                session.command('form-profile on')
                if edit:
                    journey.write_request('Unsent draft 한글 preserved')
                    session.command('form-profile edit')
                    wait_for(session, 'launch review', timeout=30)
                    expose(session); journey.capture('profile-editor-launch-review'); approve(session)
                    wait_for(session, 'editor — returned', timeout=30)
                    expose(session); journey.capture('profile-editor-returned-review'); approve(session)
                    until(session, lambda: (journey.data / 'form-profiles.json').exists(), 'reviewed profile saved')
                    with closing(sqlite3.connect(journey.data / 'workbench.sqlite3', timeout=10)) as db:
                        drafts = [json.loads(row[0]) for row in db.execute('SELECT partner_drafts FROM tasks')]
                    assert 'Unsent draft 한글 preserved' in json.dumps(drafts, ensure_ascii=False)
                    history = journey.send(TEXT, history, 'Reviewed profile applied.', 'profile-edited')
                    packet = context(protocol, peer.snapshot('POST')[-1]['body'])['behavior_profile']
                    assert packet['instructions'] == EDITED and packet['preferred_checks'] == [['cargo', 'test', '--lib']]
                    until(session, lambda: receipt(journey).get('profile_instruction_bytes') == len(EDITED.encode()), 'edited profile exact UTF-8 receipt bytes')
                    assert not journey.snapshot()['executions'], 'suggested checks executed implicitly'
                    session.command('form-profile skip')
                    history = journey.send(TEXT, history, 'Profile omitted for one Send.', 'profile-skip')
                    assert context(protocol, peer.snapshot('POST')[-1]['body']).get('behavior_profile') is None
                    history = journey.send(TEXT, history, 'Profile restored after one Send.', 'profile-after-skip')
                    assert context(protocol, peer.snapshot('POST')[-1]['body'])['behavior_profile']['instructions'] == EDITED
                peer.assert_clean()
                journey.workspace(); session.command('quit'); confirm_exit(session)
                assert session.wait_exit() == 0
                (path / 'requests.json').write_text(json.dumps(peer.snapshot(), indent=2, ensure_ascii=False))
                return {'size': size, 'protocol': protocol, 'status': 'passed', 'default_instruction_bytes': expected, 'reviewed_edit': edit}
        finally:
            if journey is not None:
                try:
                    journey.capture('final')
                finally:
                    journey.close()
            if original_visual is None: os.environ.pop('VISUAL', None)
            else: os.environ['VISUAL'] = original_visual


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    binary, output = Path(args.binary).resolve(), Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {'status': 'failed', 'cases': [], 'limits': ['Loopback fixture behavior, not a paired quality benchmark or live provider conformance.']}
    try:
        for size in smoke_sizes.selected(((120, 36), (60, 18))):
            for route, protocol in (('compatible', 'responses'), ('compatible', 'chat_completions'), ('anthropic', 'messages'), ('google', 'gemini')):
                result['cases'].append(run(binary, output, size, route, protocol, protocol == 'responses'))
                print('passed', result['cases'][-1], flush=True)
        result['status'] = 'passed'
    except BaseException:
        result['failure'] = traceback.format_exc()
        raise
    finally:
        (output / 'results.json').write_text(json.dumps(result, indent=2, ensure_ascii=False) + '\n')

if __name__ == '__main__': main()
