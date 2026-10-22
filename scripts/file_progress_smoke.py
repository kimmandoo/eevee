#!/usr/bin/env python3
"""Exercise actual file-generation previews before complete input or approval.

Uses an explicit native binary, disposable homes/projects and loopback SSE peers.
Peers send arguments only: the application performs every tested file effect.
"""
import argparse
import hashlib
import json
from copy import deepcopy
from pathlib import Path
import tempfile
import traceback

import file_creation_smoke as files
import response_smoke
from terminal_smoke import confirm_exit
from workbench_smoke import wait_for

PATH = 'generated/live.py'
CONTENT = 'value = "한글界e\u0301"\r\n# STREAM_STAGE_ONE\r\nvalue += " actual bytes"\r\n# STREAM_STAGE_TWO\r\n'
OLD = '\n'.join('OLD_RESPONSE_ANCHOR_%02d' % number for number in range(1, 31)) + '\nOLD_RESPONSE_END'
FUTURE = 'future draft 한글界e\u0301Z-end'
SECRET = 'sk-' + 'A' * 48
PRIVATE_ID = 'sk-' + 'Z' * 48


def argument_fragment(protocol, value):
    if protocol == 'responses':
        return value.get('delta', '') if value.get('type') == 'response.function_call_arguments.delta' else ''
    if protocol == 'chat_completions':
        return ''.join(call.get('function', {}).get('arguments', '')
                       for choice in value.get('choices', [])
                       for call in choice.get('delta', {}).get('tool_calls', []))
    if protocol == 'messages':
        delta = value.get('delta', {})
        return delta.get('partial_json', '') if delta.get('type') == 'input_json_delta' else ''
    return ''


def gated_plan(peer, path, content, call_id='live-preview'):
    first, final = files.Gate(), files.Gate()
    base = files.response_plan(peer, calls=[{'id': call_id, 'path': path, 'content': content}], gate=final)

    def reply(handler, event, row):
        received = ''
        held = False

        def emit(value, name=None):
            nonlocal received, held
            fragment = argument_fragment(peer.protocol, value)
            combined = received + fragment
            marker = 'STREAM_STAGE_ONE\\r\\n'
            if not held and fragment and marker in combined:
                split = combined.index(marker) + len(marker) - len(received)
                def part(text):
                    copied = deepcopy(value)
                    if peer.protocol == 'responses':
                        copied['delta'] = text
                    elif peer.protocol == 'chat_completions':
                        copied['choices'][0]['delta']['tool_calls'][0]['function']['arguments'] = text
                    else:
                        copied['delta']['partial_json'] = text
                    event(copied, name)
                part(fragment[:split])
                held = True
                first.hold(peer)
                if fragment[split:]:
                    part(fragment[split:])
            else:
                event(value, name)
            received = combined
        return base[1](handler, emit, row)
    return ('custom', reply), first, final


def visible(journey, marker):
    files.until(journey.session, lambda: marker in journey.session.display(),
                'actual generated code was not visible before argument completion: ' + marker, timeout=10)


def no_preview_journal(journey):
    encoded = json.dumps(journey.snapshot()['journals'], ensure_ascii=False)
    assert 'STREAM_STAGE_' not in encoded and SECRET not in encoded, 'generation preview entered the durable provider journal'


def run(binary, output, root, size, ascii_mode, protocol, language):
    output.mkdir(parents=True, exist_ok=False)
    result = {'status': 'failed', 'protocol': protocol, 'language': language, 'size': list(size), 'cases': []}
    journey = None
    gates = []
    route = {'messages': 'anthropic', 'gemini': 'google'}.get(protocol, 'compatible')
    keyless = route == 'compatible'
    with files.Peer(protocol, 'sk-progress-fixture-only', keyless, retain_body_bytes=True) as peer:
        try:
            journey = files.CreationJourney(binary, output, root, size, ascii_mode,
                                             route, protocol, keyless, False, peer, language=language)
            wait_for(journey.session, 'Storage: saved')
            journey.setup_route()
            seed_gate = files.Gate()
            gates.append(seed_gate)
            seed = files.response_plan(peer, text=OLD)
            def seed_reply(handler, event, row):
                if seed_gate.hold(peer):
                    return seed[1](handler, event, row)
            before = journey.begin('Explain this public display fixture.', [('custom', seed_reply)])
            seed_gate.release.set()
            journey.settled('OLD_RESPONSE_END', before + 1)
            plan, first, final = gated_plan(peer, PATH, CONTENT)
            gates.extend([first, final])
            before = journey.begin('Generate a Python file generated/live.py with the requested code.', [
                plan, files.response_plan(peer, text='ACTUAL_FILE_APPLIED', number=1)])
            if protocol == 'gemini':
                files.until(journey.session, final.ready.is_set, 'whole-call proposal did not reach the gate')
            else:
                files.until(journey.session, first.ready.is_set, 'partial arguments did not reach the gate')
            visible(journey, 'STREAM_STAGE_ONE' if protocol != 'gemini' else 'STREAM_STAGE_TWO')
            if protocol != 'gemini':
                assert 'STREAM_STAGE_TWO' not in journey.session.display(), 'fixture delivered later code before the first gate'
            journey.capture('live-code-before-complete-input')
            assert not (journey.project / PATH).exists() and not (journey.project / PATH).parent.exists(), 'preview wrote source or a parent'
            assert len(peer.snapshot('POST')) == before + 1, 'preview dispatched a continuation'
            no_preview_journal(journey)
            result['cases'].append('actual decoded numbered source appeared inline without navigation before approval, with no file/parent/continuation/journal effect')

            journey.write_request(FUTURE)
            journey.session.send('\x1b[D' * 3 + '\x13')
            journey.session.command('response')
            journey.session.send(response_smoke.ALT_HOME)
            for _ in range(8):
                if 'OLD_RESPONSE_ANCHOR_01' in journey.session.display():
                    break
                journey.session.send('\x1b[6~')
            visible(journey, 'OLD_RESPONSE_ANCHOR_01')
            old_rows = [line for line in journey.session.display().splitlines() if 'OLD_RESPONSE_ANCHOR_' in line]
            if protocol != 'gemini':
                first.release.set()
                files.until(journey.session, final.ready.is_set, 'later argument chunks did not reach the terminal gate')
                journey.session.pump(.2)
                assert [line for line in journey.session.display().splitlines() if 'OLD_RESPONSE_ANCHOR_' in line] == old_rows, 'new file chunks moved paused historical output'
            journey.capture('paused-old-output-during-generation')
            journey.session.send(response_smoke.ALT_END)
            visible(journey, 'STREAM_STAGE_TWO')
            journey.capture('grown-code-before-approval')
            no_preview_journal(journey)
            assert not (journey.project / PATH).exists(), 'completed arguments granted a write'
            result['cases'].append('later source chunks grew in place while paused prior output and a future Unicode draft/caret stayed owned')
            final.release.set()
            journey.permission(PATH, allow=True)
            journey.session.command('response')
            journey.session.send(response_smoke.ALT_END)
            journey.settled('ACTUAL_FILE_APPLIED', before + 2)
            assert (journey.project / PATH).read_bytes() == CONTENT.encode(), 'reviewed write changed literal Unicode or CRLF bytes'
            drafts = [draft for task in journey.snapshot()['tasks'] for state in task['partner_drafts'].values() for draft in state['drafts']]
            assert any(draft['text'] == FUTURE and draft['cursor'] == len(FUTURE.encode()) - 3 for draft in drafts), 'generation/approval lost future draft or caret'
            assert all(FUTURE not in json.dumps(row['body'], ensure_ascii=False) for row in peer.snapshot('POST')), 'future draft entered a submitted body'
            result['cases'].append('only exact direct consent produced actual literal bytes and retained the later unsent draft/caret')

            cancel_path = 'generated/cancelled.py'
            cancel_content = '# STREAM_STAGE_ONE\r\napi_key = "' + SECRET + '"\r\n# STREAM_STAGE_TWO\r\n'
            plan, first, final = gated_plan(peer, cancel_path, cancel_content, PRIVATE_ID)
            gates.extend([first, final])
            before = journey.begin('Generate a Python file generated/cancelled.py; this is a cancellation fixture.', [plan])
            if protocol != 'gemini':
                files.until(journey.session, first.ready.is_set, 'cancel partial gate missing')
                first.release.set()
            files.until(journey.session, final.ready.is_set, 'cancel terminal gate missing')
            journey.session.command('response')
            journey.session.send(response_smoke.ALT_END)
            visible(journey, 'STREAM_STAGE_TWO')
            assert SECRET not in journey.session.text and PRIVATE_ID not in journey.session.text, 'credential or private call identity leaked through preview'
            no_preview_journal(journey)
            journey.capture('private-preview-before-cancel')
            journey.session.command('cancel')
            files.until(journey.session, lambda: not journey.snapshot()['claims'], 'cancel did not settle owned transport', timeout=40)
            final.release.set()
            journey.session.pump(.2)
            assert not (journey.project / cancel_path).exists(), 'cancelled preview created a file'
            assert len(peer.snapshot('POST')) == before + 1, 'cancel retried or continued the request'
            assert SECRET not in json.dumps(journey.snapshot(), ensure_ascii=False), 'preview credential entered durable state'
            journey.capture('cancelled-preview-no-file')
            result['cases'].append('cumulative credential/private-ID filtering and cancellation retained zero effects, no retry and no durable preview')
            journey.workspace()
            journey.session.command('quit')
            owned_titles = ('ALL owned work', '모든 소유 작업')
            files.until(journey.session, lambda: not journey.session.alive()
                        or any(title in journey.session.display() for title in owned_titles)
                        or 'Confirm exit' in journey.session.display(), 'quit review missing')
            if journey.session.alive() and any(title in journey.session.display() for title in owned_titles):
                journey.session.send('y')
            confirm_exit(journey.session)
            assert journey.session.wait_exit() == 0
            result['status'] = 'passed'
        except Exception as error:
            result.update(error=str(error), traceback=traceback.format_exc())
            if journey:
                journey.capture('failed')
        finally:
            for gate in gates:
                gate.release.set()
            if journey:
                journey.close()
            files.save(output / 'results.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--language', choices=('en', 'ko'), default='en')
    args = parser.parse_args()
    binary, output = Path(args.binary).resolve(), Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    sha = hashlib.sha256(binary.read_bytes()).hexdigest()
    result = {'status': 'failed', 'binary': str(binary), 'sha256': sha, 'language': args.language, 'journeys': [],
              'limits': ['Deterministic loopback actual UI/files, not authenticated model or physical GUI conformance.',
                         'Gemini exposes a received whole proposal, not fabricated argument streaming.']}
    try:
        for size, ascii_mode in [((120, 36), False), ((60, 18), True)]:
            for protocol in ('responses', 'chat_completions', 'messages', 'gemini'):
                name = protocol + '-' + str(size[0])
                with tempfile.TemporaryDirectory(prefix='ev-file-progress-') as temporary:
                    result['journeys'].append(run(binary, output / name, Path(temporary), size, ascii_mode, protocol, args.language))
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == sha
        result['status'] = 'passed' if all(row['status'] == 'passed' for row in result['journeys']) else 'failed'
    finally:
        files.save(output / 'results.json', result)
    print(json.dumps({'status': result['status'], 'output': str(output)}))
    raise SystemExit(0 if result['status'] == 'passed' else 1)


if __name__ == '__main__':
    main()
