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
import threading
import unicodedata

from file_creation_smoke import codex_case
from native_connection_smoke import Peer, decoded_payload, until
from response_smoke import SurfaceJourney, saved_request, response_view, ALT_HOME, ALT_END, PAGE_DOWN
from terminal_smoke import confirm_exit
from workbench_smoke import wait_for

ESC = '\x1b'
GOAL = 'Stable public conversation fixture goal, not an earlier request.'


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def native_case(binary, output, size, ):
    output.mkdir(parents=True, exist_ok=True)
    result = {'status': 'failed', 'route': 'native-compatible', 'size': list(size), 'cases': [], 'payload_assertions': []}
    journey = None
    release = None
    with tempfile.TemporaryDirectory(prefix='ev-conversation-native-') as temporary:
        with Peer('chat_completions', 'unused-public-keyless-fixture', True) as peer:
            try:
                journey = SurfaceJourney(binary, output, Path(temporary), size, size[0] == 60,
                                         'compatible', 'chat_completions', True, False, peer,
                                         )
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
                        session.send('\r', settle=False)
                    else:
                        session.command(command, settle=False)
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
                before_network = len(peer.snapshot())
                session.command('session-new')
                def prepared_review():
                    view = session.display()
                    if any(label in view for label in (
                            'Enter to review (not approve)', '· y Run / n Cancel', )):
                        return True
                    if 'Alt+A' in view:
                        session.send(ESC + 'a')
                    return False
                until(session, prepared_review,
                      'fresh unsent native preparation reached real permission review')
                journey.capture('fresh-preparation-before-neutral-cancel')
                assert len(peer.snapshot('POST')) == before, 'preparation dispatched without consent'
                session.send('d')
                disclosure = []
                for _ in range(128):
                    disclosure.append(session.display())
                    before_view = session.display()
                    session.send(PAGE_DOWN)
                    if session.display() == before_view:
                        break
                disclosure = ''.join(''.join(disclosure).translate(str.maketrans('', '', '│║┃|')).split())
                for required in ('NativeHTTPprepared', 'freshpubliccontext', 'noproviderthread'):
                    assert required in disclosure, (required, disclosure)
                assert 'NEWproviderthread' not in disclosure
                assert len(peer.snapshot()) == before_network, 'native context inspection performed network work'
                journey.capture('native-fresh-public-context-not-provider-thread')
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
                def inspect_history(label):
                    session.send(ALT_HOME)
                    views = {}
                    for _ in range(1024):
                        current = response_view(session)
                        first, last, total = current['range']
                        assert len(current['body']) == last - first + 1
                        for index, line in enumerate(current['body'], first):
                            if index in views:
                                assert views[index] == line, 'settled transcript changed during inspection'
                            views[index] = line
                        before = session.display()
                        session.send(PAGE_DOWN)
                        if session.display() == before:
                            break
                    else:
                        raise AssertionError('bounded outcome inspection did not reach transcript end')
                    journey.capture(label)
                    assert set(views) == set(range(1, total + 1)), 'inspection omitted retained transcript rows'
                    return ''.join(''.join(views[index] for index in range(1, total + 1)).split())

                def request_terminal(observed, request):
                    # Scope outcome/reason/admission to this exact originating
                    # request, not another completed/failed card in the history.
                    tail = observed.split('Request:' + request, 1)[1]
                    outcome, tail = tail.split('Requestoutcome:', 1)[1].split('Reason:', 1)
                    reason, tail = tail.split('Publiccontext:', 1)
                    context = tail.split('Origin:', 1)[0]
                    return outcome, reason, context

                for mode, outcome, reason in (
                        ('failed', 'Failed', 'Provider reported an inference error'),
                        ('unknown', 'Unknown', 'local reader settled'),
                        ('cancel', 'Unknown', 'local reader settled')):
                    request = 'Interrupted-' + mode + '-original-request'
                    partial = 'PARTIAL_' + mode + '_界e\u0301_NOT_CANONICAL\n'
                    shown_partial = 'Fixture partial output,' if mode == 'cancel' else partial
                    shown_partial = ''.join(unicodedata.normalize('NFC', shown_partial).split())
                    release, ready = threading.Event(), threading.Event()
                    if mode == 'cancel':
                        peer.cancel_started.clear()
                        peer.cancel_closed.clear()
                        peer.plans.put(('cancel', ''))
                    else:
                        def interrupted(handler, event, packet, mode=mode, partial=partial,
                                        ready=ready, release=release):
                            event({'model': peer.expected_model, 'choices': [{'index': 0,
                                   'delta': {'content': partial}, 'finish_reason': None}]})
                            ready.set()
                            assert release.wait(12), 'controlled partial settlement gate expired'
                            if mode == 'failed':
                                event({'error': {'message': 'synthetic private diagnostic'}})
                            # Unknown closes after real bytes without a terminal frame.
                        peer.plans.put(('custom', interrupted))
                    journey.write_request(request)
                    before = len(peer.snapshot('POST'))
                    # The peer deliberately holds this turn open. Windows
                    # console quiet is not a completion/ready signal here.
                    session.send('\r', settle=False)
                    until(session, lambda: len(peer.snapshot('POST')) == before + 1,
                          'one explicit interrupted request')
                    _, actual = decoded_payload('chat_completions', peer.snapshot('POST')[-1]['body'])
                    assert actual == history + [('user', request)], actual
                    until(session, peer.cancel_started.is_set if mode == 'cancel' else ready.is_set,
                          'actual partial bytes before future editing')
                    until(session, lambda: shown_partial in ''.join(
                        unicodedata.normalize('NFC', session.display()).split()),
                        'admissible partial rendered before future editing')
                    future = 'Future界e\u0301Z-end'
                    # Send left the Request editor available for inert future
                    # work. Do not wait for console silence while it streams.
                    session.paste(future, settle=False)
                    session.send((ESC + '[D') * 3, settle=False)
                    session.send('\x13', settle=False)
                    cursor = len(future[:-3].encode())
                    until(session, lambda: saved_request(journey, future, cursor),
                          'future Unicode draft and nonterminal caret saved')
                    if mode == 'cancel':
                        session.command('cancel', settle=False)
                        until(session, peer.cancel_closed.is_set, 'actual cancel closed reader')
                    else:
                        release.set()
                    until(session, lambda: not journey.snapshot()['claims']
                          and outcome + ' · ' in session.display(),
                          'request-bound terminal outcome remained visible')
                    observed = inspect_history(mode + '-original-terminal')
                    original_outcome, original_reason, original_context = request_terminal(observed, request)
                    assert original_outcome == outcome, (original_outcome, outcome)
                    assert reason.replace(' ', '') in original_reason, original_reason
                    assert original_context == 'excludedfromfollow-up', original_context
                    assert shown_partial in observed, 'admissible partial display was lost at settlement'
                    session.send('!')
                    expected = future[:-3] + '!' + future[-3:]
                    expected_cursor = len((future[:-3] + '!').encode())
                    session.send('\x13')
                    until(session, lambda: saved_request(journey, expected, expected_cursor),
                          'terminal/history inspection preserved future insertion caret')
                    assert len(peer.snapshot('POST')) == before + 1, 'inspection or typing sent automatically'
                    if mode == 'unknown':
                        # Observed EOF requires explicit existing local remediation.
                        session.command('connect')
                        session.command('diagnostics')
                        wait_for(session, 'Transport failed')
                        session.send('n')
                        wait_for(session, 'Ctrl+S Local')
                        journey.workspace()
                        assert saved_request(journey, expected, expected_cursor)
                        assert len(peer.snapshot('POST')) == before + 1, 'diagnostics reconnected/resubmitted'
                    session.send(ALT_END)
                    history = send(expected, history, 'FOLLOWUP_' + mode + '_COMPLETE',
                                   mode + '-explicit-followup-excludes-partial')
                    observed = inspect_history(mode + '-historical-terminal-after-followup')
                    retained_outcome, retained_reason, retained_context = request_terminal(observed, request)
                    assert (retained_outcome, retained_reason, retained_context) == (
                        original_outcome, original_reason, original_context), 'new request rewrote old terminal evidence'
                    assert shown_partial in observed, 'explicit follow-up erased the retained partial display'
                    assert 'Publiccontext:admittedatsettlement' in observed
                    assert partial not in json.dumps(peer.snapshot('POST')[-1]['body'], ensure_ascii=False)
                    assert len(peer.snapshot('POST')) == before + 2, 'hidden retry or automatic follow-up'
                    result['cases'].append(mode + '-retained-terminal-exact-exclusions-future-caret-no-replay')
                    session.send(ALT_END)

                peer.assert_clean()
                session.command('quit')
                confirm_exit(session, owned_work=True)
                assert session.wait_exit() == 0
                result['status'] = 'passed'
            except Exception as error:
                result.update(error=str(error), traceback=traceback.format_exc())
                if journey is not None:
                    journey.capture('failed')
            finally:
                # An assertion must not strand the owned HTTP fixture on its
                # gate and replace the useful failure with a cleanup timeout.
                if release is not None:
                    release.set()
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
    parser.add_argument('--mode', choices=('native', 'codex', 'all'), default='all')
    args = parser.parse_args()
    binary, output = Path(args.binary).resolve(), Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    result = {'status': 'failed', 'binary': str(binary),
              'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'scope': 'Credential-free actual PTY and serialized protocol paths; not live-provider qualification',
              'cases': []}
    try:
        for size in ((60, 18), (120, 36)):
            stem = str(size[0])
            if args.mode in ('native', 'all'):
                result['cases'].append(native_case(binary, output / ('native-' + stem), size, ))
            if args.mode in ('codex', 'all'):
                controlled_output = output / ('codex-' + stem)
                controlled_output.mkdir(parents=True, exist_ok=True)
                result['cases'].append(codex_case(binary, controlled_output, size, ))
        assert all(case['status'] == 'passed' for case in result['cases']), 'conversation runtime cases failed; inspect retained results'
        result['status'] = 'passed'
    finally:
        save(output / 'results.json', result)


if __name__ == '__main__':
    main()
