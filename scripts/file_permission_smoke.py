#!/usr/bin/env python3
"""Exercise ordinary file intent, visible one-shot consent and actual filesystem effects.

Runs the real executable with disposable projects and loopback protocol peers.
No account credentials, paid inference, SQL writes or user installation changes.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import traceback

from file_creation_smoke import (
    CreationJourney, Gate, Peer, confirm_exit, response_plan, save, tool_results,
    until, wait_for,
)

RIGHT, LEFT, PAGE_DOWN = '\x1b[C', '\x1b[D', '\x1b[6~'
TARGET = 'src/components/workspace/editor/permissions/FilePermissionDialog.tsx'
CONTENT = '// exact 한글 / café\r\nexport const approved = true;\r\n'
EDITED = CONTENT.replace('true', 'false')


def run_case(binary, output, root, size, language, route, protocol):
    output.mkdir(parents=True, exist_ok=False)
    root.mkdir(mode=0o700)
    result = {'status': 'failed', 'size': list(size), 'language': language,
              'route': route, 'protocol': protocol, 'cases': [],
              'limits': 'Actual terminal and filesystem with local protocol fixtures; not live-account acceptance'}
    journey = None
    gates = []
    sequence = 0
    with Peer(protocol, 'sk-public-permission-fixture-only', route == 'compatible', retain_body_bytes=True) as peer:
        try:
            journey = CreationJourney(binary, output, root, size, size[0] == 60,
                                      route, protocol, route == 'compatible', False,
                                      peer, language=language)
            wait_for(journey.session, 'Storage: saved')
            (journey.project / 'seed.rs').write_text('pub const SEED: bool = true;\n')
            journey.setup_route()

            def start(request, rounds, reply, *, writing=True):
                gate = Gate()
                gates.append(gate)
                plans = [response_plan(peer, calls=calls, number=index,
                                       gate=gate if index == 0 else None)
                         for index, calls in enumerate(rounds)]
                plans.append(response_plan(peer, text=reply, number=len(rounds)))
                before = journey.begin(request, plans, writing=writing)
                until(journey.session, gate.ready.is_set, 'owned tool round did not reach its gate')
                gate.release.set()
                return before, len(plans)

            def decide(path, allow, original):
                nonlocal sequence
                sequence += 1
                session = journey.session
                automatic = ('File proposal ready', '파일 제안 준비됨')
                unavailable = ('review all scope before allowing', '전체 범위를 확인한 뒤')
                until(session, lambda: any(label in session.display() for label in automatic),
                      'file permission did not present automatically', timeout=40)
                target = journey.project / path
                current = lambda: target.read_bytes() if target.is_file() else None
                assert current() == original, 'proposal changed bytes before consent'
                posts = len(peer.snapshot('POST'))
                journey.capture(f'{sequence}-automatic-file-popup')
                session.send('ay')
                session.paste('a\ny')
                assert current() == original and len(peer.snapshot('POST')) == posts
                assert any(label in session.display() for label in automatic), 'arrival typing became consent'
                if sequence == 1:
                    session.send('\t')
                    session.send('\t')
                else:
                    session.send(RIGHT if allow else LEFT)
                until(session, lambda: not any(label in session.display() for label in automatic),
                      'explicit decision selection did not engage the popup')
                pages = 0
                if allow and any(label in session.display() for label in unavailable):
                    session.send('\r')
                    assert current() == original and len(peer.snapshot('POST')) == posts, 'unseen scope was approved'
                    while any(label in session.display() for label in unavailable):
                        assert pages < 64, 'scope never became reviewable without terminal resizing'
                        session.send(PAGE_DOWN)
                        pages += 1
                        assert current() == original and len(peer.snapshot('POST')) == posts
                visible = session.display()
                assert ('[ Allow once ]' in visible if language == 'en' else '[ 한 번 허용 ]' in visible), visible
                selected = ('Allow once' if allow else 'Deny') if language == 'en' else ('한 번 허용' if allow else '거부')
                assert f'> [ {selected} ]' in visible, 'plain-text choice marker did not match the explicit decision'
                journey.capture(f'{sequence}-selected-decision-complete-scope')
                session.send('\r')
                return pages

            read_reply = 'Read-only source inspection finished.'
            before, count = start('Read `seed.rs`' if language == 'en' else '`seed.rs`를 읽어줘.',
                                  [[{'id': 'read-seed', 'name': 'eevee_read_file',
                                     'arguments': {'path': 'seed.rs', 'start_line': 1, 'end_line': 0}}]],
                                  read_reply, writing=False)
            journey.settled(read_reply, before + count)
            observed = json.dumps(tool_results(protocol, peer.snapshot('POST')[before + 1]['body']))
            assert hashlib.sha256((journey.project / 'seed.rs').read_bytes()).hexdigest() in observed
            result['cases'].append('actual read-only discovery established the previous automatic nonwriting role')

            create_request = f'Create `{TARGET}`' if language == 'en' else f'`{TARGET}`를 생성해줘.'
            for allow in (False, True):
                reply = 'Creation approved and completed.' if allow else 'Creation denied; no file was written.'
                before, count = start(create_request,
                                      [[{'id': f'create-{allow}', 'path': TARGET, 'content': CONTENT}]], reply)
                assert not (journey.project / 'src').exists(), 'unapproved parent creation'
                pages = decide(TARGET, allow, None)
                journey.settled(reply, before + count)
                if allow:
                    assert (journey.project / TARGET).read_bytes() == CONTENT.encode()
                    if size[0] == 60:
                        assert pages > 0, 'compact nested-path case did not exercise scope scrolling'
                else:
                    assert not (journey.project / 'src').exists(), 'denial created a file or parent'
                assert f'create-{allow}' in tool_results(protocol, peer.snapshot('POST')[before + 1]['body'])
                result['cases'].append('quoted create after discovery: ' + ('scrolled exact scope and button-approved literal bytes' if allow else 'button denial settled without files or parents'))

            edit_request = f'Edit `{TARGET}`' if language == 'en' else f'`{TARGET}`를 수정해줘.'
            for allow in (False, True):
                reply = 'Exact edit completed.' if allow else 'Edit denied; original file preserved.'
                before, count = start(edit_request, [
                    [{'id': f'read-edit-{allow}', 'name': 'eevee_read_file',
                      'arguments': {'path': TARGET, 'start_line': 1, 'end_line': 0}}],
                    [{'id': f'edit-{allow}', 'name': 'eevee_edit_file',
                      'arguments': {'path': TARGET, 'sha256': hashlib.sha256(CONTENT.encode()).hexdigest(),
                                    'old_text': 'true', 'new_text': 'false'}}],
                ], reply)
                decide(TARGET, allow, CONTENT.encode())
                journey.settled(reply, before + count)
                assert (journey.project / TARGET).read_bytes() == (EDITED if allow else CONTENT).encode()
                assert f'edit-{allow}' in tool_results(protocol, peer.snapshot('POST')[before + 2]['body'])
                result['cases'].append('quoted hash-bound edit: ' + ('button approval changed only selected bytes and preserved CRLF' if allow else 'button denial preserved every original byte and resumed the turn'))

            handoff = '# Reviewed handoff\n'
            reply = 'Approved handoff file created.'
            before, count = start('Create handoff.md' if language == 'en' else 'handoff.md를 생성해줘.',
                                  [[{'id': 'create-handoff', 'path': 'handoff.md', 'content': handoff}]], reply)
            decide('handoff.md', True, None)
            journey.settled(reply, before + count)
            assert (journey.project / 'handoff.md').read_bytes() == handoff.encode()
            result['cases'].append('explicit handoff filename advertised mutation tools and required its own exact file permission')
            journey.session.command('quit')
            confirm_exit(journey.session)
            assert journey.session.wait_exit() == 0
            peer.assert_clean()
            result['status'] = 'passed'
        except Exception as error:
            result.update(error=str(error), traceback=traceback.format_exc())
            if journey:
                journey.capture('failed')
        finally:
            for gate in gates:
                gate.release.set()
            save(output / 'http-requests.json', peer.snapshot())
            if journey:
                journey.session.close()
            save(output / 'results.json', result)
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--language', choices=('en', 'ko'), required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    results = []
    with tempfile.TemporaryDirectory(prefix='ev-file-permission-') as temporary:
        for route, protocol in [('openai', 'responses'), ('compatible', 'chat_completions')]:
            for size in [(120, 36), (60, 18)]:
                label = f'{route}-{size[0]}'
                results.append(run_case(args.binary.resolve(), output / label,
                                        Path(temporary) / label, size, args.language, route, protocol))
    report = {'status': 'passed' if all(row['status'] == 'passed' for row in results) else 'failed',
              'cases': results}
    save(output / 'results.json', report)
    print(json.dumps({'status': report['status'], 'output': str(output)}))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
