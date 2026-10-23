#!/usr/bin/env python3
"""Actual Sources optimizer/ledger PTY acceptance against an already-built ev.

Uses isolated multi-file Git fixtures, read-only SQLite observations, an actual
explicitly approved failed local command and deterministic loopback HTTP. No
builds, SQL writes, external providers, credentials or provider-token estimates.
"""
from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import platform
import sqlite3
import tempfile
import traceback

from moves_smoke import digest, save
from native_connection_smoke import Peer, decoded_payload
from records_smoke import Journey as RecordsJourney, ESC, HOME, DELETE
from response_smoke import SurfaceSession
from file_creation_smoke import response_plan, tool_results
from workbench_smoke import wait_for


REQUEST = ('Explain normalize_total subtotal underflow, its submit_total caller and zero_total '
           'boundary check, plus cedar_requirementunsupported; preserve total and do not edit. '
           'optimizer-marker-cedar-927.')


def selected_packet(wire):
    """Extract the exact selected_sources JSON slice of the actual policy string."""
    policy, messages = decoded_payload('chat_completions', wire['body'])
    marker = 'UNTRUSTED CURRENT TASK/SOURCE DATA (JSON):\n'
    assert marker in policy, 'actual native envelope omitted frozen task/source context'
    context = policy.split(marker, 1)[1]
    key = '"selected_sources":'
    start = context.index(key) + len(key)
    value, end = json.JSONDecoder().raw_decode(context[start:])
    return context[start:start + end].encode('utf-8'), value, messages


class OptimizerSession(SurfaceSession):
    def send(self, keys):
        super().send(keys)
        self.pump(0, settle=True)


class Journey(RecordsJourney):
    def launch(self):
        self.serial += 1
        self.session = OptimizerSession(
            [str(self.binary), '--data-dir', str(self.data), '--images', 'off',
             '--color', 'mono', *(['--ascii'] if self.ascii_mode else [])],
            self.project, self.environment, self.size,
            self.output / ('terminal-%02d' % self.serial),
            language=self.language, controlling_tty=True)
        self.sessions.append(self.session)
        wait_for(self.session, 'Storage: saved', timeout=35)

    def state(self):
        state = super().state()
        with closing(sqlite3.connect(self.database.as_uri() + '?mode=ro', uri=True, timeout=1)) as db:
            db.execute('PRAGMA query_only=ON')
            db.row_factory = sqlite3.Row
            state['context_revisions'] = [dict(row) for row in db.execute(
                'SELECT * FROM context_revisions ORDER BY task_id,source_key')]
            state['tool_artifacts'] = [dict(row) for row in db.execute(
                'SELECT * FROM tool_artifacts ORDER BY rowid')]
            for row in state['tool_artifacts']:
                row['record'] = json.loads(row['record'])
        return state

    def seed_optimizer_sources(self):
        def padded(body):
            return body + ''.join(
                '\n// archival display records retained independently from arithmetic\n'
                "pub fn archival_%d() -> &'static str {\n"
                '    "independent historical display metadata"\n}\n' % index
                for index in range(180))
        sources = {
            'src/optimizer_math.rs': padded(
                'pub fn normalize_total(amount: u32) -> u32 {\n'
                '    // subtotal underflow occurs at zero\n    amount - 1\n}\n'),
            'src/optimizer_submit.rs': padded(
                'pub fn submit_total(amount: u32) -> u32 {\n'
                '    crate::optimizer_math::normalize_total(amount)\n}\n'),
            'tests/optimizer_boundary.rs': padded(
                '#[test]\nfn zero_total() {\n'
                '    assert_eq!(crate::optimizer_math::normalize_total(0), 0);\n}\n'),
            'notes/unsupported.context': (
                'archived unrelated display records\n' * 100
                + 'cedar_requirementunsupported: zero must retain the boundary invariant\n'
                + 'archived unrelated display records\n' * 100),
            'notes/shared_capture.txt': ''.join(
                'Public cedar observation %02d: retain literal boundaries and historical references without granting execution.\r\n' % index
                for index in range(24)),
        }
        for path, text in sources.items():
            destination = self.project / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(text.encode('utf-8'))
        self.git_command('add', '--', *sources)
        self.git_command('-c', 'user.name=Local acceptance', '-c', 'user.email=local@example.invalid',
                         'commit', '-qm', 'offline optimizer multi-file source fixture')
        save(self.output / 'optimizer-source-fixture.json', {
            path: {'sha256': hashlib.sha256(text.encode()).hexdigest(),
                   'bytes': len(text.encode()), 'full_source': text}
            for path, text in sources.items()})
        return sources

    def assert_ranked_evidence(self, packet, selected):
        ranked = packet['ranked_excerpts']
        for path, exact in (
                ('src/optimizer_math.rs', 'amount - 1'),
                ('src/optimizer_submit.rs', 'normalize_total(amount)'),
                ('tests/optimizer_boundary.rs', 'assert_eq!(crate::optimizer_math::normalize_total(0), 0)')):
            matches = [row for row in ranked if row['path'] == path and exact in row['text']]
            assert matches, 'missing actual defect/caller/check bytes: ' + path
            assert any('Rust ' in row['selection_reason'] for row in matches), \
                'supported Rust source was not labelled as local syntax evidence: ' + path
        fallback = [row for row in ranked if row['path'] == 'notes/unsupported.context'
                    and 'cedar_requirementunsupported' in row['text']]
        assert fallback, 'seeded unsupported-file requirement is not discoverable'
        assert all('Text-range fallback (unsupported syntax)' in row['selection_reason']
                   and 'Rust ' not in row['selection_reason'] for row in fallback), \
            'unsupported source received fabricated Rust symbol evidence'
        captures = packet['selected_sources']['captures']
        assert any(row['id'] == selected['id'] and row['text'] == selected['text']
                   for row in captures), 'mandatory pin was omitted or silently reduced'
    def commit_provider_decision(self, request, proposal, label):
        self.form('espeon')
        before = self.task()['workflows'].get('chosen_decision')
        posts = len(self.peer.snapshot('POST'))
        self.send_request(request, proposal)
        assert len(self.peer.snapshot('POST')) == posts + 1
        assert self.task()['workflows'].get('chosen_decision') == before
        self.session.command('response')
        self.session.send(ESC + '[F')
        wait_for(self.session, proposal, timeout=35)
        self.capture(label + '-selected-real-provider-response')
        self.session.command('decision')
        self.wait_caption('Next action', '다음 행동')
        assert self.task()['workflows'].get('chosen_decision') == before
        self.capture(label + '-next-action-explicit-approval')
        self.session.send('\r')

        def committed():
            task = self.task()
            decision = task['workflows'].get('chosen_decision')
            return (decision is not None and decision != before
                    and decision['text'].startswith(proposal + '\n\n')
                    and task['next_action'] == decision['text'])

        self.until(committed, label + ' canonical chosen decision committed through UI')
        wait_for(self.session, 'Storage: saved', timeout=35)
        decision = self.task()['workflows']['chosen_decision']
        assert '[User-selected, unverified ' in decision['text']
        assert 'Not local execution evidence.]' in decision['text']
        assert decision['id'] == 'decision:' + hashlib.sha256(
            decision['text'].encode('utf-8')).hexdigest()
        assert len(self.peer.snapshot('POST')) == posts + 1
        self.capture(label + '-canonical-decision-committed')
        self.workspace()
        return decision

    def seed_conflicting_decisions(self):
        posts = len(self.peer.snapshot('POST'))
        first_proposal = 'Use amber layout; reject cobalt layout.'
        latest_proposal = 'Use cobalt layout; reject amber layout.'
        first = self.commit_provider_decision(
            'Propose the amber layout decision for review.', first_proposal,
            'decision-first-amber')
        criterion = first_proposal
        before_criteria = self.task()['criteria']
        before_origins = self.task()['workflows'].get('criterion_origins', [])
        assert not any(row['text'] == criterion for row in before_criteria)
        self.session.command('workflow-recipe')
        self.wait_caption('Specialist desk', '전문가 데스크')
        self.session.command('decision-criteria')
        self.wait_caption('Decision criteria', '결정 기준')
        self.session.send((ESC + '[A') * 100 + HOME + DELETE * 512)
        self.session.paste(criterion)
        self.capture('first-decision-criterion-real-editor')
        self.session.send('\x13')
        assert self.task()['criteria'] == before_criteria
        assert self.task()['workflows'].get('criterion_origins', []) == before_origins
        self.session.command('confirm-decision-criteria')
        self.wait_caption('y Apply', 'y 적용')
        self.capture('first-decision-criterion-explicit-confirmation')
        assert self.task()['criteria'] == before_criteria
        self.session.send('y')
        origin = {'criterion': criterion, 'decision_id': first['id'],
                  'decision_text': first['text']}
        expected_criteria = [*before_criteria, {'text': criterion, 'met': False}]
        expected_origins = [*before_origins, origin]
        self.until(lambda: self.task()['criteria'] == expected_criteria
                   and self.task()['workflows']['criterion_origins'] == expected_origins,
                   'first-decision criterion and exact canonical origin committed')
        wait_for(self.session, 'Storage: saved', timeout=35)
        self.capture('first-decision-criterion-origin-committed')
        self.workspace()
        latest = self.commit_provider_decision(
            'Replace the amber layout decision with its conflicting cobalt choice.',
            latest_proposal, 'decision-latest-cobalt')
        task = self.task()
        assert latest['id'] != first['id'] and latest['text'] != first['text']
        assert task['workflows']['chosen_decision'] == latest
        assert task['criteria'] == expected_criteria
        assert task['workflows']['criterion_origins'] == expected_origins
        assert len(self.peer.snapshot('POST')) == posts + 2
        self.capture('conflicting-decisions-first-origin-latest-canonical')
        self.workspace()
        return {'first': first, 'latest': latest,
                'criterion_origins': task['workflows']['criterion_origins'],
                'actual_request_count': len(self.peer.snapshot('POST')) - posts,
                'actual_inference_posts': len(self.peer.snapshot('POST'))}


    def optimizer_metric_rows(self):
        # Optimizer has NO file/basket list rows. Existing detail_rows('sources')
        # assumes that list and would silently skip the measurements at 60x18.
        # SOURCE_TITLE is defined by context_smoke and reused by RecordsJourney.
        from context_smoke import SOURCE_TITLE
        titles = (SOURCE_TITLE, '출처 / 작업 소유 맥락')
        screen = self.session.screen
        top = next(y for y in range(screen.lines)
                   if any(title in self.visual_row(y, 0, screen.columns) for title in titles)
                   and any(screen.buffer[y][x].data in ('┌', '╔', '╭', '+')
                           for x in range(screen.columns)))
        title_x = next(x for x in range(screen.columns)
                       if any(self.visual_row(top, x, screen.columns).startswith(title)
                              for title in titles))
        left = max(x for x in range(title_x)
                   if screen.buffer[top][x].data in ('┌', '╔', '╭', '+'))
        right = next(x for x in range(title_x + 1, screen.columns)
                     if screen.buffer[top][x].data in ('┐', '╗', '╮', '+'))
        bottom = next(y for y in range(top + 1, screen.lines)
                      if screen.buffer[y][left].data in ('└', '╚', '╰', '+'))
        # src/ui/sources.rs: details starts at content.y + 2, height =
        # content.height - 5. panel() has one-cell horizontal padding.
        first, last = top + 3, bottom - 3
        assert last > first, 'optimizer detail viewport is not inspectable'
        return [self.visual_row(y, left + 2, right - 1).rstrip()
                for y in range(first, last)]

    def read_preview_metrics(self):
        import re
        import unicodedata

        tokenizer = 'unicode-segmentation-1.13.3/UAX29-word-boundaries-v1'
        wait_for(self.session, 'PgUp/Dn')
        # Home is Action::Scroll in Mode::Optimizer, unlike Files/Basket.
        self.session.send(HOME)
        self.wait_caption('Local context optimizer', '로컬 컨텍스트 최적화')
        pages, observed, previous = [], [], None
        label = 'optimizer-metrics-%03d' % self.serial
        try:
            for index in range(16):
                rows = self.optimizer_metric_rows()
                if rows == previous:
                    break
                pages.append(rows)
                overlap = max((n for n in range(1, min(len(rows), len(observed)) + 1)
                               if observed[-n:] == rows[:n]), default=0)
                observed.extend(rows[overlap:])
                self.capture(label + '-page-%02d' % index)
                # Keep row boundaries: removing every newline concatenates a
                # baseline count with the next row's byte count. Literal text
                # may wrap, but each complete numeric word stays a separate
                # observed field rather than being reconstructed from a guess.
                visible = ''.join(ch for ch in unicodedata.normalize('NFC', '\n'.join(observed))
                                  if unicodedata.category(ch) != 'Cf')
                def literal(text):
                    return r'\s*'.join(re.escape(ch) for ch in text if not ch.isspace())
                kind_pattern = '(' + literal('Rankedcontext') + '|' + literal('Evidence-linkedcompaction') + ')'
                gap = r'\s*'
                if self.language == 'ko':
                    summary_pattern = (kind_pattern + gap + literal('·로컬토큰') + gap
                                       + r'([0-9]+)' + gap + literal('/') + gap + r'([0-9]+)'
                                       + gap + literal('·전체기준') + gap + r'([0-9]+)(?=\s*(?:\n|$))')
                    measurements_pattern = (r'(?:^|\n)\s*([0-9]+)' + gap + literal('B·처리') + gap
                                            + r'([0-9]+)' + gap + r'[µμ]' + gap + literal('s·로컬리비전/토큰캐시적중')
                                            + gap + r'([0-9]+)' + gap + literal('회'))
                    contributions_pattern = (literal('로컬구성:패킷') + gap + r'([0-9]+)'
                                             + gap + literal('+필수메모') + gap + r'([0-9]+)' + gap + literal('.'))
                    tokenizer_label = '토크나이저:'
                else:
                    summary_pattern = (kind_pattern + gap + literal('·') + gap
                                       + r'([0-9]+)' + gap + literal('/') + gap + r'([0-9]+)'
                                       + gap + literal('localtokens·baseline') + gap + r'([0-9]+)(?=\s*(?:\n|$))')
                    measurements_pattern = (r'(?:^|\n)\s*([0-9]+)' + gap + literal('B·') + gap
                                            + r'([0-9]+)' + gap + r'[µμ]' + gap + literal('sprocessing·')
                                            + gap + r'([0-9]+)' + gap + literal('localrevision/token-cachehits'))
                    contributions_pattern = (literal('Localcontribution:packet') + gap + r'([0-9]+)'
                                             + gap + literal('+mandatorynotes') + gap + r'([0-9]+)' + gap + literal(';'))
                    tokenizer_label = 'Tokenizer:'
                summary = re.search(summary_pattern, visible)
                measurements = re.search(measurements_pattern, visible)
                named_tokenizer = re.search(literal(tokenizer_label + tokenizer + '.'), visible)
                contributions = re.search(contributions_pattern, visible)
                if summary and measurements and named_tokenizer and contributions:
                    result = {
                        'source': 'actual Sources optimizer rendered cells',
                        'language': self.language, 'size': list(self.size),
                        'tokenizer': tokenizer,
                        'kind': ('Ranked context' if ''.join(summary.group(1).split()) == 'Rankedcontext'
                                 else 'Evidence-linked compaction'),
                        'tokens': int(summary.group(2)),
                        'budget': int(summary.group(3)),
                        'baseline_tokens': int(summary.group(4)),
                        'packet_tokens': int(contributions.group(1)),
                        'mandatory_note_tokens': int(contributions.group(2)),
                        'bytes': int(measurements.group(1)),
                        'processing_micros': int(measurements.group(2)),
                        'cache_hits': int(measurements.group(3)),
                        'screen_pages': pages,
                    }
                    assert result['tokens'] == result['packet_tokens'] + result['mandatory_note_tokens']
                    save(self.output / (label + '.json'), result)
                    return result
                previous = rows
                self.session.send(ESC + '[6~')
        finally:
            self.session.send(HOME)
        raise AssertionError(
            'Missing UI prerequisite: the actual paged optimizer renderer did not expose '
            'its complete localized selected/budget/full-file-baseline summary, bytes/'
            'processing_micros/cache_hits measurements and complete named tokenizer. '
            'SQLite context_revisions and selected_sources do not retain those missing '
            'Plan metrics; no estimate or reconstructed tokenizer count is accepted.\n'
            + '\n'.join(observed))

    def reveal_optimizer_notice(self, english, korean):
        target = korean if self.language == 'ko' else english
        previous = None
        self.session.send(HOME)
        try:
            for index in range(16):
                rows = self.optimizer_metric_rows()
                if target in '\n'.join(rows):
                    self.capture('optimizer-required-notice-page-%02d' % index)
                    return
                if rows == previous:
                    break
                previous = rows
                self.session.send(ESC + '[6~')
        finally:
            self.session.send(HOME)
        raise AssertionError('required optimizer notice absent from actual scrollable review: '
                             + target + '\n' + self.session.display())

    def observe_optimizer_preview(self, label, expected_file=None):
        metrics = self.read_preview_metrics()
        assert metrics['kind'] == 'Ranked context'
        task_id = self.task()['id']
        retained = {}

        def persisted_preview():
            # Journey.state() uses mode=ro and PRAGMA query_only=ON. No SQL
            # mutation, cache seeding or alternate optimizer is introduced.
            rows = [row for row in self.state()['context_revisions']
                    if row['task_id'] == task_id]
            manifests = [row for row in rows if row['source_key'].startswith('manifest:')]
            if len(manifests) != 1 or manifests[0]['tokens'] != metrics['packet_tokens']:
                return False
            files = []
            for row in rows:
                if not row['source_key'].startswith('file:'):
                    continue
                configuration, separator, tail = row['source_key'][5:].partition(':')
                prefix = self.task()['project'] + ':'
                assert separator and tail.startswith(prefix), 'unexpected real file-cache identity'
                path, separator, revision = tail[len(prefix):].rpartition(':')
                assert separator, 'real file-cache key has no source revision'
                files.append({'path': path, 'source_revision': revision,
                              'configuration': configuration, 'row': row})
            if expected_file is not None:
                path, sha256 = expected_file
                if not any(item['path'] == path and item['source_revision'] == sha256
                           for item in files):
                    return False
            # Corroboration only: displayed baseline is never computed/replaced
            # from SQLite. These are the engine's actual retained count rows.
            if sum(item['row']['tokens'] for item in files) != metrics['baseline_tokens']:
                return False
            manifest = manifests[0]
            identity = manifest['source_key']
            assert len(identity) == len('manifest:') + 64 and '\n' not in identity, \
                'assembly cache identity must be a stable hash, not embedded JSON'
            encoded = manifest['assembly']
            assert isinstance(encoded, str) and len(encoded.encode('utf-8')) <= 4 * 1024 * 1024, \
                'missing or oversized explicit assembly metadata'
            retained.update({
                'rows': rows, 'files': files,
                'manifest_identity': identity,
                'manifest': json.loads(encoded),
                'packet_sha256': manifest['source_sha256'],
                'policy': manifest['policy'], 'tokenizer': manifest['tokenizer'],
            })
            return True

        self.until(persisted_preview, 'real persisted revision rows match rendered preview: ' + label)
        assert retained['tokenizer'] == metrics['tokenizer']
        assert all(row['policy'] == retained['policy'] and row['tokenizer'] == metrics['tokenizer']
                   for row in retained['rows'])
        result = {'metrics': metrics, 'cache': retained}
        save(self.output / (label + '-metrics-and-real-cache.json'), result)
        return result

    def observe_optimizer_cache_metrics(self, command='optimize 20000'):
        # Invoke before ANY optimizer preview for this isolated task; existing
        # RecordsJourney fixture source changes and refresh must already be done.
        task_id = self.task()['id']
        assert not any(row['task_id'] == task_id for row in self.state()['context_revisions']), \
            'cold-cache proof requires a real task with no previous optimizer cache'
        effects = self.effects()
        basket = self.state()['excerpts']
        draft = self.task()['partner_drafts']
        path = self.project / 'changed.py'  # existing, unpinned fixture source
        original = path.read_bytes()
        original_sha256 = hashlib.sha256(original).hexdigest()
        self.optimizer(command)
        cold = self.observe_optimizer_preview('optimizer-cache-cold', ('changed.py', original_sha256))
        assert cold['metrics']['cache_hits'] == 0, 'first real preview was not cold'
        self.optimizer(command)
        warm = self.observe_optimizer_preview('optimizer-cache-warm', ('changed.py', original_sha256))
        assert warm['cache']['rows'] == cold['cache']['rows'], 'identical preview changed retained identities'
        assert warm['metrics']['cache_hits'] == len(warm['cache']['files']) + 1, \
            'warm renderer did not report all full-file hits plus the assembly-manifest hit'
        for key in ('tokens', 'packet_tokens', 'mandatory_note_tokens', 'baseline_tokens', 'budget', 'bytes', 'tokenizer'):
            assert warm['metrics'][key] == cold['metrics'][key], 'warm preview changed ' + key
        # Never claim warm elapsed time is smaller: each time is independently
        # observed renderer output, not an acceptance threshold or estimate.
        try:
            self.session.send(ESC)
            changed = original + b'\noptimizer_cache_source_revision_probe = 927\n'
            path.write_bytes(changed)
            changed_sha256 = hashlib.sha256(changed).hexdigest()
            self.refresh()
            self.optimizer(command)
            invalidated = self.observe_optimizer_preview(
                'optimizer-cache-source-invalidated', ('changed.py', changed_sha256))
            old = next(item for item in cold['cache']['files'] if item['path'] == 'changed.py')
            new = next(item for item in invalidated['cache']['files'] if item['path'] == 'changed.py')
            assert old['source_revision'] != new['source_revision']
            assert old['configuration'] == new['configuration'], 'probe unexpectedly changed optimizer configuration'
            assert old['row']['source_key'] != new['row']['source_key']
            assert old['row']['source_key'] not in {row['source_key'] for row in invalidated['cache']['rows']}
            assert cold['cache']['manifest_identity'] != invalidated['cache']['manifest_identity'], \
                'source revision did not invalidate real assembly identity'
            # Unchanged files may legitimately remain hits. Do not assert that
            # source invalidation makes the entire multi-source cache cold.
            old_files = {item['path']: item['row'] for item in cold['cache']['files']}
            new_files = {item['path']: item['row'] for item in invalidated['cache']['files']}
            assert old_files.keys() == new_files.keys()
            assert all(old_files[name] == new_files[name] for name in old_files if name != 'changed.py')
            self.optimizer(command)
            revised_warm = self.observe_optimizer_preview(
                'optimizer-cache-revised-warm', ('changed.py', changed_sha256))
            assert revised_warm['cache']['rows'] == invalidated['cache']['rows']
            assert revised_warm['metrics']['cache_hits'] == len(revised_warm['cache']['files']) + 1
        finally:
            path.write_bytes(original)
            self.refresh()
        # Restore the actual fixture and produce a real corresponding preview;
        # no optimizer selection is applied or sent by this observation slice.
        self.optimizer(command)
        restored = self.observe_optimizer_preview('optimizer-cache-restored', ('changed.py', original_sha256))
        assert self.effects() == effects, 'metric/cache observations executed work or dispatched inference'
        assert self.state()['excerpts'] == basket and self.task()['partner_drafts'] == draft
        result = {'cold': cold, 'warm': warm, 'source_invalidated': invalidated,
                  'revised_warm': revised_warm, 'restored': restored}
        save(self.output / 'optimizer-cache-metrics.json', result)
        return result
    def optimizer(self, command='optimize 20000'):
        self.session.command(command)
        self.wait_caption('Local context optimizer', '로컬 컨텍스트 최적화')
        wait_for(self.session, 'PgUp/Dn')
        self.until(lambda: 'local only' in '\n'.join(self.session.display().splitlines()[:8]),
                   'specific optimizer worker operation completed, not an older preview')
        self.capture(command.replace(' ', '-') + '-real-preview')

    def apply(self):
        before = self.effects()
        self.session.send('a')
        self.wait_caption('Applied to the next ordinary request', '다음 일반 요청에 적용됨')
        assert self.effects() == before, 'apply itself started a command or provider request'

    def refresh(self):
        self.workspace()
        self.session.command('diff')
        wait_for(self.session, 'Repository', timeout=35)
        self.session.command('refresh')
        wait_for(self.session, 'Repository: refreshed', timeout=35)
        self.workspace()

    def retained_request(self):
        task = self.task()
        drafts = task['partner_drafts'].get(str(task['lead_partner']), {}).get('drafts', [])
        return next((draft['text'] for draft in drafts if draft['field'] == 'Request'), '')

    def write_request(self, text):
        footer = self.workspace()
        wait_for(self.session, 'Storage: saved')
        if self.retained_request() != text:
            return super().write_request(text)
        # Sending the already exact committed request is not a user text edit.
        # Avoid artificial delete/retype commits invalidating its frozen packet.
        if footer.startswith('Type Request /'):
            self.session.send('\r')
        # A restored attachment chip owns Enter until ordinary caret navigation.
        # Home returns to literal text without modifying the committed Request.
        self.session.send(HOME)
        self.session.send('\x13')
        self.until(lambda: self.retained_request() == text
                   and 'Storage: saved' in self.session.display(),
                   'unchanged exact Request remains committed')

    def exact_send(self, label):
        answer = 'CTX_' + hashlib.sha256(label.encode('utf-8')).hexdigest()[:12]
        wire = self.send_request(REQUEST, answer)
        packet, value, messages = selected_packet(wire)
        raw = bytes.fromhex(wire['body_bytes_hex'])
        assert json.loads(raw) == wire['body'], 'captured exact HTTP bytes differ from decoded transport'
        assert any(role == 'user' and text == REQUEST for role, text in messages)
        save(self.output / (label + '-wire-proof.json'), {
            'http_body_sha256': hashlib.sha256(raw).hexdigest(),
            'http_body_bytes': len(raw), 'selected_packet_utf8_hex': packet.hex(),
            'selected_packet_sha256': hashlib.sha256(packet).hexdigest(),
            'selected_sources': value,
        })
        self.capture(label)
        return packet, value

    def selected_memories(self):
        value = self.task()['approved_memories']
        return json.loads(value) if isinstance(value, str) else value

    def adopt_source_memory(self, capture):
        before_ids = {row['id'] for row in self.state()['memories']}
        before_effects = self.effects()
        before_pins = self.state()['excerpts']
        before_draft = self.task()['partner_drafts']
        reference = 'task:%s/source:%s' % (capture['task_id'], capture['id'])

        def propose():
            self.focus_chip(capture['id'])
            self.session.send('\r')
            self.wait_caption('Sources / task-owned context', '출처 / 작업 소유 맥락')
            self.session.command('remember-source')
            self.wait_caption('APPROVAL PREVIEW / no memory saved yet', '승인 미리보기 / 아직 저장된 메모리 없음')
            self.wait_caption('y Approve', 'y 승인')
            self.wait_caption('/delete-memory removes it separately.',
                              '/delete-memory로 노트는 별도 삭제합니다.')

        propose()
        assert {row['id'] for row in self.state()['memories']} == before_ids
        assert self.effects() == before_effects
        self.capture('bound-capture-note-fresh-preview-before-any-approval')
        # Preview -> unchanged editor -> discard-free browse. No note, outgoing
        # selection, tool call or source mutation is created by this cancellation.
        self.session.send(ESC)
        self.session.send(ESC)
        # This is the known memory browser, whose Korean help also mentions
        # approval previews; it is not another consent surface.
        wait_for(self.session, '/held-set-memory')
        self.session.send(ESC)
        self.workspace()
        assert {row['id'] for row in self.state()['memories']} == before_ids
        assert self.state()['excerpts'] == before_pins
        assert self.task()['partner_drafts'] == before_draft
        assert self.effects() == before_effects

        propose()
        self.session.send('y')
        self.until(lambda: len({row['id'] for row in self.state()['memories']} - before_ids) == 1,
                   'explicit user-adopted source memory committed by normal writer')
        note = next(row for row in self.state()['memories'] if row['id'] not in before_ids)
        assert note['proposal']['content'] == capture['text']
        assert note['proposal']['scope'] == 'task'
        assert note['proposal']['provenance'] == {
            'kind': 'user_authored', 'reference': reference, 'task_id': capture['task_id']}
        assert all(row['id'] != note['id'] for row in self.selected_memories()), \
            'approving a source note implicitly selected it for outgoing context'
        self.capture('bound-capture-note-approved-not-selected')
        self.session.command('select-memory')
        self.until(lambda: any(row['id'] == note['id'] for row in self.selected_memories()),
                   'separate explicit outgoing selection of approved capture note')
        assert self.effects() == before_effects
        self.capture('bound-capture-note-explicit-outgoing-selection')
        wait_for(self.session, '/held-set-memory')
        self.session.send(ESC)
        self.workspace()
        return note

    def prove_live_capture_dedup(self, capture, memory):
        self.write_request(REQUEST)
        before = len(self.peer.snapshot('POST'))
        call_id = 'context-dedup-actual-source-read'
        answer = 'CONTEXT_THREE_ENTRY_REAL_READ_COMPLETED'
        self.peer.plans.put(response_plan(self.peer, calls=[{
            'id': call_id, 'name': 'eevee_read_file',
            'arguments': {'path': capture['path'], 'start_line': capture['start_line'],
                          'end_line': capture['end_line']},
        }]))
        self.peer.plans.put(response_plan(self.peer, text=answer, number=1))
        self.session.send('\r')
        self.until(lambda: len(self.peer.snapshot('POST')) == before + 2,
                   'actual read tool executed and its real continuation reached loopback transport')
        self.session.command('response')
        self.session.send(ESC + '[F')
        wait_for(self.session, answer, timeout=35)
        self.until(lambda: not self.state()['active_work'], 'actual source-read request settled')
        posts = self.peer.snapshot('POST')[before:]
        assert len(posts) == 2
        policy, _ = decoded_payload('chat_completions', posts[0]['body'])
        context = json.loads(policy.split('UNTRUSTED CURRENT TASK/SOURCE DATA (JSON):\n', 1)[1])
        result_text = tool_results('chat_completions', posts[1]['body'])[call_id]
        result = json.loads(result_text)
        carriers = {}

        def collect(node):
            if isinstance(node, dict):
                for row in node.get('captures', []):
                    carriers[row['id']] = row
                for value in node.values():
                    collect(value)
            elif isinstance(node, list):
                for value in node:
                    collect(value)

        collect(context['selected_sources'])
        carrier = carriers[capture['id']]
        assert carrier['text'] == capture['text']
        note = next(row for row in context['approved_memories'] if row['id'] == memory['id'])
        assert note['proposal']['content_reference']['same_bytes_as'] == capture['id']
        assert 'content' not in note['proposal']
        assert result['status'] == 'success' and result['same_bytes_as'] == capture['id']
        assert 'content' not in result
        assert result['capture_reference']['task_id'] == capture['task_id']
        assert result['start_line'] == capture['start_line'] and result['end_line'] == capture['end_line']
        assert result['sha256'] == bytes(capture['source_sha256']).hex()
        aliases, occurrences = [], []

        def prove(node):
            if isinstance(node, dict):
                if isinstance(node.get('mandatory_instructions'), str):
                    declared_instructions = json.loads(node['mandatory_instructions'])
                    assert 'approved_memories' not in declared_instructions, \
                        'structural notes were duplicated inside the serialized instruction string'
                    prove(declared_instructions)
                if 'same_bytes_as' in node:
                    target = carriers[node['same_bytes_as']]
                    assert isinstance(target.get('text'), str) and 'same_bytes_as' not in target
                    aliases.append(node['same_bytes_as'])
                for value in node.values():
                    prove(value)
            elif isinstance(node, list):
                for value in node:
                    prove(value)
            elif isinstance(node, str) and capture['text'] in node:
                occurrences.append(node)

        prove(context)
        prove(result)
        assert aliases.count(capture['id']) >= 2, 'memory and real read must both cite the actual carrier'
        assert len(occurrences) == 1, 'capture bytes repeated in notes, tool result or instruction-string JSON'
        self.until(lambda: any(row['record']['origin'] == {'OwnedTool': {'call_id': call_id, 'tool': 'eevee_read_file'}}
                               for row in self.state()['tool_artifacts']),
                   'actual pre-dedup read retained through artifact single writer')
        artifact = next(row for row in self.state()['tool_artifacts']
                        if row['record']['origin'] == {'OwnedTool': {'call_id': call_id, 'tool': 'eevee_read_file'}})
        assert artifact['task_id'] == capture['task_id']
        assert artifact['record']['captured_bytes'] == artifact['record']['total_bytes']
        assert artifact['record']['omitted_bytes'] == 0 and not artifact['record']['unavailable']
        original_bytes = (self.data / artifact['output_name']).read_bytes()
        assert hashlib.sha256(original_bytes).hexdigest() == artifact['record']['sha256']
        original = json.loads(original_bytes)
        assert original['content'] == capture['text'] and original['sha256'] == result['sha256']
        self.capture('actual-attachment-memory-read-one-carrier')
        proof = {
            'actual_http_body_sha256': hashlib.sha256(bytes.fromhex(posts[1]['body_bytes_hex'])).hexdigest(),
            'actual_outgoing_read_result': result, 'actual_outgoing_result_json': result_text,
            'retained_original_read_result': original, 'retained_original_utf8_hex': original_bytes.hex(),
            'artifact': artifact, 'carrier': carrier, 'approved_memory': note,
            'capture_occurrences_in_context_and_result': len(occurrences),
            'resolving_aliases': aliases,
            'meaning': 'Actual initial context and read continuation; retained original read JSON was observed but not resent.',
        }
        save(self.output / 'actual-three-entry-dedup-proof.json', proof)
        self.workspace()
        return proof

    def measure_live_dedup_tokens(self, proof):
        before = self.effects()
        actual = proof['actual_outgoing_result_json'].encode('utf-8')
        original = bytes.fromhex(proof['retained_original_utf8_hex'])
        files = {'measurements/t03a-actual-sent-result.json': actual,
                 'measurements/t03a-retained-original-not-sent.json': original}
        for path, content in files.items():
            destination = self.project / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(content)
        self.session.command('context-clear')
        self.refresh()
        self.optimizer('optimize 20000')
        counts = {}

        def observed_counts():
            rows = self.state()['context_revisions']
            for path, content in files.items():
                digest = hashlib.sha256(content).hexdigest()
                matches = [row for row in rows if row['source_key'].startswith('file:')
                           and row['source_key'].endswith(':' + path + ':' + digest)
                           and row['source_sha256'] == digest]
                if len(matches) != 1:
                    return False
                row = matches[0]
                assert row['tokenizer'] == 'unicode-segmentation-1.13.3/UAX29-word-boundaries-v1'
                counts[path] = {'tokens': row['tokens'], 'bytes': len(content), 'sha256': digest,
                                'tokenizer': row['tokenizer'], 'cache_row': row}
            return True

        self.until(observed_counts, 'same real local tokenizer counted both exact public read-result files')
        actual_count = counts['measurements/t03a-actual-sent-result.json']['tokens']
        original_count = counts['measurements/t03a-retained-original-not-sent.json']['tokens']
        assert actual_count < original_count, 'actual capture-reference result did not save named local tokens'
        assert self.effects() == before, 'offline paired measurements dispatched work or a provider call'
        measurement = {'source': 'actual ev source worker file-tokenization rows',
                       'actual_continuation_http_sha256': proof['actual_http_body_sha256'],
                       'paired_files': counts, 'saved_local_read_result_tokens': original_count - actual_count,
                       'meaning': 'Actual sent read JSON vs exact retained pre-dedup read JSON, not full provider prompt/usage/billing.'}
        save(self.output / 'actual-three-entry-named-token-measurement.json', measurement)
        self.session.command('context-cancel')
        for path, content in files.items():
            observed_copy = self.output / path
            observed_copy.parent.mkdir(parents=True, exist_ok=True)
            observed_copy.write_bytes(content)
            (self.project / path).unlink()
        self.refresh()
        return measurement

    def run_optimizer(self):
        self.connect()
        decision_proof = self.seed_conflicting_decisions()
        source_fixture = self.seed_optimizer_sources()
        # Establish the committed multi-file baseline through a genuine restart,
        # not a SQL/cache seed or a false "all files changed" ranking fixture.
        self.restart()
        self.connect_again()
        assert self.task()['workflows']['chosen_decision'] == decision_proof['latest']
        assert self.task()['workflows']['criterion_origins'] == decision_proof['criterion_origins']
        self.form('espeon')
        self.write_request(REQUEST)
        selected = self.pin_range('calc.py', '1:2')
        shared_capture = self.pin_range('notes/shared_capture.txt')
        bound_memory = self.adopt_source_memory(shared_capture)
        assert '\r\n' in shared_capture['text'], 'fixture lost actual captured CRLF bytes'
        assert bound_memory['proposal']['content'].encode('utf-8') == shared_capture['text'].encode('utf-8'), \
            'source-memory adoption normalized the exact captured CRLF bytes'
        # A real direct caller and test are already present in RecordsJourney.
        # A separate changed source exercises all-source revision guarding.
        changed_path = self.project / 'changed.py'
        changed_original = changed_path.read_bytes()
        changed_path.write_text('value = 2\n', encoding='utf-8')
        self.refresh()
        before = self.effects()
        basket = self.state()['excerpts']
        draft = self.task()['partner_drafts']
        self.session.command('files')
        wait_for(self.session, 'eligible files /', timeout=35)
        self.session.send('o')
        self.wait_caption('Explicit local-token budget', '명시적 로컬 토큰 예산')
        self.session.send(HOME + DELETE * 16)
        self.session.send('alon')
        wait_for(self.session, 'alon')
        self.session.send('\r')
        wait_for(self.session, 'Use a local-token budget')
        self.capture('literal-shortcuts-invalid-budget-no-operation')
        assert self.effects() == before
        self.session.send(ESC)
        assert self.state()['excerpts'] == basket and self.task()['partner_drafts'] == draft

        cache_metrics = self.observe_optimizer_cache_metrics()
        restored_metrics = cache_metrics['restored']['metrics']
        assert restored_metrics['tokens'] < restored_metrics['baseline_tokens'], \
            'ranked packet did not reduce actual named-tokenizer input against full-file baseline'
        assert restored_metrics['tokens'] <= restored_metrics['budget']
        assert self.effects() == before, 'local preview dispatched execution/inference'
        self.apply()
        self.optimizer('optimize 64')
        refusal = '거부됨:' if self.language == 'ko' else 'REFUSED:'
        assert refusal in self.session.display(), 'mandatory refusal is hidden on the initial review'
        self.wait_caption('REFUSED preview; previous packet stays applied.',
                          '미리보기 거부됨 · 이전 적용 패킷 유지')
        self.wait_caption('Apply blocked', '적용 불가')
        overflow_metrics = self.read_preview_metrics()
        assert overflow_metrics['budget'] == 64 and overflow_metrics['tokens'] > 64
        self.reveal_optimizer_notice('REFUSED:', '거부됨:')
        self.session.send('a')
        wait_for(self.session, 'exceed budget')
        self.capture('mandatory-overflow-refuses-without-truncation')
        self.session.send('n')
        assert self.state()['excerpts'] == basket and self.task()['partner_drafts'] == draft
        assert self.effects() == before
        first_packet, first = self.exact_send('first-applied-ranked-packet')
        assert first['optimizer']['tokenizer'] == 'unicode-segmentation-1.13.3/UAX29-word-boundaries-v1'
        assert first['mandatory_instructions']
        self.assert_ranked_evidence(first, selected)
        assert len(first_packet) == restored_metrics['bytes']
        assert hashlib.sha256(first_packet).hexdigest() == cache_metrics['restored']['cache']['packet_sha256']
        save(self.output / 'ranked-named-tokenizer-proof.json', {
            'metrics': restored_metrics, 'actual_selected_packet_bytes': len(first_packet),
            'actual_selected_packet_sha256': hashlib.sha256(first_packet).hexdigest(),
            'source_fixture_paths': sorted(source_fixture),
            'required_evidence': ['amount - 1', 'normalize_total(amount)',
                                  'assert_eq!(crate::optimizer_math::normalize_total(0), 0)',
                                  'cedar_requirementunsupported'],
            'mandatory_pin': selected,
            'meaning': 'Actual local UAX29 boundary counts; not provider tokens, retention or billing.',
        })

        # An ordinary send consumes its Request draft. That legitimately revokes
        # a packet bound to those exact instructions; there is no prior applied
        # packet to preserve until the user restores, reviews and reapplies A.
        assert self.retained_request() == ''
        self.write_request(REQUEST)
        self.optimizer('optimize 20000')
        cancel_baseline = self.observe_optimizer_preview('cancel-baseline-reapplied-a')
        assert cancel_baseline['cache']['packet_sha256'] == hashlib.sha256(first_packet).hexdigest(), \
            'restoring identical source facts did not reconstruct the same exact packet A'
        self.apply()
        cancellation_draft = self.task()['partner_drafts']
        cancellation_pins = self.state()['excerpts']
        self.optimizer('optimize 4000')
        self.wait_caption('Preview not applied; previous packet still active.',
                          '미리보기 미적용 · 이전 적용 패킷 유지')
        self.session.command('context-cancel')
        assert self.task()['partner_drafts'] == cancellation_draft, 'cancel changed the Request draft'
        assert self.state()['excerpts'] == cancellation_pins, 'cancel changed mandatory pins'
        second_packet, second = self.exact_send('cancel-retains-previous-exact-ranked-packet')
        assert second_packet == first_packet and second == first, 'cancel replaced the previously applied packet'
        three_entry = self.prove_live_capture_dedup(shared_capture, bound_memory)
        three_entry_tokens = self.measure_live_dedup_tokens(three_entry)

        # The tool turn consumed its Request. Bind the new preview to the
        # exact next Request before changing only the observed source bytes.
        self.write_request(REQUEST)
        self.optimizer()
        self.apply()
        self.session.send(ESC)
        changed_path.write_text('value = 3\n', encoding='utf-8')
        self.write_request(REQUEST)
        posts = len(self.peer.snapshot('POST'))
        self.session.send('\r')
        self.until(lambda: 'changed after context preview' in self.session.display(),
                   'actual dispatch refusal for a source revision changed after apply')
        self.quiet_assert(lambda: len(self.peer.snapshot('POST')) == posts,
                          'stale applied packet reached synthetic transport')
        self.capture('source-revision-change-blocks-actual-provider-send')
        # No alternate route, retry or new provider request is authorized by refusal.
        self.session.command('cancel')
        self.until(lambda: not self.state()['active_work'],
                   'explicit cancellation released the blocked prepared request')
        assert len(self.peer.snapshot('POST')) == posts, 'cancelling a blocked preview resent it'
        self.session.send('\x13')
        self.until(lambda: self.retained_request() == REQUEST
                   and 'Storage: saved' in self.session.display(),
                   'blocked Request retained locally without a discard decision or resend')
        changed_path.write_bytes(changed_original)
        self.refresh()
        self.session.command('context-clear')
        self.workspace()
        failed = self.execute_check('optimizer', 'Failed')
        self.form('espeon')
        self.write_request(REQUEST)
        self.source.write_text('def total(a, b):\n    return a + b\n', encoding='utf-8')
        self.refresh()
        before_compact = self.effects()
        compact_facts = {key: self.task()[key] for key in
                         ('goal', 'criteria', 'next_action', 'workflows', 'partner_drafts')}
        self.optimizer('compact 20000')
        compact_cancel_metrics = self.read_preview_metrics()
        assert compact_cancel_metrics['kind'] == 'Evidence-linked compaction'
        self.session.send('n')
        assert self.effects() == before_compact
        assert self.state()['excerpts'] == basket
        assert {key: self.task()[key] for key in compact_facts} == compact_facts
        self.capture('compaction-cancel-keeps-facts-failure-pins-and-draft')
        self.optimizer('compact 20000')
        compact_metrics = self.read_preview_metrics()
        assert compact_metrics['kind'] == 'Evidence-linked compaction'
        assert self.effects() == before_compact, 'compaction executed work or called a provider'
        self.apply()
        ledger_packet, ledger = self.exact_send('applied-evidence-linked-compaction')
        assert ledger['format'] == 'eevee-evidence-ledger-v1'
        assert ledger['task_id'] == self.task()['id']
        assert ledger['current_approved_decision'] == decision_proof['latest']
        assert ledger['criterion_origins'] == decision_proof['criterion_origins']
        assert any(row['decision_id'] == decision_proof['first']['id']
                   and row['decision_text'] == decision_proof['first']['text']
                   and row['superseded_by'] == decision_proof['latest']['id']
                   for row in ledger['superseded_decisions']), \
            'compaction lost the real conflicting first choice or did not keep the latest choice'
        assert len(ledger_packet) == compact_metrics['bytes']
        source = next(row for row in ledger['source_observations']
                      if row['reference']['Source']['excerpt_id'] == selected['id'])
        assert source['observed_status'] == 'changed'
        historical = next(row for row in ledger['captures'] if row['id'] == selected['id'])
        assert historical == selected, 'compaction silently refreshed historical source bytes'
        assert len([row for row in ledger['captures'] if row['id'] == selected['id']]) == 1
        run = next(row for row in ledger['run_observations']
                   if row['run_id'] == failed['prepared']['run_id'])
        assert run['state'] == 'Failed' and run['exit_code'] == 1
        prepared = failed['prepared']
        outcomes = (
            'after', 'state', 'started_at_ms', 'ended_at_ms', 'exit_code', 'signal', 'tree',
            'cancelled', 'truncated', 'output_unavailable',
            'captured_bytes', 'discarded_bytes', 'dropped_live_bytes', 'error',
        )
        expected_run = {key: failed[key] for key in outcomes}
        # Durable RunRecord omits a false revision_moving flag by schema.
        expected_run['revision_moving'] = failed.get('revision_moving', False)
        owner = prepared.get('partner')
        expected_run.update({
            'request_id': prepared['request_id'], 'run_id': prepared['run_id'],
            'task_id': prepared['task_id'], 'partner_id': owner['partner'] if owner else None,
            'command': prepared['spec'], 'cwd': prepared['cwd'],
            'executable': prepared['executable'], 'before': prepared['before'],
            'rerun_of': prepared.get('rerun_of'),
        })
        assert run == expected_run, 'outgoing observation lost facts or transferred local control metadata'
        assert owner and type(run['partner_id']) is int and run['partner_id'] == owner['partner']
        forbidden_run_keys = {
            'prepared', 'partner', 'name', 'form', 'cwd_identity',
            'prepared_at_ms', 'expires_at_ms', 'approval_arrival',
        }
        def factual_keys(value):
            if isinstance(value, dict):
                assert forbidden_run_keys.isdisjoint(value), 'local identity/approval key reached actual wire'
                for nested in value.values():
                    factual_keys(nested)
            elif isinstance(value, list):
                for nested in value:
                    factual_keys(nested)
        factual_keys(run)
        assert owner['name'] not in json.dumps(run, ensure_ascii=False), \
            'local partner display name reached actual outgoing run evidence'
        assert len(self.invocations()) == 1, 'preview/apply/cancel silently reran failed command'
        assert self.state()['excerpts'] == basket
        save(self.output / 'compaction-observed-metrics-and-evidence.json', {
            'cancelled_preview_metrics': compact_cancel_metrics,
            'applied_preview_metrics': compact_metrics,
            'outgoing_packet_bytes': len(ledger_packet),
            'outgoing_packet_sha256': hashlib.sha256(ledger_packet).hexdigest(),
            'decisions': decision_proof, 'ledger': ledger,
            'actual_run_observation': run,
            'run_projection_kept_fields': sorted(expected_run),
            'run_projection_forbidden_fields_absent': sorted(forbidden_run_keys),
            'overflow_refusal_observed_metrics': overflow_metrics,
            'cancel_preserved_facts_and_drafts': compact_facts,
        })
        self.workspace()
        self.quit()
        return {'ranked_packet_sha256': hashlib.sha256(first_packet).hexdigest(),
                'named_tokenizer': restored_metrics['tokenizer'],
                'selected_tokens': restored_metrics['tokens'],
                'full_file_baseline_tokens': restored_metrics['baseline_tokens'],
                'cache_measurements': cache_metrics,
                'actual_three_entry_dedup': three_entry,
                'actual_three_entry_named_tokens': three_entry_tokens,
                'compaction_metrics': compact_metrics,
                'real_conflicting_decisions': decision_proof,
                'actual_inference_posts': len(self.peer.snapshot('POST')),
                'actual_failed_run': failed['prepared']['run_id'],
                'stale_dispatch_additional_posts': 0, 'evidence': self.evidence}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--language', choices=('en', 'ko'), default='en')
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    assert binary.is_file(), 'use an existing already-built binary'
    output.mkdir(parents=True, exist_ok=False)
    original = digest(binary)
    result = {'status': 'failed', 'binary': str(binary), 'binary_sha256': original,
              'host': platform.platform(), 'language': args.language, 'runs': [],
              'live_provider_proof': False, 'sql_writes': False,
              'scope': 'Actual Sources UI, safe local worker, failed owned command and loopback serialized transport'}
    try:
        for size, ascii_mode in (((120, 36), False), ((60, 18), True)):
            with tempfile.TemporaryDirectory(prefix='ev-context-optimizer-') as temporary:
                with Peer('chat_completions', 'unused-keyless-public-fixture', True,
                          retain_body_bytes=True) as peer:
                    journey = Journey.__new__(Journey)
                    try:
                        journey.__init__(binary, output / ('%dx%d' % size), Path(temporary).resolve(),
                                         size, ascii_mode, {}, peer, args.language)
                        proof = journey.run_optimizer()
                        result['runs'].append({'size': list(size), 'status': 'passed', **proof})
                    finally:
                        if hasattr(journey, 'sessions'):
                            journey.close()
            assert digest(binary) == original, 'immutable executable changed during acceptance'
        result['status'] = 'passed'
    except Exception as error:
        result.update(error=str(error), traceback=traceback.format_exc())
    finally:
        save(output / 'results.json', result)
    print(json.dumps({'status': result['status'], 'results': str(output / 'results.json')}))
    return 0 if result['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
