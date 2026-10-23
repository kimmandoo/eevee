#!/usr/bin/env python3
"""Exercise fresh/public conversation boundaries through disposable real PTYs.

An already-built ev talks only to synthetic loopback HTTP and controlled JSONL
peers. No live account, credential store, installed provider or SQL write is used.
The controlled cases also perform real exact-consent file writes in disposable
projects. This proves Eevee transport/UI behavior, not live-model conformance.
"""
import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import traceback

from file_creation_smoke import codex_case
from native_connection_smoke import Peer, decoded_payload, until
from response_smoke import SurfaceJourney
from terminal_smoke import confirm_exit
from workbench_smoke import wait_for

ESC = '\x1b'
GOAL = 'Stable public conversation fixture goal, not an earlier request.'


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def native_case(binary, output, size, language):
    output.mkdir(parents=True, exist_ok=True)
    result = {'status': 'failed', 'route': 'native-compatible', 'language': language,
              'size': list(size), 'cases': [], 'payload_assertions': []}
    journey = None
    with tempfile.TemporaryDirectory(prefix='ev-conversation-native-') as temporary:
        with Peer('chat_completions', 'unused-public-keyless-fixture', True) as peer:
            try:
                journey = SurfaceJourney(binary, output, Path(temporary), size, size[0] == 60,
                                         'compatible', 'chat_completions', True, False, peer,
                                         language=language)
                session = journey.session
                wait_for(session, 'Storage: saved')
                session.command('goal')
                session.paste(GOAL)
                session.send('\r')
                until(session, lambda: any(task['goal'] == GOAL for task in journey.snapshot()['tasks']),
                      'stable task goal saved before conversation')
                journey.workspace()
                journey.setup(initial=True)
                session.command('role-plan')
                journey.workspace()

                def send(text, history, answer, label, command=None):
                    journey.write_request(text)
                    before = len(peer.snapshot('POST'))
                    peer.plans.put(('complete', answer))
                    if command is None:
                        session.send('\r')
                    else:
                        session.command(command)
                    until(session, lambda: len(peer.snapshot('POST')) == before + 1,
                          label + ': exactly one actual native POST')
                    row = peer.snapshot('POST')[-1]
                    _, actual = decoded_payload('chat_completions', row['body'])
                    expected = history + [('user', text)]
                    assert actual == expected, (label, actual, expected)
                    wait_for(session, answer)
                    until(session, lambda: not journey.snapshot()['claims']
                          and journey.surface['complete'] in session.display(),
                          label + ': completed response and owned settlement')
                    session.pump(.3)
                    assert len(peer.snapshot('POST')) == before + 1, label + ': hidden replay'
                    assert journey.snapshot()['executions'] == 0, 'conversation fabricated execution evidence'
                    journey.capture(label)
                    result['payload_assertions'].append({'case': label, 'roles_and_exact_content': actual})
                    result['cases'].append(label)
                    return history + [('user', text), ('assistant', answer)]

                history = send('A_PUBLIC_REQUEST_713', [], 'A_PUBLIC_ANSWER_829', 'ordinary-first-turn')
                # The advanced /session-new action freezes the same explicit
                # fresh boundary as /new, but deliberately does not dispatch it.
                journey.write_request('UNSENT_FRESH_PREPARATION_947')
                before = len(peer.snapshot('POST'))
                session.command('session-new')
                def prepared_review():
                    view = session.display()
                    if any(label in view for label in (
                            'Enter to review (not approve)', 'Enter 검토 (승인 아님)',
                            '· y Run / n Cancel', '· y 실행 / n 취소')):
                        return True
                    if 'Alt+A' in view:
                        session.send(ESC + 'a')
                    return False
                until(session, prepared_review,
                      'fresh unsent native preparation reached real permission review')
                journey.capture('fresh-preparation-before-neutral-cancel')
                assert len(peer.snapshot('POST')) == before, 'preparation dispatched without consent'
                session.command('cancel')
                until(session, lambda: not journey.snapshot()['claims'], 'cancelled unsent reservation settled')
                journey.workspace()
                assert len(peer.snapshot('POST')) == before, 'cancelling preparation sent a native request'
                history = send('B_AFTER_CANCEL_163', history, 'B_PUBLIC_ANSWER_277',
                               'cancelled-fresh-preparation-retains-A')
                history = send('C_EXPLICIT_RESUME_389', history, 'C_PUBLIC_ANSWER_491',
                               'explicit-resume-retains-A-and-B', command='resume')
                old_text = [text for _, text in history]
                history = send('D_EXPLICIT_NEW_503', [], 'D_PUBLIC_ANSWER_617',
                               'explicit-new-excludes-completed-A-B-C', command='new')
                encoded = json.dumps(peer.snapshot('POST')[-1]['body'])
                assert all(text not in encoded for text in old_text), 'fresh native POST leaked prior public history'
                assert 'UNSENT_FRESH_PREPARATION_947' not in encoded
                history = send('E_AFTER_NEW_739', history, 'E_PUBLIC_ANSWER_853',
                               'ordinary-enter-after-new-retains-only-D')
                peer.assert_clean()
                session.command('quit')
                confirm_exit(session)
                assert session.wait_exit() == 0
                result['status'] = 'passed'
            except Exception as error:
                result.update(error=str(error), traceback=traceback.format_exc())
                if journey is not None:
                    journey.capture('failed')
            finally:
                save(output / 'http-requests.json', peer.snapshot())
                if journey is not None:
                    save(output / 'owned-state.json', journey.snapshot())
                    journey.session.close()
                save(output / 'results.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--language', choices=('en', 'ko', 'both'), default='both')
    parser.add_argument('--mode', choices=('native', 'codex', 'all'), default='all')
    args = parser.parse_args()
    binary, output = Path(args.binary).resolve(), Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {'status': 'failed', 'binary': str(binary),
              'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'scope': 'Credential-free actual PTY and serialized protocol paths; not live-provider qualification',
              'cases': []}
    try:
        for language in (('en', 'ko') if args.language == 'both' else (args.language,)):
            for size in ((60, 18), (120, 36)):
                stem = language + '-' + str(size[0])
                if args.mode in ('native', 'all'):
                    result['cases'].append(native_case(binary, output / ('native-' + stem), size, language))
                if args.mode in ('codex', 'all'):
                    controlled_output = output / ('codex-' + stem)
                    controlled_output.mkdir(parents=True, exist_ok=True)
                    result['cases'].append(codex_case(binary, controlled_output, size, language))
        assert all(case['status'] == 'passed' for case in result['cases']), 'conversation runtime cases failed; inspect retained results'
        result['status'] = 'passed'
    finally:
        save(output / 'results.json', result)


if __name__ == '__main__':
    main()
