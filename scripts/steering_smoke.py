#!/usr/bin/env python3
"""XC03 actual-binary loopback steering/queued Send at both terminal sizes."""
from __future__ import annotations
import smoke_sizes
import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import tempfile
import threading
import traceback
from native_connection_smoke import Journey, Peer, until, ESC, MODEL
from terminal_smoke import confirm_exit
from workbench_smoke import wait_for

STEER = '  Read only note.txt; retain 한글 boundaries.  '
QUEUED = 'Explain the observed source in the next request.'
LATE = '  Late steering bytes kept  '


def finish(handler, emit, text):
    emit({'model': MODEL, 'choices': [{'index': 0, 'delta': {'content': text}, 'finish_reason': None}]})
    emit({'model': MODEL, 'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'stop'}]})
    handler.wfile.write(b'data: [DONE]\n\n')
    handler.wfile.flush()


def author(session, text, choice):
    session.send(ESC + '[200~' + text + ESC + '[201~', settle=False)
    session.send('\r', settle=False)
    until(session, lambda: 'Steer this run' in session.display(), 'visible busy-Send choice')
    session.send(choice, settle=False)


def run(binary, output, size):
    path = output / f'{size[0]}x{size[1]}'
    path.mkdir(parents=True, exist_ok=True)
    gates = [threading.Event(), threading.Event()]
    journey = None
    try:
        with tempfile.TemporaryDirectory(prefix='ev-steering-') as temporary, Peer('chat_completions', 'unused-keyless-steering-7a43', True) as peer, ExitStack() as cleanup:
            for gate in gates: cleanup.callback(gate.set)
            journey = Journey(binary, path, Path(temporary), size, size[0] < 90, 'compatible', 'chat_completions', True, False, peer)
            journey.project.joinpath('note.txt').write_text('ACTUAL_STEERING_SOURCE\n')
            journey.setup(initial=True)
            session = journey.session
            def first(_handler, emit, _row):
                emit({'model': MODEL, 'choices': [{'index': 0, 'delta': {'content': 'First round is running.'}, 'finish_reason': None}]})
                if not gates[0].wait(25): raise AssertionError('first inference gate not released')
                emit({'model': MODEL, 'choices': [{'index': 0, 'delta': {'tool_calls': [{'index': 0, 'id': 'read-before-steer', 'type': 'function', 'function': {'name': 'eevee_read_file', 'arguments': json.dumps({'path': 'note.txt', 'start_line': 1, 'end_line': 0})}}]}, 'finish_reason': None}]})
                emit({'model': MODEL, 'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'tool_calls'}]})
                _handler.wfile.write(b'data: [DONE]\n\n')
                _handler.wfile.flush()
            peer.plans.put(('custom', first)); peer.plans.put(('complete', 'Steered run completed.')); peer.plans.put(('complete', 'Queued request completed.'))
            journey.write_request('Inspect note.txt and explain its contents.')
            session.send('\r', settle=False)
            def first_started():
                # Dismissing the nonmodal notice moves the caret. It must not
                # invalidate the save barrier before the actual first POST.
                if len(peer.snapshot('POST')) != 1:
                    return False
                view = session.display()
                if 'Evolution!' in view and 'Keep typing' in view:
                    session.send(ESC + '[D', settle=False)
                    return False
                return 'First round is' in view
            until(session, first_started, 'actual active first round')
            author(session, STEER, ESC)
            assert 'Read only note.txt' in session.display(), 'neutral choice lost authored draft'
            session.send('\r', settle=False)
            until(session, lambda: 'Steer this run' in session.display(), 'second explicit steering choice')
            session.send('s', settle=False)
            until(session, lambda: 'Steer queued' in session.display(), 'steering accepted for exact generation')
            author(session, QUEUED, 'q')
            until(session, lambda: 'Next Send queued' in session.display() or 'Next Send queued' in journey.session.text, 'visible queued Send')
            journey.capture('active-steering-and-queue')
            gates[0].set()
            until(session, lambda: len(peer.snapshot('POST')) == 3 and not journey.snapshot()['claims'], 'completed parent dispatches exactly one freshly prepared queued Send', timeout=25)
            wait_for(session, 'Queued request completed.')
            rows = peer.snapshot('POST')
            messages = rows[1]['body']['messages']
            assert sum(row.get('content') == STEER and row.get('role') == 'user' for row in messages) == 1
            assert any('ACTUAL_STEERING_SOURCE' in row.get('content', '') for row in messages if row.get('role') == 'tool')
            assert rows[2]['body']['messages'][-1] == {'role': 'user', 'content': QUEUED}
            journey.capture('queued-send-completed')
            def final(_handler, emit, _row):
                emit({'model': MODEL, 'choices': [{'index': 0, 'delta': {'content': 'Final inference has no tools.'}, 'finish_reason': None}]})
                if not gates[1].wait(25): raise AssertionError('final inference gate not released')
                finish(_handler, emit, 'No later tool-result boundary.')
            peer.plans.put(('custom', final))
            journey.write_request('Explain the last observation.')
            session.send('\r', settle=False)
            until(session, lambda: len(peer.snapshot('POST')) == 4 and 'Final inference' in session.display(), 'actual final inference')
            author(session, LATE, 's')
            until(session, lambda: 'Steer queued' in session.display(), 'late steering waits visibly')
            gates[1].set()
            until(session, lambda: 'Late steering bytes kept' in session.display() and not journey.snapshot()['claims'], 'late steering returned without dispatch')
            assert len(peer.snapshot('POST')) == 4
            assert any('Response ' in line and '· complete' in line for line in session.display().splitlines()), 'late steering did not end on an authoritative completed response'
            journey.capture('late-steering-returned')
            peer.assert_clean()
            journey.workspace(); session.command('quit'); confirm_exit(session)
            assert session.wait_exit() == 0
            (path / 'requests.json').write_text(json.dumps(peer.snapshot(), indent=2, ensure_ascii=False))
            return {'size': size, 'status': 'passed', 'inference_requests': 4, 'steering_exact_once': True, 'queued_fresh_send': True, 'late_steer_unsent': True}
    finally:
        if journey is not None:
            try:
                journey.capture('final')
            finally:
                journey.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True); parser.add_argument('--output', required=True)
    args = parser.parse_args()
    output = Path(args.output).resolve(); output.mkdir(parents=True, exist_ok=True)
    result = {'status': 'failed', 'cases': []}
    try:
        for size in smoke_sizes.selected(((120, 36), (60, 18))):
            result['cases'].append(run(Path(args.binary).resolve(), output, size)); print('passed', result['cases'][-1], flush=True)
        result['status'] = 'passed'
    except BaseException:
        result['failure'] = traceback.format_exc(); raise
    finally:
        (output / 'results.json').write_text(json.dumps(result, indent=2) + '\n')

if __name__ == '__main__': main()
