#!/usr/bin/env python3
"""Credential/account-free usage and USD UI proof using an actual PTY and HTTP/SSE.

Run against an already-built executable. Native OpenAI setup uses a public fake
API token and an explicitly selected loopback endpoint; no real credential,
account, external provider, paid driver, SQLite write or billing data is used.
The custom usage.toml prices are controlled test inputs, not current list prices.
Dependencies and terminal restoration checks are shared with terminal_smoke.py.
"""
from __future__ import annotations

import argparse
from datetime import date
import hashlib
from html import escape
import json
from pathlib import Path
import re
import tempfile
import traceback

from native_connection_smoke import Journey, MODEL, Peer, decoded_payload, save_json, until
from response_smoke import (DRAFT, ESC, PAGE_DOWN, PAGE_UP, SurfaceJourney,
                            response_view, saved_request)
from terminal_smoke import confirm_exit
from workbench_smoke import wait_for

HOME, END = ESC + "[H", ESC + "[F"
PUBLIC_TOKEN = "public-synthetic-usage-smoke-token-not-a-credential"
UNPRICED_MODEL = "fixture-unpriced"
FIRST_ANSWER = '\n'.join(['USAGE_FIRST_ANSWER',
                          *(f'U{index:02} evidence.' for index in range(40)), 'USAGE_FIRST_END'])
PRICES = f'''# Synthetic deterministic test prices, not provider list prices.
meter = true
[[price]]
provider = "openai"
model = "{MODEL}"
input = 2.0
cached_input = 0.6
output = 8.0
'''
LIMITS = [
    "Controlled loopback HTTP/SSE reports only; not live-provider protocol, model quality, account entitlement, receipts or billing qualification.",
    "The public fake token only satisfies native setup; no real credentials or accounts are read or used. All homes/data/PATH are isolated.",
    "USD amounts use explicit synthetic usage.toml rates, not a claim about current provider prices or FX. Built-in source/date/staleness is inspected separately.",
    "Screenshots are SVG renders of actual drained PTY cells, not GUI-terminal or physical-display captures.",
    "No plan/agent-route billing exercise: this existing native HTTP peer does not implement account or agent protocols. No paid Codex usage driver is invoked.",
    "No provider tool call is emitted. Inspection checks no hidden inference, local execution or ownership changes; it does not qualify an interactive tool-permission approval journey.",
]


class UsageJourney(SurfaceJourney):
    def setup(self, *, initial=False):
        # Reuse current primary/Advanced connection controls rather than the
        # compatible-only response fixture's setup field order.
        return Journey.setup(self, secret=PUBLIC_TOKEN, initial=initial)

    def capture(self, label):
        super().capture(label)
        session = self.session
        session.capture(label)
        rows = session.display().splitlines()
        width, height = session.screen.columns * 9 + 20, session.screen.lines * 18 + 20
        content = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
                   '<rect width="100%" height="100%" fill="#111827"/>',
                   '<g fill="#e5e7eb" font-family="monospace" font-size="15" xml:space="preserve">']
        content.extend(f'<text x="10" y="{25 + index * 18}">{escape(row)}</text>'
                       for index, row in enumerate(rows))
        content.extend(['</g>', '</svg>'])
        (self.output / (label + '.svg')).write_text('\n'.join(content) + '\n', encoding='utf-8')
        save_json(self.output / (label + '-cells.json'), {
            'columns': session.screen.columns, 'lines': session.screen.lines,
            'rows': rows, 'cursor': {'x': session.screen.cursor.x, 'y': session.screen.cursor.y,
                                      'hidden': session.screen.cursor.hidden},
            'evidence': 'actual PTY decoded cells; no simulated application frames',
        })


def usage(input_tokens, output_tokens, cached, reasoning):
    return {'input_tokens': input_tokens, 'output_tokens': output_tokens,
            'input_tokens_details': {'cached_tokens': cached, 'cache_write_tokens': 0},
            'output_tokens_details': {'reasoning_tokens': reasoning},
            'total_tokens': input_tokens + output_tokens}


def plan_response(peer, number, answer, counters, model=MODEL):
    def respond(handler, event, row):
        response = {'id': f'resp_usage_{number}', 'model': model, 'status': 'completed',
                    'output': [{'type': 'message', 'role': 'assistant',
                                'content': [{'type': 'output_text', 'text': answer}]}]}
        if counters is not None:
            response['usage'] = counters
        frames = [{'type': 'response.output_text.delta', 'delta': answer},
                  {'type': 'response.completed', 'response': response}]
        # Identical terminal snapshots must not double-charge one response.
        if counters is not None:
            frames.append(frames[-1])
        row['fixture_events'] = frames
        for frame in frames:
            event(frame)
    peer.plans.put(('custom', respond))


def send_turn(journey, number, text, answer, counters, expected_history, model=MODEL):
    journey.write_request(text)
    before = len(journey.peer.snapshot('POST'))
    plan_response(journey.peer, number, answer, counters, model)
    journey.session.send('\r')
    until(journey.session, lambda: len(journey.peer.snapshot('POST')) == before + 1,
          'one explicitly submitted usage turn')
    wait_for(journey.session, answer.splitlines()[-1])
    until(journey.session, lambda: not journey.snapshot()['claims']
          and 'Storage: saved' in journey.session.display(), 'usage turn ownership/storage settlement')
    row = journey.peer.snapshot('POST')[-1]
    _, messages = decoded_payload('responses', row['body'])
    assert messages == expected_history + [('user', text)], messages
    assert row['body']['model'] == MODEL, 'selected custom model did not control actual request'
    assert journey.snapshot()['executions'] == 0, 'tool-free fixture created command execution'
    journey.capture(f'turn-{number}-settled')
    return expected_history + [('user', text), ('assistant', answer)]


def normalized(text):
    return ' '.join(text.split())


def scope_block(view, label):
    # Scope labels are selectors; assertions below cover consumer-visible
    # numeric boundaries, not full wording/layout snapshots.
    match = re.search(r'\b' + re.escape(label) + r'\b', view)
    if match is None:
        return None
    tail = view[match.end():]
    boundary = re.search(r'\b(?:Latest request|Task|This launch|BY PARTNER|BY MODEL|PRICE SOURCE)\b', tail)
    return normalized(view[match.start():match.end()] + (tail[:boundary.start()] if boundary else tail))


def assert_scope(block, expected):
    assert block is not None, expected
    if expected.get('unknown'):
        assert 'tokens unreported' in block and 'USD unknown' in block, block
        assert not re.search(r'\$0(?:\.0+)?(?:\D|$)|\b0 tok\b', block), block
        return
    assert re.search(r'(?<![\w.])' + re.escape(expected['tokens']) + r' tok\b', block), block
    assert expected['cost'] in block, block
    if '$' in expected['cost']:
        assert 'est.' in block and 'USD' in block, block
    else:
        assert 'USD unknown' in block and '$' not in block, block
    for label, count in [('Input', expected['input']), ('Output', expected['output']),
                         ('Reasoning', expected['reasoning'])]:
        assert re.search(r'\b' + label + r'\s+(?:≥|>=)?' + str(count) + r'\b', block), block
    assert re.search(r'\bCached read\s+' + str(expected['cached']) + r'\b', block), block
    assert re.search(r'\bwrite\s+0\b', block), block
    if expected.get('lower_bound'):
        assert '≥' in block or '>=' in block, block


def inspect(journey, label, expected, *, source=False):
    session, peer = journey.session, journey.peer
    before_posts, before_gets = len(peer.snapshot('POST')), len(peer.snapshot('GET'))
    before = journey.snapshot()
    caret = (session.screen.cursor.x, session.screen.cursor.y)
    editing = not session.screen.cursor.hidden
    session.command('usage')
    wait_for(session, 'Latest request')
    session.send(HOME)
    observed, frames = {}, []
    for page in range(200):
        view = session.display()
        frames.append(view)
        journey.capture(f'{label}-page-{page:02}')
        for scope, wanted in expected.items():
            block = scope_block(view, scope)
            if block is not None:
                try:
                    assert_scope(block, wanted)
                except AssertionError:
                    # A page may expose only a scope heading or its last row;
                    # one-line navigation below must reach the complete group.
                    continue
                observed[scope] = block
        session.send(PAGE_DOWN if set(observed) == set(expected) else ESC + '[B')
        if session.display() == view:
            break
    else:
        raise AssertionError('usage details did not reach a clamped bottom')
    assert set(observed) == set(expected), (expected, observed, '\n'.join(frames))
    all_text = normalized('\n'.join(frames))
    if source:
        assert 'PRICE SOURCE' in all_text and 'usage.toml' in all_text, all_text
        checked = re.search(r'Built-in list prices as of (\d{4}-\d{2}-\d{2})', all_text)
        assert checked is not None, 'price source date unreachable through real scrolling'
        if (date.today() - date.fromisoformat(checked[1])).days > 90:
            assert 'stale' in all_text.lower(), 'old built-in list date lacked stale disclosure'
        assert re.search(r'1\s+user', all_text, re.IGNORECASE) or re.search(r'user[^.]{0,35}\b1\b', all_text, re.IGNORECASE), all_text
        assert 'billing' in all_text.lower(), 'local estimate/billing boundary unreachable'
        assert 'Budget is warning-only' in all_text, 'bottom usage details were not reachable'
    bottom = session.display()
    session.send(END)
    assert session.display() == bottom, 'End changed already-clamped usage bottom'
    session.send(PAGE_UP)
    assert session.display() != bottom, 'PgUp did not move usage details'
    session.send(PAGE_DOWN)
    session.send(HOME)
    wait_for(session, 'Latest request')
    session.send('\r')  # Inspection Enter is not consent to send a future draft.
    assert 'Latest request' in session.display(), session.display()
    session.send(ESC)
    if editing:
        assert not session.screen.cursor.hidden
        assert (session.screen.cursor.x, session.screen.cursor.y) == caret, 'usage moved future draft caret/focus'
    assert len(peer.snapshot('POST')) == before_posts, 'usage navigation dispatched inference'
    assert len(peer.snapshot('GET')) == before_gets, 'usage navigation fetched provider metadata'
    journey.assert_owned_unchanged(before)
    return {'scopes': observed, 'actual_post_count': before_posts,
            'pages': len(frames), 'price_source_reached': source,
            'navigation_no_dispatch_or_execution': True}


def totals(tokens, input_tokens, output_tokens, cached, reasoning, cost, **extra):
    return {'tokens': tokens, 'input': input_tokens, 'output': output_tokens,
            'cached': cached, 'reasoning': reasoning, 'cost': cost, **extra}


def run(binary, output, root, size, ascii_mode):
    root.mkdir()
    data = root / 'data'
    data.mkdir(mode=0o700)
    (data / 'usage.toml').write_text(PRICES, encoding='utf-8')
    save_json(output / 'fixture-prices.json', {'file_contents': PRICES,
              'units': 'synthetic USD per million reported tokens',
              'first_usd': 0.00246, 'second_usd': 0.001316, 'sum_usd': 0.003776})
    checks = []
    with Peer('responses', PUBLIC_TOKEN, False, retain_body_bytes=True) as peer:
        journey = UsageJourney(binary, output, root, size, ascii_mode, 'openai',
                               'responses', False, False, peer)
        session = journey.session
        try:
            wait_for(session, 'Storage: saved')
            journey.setup(initial=True)
            history = send_turn(journey, 1, 'First explicit usage fixture turn.',
                                FIRST_ANSWER, usage(500, 200, 100, 80), [])
            first = totals('700', 500, 200, 100, 80, 'est. $0.0025')
            checks.append(inspect(journey, 'first', {
                'Latest request': first, 'Task': first, 'This launch': first}))
            history = send_turn(journey, 2, 'Second explicit usage fixture turn.',
                                'USAGE_SECOND_ANSWER', usage(300, 100, 60, 40), history)
            second = totals('400', 300, 100, 60, 40, 'est. $0.0013')
            cumulative = totals('1.1k', 800, 300, 160, 120, 'est. $0.0038')
            session.paste(DRAFT)
            session.send((ESC + '[D') * 3)
            session.send('\x13')
            until(session, lambda: saved_request(journey, DRAFT, len(DRAFT[:-3].encode())),
                  'future Unicode draft and insertion caret saved before opening usage')
            session.send(PAGE_UP)
            anchor = response_view(session)
            assert not anchor['following'], 'fixture transcript did not pause before usage'
            checks.append(inspect(journey, 'second-unicode-draft', {
                'Latest request': second, 'Task': cumulative, 'This launch': cumulative}, source=True))
            assert response_view(session) == anchor, 'usage open/close changed paused transcript anchor'
            session.send('!')
            exact = DRAFT[:-3] + '!' + DRAFT[-3:]
            cursor = len((DRAFT[:-3] + '!').encode())
            session.send('\x13')
            until(session, lambda: saved_request(journey, exact, cursor),
                  'exact future Unicode bytes and original insertion caret saved after usage')
            journey.capture('unicode-draft-caret-restored')

            # A new actual task separates task totals from this process launch.
            journey.workspace()
            old_task_ids = {row['id'] for row in journey.snapshot()['tasks']}
            session.command('task-new')
            journey.workspace()
            session.command('provider-openai')
            if 'y Switch' in session.display():
                session.send('y')
            journey.workspace()
            history = send_turn(journey, 3, 'Missing usage is not free.',
                                'USAGE_MISSING_ANSWER', None, [])
            assert {row['id'] for row in journey.snapshot()['tasks']} > old_task_ids
            missing = {'unknown': True}
            lower = cumulative | {'lower_bound': True, 'cost': 'est. $0.0038+?'}
            checks.append(inspect(journey, 'missing-new-task', {
                'Latest request': missing, 'Task': missing, 'This launch': lower}))
            history = send_turn(journey, 4, 'Unknown model price is not zero USD.',
                                'USAGE_UNPRICED_ANSWER', usage(70, 30, 10, 12), history,
                                model=UNPRICED_MODEL)
            unpriced = totals('100', 70, 30, 10, 12, 'no price')
            checks.append(inspect(journey, 'unpriced', {
                'Latest request': unpriced,
                'Task': unpriced | {'lower_bound': True},
                'This launch': totals('1.2k', 870, 330, 170, 132,
                                      'est. $0.0038+?', lower_bound=True)}, source=True))
            assert len(peer.snapshot('POST')) == 4, 'duplicate usage snapshot or navigation caused a hidden request'
            assert len(peer.snapshot('GET')) == 1, 'unrequested metadata refresh'
            assert journey.snapshot()['executions'] == 0
            session.command('quit')
            confirm_exit(session)
            assert session.wait_exit() == 0
            peer.assert_clean()
            return {'status': 'passed', 'size': list(size), 'ascii': ascii_mode,
                    'checks': checks, 'actual_posts': 4, 'actual_metadata_gets': 1,
                    'unicode_draft_and_byte_caret_preserved': True,
                    'paused_transcript_anchor_preserved': True,
                    'terminal_restoration': 'Session.wait_exit checked raw/canonical, mouse, alternate-screen, paste and cursor modes'}
        finally:
            save_json(output / 'http-evidence.json', peer.snapshot())
            session.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path, help='Fresh retained evidence directory')
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    with binary.open('rb') as handle:
        binary_hash = hashlib.file_digest(handle, 'sha256').hexdigest()
    result = {'status': 'failed', 'binary': str(binary), 'sha256': binary_hash,
              'scope': 'actual PTY + controlled loopback Responses SSE + explicit synthetic local USD prices',
              'limits': LIMITS, 'journeys': []}
    try:
        with tempfile.TemporaryDirectory(prefix='ev-usage-空 格-') as temporary:
            for size, ascii_mode, label in [((120, 36), False, 'wide'), ((60, 18), True, 'compact')]:
                target = output / label
                target.mkdir()
                result['journeys'].append(run(binary, target, Path(temporary) / label, size, ascii_mode))
        result['status'] = 'passed'
    except BaseException:
        result['error'] = traceback.format_exc()
        raise
    finally:
        save_json(output / 'results.json', result)
    print(json.dumps(result, indent=2, ensure_ascii=False))


if __name__ == '__main__':
    main()
