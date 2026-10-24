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
    CreationJourney, Gate, Peer, confirm_exit, file_review_pending, response_plan, save, tool_results,
    until, wait_for,
)

RIGHT, LEFT, PAGE_DOWN = '\x1b[C', '\x1b[D', '\x1b[6~'
TARGET = 'src/components/workspace/editor/permissions/FilePermissionDialog.tsx'
CONTENT = '// exact 한글 / café\r\nexport const approved = true;\r\n'
EDITED = CONTENT.replace('true', 'false')
FORMS = ('eevee', 'vaporeon', 'jolteon', 'flareon', 'espeon',
         'umbreon', 'leafeon', 'glaceon', 'sylveon')


def run_case(binary, output, root, size, language, route, protocol, form):
    output.mkdir(parents=True, exist_ok=False)
    root.mkdir(mode=0o700)
    result = {'status': 'failed', 'size': list(size), 'language': language,
              'route': route, 'protocol': protocol, 'form': form, 'cases': [],
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
            (journey.project / '.env').write_text('TEST_ONLY_PRIVATE_BODY\n')
            (root / 'outside-sibling.rs').write_text('outside the current project\n')
            journey.setup_route()
            journey.session.command(form)
            journey.workspace()

            def assert_form():
                snapshot = journey.snapshot()
                lead = snapshot['tasks'][0]['lead_partner']
                partner = next(row for row in snapshot['partners'] if row['id'] == lead)
                assert bool(partner['auto_form']) is (form == 'eevee'), partner
                if form != 'eevee':
                    assert partner['form'] == FORMS.index(form), partner

            def assert_project(body):
                policy = body['instructions'] if protocol == 'responses' else body['messages'][0]['content']
                envelope = json.loads(policy.split('UNTRUSTED CURRENT TASK/SOURCE DATA (JSON):\n', 1)[1])
                context = envelope['project_context']
                assert context['root'] == str(journey.project.resolve()), context
                names = context['filenames']
                assert names['status'] == 'success' and names['complete'] is True, names
                assert 'seed.rs' in names['paths'], names
                assert '.env' not in names['paths'] and 'outside-sibling.rs' not in names['paths'], names
                assert 'TEST_ONLY_PRIVATE_BODY' not in policy and 'pub const SEED' not in policy
                assert envelope['selected_sources'] == [], 'filename observation became an attachment'
                if (journey.project / TARGET).exists():
                    assert TARGET in names['paths'], 'a later request retained a stale filename observation'

            def start(request, rounds, reply, *, writing=True):
                gate = Gate()
                gates.append(gate)
                plans = [response_plan(peer, calls=calls, number=index,
                                       gate=gate if index == 0 else None)
                         for index, calls in enumerate(rounds)]
                plans.append(response_plan(peer, text=reply, number=len(rounds)))
                before = journey.begin(request, plans, writing=writing)
                until(journey.session, gate.ready.is_set, 'owned tool round did not reach its gate')
                assert_project(peer.snapshot('POST')[before]['body'])
                assert_form()
                gate.release.set()
                return before, len(plans)

            def decide(path, allow, original):
                nonlocal sequence
                sequence += 1
                session = journey.session
                unavailable = ('review all scope before allowing', '전체 범위를 확인한 뒤')
                until(session, lambda: file_review_pending(session),
                      'file permission did not present automatically', timeout=40)
                target = journey.project / path
                current = lambda: target.read_bytes() if target.is_file() else None
                assert current() == original, 'proposal changed bytes before consent'
                posts = len(peer.snapshot('POST'))
                journey.capture(f'{sequence}-automatic-file-popup')
                session.send('ay')
                session.paste('a\ny')
                assert current() == original and len(peer.snapshot('POST')) == posts
                assert file_review_pending(session), 'arrival typing became consent'
                if sequence == 1:
                    session.send('\t')
                    session.send('\t')
                else:
                    session.send(RIGHT if allow else LEFT)
                until(session, lambda: not file_review_pending(session),
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
            result['cases'].append('current-folder filename context excluded private/outside paths; actual source discovery used a nonwriting lease in the selected form')
            journey.paths([])
            assert_form()
            result['cases'].append('optional path restriction opened directly without changing form, attaching context or granting writes')

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
                                    'replacements': [{'old_text': 'true', 'new_text': 'false'}]}}],
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
            body = peer.snapshot('POST')[before]['body']
            assert edit_request in json.dumps(body, ensure_ascii=False), 'same-policy form change erased the previous request context'
            decide('handoff.md', True, None)
            journey.settled(reply, before + count)
            assert (journey.project / 'handoff.md').read_bytes() == handoff.encode()
            result['cases'].append('explicit handoff filename advertised mutation tools and required its own exact file permission')
            assert_form()
            result['cases'].append('same-policy specialty transition retained the prior canonical request context')
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
    parser.add_argument('--all-forms', action='store_true',
                        help='Exercise automatic mode and every manually fixed form')
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    results = []
    with tempfile.TemporaryDirectory(prefix='ev-file-permission-') as temporary:
        for form in FORMS if args.all_forms else ('eevee',):
            for route, protocol in [('openai', 'responses'), ('compatible', 'chat_completions')]:
                for size in [(120, 36), (60, 18)]:
                    label = f'{route}-{size[0]}' + (f'-{form}' if args.all_forms else '')
                    results.append(run_case(args.binary.resolve(), output / label,
                                            Path(temporary) / label, size, args.language, route, protocol, form))
    report = {'status': 'passed' if all(row['status'] == 'passed' for row in results) else 'failed',
              'cases': results}
    save(output / 'results.json', report)
    print(json.dumps({'status': report['status'], 'output': str(output)}))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
