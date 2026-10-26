#!/usr/bin/env python3
"""Real Wave-6 native TUI acceptance using an already-built binary.

All thirteen task IDs run at 120x36 Unicode and 60x18 ASCII. Detail assertions
inspect actual paged cells, not incidental first-frame captions. Uses the existing
Moves/Session and native loopback fixtures; no second executor, SQL writes,
installed provider, real credential, external request, or application build.
"""
from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import sqlite3
import subprocess
import sys
import tempfile
import time
import traceback
import unicodedata

from wcwidth import wcswidth

from terminal_smoke import Session, confirm_exit
from workbench_smoke import wait_for
from moves_smoke import Journey as MovesJourney, save, digest, RUN_REVIEW
from native_connection_smoke import Peer, MODEL, decoded_payload
from workflow_smoke import HOME, DELETE, UP, DOWN, RIGHT, LEFT, PAGE, ESC
from context_smoke import SOURCE_TITLE

FORMS = ('Eevee', 'Vaporeon', 'Jolteon', 'Flareon', 'Espeon', 'Umbreon',
         'Leafeon', 'Glaceon', 'Sylveon')
RECORD_TABLES = ('local_artifacts', 'diagnosis_transitions', 'form_routes',
                 'local_activity', 'local_settings', 'failure_signatures',
                 'failure_observations', 'failure_resolutions',
                 'instruction_candidates', 'instruction_sources',
                 'partner_pairings', 'provider_usage', 'provider_usage_events')
CASES = {
    'C03A': 'Exact-ID file/range chips, keyboard remove one, Unicode controls, restart remaining',
    'C03B': 'Captured preview, stale/missing dispatch refusal, excluded/link refusal, actual HTTP wire bytes',
    'T01A': 'Two native HTTP turns and process restart; repeated usage snapshots reconcile by actual request owner',
    'EV05': 'Three real artifact forms and producers, invalidation, none, restart, monochrome',
    'EV06': 'Two partner routes and final forms survive restart; dim empty visits; non-evidence/export/purge',
    'PK02': 'Per-partner summary, met date/no moves, SQL-distinct produced tasks, Bond count, compact paging',
    'EV08A': 'One composer line per actually owned terminal run; journal/restart no replay',
    'EV08B': 'Review/handoff/staleness event lines once, durable off leaves run facts unchanged',
    'PK05A': 'Same real failed test twice, one signature/two run observations, project isolation and purge',
    'PK05B': 'Supported diagnosis, observed manual source transition, matching passing run, recurrence historical lead',
    'EV09': 'Three actual dispatches, explicit S02 project approval, next-request substitution, durable hash dismiss',
    'EV17': 'Two explicit partner/form/model pairings, local restore, remove restores default, unavailable no network',
    'EV11': 'Actual failure/diff form ranking/reasons, all-form identical query match sets, exclusions',
}

CHECKER = '''import json, os, sys, traceback
from pathlib import Path
from calc import total
root = Path(__file__).resolve().parent
with (root / "invocations.jsonl").open("a", encoding="utf-8") as log:
    log.write(json.dumps({"pid": os.getpid(), "argv": sys.argv[1:], "cwd": os.getcwd()}) + "\\n")
try:
    actual = total(7, 2)
    assert actual == 9, f"observed total={actual}; inputs=7,2; expected=9"
except AssertionError:
    print("test records::total ... FAILED", flush=True)
    traceback.print_exc()
    print("calc.py:2:1: total implementation returned the wrong result", flush=True)
    sys.exit(1)
print("test records::total ... ok", flush=True)
'''
GOOD = 'def total(a, b):\n    return a + b\n'
BAD = GOOD.replace('a + b', 'a - b')

class RecordsSession(Session):
    def command(self, name):
        # Ctrl+K opens/resets the semantic chooser from any drawer. The base
        # helper guesses from an English footer, including while editing Request.
        self.send('\x0b')
        self.send(name)
        self.send('\r')

    def send(self, keys):
        super().send(keys)
        # A committed store row can precede the PTY frame acknowledging it.
        # Drain the real stream; never synthesize a redraw or another action.
        self.pump(0, settle=True)


class Journey(MovesJourney):
    """Extend existing actual UI helpers; persistence is observed read-only."""

    def __init__(self, binary, output, root, size, ascii_mode, results, peer, ):
        self.record_task = None
        self.wire_proof = None
        self.failed_runs = []
        self.activity_presentations = {}
        super().__init__(binary, output, root, size, ascii_mode, results, peer)
        self.record_task = self.task()['id']
        self.owner = self.task()['lead_partner']
        self.long_name = '資料 한글 é ' + '長い名前' * 12 + '.txt'
        self.source = self.project / 'calc.py'
        self.source.write_text(BAD, encoding='utf-8')
        (self.project / 'check.py').write_text(CHECKER, encoding='utf-8')
        (self.project / self.long_name).write_text('first\r\n東京 \\" exact\r\nthird\r\nlast\r\n', encoding='utf-8', newline='')
        (self.project / 'changed.py').write_text('value = 1\n', encoding='utf-8')
        (self.project / 'test_changed.py').write_text('from changed import value\nassert value == 1\n', encoding='utf-8')
        (self.project / '.gitignore').write_text('invocations.jsonl\nstop\n.env.local\n__pycache__/\nignored.py\n', encoding='utf-8')
        (self.project / 'ignored.py').write_text('excluded = True\n', encoding='utf-8')
        self.outside = root / 'outside.py'
        self.outside.write_text('outside = True\n', encoding='utf-8')
        (self.project / 'outside.py').symlink_to(self.outside)
        self.git_command('add', '--', 'calc.py', 'check.py', self.long_name, 'changed.py', 'test_changed.py', '.gitignore')
        self.git_command('-c', 'user.name=Local acceptance', '-c', 'user.email=local@example.invalid', 'commit', '-qm', 'records fixture')
        # Git on Darwin may precompose the tracked name, while the fixture and
        # filesystem spelling are decomposed. Source search is deliberately a
        # literal query of Git's inventory, not a Unicode-normalizing policy.
        requested_name = self.long_name
        tracked = self.git_command('ls-files', '-z').stdout.decode('utf-8').split('\0')
        canonical = [name for name in tracked
                     if unicodedata.normalize('NFC', name) == unicodedata.normalize('NFC', requested_name)]
        assert len(canonical) == 1, 'fixture must have one actual tracked Unicode identity'
        self.long_name = canonical[0]
        precompose = subprocess.run([self.git, 'config', '--get', 'core.precomposeUnicode'],
                                   cwd=self.project, capture_output=True, check=False, timeout=15)
        assert precompose.returncode in (0, 1), 'could not observe fixture Git Unicode configuration'
        save(self.output / 'source-fixture-identity.json', {
            'requested_name': requested_name, 'tracked_name': self.long_name,
            'requested_utf8_hex': requested_name.encode('utf-8').hex(),
            'tracked_utf8_hex': self.long_name.encode('utf-8').hex(),
            'core.precomposeUnicode': precompose.stdout.decode('utf-8').strip() or 'unset',
            'query_bytes': len(self.long_name.encode('utf-8')), 'query_limit_bytes': 1024,
            'tracked': True, 'exists': (self.project / self.long_name).is_file(),
            'refresh': 'explicit post-commit repository refresh acknowledged before source selection',
        })
        assert (self.project / self.long_name).is_file()
        assert len(self.long_name.encode('utf-8')) < 1024, 'fixture exceeds literal source query bound'
        self.session.command('diff')
        wait_for(self.session, 'Repository', timeout=35)
        self.session.command('refresh')
        wait_for(self.session, 'Repository: refreshed', timeout=35)
        self.home_surface()

    def git_command(self, *args):
        return subprocess.run([self.git, '-c', 'core.autocrlf=false', '-c', 'core.hooksPath=', *args],
                              cwd=self.project, check=True, capture_output=True, timeout=15)

    def launch(self):
        self.serial += 1
        self.session = RecordsSession([str(self.binary), '--data-dir', str(self.data), '--images', 'off',
                                '--color', 'mono', *(['--ascii'] if self.ascii_mode else [])],
                               self.project, self.environment, self.size,
                               self.output / ('terminal-%02d' % self.serial), controlling_tty=True)
        self.sessions.append(self.session)
        wait_for(self.session, 'Storage: saved', timeout=35)

    def state(self):
        state = super().state()
        with closing(sqlite3.connect(self.database.as_uri() + '?mode=ro', uri=True, timeout=1)) as db:
            db.execute('PRAGMA query_only=ON')
            db.row_factory = sqlite3.Row
            available = {row[0] for row in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
            for table in (*RECORD_TABLES, 'provider_preferences'):
                assert table in available, 'required Schema15 records table missing: ' + table
                state[table] = [dict(row) for row in db.execute('SELECT * FROM "' + table + '" ORDER BY rowid')]
                for row in state[table]:
                    if 'record' in row:
                        row['record'] = json.loads(row['record'])
        return state

    def rows(self, table, project=None):
        project = self.project if project is None else Path(project)
        return [row for row in self.state()[table]
                if 'project' not in row or Path(row['project']).samefile(project)]

    def records(self, table):
        return [row['record'] for row in self.rows(table)]

    def wait_caption(self, english, timeout=35):
        # Navigation only: assertions below still inspect real screen cells,
        # retained facts and wire bytes, never a translated/fabricated display.
        wait_for(self.session, english, timeout=timeout)

    def workspace(self, allow_offer=False):
        # Only identified, non-consent drawers may be closed. Each actual
        # permission/memory/resolution review below has its own explicit action.
        prefixes = ('Type Request /', 'Type a request /', 'Enter Send /', 'Draft only /',
                    )
        known = (SOURCE_TITLE, 'Specialist desk', 'Local command evidence; no command running',
                 'Approved memories', 'Task cards / committed snapshots', 'Repository / read-only evidence',
                 'Partner summary', 'Form routes', 'Failure Dex',
                 'Ribbons / project history', 'Repeated authored instructions', 'Partner provider pairings',
                 'Portable task handoff / local-only', 'Party roster', 'Espeon /', 'Flareon /',
                 'Glaceon /', 'Model /', )
        for _ in range(6):
            view = self.session.display()
            footer = view.rstrip().splitlines()[-1].strip()
            if footer.startswith(prefixes):
                return 'Type Request /' if footer.startswith(('Type Request /', 'Type a request /', )) else 'Enter Send /'
            assert any(title in view for title in known), 'refusing to dismiss unknown popup\n' + view
            consent = (RUN_REVIEW, 'Confirm local change', 'wants to learn a move!',
                       'LOCAL PREVIEW / no provider connection authorized',
                       'PROVIDER REQUEST / explicit response required',
                       'Review historical resolution link', 'APPROVAL PREVIEW', )
            assert not any(marker in view for marker in consent), \
                'an explicit consent decision is required\n' + view
            self.session.send(ESC)
        raise AssertionError('identified drawers did not return to Request\n' + self.session.display())

    def home_surface(self):
        return self.workspace()

    def form(self, name):
        self.workspace()
        self.session.command(name)
        expected = next(index for index, form in enumerate(FORMS) if form.lower() == name)
        self.until(lambda: self.task()['form'] == expected, 'actual saved form ' + name)
        wait_for(self.session, 'Storage: saved')
        self.workspace()

    def capture(self, label):
        state = super().capture(label)
        save(self.output / 'http-requests.json', self.peer.snapshot())
        return state

    def open_records(self, command):
        self.workspace()
        self.session.command(command)
        titles = {'dex': ('Failure Dex', ),
                  'routes': ('Form routes', ),
                  'partner-summary': ('Partner summary', ),
                  'habits': ('Repeated authored instructions', )}
        self.wait_caption(*titles[command])
        if command in ('routes', 'partner-summary'):
            count = len(self.state()['partners'])
        elif command == 'dex':
            count = len(self.rows('failure_signatures'))
        else:
            sources = self.rows('instruction_sources')
            count = sum(not row['dismissed'] and row['memory'] is None
                        and len({source['record']['request_id'] for source in sources
                                 if source['signature'] == row['key']
                                 and source['record']['task_id'] > 0
                                 and source['record']['request_id']}) >= 3
                        for row in self.records('instruction_candidates'))
        position = r'Entr(?:y|ies) '
        position += str(min(count, 1)) + r' / ' + str(count) + r'(?!\d)'
        self.until(lambda: re.search(position, self.session.display()) is not None,
                   'records selection count matches actual committed local rows')

    def visual_cells(self, y, left, right):
        # Match Session.display: wide-glyph continuation cells may retain stale
        # diff data and must not be exported as additional visible characters.
        cells, x = [], left
        while x < right:
            cell = self.session.screen.buffer[y][x]
            data = cell.data or ' '
            cells.append((unicodedata.normalize('NFC', data), bool(cell.bold)))
            x += max(1, wcswidth(data))
        return cells

    def visual_row(self, y, left, right):
        return ''.join(data for data, _ in self.visual_cells(y, left, right))

    def detail_rows(self, surface, full=False):
        titles = {
            'sources': (SOURCE_TITLE, ),
            'records': ('Partner summary', 'Form routes', 'Failure Dex', 'Repeated authored instructions',
                        'Partner provider pairings',
                        'Review historical resolution link',
                        ),
            'memory': ('Approved memories', ),
        }[surface]
        screen = self.session.screen
        top = next(y for y, line in enumerate(self.session.display().splitlines())
                   if any(title in line for title in titles) and any(c in line for c in ('┌', '╔', '╭', '+')))
        cells = screen.buffer[top]
        title_x = next(x for x in range(screen.columns)
                       if any(self.visual_row(top, x, screen.columns).startswith(title)
                              for title in titles))
        left = max(x for x in range(title_x) if cells[x].data in ('┌', '╔', '╭', '+'))
        right = next(x for x in range(title_x + 1, screen.columns)
                     if cells[x].data in ('┐', '╗', '╮', '+'))
        bottom = next(y for y in range(top + 1, screen.lines)
                      if screen.buffer[y][left].data in ('└', '╚', '╰', '+'))
        height = bottom - top - 1
        if surface == 'sources':
            first, last = top + 3 + min(max(height // 3, 1), 7), bottom - 2
        elif surface == 'records':
            first, last = (top + 1, bottom) if full else (top + 4, bottom - 3)
        else:
            # This journey explicitly enables verbose memory details (3-row footer).
            first, last = top + 5, bottom - 3
        assert last > first, 'detail viewport is not inspectable'
        return [self.visual_row(y, left + 2, right - 1).rstrip()
                for y in range(first, last)]

    def pages_text(self, label, count=40, surface='records'):
        observed = []
        previous = None
        for index in range(count):
            text = self.session.display()
            rows = self.detail_rows(surface) if surface else text.splitlines()
            # Autosave/status changes outside the detail are not another page.
            if rows == previous:
                break
            # Keep overlapping final pages once, without injecting repeated
            # headers, borders or the underlying Request into wrapped detail.
            overlap = max((n for n in range(1, min(len(rows), len(observed)) + 1)
                           if observed[-n:] == rows[:n]), default=0)
            observed.extend(rows[overlap:])
            self.capture(label + '-%02d' % index)
            previous = rows
            self.session.send(PAGE)
        else:
            raise AssertionError('detail paging did not reach its actual end: ' + label)
        # Home selects a different capture in Sources; only documented detail
        # paging is safe there. Memory also has its own paging controls.
        self.session.send('\x1b[5~' * count if surface in ('sources', 'memory') else HOME)
        return '\n'.join(observed)

    def assert_wrapped_payload(self, text, expected):
        marker = 'Exact captured bytes / escaped display (not reread from the current file):'
        # Locate only the actual preview body, not another occurrence in its
        # metadata. The marker itself may wrap anywhere at a narrow viewport.
        pattern = r'\s*'.join(re.escape(ch) for ch in marker if not ch.isspace())
        match = re.search(pattern, text)
        assert match is not None, 'actual escaped-preview marker is missing\n' + text
        rows = text[match.end():].splitlines()
        while rows and not rows[0].strip():
            rows.pop(0)
        while rows and not rows[-1].strip():
            rows.pop()
        assert rows, 'actual escaped preview is empty\n' + text
        # NFC is exclusively a visual terminal convention. Retained SQL text
        # and actual HTTP bytes are compared separately without normalization.
        remaining = unicodedata.normalize('NFC', expected)
        for index, row in enumerate(rows):
            visible = row.strip()
            assert visible and remaining.startswith(visible), \
                'actual ordered preview differs beyond physical wrapping\n' + text
            remaining = remaining[len(visible):]
            if index + 1 < len(rows):
                # Only a physical row boundary can consume wrapping whitespace.
                # Interior spaces, all escapes and every non-space must match.
                remaining = remaining.lstrip()
        assert not remaining, 'actual paged preview omitted stored content\n' + text

    def inspect_capture(self, capture, freshness, label):
        self.wait_caption(SOURCE_TITLE, )
        self.until(lambda: freshness in self.session.display(), 'explicit source freshness result')
        text = self.pages_text(label, surface='sources')
        compact = ''.join(text.split())
        # Rust Debug escapes combining marks in paths. Normalize only visual
        # comparisons; captured content/hash and wire bytes remain untouched.
        path = json.dumps(capture['path'], ensure_ascii=False)
        path = ''.join('\\u{' + format(ord(ch), 'x') + '}'
                       if unicodedata.category(ch) in ('Mn', 'Me') else ch for ch in path)
        assert unicodedata.normalize('NFC', ''.join(('#{} / {}'.format(capture['id'], path)).split())) in compact, text
        assert ''.join(('Task {} / root'.format(capture['task_id'])).split()) in compact, text
        assert unicodedata.normalize('NFC', ''.join(json.dumps(capture['root'], ensure_ascii=False).split())) in compact, text
        scope = 'Lines {}:{} inclusive / {} captured UTF-8 bytes'.format(
            capture['start_line'], capture['end_line'], len(capture['text'].encode('utf-8')))
        assert ''.join(scope.split()) in compact, text
        self.assert_wrapped_payload(text, json.dumps(capture['text'], ensure_ascii=False))
        retained = next(row['record'] for row in self.state()['excerpts']
                        if row['record']['id'] == capture['id'])
        assert retained == capture, 'inspection changed the retained source snapshot'
        assert retained['text'].encode('utf-8') == capture['text'].encode('utf-8')
        assert retained['source_sha256'] == capture['source_sha256']
        return text

    def pin_range(self, filename, lines=None):
        self.workspace()
        before = {row['record']['id'] for row in self.state()['excerpts']}
        self.session.command('files')
        wait_for(self.session, 'eligible files /', timeout=35)
        self.session.command('search')
        self.wait_caption('Filename filter', )
        self.replace_editor(self.query, filename)
        self.session.send('\r')
        self.query = filename
        wait_for(self.session, '1 eligible files /', timeout=35)
        self.session.send('\r')
        wait_for(self.session, 'Review local capture /')
        if lines:
            self.session.command('range')
            self.wait_caption('Source range / inclusive', )
            self.replace_editor('1:999999', lines)
            self.session.send('\r')
            wait_for(self.session, 'Lines ' + lines + ' inclusive')
        self.session.command('attach')
        self.until(lambda: len({row['record']['id'] for row in self.state()['excerpts']} - before) == 1,
                   'exact reviewed source capture committed')
        self.workspace()
        return next(row['record'] for row in self.state()['excerpts'] if row['record']['id'] not in before)

    def focus_chip(self, capture_id):
        self.workspace()
        count = len(self.state()['excerpts'])
        for _ in range(count + 1):
            self.session.send('\t')
            if '>#' + str(capture_id) + ' ' in self.session.display():
                return
        raise AssertionError('exact capture chip not focusable: ' + str(capture_id) + '\n' + self.session.display())

    def remove_chip(self, capture_id):
        self.focus_chip(capture_id)
        self.session.send(DELETE)
        self.wait_caption('Review local excerpt removal', )
        before = self.state()['excerpts']
        self.session.send('\r')
        assert self.state()['excerpts'] == before, 'neutral Enter removed a capture'
        # Re-enter the same explicit keyboard review after the neutral stay.
        self.workspace()
        self.focus_chip(capture_id)
        self.session.send(DELETE)
        self.wait_caption('Review local excerpt removal', )
        self.session.send(PAGE * 8 + RIGHT + '\r')
        self.until(lambda: all(row['record']['id'] != capture_id for row in self.state()['excerpts']),
                   'only the reviewed capture was removed')
        self.workspace()

    def produced_marker(self, form, producer):
        self.workspace()
        self.session.command('forms')
        self.session.send(HOME + RIGHT * (form + 1))
        self.until(lambda: 'produced' in self.session.display().lower()
                   and producer in self.session.display(), 'current produced marker with frozen producer')
        self.capture('produced-before-invalidation-' + FORMS[form])
        self.session.send(ESC)

    def attachments(self):
        self.form('vaporeon')
        self.full_capture = self.pin_range(self.long_name)
        self.range_capture = self.pin_range(self.long_name, '2:3')
        assert self.full_capture['id'] != self.range_capture['id']
        assert self.full_capture['path'] == self.range_capture['path'] == self.long_name
        assert self.full_capture['text'] == 'first\r\n東京 \\" exact\r\nthird\r\nlast\r\n'
        assert self.range_capture['text'] == '東京 \\" exact\r\nthird\r\n'
        self.produced_marker(1, next(row['producer']['name'] for row in self.records('local_artifacts')
                                    if row['reference'] == str(self.full_capture['id']) and row['kind'] == 'Capture'))
        draft = 'Retain this exact draft 漢字 and caret.'
        self.write_request(draft)
        self.focus_chip(self.full_capture['id'])
        controls = ('Enter inspect', 'Del remove')
        for control in controls:
            wait_for(self.session, control)
        self.capture('long-unicode-fixed-chip-controls')
        self.session.send('\r')
        self.inspect_capture(self.full_capture, 'CURRENT at last explicit worker check',
                             'full-capture-exact-preview')
        self.workspace()
        self.remove_chip(self.full_capture['id'])
        assert [row['record']['id'] for row in self.state()['excerpts']] == [self.range_capture['id']]
        self.restart()
        self.focus_chip(self.range_capture['id'])
        for control in controls:
            wait_for(self.session, control)
        state = self.task()['partner_drafts'][str(self.owner)]['drafts']
        assert any(row['field'] == 'Request' and row['text'] == draft
                   and row['cursor'] == len(draft.encode('utf-8')) for row in state), 'chip action lost exact draft/caret'
        self.session.send(ESC)

    def chip_validation(self):
        source = self.project / self.long_name
        original = source.read_bytes()
        source.write_bytes(original.replace('third'.encode(), 'edited third'.encode()))
        self.focus_chip(self.range_capture['id'])
        self.session.send('\r')
        self.until(lambda: 'STALE:' in self.session.display(), 'source worker must mark changed chip stale')
        self.inspect_capture(self.range_capture, 'STALE:', 'stale-chip-retains-captured-not-current-bytes')
        self.session.command('dispatch-preview')
        wait_for(self.session, 'Context unavailable or stale', timeout=35)
        assert not self.peer.snapshot('POST'), 'stale inspection dispatched a provider request'
        self.workspace()
        source.unlink()
        self.focus_chip(self.range_capture['id'])
        self.session.send('\r')
        self.until(lambda: 'UNAVAILABLE:' in self.session.display(), 'missing source must be unavailable')
        self.inspect_capture(self.range_capture, 'UNAVAILABLE:', 'missing-chip-retains-exact-stored-bytes')
        self.session.command('dispatch-preview')
        wait_for(self.session, 'Context unavailable or stale', timeout=35)
        self.capture('missing-chip-blocks-assembly')
        self.workspace()
        source.write_bytes(original)
        self.remove_chip(self.range_capture['id'])
        self.range_capture = self.pin_range(self.long_name, '2:3')
        for forbidden in ('.env.local', 'ignored.py', 'outside.py'):
            self.workspace()
            before = self.state()['excerpts']
            self.session.command('files')
            wait_for(self.session, 'eligible files /', timeout=35)
            self.session.command('search')
            self.wait_caption('Filename filter', )
            self.replace_editor(self.query, forbidden)
            self.session.send('\r')
            self.query = forbidden
            wait_for(self.session, '0 eligible files /', timeout=35)
            self.capture('excluded-or-outroot-' + forbidden.replace('.', '_'))
            assert self.state()['excerpts'] == before, 'excluded path became a chip'
            self.workspace()
        self.form('eevee')
        self.connect()
        self.session.command('model ' + MODEL)
        self.workspace()
        post = self.send_request('Inspect only the explicitly selected public range.', 'fixture-range-reply')
        raw = bytes.fromhex(post['body_bytes_hex'])
        assert json.loads(raw) == post['body']
        policy, messages = decoded_payload('chat_completions', post['body'])
        assert messages[-1] == ('user', 'Inspect only the explicitly selected public range.')
        context_text = policy.split('\nUNTRUSTED CURRENT TASK/SOURCE DATA (JSON):\n', 1)[1]
        context = json.loads(context_text)
        captures = context['selected_sources']['captures']
        assert len(captures) == 1 and captures[0]['id'] == self.range_capture['id']
        assert captures[0]['text'] == self.range_capture['text']
        # Decode the actual selected array span, preserving serializer whitespace.
        array_start = context_text.index('[', context_text.index('"captures"')) + 1
        while context_text[array_start].isspace():
            array_start += 1
        _, end = json.JSONDecoder().raw_decode(context_text, array_start)
        entry = context_text[array_start:end]
        escaped = json.dumps(entry, ensure_ascii=False)[1:-1].encode('utf-8')
        assert raw.count(escaped) == 1, 'selected entry is not represented exactly once in raw HTTP bytes'
        self.focus_chip(self.range_capture['id'])
        self.session.send('\r')
        self.inspect_capture(self.range_capture, 'CURRENT at last explicit worker check',
                             'actual-serialized-chip-contribution')
        self.wire_proof = {'capture_id': self.range_capture['id'], 'raw_request_bytes': len(raw),
                           'serialized_entry_bytes': len(escaped), 'other_bytes': len(raw) - len(escaped),
                           'raw_request_sha256': hashlib.sha256(raw).hexdigest()}
        save(self.output / 'wire-proof.json', self.wire_proof)
        self.workspace()
        self.remove_chip(self.range_capture['id'])
        (self.project / 'outside.py').unlink()
        self.session.command('diff')
        wait_for(self.session, 'Repository')
        self.session.command('refresh')
        wait_for(self.session, 'Repository: refreshed', timeout=35)
        self.workspace()

    def usage_turn(self, number):
        final = {'prompt_tokens': 100 + number, 'completion_tokens': 20 + number,
                 'prompt_tokens_details': {'cached_tokens': 30 + number},
                 'completion_tokens_details': {'reasoning_tokens': 5 + number}}
        self.peer.usage_plans.put({'id': 'records-response-' + str(number), 'snapshots': [final, final]})
        before = {row['request_id'] for row in self.records('provider_usage')}
        post = self.send_request('Public usage fixture turn ' + str(number), 'usage-reply-' + str(number))
        self.until(lambda: len({row['request_id'] for row in self.records('provider_usage')} - before) == 1,
                   'provider-reported usage committed before owner release')
        record = next(row for row in self.records('provider_usage') if row['request_id'] not in before)
        assert record['task_id'] == self.record_task and record['owner']['partner'] == self.owner
        assert len(record['measurements']) == 1, 'repeated final usage was counted as multiple scopes'
        measurement = record['measurements'][0]
        expected = {'input': 100 + number, 'output': 20 + number, 'cache_read': 30 + number,
                    'cache_write': None, 'reasoning': 5 + number}
        assert measurement['attributable'] == expected
        assert measurement['report']['model'] == MODEL and measurement['report']['tokenizer'] is None
        assert measurement['report']['identity']['response'] == 'records-response-' + str(number)
        assert measurement['report']['semantics']['cache_read_in_input'] is True
        assert measurement['report']['semantics']['reasoning_in_output'] is True
        assert post['body']['stream_options']['include_usage'] is True
        assert not self.state()['active_work'], 'usage write released neither actual owner nor active work'
        self.capture('usage-turn-' + str(number))
        return record

    def usage(self):
        first, second = self.usage_turn(1), self.usage_turn(2)
        policy, messages = decoded_payload('chat_completions', self.peer.snapshot('POST')[-1]['body'])
        assert ('assistant', 'usage-reply-1') in messages, 'second turn lost process-local native history'
        before = self.records('provider_usage')
        self.restart()
        assert self.records('provider_usage') == before, 'restart replayed or dropped reported usage'
        # Session-only keyless fixture connection is deliberately reapproved, not
        # mistaken for an opaque CLI provider resume or user credential restore.
        self.connect_again()
        third = self.usage_turn(3)
        _, messages = decoded_payload('chat_completions', self.peer.snapshot('POST')[-1]['body'])
        assert messages == [('user', 'Public usage fixture turn 3')], 'native process-local history replayed across restart'
        save(self.output / 'usage-boundary.json', {'kind': 'credential-free loopback native HTTP, not live',
             'first_two_turns': [first, second], 'after_process_restart': third,
             'opaque_provider_resume': 'not exercised; parent-owned live Codex gate',
             'billing': 'unknown; no price or cost asserted'})

    def connect(self):
        self.connect_again()
        assert not self.peer.snapshot('POST'), 'connection setup dispatched inference'

    def connect_again(self):
        # Existing Moves connect helper asserts the first connection has no POSTs;
        # reuse its exact keyless setup by checking the immutable POST prefix here.
        before = self.peer.snapshot('POST')
        self.prepare_connection()
        self.session.command('connect compatible')
        self.wait_caption('API key:', )
        self.session.send('\t\x15')
        self.session.paste(self.peer.base)
        self.session.send('\t\x15')
        self.session.paste(MODEL)
        self.session.send('\t\t \t\t\r')
        wait_for(self.session, MODEL, timeout=35)
        self.wait_caption('1 matches', )
        self.session.send(MODEL + '\r')
        self.workspace()
        assert self.peer.snapshot('POST') == before, 'reconnecting sent hidden inference'

    def execute_check(self, trial, expected):
        self.form('jolteon')
        self.edit_command([self.python, '-B', 'check.py', '--trial=' + str(trial)])
        self.session.command('execute')
        self.wait_caption(RUN_REVIEW, )
        self.capture('explicit-local-check-review-' + str(trial))
        run = self.approve_run(expected)
        self.session.send(ESC)
        if 'wants to learn a move!' in self.session.display():
            self.capture('unrelated-command-learning-explicit-not-now')
            self.session.send(ESC)  # Existing neutral Not now; never learns/runs.
        self.workspace()
        run_id = run['prepared']['run_id']
        event_id = 'activity:run:{}:{}'.format(run['prepared']['task_id'], run_id)
        self.until(lambda: any(row['id'] == event_id for row in self.records('local_activity')),
                   'one actual owned run activity committed')
        events = [row for row in self.records('local_activity') if row['id'] == event_id]
        assert len(events) == 1 and events[0]['owner'] == run['prepared']['partner']
        self.capture('finished-check-composer-line-' + str(trial))
        detail = events[0]['detail']
        assert events[0]['owner']['name'] in detail, 'factual line omitted the frozen acting partner'
        # The fixed composer row may elide a long command at sixty columns;
        # identity + observed outcome must still be visible exactly once.
        prefix = detail[:min(32, len(detail))]
        self.until(lambda: prefix in self.session.display(), 'finished owned run factual composer line')
        assert sum(prefix in line for line in self.session.display().splitlines()) == 1
        self.activity_presentations[events[0]['id']] = prefix
        return run

    def failures(self):
        self.failed_runs = [self.execute_check(1, 'Failed'), self.execute_check(2, 'Failed')]
        self.until(lambda: len(self.rows('failure_observations')) == 2, 'distinct failing runs indexed once')
        signatures = self.records('failure_signatures')
        assert len(signatures) == 1 and signatures[0]['kind'] == 'test'
        self.signature = signatures[0]['key']
        assert signatures[0]['detail'] == 'records::total'
        observations = self.rows('failure_observations')
        actual_runs = {run['prepared']['run_id'] for run in self.failed_runs}
        assert len(actual_runs) == 2 and {row['run_id'] for row in observations} == actual_runs
        assert all(row['signature'] == self.signature and row['task_id'] == self.record_task
                   for row in observations), 'failure count borrowed foreign task/signature observations'
        paths = {path for row in observations for path in json.loads(row['paths'])}
        assert 'calc.py' in paths and not {'outside.py', 'ignored.py', '.env.local'} & paths, \
            'actual retained failure locations were not safely projected for form ranking'
        self.produced_marker(2, self.failed_runs[-1]['prepared']['partner']['name'])
        self.open_records('dex')
        text = self.pages_text('one-signature-two-real-failures')
        normalized = ''.join(text.split())
        assert 'test:records::total' in normalized and self.signature in normalized, text
        count_text = normalized.split('test:records::total', 1)[1].split(self.signature, 1)[0]
        assert re.search(r'(?<!\d){}(?!\d)'.format(len(actual_runs)), count_text), text
        assert all(normalized.count(run_id) >= 1 for run_id in actual_runs), \
            'Dex must cite each actual retained failure run in its derived comparison\n' + text
        save(self.output / 'failure-signature-proof.json', {
            'signature': self.signature, 'task_id': self.record_task,
            'distinct_failed_runs': sorted(actual_runs), 'count': len(actual_runs),
            'visible_count_context': count_text, 'visible_detail': text,
        })
        self.workspace()

    def workflow_menu(self, index):
        self.session.send(UP * 24 + DOWN * index + '\r')

    def workflow_edit(self, index, label, value):
        self.workflow_menu(index)
        self.wait_caption(label, )
        self.session.paste(value)
        self.session.send('\x13')

    def workflow_attach(self, action, choice):
        self.workflow_menu(action)
        self.session.send(DOWN * choice + '\r')
        self.wait_caption('Specialist desk', )

    def resolution(self):
        self.form('umbreon')
        capture = self.pin_range('calc.py', '1:2')
        self.session.command('workflows')
        self.wait_caption('Specialist desk', )
        self.workflow_edit(0, 'Hypothesis / explanation', 'total subtracts b where the observed contract requires addition.')
        self.workflow_menu(1)
        self.until(lambda: len(self.task()['workflows']['hypotheses']) == 1, 'actual diagnosis committed')
        self.workflow_attach(3, 0)  # source then newest real run, existing workflow order
        self.workflow_attach(3, 1)
        wait_for(self.session, 'Supported')
        self.until(lambda: any(row['supported_diagnosis'] for row in self.records('local_artifacts')),
                   'currently supported diagnosis receipt retained')
        self.support = next(row for row in self.records('local_artifacts') if row['supported_diagnosis'])
        assert self.support['producer']['partner'] == self.owner and self.support['producer']['form'] == 5
        self.capture('supported-diagnosis-current-source-and-real-failure')
        self.workspace()
        self.produced_marker(5, self.support['producer']['name'])
        self.source.write_text(GOOD, encoding='utf-8')
        self.session.command('workflows')
        self.wait_caption('Specialist desk', )
        self.session.command('refresh')
        self.until(lambda: bool(self.rows('diagnosis_transitions')), 'manual source transition observed before passing run')
        self.capture('observed-manual-fix-author-unknown')
        self.workspace()
        # The supported diagnosis references the newest actual failing run.
        # A pass of a different --trial argv is intentionally not eligible.
        supported_failed = self.failed_runs[-1]['prepared']
        assert {'Run': {'task_id': self.record_task, 'run_id': supported_failed['run_id']}} in self.support['evidence']
        passed = self.execute_check(2, 'Succeeded')
        self.pass_id = passed['prepared']['run_id']
        assert passed['prepared']['spec'] == supported_failed['spec']
        self.open_records('dex')
        self.session.command('dex-resolve')
        self.wait_caption('Review historical resolution link', )
        self.until(lambda: re.search(r'Entr(?:y|ies) 1 / [1-9]\d*', self.session.display()) is not None,
                   'supported diagnosis and exact matching pass are actually eligible')
        def scope_visible():
            visible = ''.join(''.join(self.detail_rows('records', full=True)).split())
            return all(identity in visible for identity in
                       (self.signature, self.support['id'], self.pass_id))

        def affirmative_visible():
            return any("Enter" in row and "←→" in row
                       and re.search(r"/[^\n]*\[[^\]]+\]", row)
                       for row in self.session.display().splitlines())

        self.until(scope_visible, 'actual selected signature/support/pass scope is visible before paging')
        self.capture('resolution-visible-summary-neutral-enter-no-end-paging')
        before = self.rows('failure_resolutions')
        self.session.send('\r')
        assert self.rows('failure_resolutions') == before, 'neutral Enter inferred a resolution'
        # Neutral Enter remains in Resolve. Escape returns to Dex, whose
        # semantic chooser actually offers dex-resolve; do not nest a chooser.
        self.session.send(ESC)
        self.wait_caption('Failure Dex', )
        self.session.command('dex-resolve')
        self.wait_caption('Review historical resolution link', )
        first_rows = self.detail_rows('records')
        review = self.pages_text('explicit-supported-resolution-review')
        normalized = ''.join(review.split())
        assert self.support['id'] in normalized and self.pass_id in normalized, review
        self.session.send(HOME)
        self.until(lambda: self.detail_rows('records') == first_rows and scope_visible(),
                   'actual selected historical-link scope is visible again on the initial page')
        self.capture('resolution-visible-summary-confirm-without-end-paging')
        self.session.send(RIGHT)
        self.until(affirmative_visible, 'actual affirmative historical-link choice rendered')
        self.session.send('\r')
        self.until(lambda: len(self.records('failure_resolutions')) == 1, 'explicit supported relation committed')
        resolution = self.records('failure_resolutions')[0]
        assert resolution['signature'] == self.signature and resolution['support_artifact'] == self.support['id']
        assert resolution['passing_run'] == self.pass_id and 'unknown' in resolution['cause']
        committed_ack = 'Localmetadataonly;'
        self.until(lambda: committed_ack in ''.join(self.session.display().split()),
                   'records response rendered after the historical relation committed')
        committed = self.rows('failure_resolutions')
        self.wait_caption('Failure Dex', )
        self.session.command('dex-resolve')
        self.wait_caption('Review historical resolution link', )
        self.until(scope_visible, 'repeated relation has its actual selected scope visible without paging')
        self.session.send('\r')
        assert self.rows('failure_resolutions') == committed, 'neutral repeat changed the historical relation'
        self.session.send(RIGHT)
        self.until(affirmative_visible, 'actual repeated affirmative choice rendered')
        self.session.send('\r')
        self.wait_caption('Failure Dex', )
        self.until(lambda: committed_ack in ''.join(self.session.display().split()),
                   'repeated relation request actually acknowledged by committed records view')
        assert self.rows('failure_resolutions') == committed, 'explicit repeated relation was not deduplicated'
        self.capture('resolution-explicit-repeat-deduplicated-no-end-paging')
        historical = self.pages_text('pk07-supported-historical-pass')
        assert 'Historicalsupportedpass' in ''.join(historical.split()), historical
        self.workspace()
        self.source.write_text(BAD, encoding='utf-8')
        self.execute_check(2, 'Failed')
        self.open_records('dex')
        text = self.pages_text('recurrence-historical-lead-not-current-verification')
        normalized = ''.join(text.split())
        assert ('historical' in text.lower() and 'NOTacurrent' in normalized)
        assert str(self.record_task) in text
        assert self.support['id'] in normalized and self.pass_id in normalized
        assert 'Unresolvedrecurringfailure' in normalized, text
        self.workspace()

    def markers(self):
        expected = {1, 2, 5}
        artifacts = self.records('local_artifacts')
        assert expected <= {row['role_form'] for row in artifacts}
        assert all(row['producer'] and row['producer']['partner'] == self.owner for row in artifacts
                   if row['kind'] in ('Capture', 'Run', 'Hypothesis'))
        # A distinct observed content change invalidates all three retained
        # source/run/diagnosis supports, rather than reviving the old fingerprint.
        self.source.write_text(BAD + '# changed after recurrence\n', encoding='utf-8')
        self.session.command('workflows')
        self.wait_caption('Specialist desk', )
        self.session.command('refresh')
        latest = self.state()['executions'][-1]['record']['prepared']
        stale_id = 'stale:run:{}:{}'.format(latest['task_id'], latest['run_id'])
        self.until(lambda: any(row['id'] == stale_id for row in self.records('local_activity')),
                   'source worker observed current-to-stale verification transition')
        self.workspace()
        stale = next(row for row in self.records('local_activity') if row['id'] == stale_id)
        self.until(lambda: stale['detail'][:28] in self.session.display(), 'source-changed factual composer line')
        self.capture('one-observed-source-staleness-line')
        for phase in ('invalidated', 'restarted'):
            if phase == 'restarted':
                self.restart()
            self.session.command('forms')
            self.session.send(HOME + RIGHT)
            for index, form in enumerate(FORMS):
                if index:
                    self.session.send(RIGHT)
                text = self.session.display()
                self.capture('form-marker-%s-%s' % (phase, form))
                assert form in text, 'form picker selection is not inspectable'
                if index in expected:
                    assert 'stale' in text.lower(), (phase, form, text)
                    producers = {row['producer']['name'] for row in artifacts
                                 if row['role_form'] == index and row['producer']}
                    assert any(name in text for name in producers), (phase, form, producers, text)
                elif index in (0, 3, 4, 6, 7, 8):
                    assert 'none' in text.lower(), (phase, form, text)
            self.session.send(ESC)

    def routes(self):
        self.form('eevee')
        self.form('flareon')  # deliberately no exit artifact: this visit must be dim
        self.form('jolteon')
        self.session.command('party')
        self.session.send('a')
        self.until(lambda: len(self.state()['partners']) == 2, 'explicit second partner added')
        self.workspace()
        self.other = next(row['id'] for row in self.state()['partners'] if row['id'] != self.owner)
        self.lead(self.other)
        self.nickname('Mizu')
        self.form('vaporeon')
        self.form('glaceon')
        self.routes_before = self.records('form_routes')
        assert {row['owner']['partner'] for row in self.routes_before} == {self.owner, self.other}
        by_owner = {owner: [row for row in self.routes_before if row['owner']['partner'] == owner]
                    for owner in (self.owner, self.other)}
        assert [(row['from'], row['to']) for row in by_owner[self.owner]][-3:] == [(2, 0), (0, 3), (3, 2)]
        self.open_records('routes')
        dim_flareon, bold_produced = False, False
        for partner_index in range(2):
            if partner_index:
                self.session.send(DOWN)
            texts = []
            for page in range(40):
                view = self.session.display()
                texts.extend(self.detail_rows('records'))
                for y in range(self.session.screen.lines):
                    styled = [(character, bold) for data, bold in
                              self.visual_cells(y, 0, self.session.screen.columns) for character in data]
                    line = ''.join(character for character, _ in styled)
                    for match in re.finditer(r'Flareon|Jolteon', line):
                        span = [bold for _, bold in styled[match.start():match.end()]]
                        if match.group() == 'Flareon':
                            dim_flareon |= not any(span)
                        else:
                            bold_produced |= all(span)
                self.capture('routes-partner-%d-page-%d' % (partner_index, page))
                self.session.send(PAGE)
                if self.session.display() == view:
                    break
            text = '\n'.join(texts)
            normalized = ''.join(text.split())
            assert 'NOTevidence' in normalized, text
            if partner_index:
                assert 'Mizu' in text
            self.session.send(HOME)
        assert dim_flareon and bold_produced, 'empty and producing visits lack distinct actual monochrome span emphasis'
        self.workspace()
        self.task_card_routes('before-restart')
        self.restart()
        assert self.records('form_routes') == self.routes_before, 'startup hydration fabricated route visits'
        assert self.task()['lead_partner'] == self.other and self.task()['form'] == 7
        self.lead(self.owner)
        assert self.task()['form'] == 2, 'first partner did not restore its own final form'
        self.lead(self.other)
        assert self.task()['form'] == 7, 'second partner did not restore its own final form'
        self.task_card_routes('after-restart')
        self.lead(self.owner)

    def task_card_routes(self, phase):
        self.workspace()
        before = self.effects()
        self.session.command('tasks')
        wait_for(self.session, 'Task cards / committed snapshots')
        self.session.send(HOME)
        collected, screens = [], []
        for page in range(40):
            view = self.session.display()
            screens.append(view)
            display = view.splitlines()
            title_y = next(y for y, line in enumerate(display) if 'Task cards / committed snapshots' in line)
            title = 'Task cards / committed snapshots'
            title_cells = self.session.screen.buffer[title_y]
            title_x = next(x for x in range(self.session.screen.columns - len(title) + 1)
                           if ''.join(title_cells[x + n].data for n in range(len(title))) == title)
            left = max(x for x in range(title_x) if title_cells[x].data in ('┌', '╔', '╭', '+'))
            right = next(x for x in range(title_x + len(title), self.session.screen.columns)
                         if title_cells[x].data in ('┐', '╗', '╮', '+'))
            marker_y = next(y for y in range(title_y + 1, len(display))
                            if 'cards / /archived' in ''.join(
                                self.session.screen.buffer[y][x].data for x in range(left + 1, right)))
            rows = []
            for y in range(marker_y + 2, len(display)):
                cells = self.session.screen.buffer[y]
                if cells[left].data in ('└', '╚', '╰', '+'):
                    break
                rows.append(self.visual_cells(y, left + 1, right))
            # Detail pages may overlap on the final bounded scroll. Join actual
            # cell rows, not whole screenshots with repeated headers in between.
            overlap = max((n for n in range(1, min(len(rows), len(collected)) + 1)
                           if collected[-n:] == rows[:n]), default=0)
            collected.extend(rows[overlap:])
            self.capture('task-card-routes-%s-page-%02d' % (phase, page))
            self.session.send(PAGE)
            if self.session.display() == view:
                break
        text = '\n'.join(screens)
        disclosure = 'local non-evidence'
        assert disclosure in text, 'task card omitted route non-evidence label\n' + text
        cells = [(character, bold) for row in collected for data, bold in row
                 for character in data if not character.isspace()]
        normalized = ''.join(character for character, _ in cells)
        artifacts = self.records('local_artifacts')
        proof = []
        for partner in self.state()['partners']:
            owner = partner['id']
            routes = sorted((row for row in self.routes_before if row['task_id'] == self.record_task
                             and row['owner']['partner'] == owner), key=lambda row: row['ordinal'])
            assert routes, 'route journey must retain real visits for both partners'
            name = partner['nickname'] or 'Eevee #' + str(owner)
            chain = ''.join(name.split()) + ':'
            visits = []

            def visit(form, start, end):
                nonlocal chain
                token = FORMS[form]
                produced = any(row['task_id'] == self.record_task and row['ordinal'] > start
                               and (end is None or row['ordinal'] < end)
                               and row['producer'] and row['producer']['partner'] == owner
                               and row['producer']['form'] == form for row in artifacts)
                visits.append((len(chain), token, produced))
                chain += token

            visit(routes[0]['from'], 0, routes[0]['ordinal'])
            for index, route in enumerate(routes):
                if index and route['from'] != routes[index - 1]['to']:
                    chain += '/'
                    visit(route['from'], routes[index - 1]['ordinal'], route['ordinal'])
                chain += '->'
                visit(route['to'], route['ordinal'],
                      routes[index + 1]['ordinal'] if index + 1 < len(routes) else None)
            at = normalized.find(chain)
            assert at >= 0, ('task card lacks exact per-partner committed route chain', owner, chain, text)
            for offset, token, produced in visits:
                emphasis = [bold for _, bold in cells[at + offset:at + offset + len(token)]]
                assert emphasis == [produced] * len(token), \
                    ('task-card visit emphasis disagrees with actual frozen producer/ordinal', owner, token, produced, emphasis)
            proof.append({'partner': owner, 'chain': chain,
                          'visits': [{'form': token, 'produced': produced} for _, token, produced in visits]})
        assert self.records('form_routes') == self.routes_before, 'task-card inspection manufactured route visits'
        assert self.effects() == before, 'task-card route inspection performed work'
        save(self.output / ('task-card-routes-' + phase + '.json'), proof)
        self.workspace()

    def summary(self):
        with closing(sqlite3.connect(self.database.as_uri() + '?mode=ro', uri=True)) as db:
            produced = db.execute("SELECT json_extract(record,'$.producer.partner'), json_extract(record,'$.producer.form'), count(DISTINCT task_id) FROM local_artifacts WHERE project=? AND json_extract(record,'$.producer.partner') IS NOT NULL GROUP BY 1,2", (str(self.project),)).fetchall()
            bond = db.execute("SELECT count(*) FROM memories WHERE scope='personal' OR (project=? AND scope='project')", (str(self.project),)).fetchone()[0]
            moves = db.execute("SELECT partner,form,count(*) FROM moves WHERE forgotten=0 AND (scope='personal' OR project=?) GROUP BY partner,form", (str(self.project),)).fetchall()
        self.open_records('partner-summary')
        for index, partner in enumerate(self.state()['partners']):
            if index:
                self.session.send(DOWN)
            text = self.pages_text('partner-summary-sql-%d' % partner['id'])
            normalized = ''.join(text.split())
            if partner['met_at'] > 0:
                assert time.strftime('%Y-%m-%d', time.gmtime(partner['met_at'] / 1000)) in normalized
            else:
                assert '1970-01-01' not in normalized, 'unknown provenance became a fabricated epoch date'
            for form, name in enumerate(FORMS):
                tasks = next((count for owner, kind, count in produced if owner == partner['id'] and kind == form), 0)
                count = next((count for owner, kind, count in moves if owner == partner['id'] and kind == form), 0)
                expected = '{}:{}moves·{}distinctproducingtasks'.format(name, count, tasks)
                assert expected in normalized, (partner['id'], form, count, tasks, text)
            assert 'Bond:{}shared'.format(bond) in normalized
        save(self.output / 'summary-sql.json', {'produced_distinct_tasks': produced, 'moves': moves,
                                               'shared_bond_memories': bond})
        self.workspace()

    def run_lines(self):
        before = self.records('local_activity')
        run_id = self.state()['executions'][-1]['record']['prepared']['run_id']
        self.session.command('runs')
        self.until(lambda: run_id in self.session.display(),
                   'actual owned command journal is visible without starting work')
        self.capture('journal-replay-is-inspection-only')
        self.session.send(ESC)
        assert self.records('local_activity') == before
        self.restart()
        assert self.records('local_activity') == before
        assert not any(detail in self.session.display() for detail in self.activity_presentations.values()), 'startup replayed a prior activity'
        assert all(row['delivered'] == 1 for row in self.rows('local_activity'))

    def more_lines(self):
        self.form('glaceon')
        before = {row['id'] for row in self.records('local_activity')}
        self.session.command('workflows')
        self.wait_caption('Specialist desk', )
        self.workflow_edit(1, 'Finding detail', 'The actual local regression failed; behavior remains unverified.')
        self.workflow_edit(2, 'Actionable recommendation', 'Fix the selected source and rerun the same bounded check.')
        self.workflow_menu(4)
        self.workflow_menu(11)
        wait_for(self.session, 'RequestChanges')
        self.until(lambda: any(row['id'].startswith('review:') and row['id'] not in before for row in self.records('local_activity')), 'one committed effective review activity')
        self.workspace()
        event = next(row for row in self.records('local_activity') if row['id'].startswith('review:') and row['id'] not in before)
        self.until(lambda: event['detail'][:28] in self.session.display(), 'review verdict factual composer line')
        self.capture('review-verdict-factual-composer-line')
        self.form('sylveon')
        self.session.command('handoff')
        self.wait_caption('Portable task handoff / local-only', )
        self.session.command('export-json')
        self.wait_caption('LITERAL FILE PATH', )
        exported = self.root / 'records-handoff.json'
        self.session.paste(str(exported))
        self.session.send('\r')
        self.wait_caption('SANITIZED EXPORT PREVIEW', )
        self.session.send(PAGE * 24 + RIGHT + '\r')
        self.until(exported.exists, 'explicit local handoff export')
        self.until(lambda: any(row['id'].startswith('handoff:') for row in self.records('local_activity')), 'committed handoff-ready receipt activity')
        document = json.loads(exported.read_text(encoding='utf-8'))
        encoded = json.dumps(document)
        assert 'form_routes' not in encoded and all(route['id'] not in encoded for route in self.routes_before)
        self.workspace()
        event = next(row for row in self.records('local_activity') if row['id'].startswith('handoff:'))
        self.until(lambda: event['detail'][:28] in self.session.display(), 'handoff-ready factual composer line')
        self.capture('handoff-ready-factual-composer-line')
        assert any(row['id'].startswith('stale:') for row in self.records('local_activity')), 'observed verification invalidation has no factual event'
        stable = self.records('local_activity')
        self.open_records('routes')
        self.workspace()
        assert self.records('local_activity') == stable, 'inspection duplicated review/handoff/stale activities'
        self.session.command('flavor-off')
        self.until(lambda: self.rows('local_settings')[0]['flavor'] == 0, 'explicit persistent flavor off')
        self.restart()
        assert self.rows('local_settings')[0]['flavor'] == 0
        self.form('jolteon')
        self.edit_command([self.python, '-B', 'check.py', '--trial=off'])
        count = len(self.state()['executions'])
        self.session.command('execute')
        self.wait_caption(RUN_REVIEW, )
        run = self.approve_run('Failed')
        self.session.send(ESC)
        self.workspace()
        event_id = 'activity:run:{}:{}'.format(run['prepared']['task_id'], run['prepared']['run_id'])
        self.until(lambda: any(row['id'] == event_id for row in self.records('local_activity')), 'off still records and consumes actual event')
        event = next(row for row in self.records('local_activity') if row['id'] == event_id)
        assert event['detail'] not in self.session.display(), 'off rendered a partner activity line'
        assert len(self.state()['executions']) == count + 1 and run['exit_code'] == 1
        self.session.command('flavor-on')
        self.until(lambda: self.rows('local_settings')[0]['flavor'] == 1, 'explicit flavor on')
        assert event['detail'] not in self.session.display(), 're-enabling replayed an off-period event'

    def habits(self):
        self.form('eevee')
        self.connect_again()
        # EV05 deliberately left stale source evidence. Remove those exact
        # chips through ordinary neutral/explicit review; do not bypass send revalidation.
        for excerpt in self.state()['excerpts']:
            self.remove_chip(excerpt['record']['id'])
        instruction = 'Keep the exact public regression evidence; do not infer a successful check.'
        dismissed = 'Use a separate short public explanation for this fixture.'
        self.write_request(instruction)
        assert all(row['instruction'] != instruction for row in self.records('instruction_candidates')), 'queued/unsent draft became a habit source'
        for number in range(3):
            self.send_request(instruction, 'habit-reply-' + str(number))
        matching = [row for row in self.records('instruction_candidates') if row['instruction'] == instruction]
        assert len(matching) == 1, 'actual repeated instruction did not produce exactly one candidate'
        key = matching[0]['key']
        self.until(lambda: sum(row['signature'] == key for row in self.rows('instruction_sources')) == 3, 'three distinct actual dispatch receipts')
        sources = [row['record'] for row in self.rows('instruction_sources') if row['signature'] == key]
        assert len({row['request_id'] for row in sources}) == 3
        assert all(row['task_id'] == self.record_task and row['owner']['partner'] == self.owner for row in sources)
        self.open_records('habits')
        text = self.pages_text('habit-three-actual-request-sources')
        flattened = re.sub(r'[\s│|┆]', '', text)
        assert re.sub(r'\s', '', instruction) in flattened
        assert all(source['request_id'] in flattened for source in sources)
        before = self.state()['memories']
        self.session.command('habit-approve')
        self.wait_caption('APPROVAL PREVIEW', )
        self.session.command('verbose')
        text = self.pages_text('s02-exact-repeat-substitution-disclosure', surface='memory')
        normalized = ''.join(text.split())
        assert ('substitut' in normalized and 'project' in normalized.lower())
        self.session.send('\r')
        assert self.state()['memories'] == before, 'neutral Enter approved an inferred memory'
        self.session.send(PAGE * 24 + RIGHT + '\r')
        self.until(lambda: len(self.state()['memories']) == len(before) + 1, 'explicit S02 project memory committed')
        self.until(lambda: any(row['key'] == key and row['memory'] is not None for row in self.records('instruction_candidates')), 'approved memory bound to exact instruction hash')
        candidate = next(row for row in self.records('instruction_candidates') if row['key'] == key)
        binding = candidate['memory']
        self.workspace()
        post = self.send_request(instruction, 'habit-substitution-reply')
        policy, messages = decoded_payload('chat_completions', post['body'])
        reference = messages[-1][1]
        assert messages[-1][0] == 'user' and reference != instruction
        assert '#' + str(binding['id']) in reference
        assert re.search(r'\brevision\s+' + str(binding['revision']) + r'\b', reference)
        assert messages.count(('user', instruction)) == 3, 'substitution rewrote or duplicated canonical prior history'
        assert instruction in policy, 'substitution omitted the selected approved memory'
        encoded = json.dumps(post['body'], ensure_ascii=False)
        assert 'form_routes' not in encoded and all(route['id'] not in encoded for route in self.routes_before), \
            'local navigation route metadata leaked into actual provider packet'
        self.write_request(instruction)
        posts_before_preview = len(self.peer.snapshot('POST'))
        self.session.command('session-new')
        controls = ('y Run', )
        self.until(lambda: any(control in self.session.display() for control in controls)
                   or 'Alt+A' in self.session.display(),
                   'fresh native request reaches its explicit local scope review')
        if not any(control in self.session.display() for control in controls):
            self.session.send(ESC + 'a')
        self.until(lambda: any(control in self.session.display() for control in controls),
                   'actual native scope offers deliberate run/cancel without a resume capability')
        self.capture('next-current-instruction-deliberately-unsent-scope')
        self.session.send('n')  # cancel this deliberately unsent preview, never approve
        self.until(lambda: not self.rows('active_work'), 'unsent preview releases only its owned claim')
        assert len(self.peer.snapshot('POST')) == posts_before_preview
        self.workspace()
        for number in range(3):
            self.send_request(dismissed, 'dismiss-reply-' + str(number))
        matching = [row for row in self.records('instruction_candidates') if row['instruction'] == dismissed]
        assert len(matching) == 1
        dismiss_key = matching[0]['key']
        self.open_records('habits')
        text = self.pages_text('dismiss-candidate-before-explicit-decision')
        assert re.sub(r'\s', '', dismissed) in re.sub(r'[\s│|┆]', '', text)
        self.session.command('habit-dismiss')
        self.until(lambda: any(row['key'] == dismiss_key and row['dismissed'] for row in self.records('instruction_candidates')), 'explicit per-hash dismissal committed')
        self.workspace()
        self.restart()
        self.connect_again()
        self.send_request(dismissed, 'dismiss-after-restart')
        self.open_records('habits')
        text = self.pages_text('dismissed-hash-does-not-return')
        assert re.sub(r'\s', '', dismissed) not in re.sub(r'[\s│|┆]', '', text), \
            'dismissed proposal returned after another actual dispatch'
        assert next(row for row in self.records('instruction_candidates') if row['key'] == dismiss_key)['dismissed']
        self.workspace()

    def pairings(self):
        self.form('eevee')
        self.session.command('model fixture-default')
        self.workspace()
        self.until(lambda: any(row['record'].get('provider') for row in self.state()['provider_preferences']),
                   'independent global provider/model default committed')
        for owner, form, model in ((self.owner, 'flareon', 'fixture-bolt'), (self.other, 'vaporeon', 'fixture-mizu')):
            self.lead(owner)
            self.form(form)
            before = self.peer.snapshot()
            self.session.command('model ' + model)
            footer_prefixes = ('Type Request /', 'Type a request /', 'Enter Send /', 'Draft only /',
                               )
            self.until(lambda: self.session.display().rstrip().splitlines()[-1].strip().startswith(footer_prefixes),
                       'exact model selection acknowledges directly in the actual composer')
            wait_for(self.session, '/save-pairing')
            wait_for(self.session, model)
            offer_line = next(line for line in self.session.display().splitlines() if '/save-pairing' in line)
            partner = next(row for row in self.state()['partners'] if row['id'] == owner)
            name = partner['nickname'] or 'Eevee #' + str(owner)
            form_index = next(index for index, name in enumerate(FORMS) if name.lower() == form)
            assert name in offer_line, 'inline pairing offer omitted its actual current partner: ' + offer_line
            assert self.task()['lead_partner'] == owner and self.task()['form'] == form_index
            assert form.lower() in self.session.display().lower(), 'current offered form is not visible'
            assert all(row['partner'] != owner or row['form'] != form_index
                       for row in self.records('partner_pairings')), 'inline offer silently saved a pairing'
            self.capture('inline-unsaved-pairing-offer-' + str(owner))
            assert self.peer.snapshot() == before, 'exact local model selection fetched or sent without consent'
            self.session.command('save-pairing')
            self.until(lambda: any(row['partner'] == owner and row['form'] == form_index
                                   and row['selection']['model'] == model for row in self.records('partner_pairings')),
                       'direct explicit exact owner/form pairing committed without visiting pairings desk')
            self.until(lambda: '/save-pairing' not in self.session.display()
                       and self.session.display().rstrip().splitlines()[-1].strip().startswith(footer_prefixes),
                       'committed direct save acknowledges in composer without opening the pairings desk')
            assert not any(title in self.session.display()
                           for title in ('Partner provider pairings', )), \
                'direct pairing save navigated to an extra records desk'
            assert self.peer.snapshot() == before, 'saving a pairing performed network activity'
        before = self.peer.snapshot()
        self.restart()
        for owner, form, model in ((self.owner, 'flareon', 'fixture-bolt'), (self.other, 'vaporeon', 'fixture-mizu')):
            self.lead(owner)
            self.form(form)
            self.until(lambda: 'remembered' in self.session.display().lower()
                       and model in self.session.display(),
                       'actual restored composer labels its exact selection as remembered')
            self.capture('remembered-composer-' + str(owner))
            self.session.command('model')
            wait_for(self.session, model)
            text = self.pages_text('remembered-unavailable-' + str(owner), surface=None)
            normalized = re.sub(r'[\s│|┆║]', '', text).lower()
            assert 'unavailable' in normalized, text
            assert self.peer.snapshot() == before, 'restore inspected/connected/sent without approval'
            self.session.send(ESC)
            self.workspace()
        self.lead(self.owner)
        self.form('flareon')
        self.workspace()
        self.session.command('pairings')
        self.wait_caption('Partner provider pairings', )
        self.session.command('remove-pairing')
        self.until(lambda: all(row['partner'] != self.owner or row['form'] != 3 for row in self.records('partner_pairings')), 'explicit pairing removed')
        self.workspace()
        self.session.command('model')
        text = self.pages_text('remove-pairing-restores-global-default', surface=None)
        assert 'fixture-mizu' in text and 'fixture-bolt' not in text, 'remove must restore last explicitly chosen global default, never remembered auto-use'
        self.session.send(ESC)
        self.workspace()
        assert self.peer.snapshot() == before, 'pairing selection/removal performed network activity'

    def ranked_query(self, form, query='py'):
        self.form(form)
        self.write_request('')
        self.session.send('@' + query)
        self.until(lambda: 'calc.py' in self.session.display() or 'changed.py' in self.session.display(), 'actual @ candidate catalogue')
        rows = []
        screens = []
        for _ in range(32):
            view = self.session.display()
            screens.append(view)
            for line in view.splitlines():
                match = re.search(r'"((?:test_)?changed\.py|calc\.py|check\.py|fixture\.py)"', line)
                if match and match.group(1) not in rows:
                    rows.append(match.group(1))
            self.session.send(DOWN)
        self.capture('rank-' + form)
        self.session.send(ESC)
        self.write_request('')
        return rows, '\n'.join(screens)

    def ranking(self):
        self.source.write_text(BAD, encoding='utf-8')
        # Refresh preserves the first process baseline. Reopen after fixture
        # seeding so only the following working edit is ranked as changed.
        self.restart()
        self.session.command('diff')
        wait_for(self.session, 'Repository')
        self.session.command('refresh')
        wait_for(self.session, 'Repository: refreshed', timeout=35)
        self.workspace()
        (self.project / 'outside.py').symlink_to(self.outside)
        (self.project / 'changed.py').write_text('value = 2\n', encoding='utf-8')
        self.session.command('diff')
        wait_for(self.session, 'Repository')
        self.session.command('refresh')
        wait_for(self.session, 'Repository: refreshed', timeout=35)
        self.workspace()
        order, screens = {}, {}
        for form in (name.lower() for name in FORMS):
            order[form], screens[form] = self.ranked_query(form)
            assert set(order[form]) == {'calc.py', 'changed.py', 'test_changed.py', 'check.py', 'fixture.py'}, (form, order[form])
            assert 'ignored.py' not in screens[form] and 'outside.py' not in screens[form] and '.env.local' not in screens[form]
        assert order['umbreon'][0] == 'calc.py', order
        assert order['flareon'][0] == 'changed.py', order
        assert order['umbreon'][0] != order['flareon'][0]
        failure = 'failure location'
        changed = 'changed file'
        assert failure in screens['umbreon'] and changed in screens['flareon'], 'rank reasons missing in actual @ picker'
        save(self.output / 'form-ranking.json', {'query': 'py', 'orders': order,
             'same_match_set': sorted(set(order['eevee'])), 'source': 'actual retained failing run and observed working diff'})

    def isolation_purge(self):
        # Same data directory, different actual Git root; never seed a fixture row.
        original_project = self.project
        original_activity = self.rows('local_activity')
        self.quit()
        foreign = self.root / 'different-project'
        foreign.mkdir()
        for args in (('init', '-q'), ('-c', 'user.name=Local acceptance', '-c', 'user.email=local@example.invalid', 'commit', '--allow-empty', '-qm', 'isolated')):
            subprocess.run([self.git, '-c', 'core.hooksPath=', *args], cwd=foreign, check=True, capture_output=True, timeout=15)
        self.project = foreign
        self.launch()
        self.workspace()
        self.session.command('goal')
        self.session.paste('Separate project has no borrowed failures')
        self.session.send('\r')
        wait_for(self.session, 'Storage: saved')
        self.open_records('dex')
        text = self.pages_text('different-project-no-borrowed-signature')
        assert 'records::total' not in text and not self.rows('failure_signatures')
        assert self.rows('local_activity', original_project) == original_activity
        self.workspace()
        self.quit()
        self.project = original_project
        self.launch()
        # A selected approved memory sheet is a known startup surface, not an
        # arbitrary popup; its explicit Escape keeps the selected state unchanged.
        if any(title in self.session.display() for title in ('Approved memories', )):
            self.session.send(ESC)
        self.workspace()
        self.session.command('tasks')
        wait_for(self.session, 'Task cards / committed snapshots')
        self.session.send(HOME)
        self.session.command('delete')
        wait_for(self.session, 'Review local task deletion')
        self.capture('explicit-task-purge-impact')
        self.session.send(PAGE * 24 + RIGHT + '\r')
        self.until(lambda: all(row['id'] != self.record_task for row in self.state()['tasks']), 'actual owned task purge')
        for table in RECORD_TABLES:
            assert not self.rows(table), 'last project task purge retained ' + table
        assert self.outside.read_text(encoding='utf-8') == 'outside = True\n'
        self.workspace()

    def quit(self):
        self.workspace()
        self.session.command('quit')
        confirm_exit(self.session)
        assert self.session.wait_exit() == 0, 'owned native terminal returned nonzero'
        self.session.close()


def run(binary, output, root, size, ascii_mode, ):
    results = {key: {'description': text, 'status': 'not-run', 'evidence': []} for key, text in CASES.items()}
    result = {'status': 'failed', 'size': list(size), 'ascii': ascii_mode, 'cases': results, 'scope': 'Actual native UI/local store/loopback HTTP; NOT live provider or opaque session resume'}
    journey = None
    try:
        with Peer('chat_completions', 'unused-keyless-public-fixture', True, retain_body_bytes=True) as peer:
            try:
                journey = Journey.__new__(Journey)
                journey.__init__(binary, output, root, size, ascii_mode, results, peer, )
                for key, action in (
                    ('C03A', journey.attachments), ('C03B', journey.chip_validation),
                    ('T01A', journey.usage), ('PK05A', journey.failures), ('PK05B', journey.resolution),
                    ('EV05', journey.markers), ('EV06', journey.routes), ('PK02', journey.summary),
                    ('EV08A', journey.run_lines), ('EV08B', journey.more_lines), ('EV09', journey.habits),
                    ('EV17', journey.pairings), ('EV11', journey.ranking)):
                    journey.case(key, action)
                # PK05A/EV06 include the real project boundary and final purge,
                # which must happen only after all record-dependent journeys.
                journey.current_case = 'PK05A'
                journey.isolation_purge()
                journey.capture('last-task-purge-project-records-empty')
                journey.current_case = None
                journey.quit()
                peer.assert_clean()
                assert peer.usage_plans.empty(), 'usage event plan not consumed by a real HTTP request'
                result['status'] = 'passed'
            finally:
                output.mkdir(parents=True, exist_ok=True)
                save(output / 'http-requests.json', peer.snapshot())
                if journey:
                    journey.close()
    except Exception as error:
        result.update(error=str(error), traceback=traceback.format_exc())
        if journey and journey.current_case:
            results[journey.current_case].update(status='failed', error=str(error))
        # Unreached IDs are not silently skipped or counted as accepted.
        for row in results.values():
            if row['status'] == 'not-run':
                row.update(status='blocked-by-earlier-failure', error=str(error))
    finally:
        save(output / 'results.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path, help='already-built application; never builds')
    parser.add_argument('--output', required=True, type=Path, help='new retained evidence directory')
    args = parser.parse_args()
    binary = args.binary.resolve()
    if os.name == 'nt' and not binary.exists():
        binary = binary.with_suffix('.exe')
    assert binary.is_file(), 'already-built --binary is required'
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    before = digest(binary)
    report = {'status': 'failed', 'host': platform.platform(), 'binary': str(binary),
              'binary_sha256': before, 'journeys': [],
              'limitations': ['Loopback protocol fixtures are not live-provider/model/billing conformance.',
                              'Native follow-up and process restart are not opaque-provider session resume.',
                              'Parent separately owns authorized actual Codex two-turn/resume live proof.',
                              'PTY/ConPTY cell screenshots are not GUI/native-image certification.']}
    try:
        for size, ascii_mode in (((120, 36), False), ((60, 18), True)):
            label = '{}x{}-{}'.format(*size, 'ascii' if ascii_mode else 'unicode')
            with tempfile.TemporaryDirectory(prefix='ev-records-漢字-') as temporary:
                result = run(binary, output / label, Path(temporary).resolve(), size, ascii_mode, )
            report['journeys'].append(result)
        assert digest(binary) == before, 'supplied binary changed during qualification'
        report['status'] = 'passed' if all(row['status'] == 'passed' for row in report['journeys']) else 'failed'
    except Exception as error:
        report.update(error=str(error), traceback=traceback.format_exc())
    finally:
        save(output / 'results.json', report)
    print(json.dumps({'status': report['status'], 'output': str(output)}, ensure_ascii=False))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
