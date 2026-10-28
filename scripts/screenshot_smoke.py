#!/usr/bin/env python3
"""Opt-in GP06 actual TUI + SQLite + controlled native Responses journey.

Requires an already-built ev and existing smoke dependencies (pyte/wcwidth).
No SQL writes, production bypass, credentials, paid calls or live interpretation.
Terminal metadata is not visual review. Artifacts retain disposable public images.
"""
from __future__ import annotations
import argparse
import base64
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import struct
import tempfile
import traceback
import zlib

from native_connection_smoke import Peer
from receipts_smoke import ReceiptJourney
from records_smoke import ESC, RIGHT, PAGE
from response_smoke import SurfaceSession
from workbench_smoke import wait_for


def png(width=2, height=2, noisy=False):
    def chunk(name, body):
        return struct.pack('>I', len(body)) + name + body + struct.pack('>I', zlib.crc32(name + body))
    rows = b''.join(b'\0' + (os.urandom(width * 4) if noisy else bytes([17, 52, 91, 255]) * width) for _ in range(height))
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b'')


def jpeg():
    # Own solid-colour 2×2 image, encoded once with ImageMagick; no runtime image tool.
    return base64.b64decode(
        '/9j/4AAQSkZJRgABAQAAAAAAAAD/2wBDAAYEBQYFBAYGBQYHBwYIChAKCgkJChQODwwQFxQYGBcUFhYaHSUfGhsjHBYWICwgIyYnKSopGR8tMC0oMCUoKSj/2wBDAQcHBwoIChMKChMoGhYaKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCj/wAARCAACAAIDASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAf/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/8QAFAEBAAAAAAAAAAAAAAAAAAAABf/EABQRAQAAAAAAAAAAAAAAAAAAAAD/2gAMAwEAAhEDEQA/AIkAdDv/2Q==',
        validate=True,
    )


class ScreenshotJourney(ReceiptJourney):
    def launch(self):
        self.environment.update(EV_API_BASE_URL=self.peer.base, EV_API_PROTOCOL='responses',
                                EV_API_MODEL='gpt-4.1', EV_API_KEYLESS='1')
        self.serial += 1
        quota = getattr(self, 'quota_kib', None)
        self.session = SurfaceSession([
            str(self.binary), '--data-dir', str(self.data), '--images', 'off', '--color', 'mono',
            *(['--ascii'] if self.ascii_mode else []),
            *(['--quota-kib', str(quota)] if quota is not None else []),
        ], self.project, self.environment, self.size, self.output / ('terminal-%02d' % self.serial), controlling_tty=True)
        self.sessions.append(self.session)
        wait_for(self.session, 'Storage: saved', timeout=35)

    def workspace(self, allow_offer=False):
        if self.session and any(title in self.session.display() for title in (
                'Reviewed screenshot input', 'Explicit screenshot path', 'Review screenshot removal')):
            self.session.send(ESC)
        return super().workspace(allow_offer)

    def connect(self):
        self.workspace()
        self.session.command('provider-compatible')
        self.session.command('model gpt-4.1')
        if 'Reasoning level' in self.session.display():
            self.session.send('\r')
        self.workspace()
        assert not self.peer.snapshot('POST'), 'configuration/navigation must not send inference'

    def images(self):
        with closing(sqlite3.connect(self.database.as_uri() + '?mode=ro', uri=True, timeout=10)) as db:
            db.execute('PRAGMA query_only=ON')
            return [{'task_id': t, 'partner': p, 'capture': json.loads(c), 'bytes': b, 'selected': s}
                    for t, p, c, b, s in db.execute('SELECT task_id,partner,capture,bytes,selected FROM screenshots ORDER BY task_id,partner')]

    def refresh_image(self):
        self.workspace()
        self.session.command('diff')
        wait_for(self.session, 'Repository', timeout=35)
        self.session.command('refresh')
        wait_for(self.session, 'Repository: refreshed', timeout=35)
        self.workspace()

    def review_image(self, filename):
        self.workspace()
        self.session.command('screenshot')
        wait_for(self.session, 'Explicit screenshot path')
        self.session.paste(filename)
        self.session.send('\r')
        wait_for(self.session, 'Reviewed screenshot input', timeout=35)
        # Every page is an actual drained PTY frame; End cannot authorize unseen rows.
        pages = []
        for index in range(30):
            pages.append(self.session.display())
            self.session.capture('screenshot-disclosure-%02d' % index)
            if 'Enter: attach exact image locally' in pages[-1]:
                break
            self.session.send(PAGE)
        else:
            raise AssertionError('complete supported disclosure never became eligible\n' + self.session.display())
        return pages

    def attach(self, filename, expected):
        self.review_image(filename)
        self.session.send('\r')
        self.until(lambda: len(self.images()) == 1 and self.images()[0]['selected'] == 1,
                   'single-writer screenshot commit')
        row = self.images()[0]
        assert row['bytes'] == expected and bytes(row['capture']['sha256']) == hashlib.sha256(expected).digest()
        self.workspace()
        wait_for(self.session, f'image ({len(expected)} B)', timeout=35)
        self.capture('selected-image-visible-before-send')
        return row

    def removal_review(self, command):
        self.session.command(command)
        wait_for(self.session, 'Review screenshot removal')
        self.session.send('\x1b[5~' * 30)
        for _ in range(30):
            if 'Keep / cancel' in self.session.display():
                return
            self.session.send(PAGE)
        raise AssertionError('complete exact removal scope never became eligible\n' + self.session.display())

    def refuse_send(self, marker):
        before = len(self.peer.snapshot('POST'))
        self.write_request(marker)
        self.session.send('\r')
        refusals = ('Request blocked: Screenshot changed',
                    'Request blocked: Screenshot source is now ignored')
        self.until(lambda: any(reason in ' '.join(self.session.display().split()) for reason in refusals),
                   'explicit changed/deleted screenshot refusal, not the persistent selected-image indicator')
        assert len(self.peer.snapshot('POST')) == before, 'invalidated image reached transport'
        self.capture(marker)
        # A blocked frozen request retains its owner until an explicit cancel.
        # Refusal is not permission to silently release/retry the user's work.
        self.session.command('cancel')
        self.until(lambda: not self.state()['active_work'], 'refused screenshot claim settles')
        assert len(self.peer.snapshot('POST')) == before, 'invalidated image reached transport'
        self.capture(marker + '-explicit-cancel-settled')
        self.workspace()

    def run_images(self):
        self.connect()
        self.form('espeon')
        original = png()
        shot = self.project / 'public screenshot.png'
        shot.write_bytes(original)
        self.refresh_image()
        posts = len(self.peer.snapshot('POST'))
        self.review_image(shot.name)
        self.session.send(ESC)
        assert not self.images() and len(self.peer.snapshot('POST')) == posts, 'cancel saved/sent an image'
        retained = self.attach(shot.name, original)
        self.restart()
        assert self.images() == [retained] and len(self.peer.snapshot('POST')) == posts, 'restart replayed approval or changed bytes'
        self.session.command('screenshot-show')
        self.session.send('\x1b[5~' * 30)
        for page in range(30):
            before_page = self.session.display()
            self.session.capture('restart-retained-disclosure-%02d' % page)
            self.session.send(PAGE)
            if self.session.display() == before_page:
                break
        self.capture('restart-retained-metadata-no-replay')
        self.workspace()
        wire = self.send_request('Describe this explicitly attached public image; do not edit files.', 'Fixture image received; no interpretation claim.')
        body = wire['body']
        parts = [part for message in body['input'] if isinstance(message.get('content'), list)
                 for part in message['content'] if part.get('type') == 'input_image']
        assert len(parts) == 1 and parts[0]['detail'] == 'auto'
        assert base64.b64decode(parts[0]['image_url'].split(',', 1)[1], validate=True) == original
        assert body['model'] == 'gpt-4.1' and body['store'] is False
        self.until(lambda: len(self.state()['outgoing_receipts']) == 1, 'image receipt committed')
        receipt = self.state()['outgoing_receipts'][0]
        assert receipt['source'] is None, 'receipt duplicated retained binary as base64'
        assert receipt['record']['image']['compressed_bytes'] == len(original)
        assert receipt['record']['image']['tokens'] is None
        assert receipt['record']['bytes'] == len(bytes.fromhex(wire['body_bytes_hex']))
        shot.write_bytes(png(3, 2))
        self.refuse_send('changed screenshot refuses dispatch')
        shot.unlink()
        self.refuse_send('deleted screenshot refuses dispatch')
        shot.write_bytes(original)
        self.refresh_image()
        self.removal_review('screenshot-remove')
        self.session.send('\r')  # neutral Keep
        assert self.images()[0]['selected'] == 1
        self.removal_review('screenshot-remove')
        self.session.send(RIGHT + ESC)
        assert self.images()[0]['selected'] == 1
        self.removal_review('screenshot-remove')
        self.session.send(RIGHT + '\r')
        self.until(lambda: self.images()[0]['selected'] == 0, 'image deselection acknowledgment')
        self.workspace()
        without = self.send_request('No image is attached to this request.', 'Fixture text-only response after removal.')
        assert 'input_image' not in json.dumps(without['body']), 'removed image leaked through native history'
        self.restart()
        assert self.images()[0]['bytes'] == original and self.images()[0]['selected'] == 0
        self.removal_review('screenshot-purge')
        self.session.send(RIGHT + '\r')
        self.until(lambda: not self.images(), 'committed compressed image purge')
        self.workspace()
        self.restart()
        assert not self.images() and len(self.peer.snapshot('POST')) == posts + 2
        self.capture('restart-after-purge-no-binary-no-replay')
        jpeg_bytes = jpeg()
        jpeg_shot = self.project / 'public screenshot.jpg'
        jpeg_shot.write_bytes(jpeg_bytes)
        self.refresh_image()
        self.attach(jpeg_shot.name, jpeg_bytes)
        jpeg_wire = self.send_request('Describe this explicit public JPEG; do not edit files.', 'Fixture JPEG received; no interpretation claim.')
        jpeg_parts = [part for message in jpeg_wire['body']['input'] if isinstance(message.get('content'), list)
                      for part in message['content'] if part.get('type') == 'input_image']
        assert len(jpeg_parts) == 1 and jpeg_parts[0]['image_url'].startswith('data:image/jpeg;base64,')
        assert base64.b64decode(jpeg_parts[0]['image_url'].split(',', 1)[1], validate=True) == jpeg_bytes
        self.removal_review('screenshot-purge')
        self.session.send(RIGHT + '\r')
        self.until(lambda: not self.images(), 'exact JPEG capture purge acknowledgement')
        self.workspace()
        assert len(self.peer.snapshot('POST')) == posts + 3
        return {'image_sha256': hashlib.sha256(original).hexdigest(), 'wire_image_exact': True,
                'jpeg_sha256': hashlib.sha256(jpeg_bytes).hexdigest(), 'jpeg_wire_exact': True,
                'image_tokens': None, 'posts': posts + 3, 'capture_cancel_remove_changed_deleted_restart_purge': True}

    def run_quota(self):
        self.connect()
        shot = self.project / 'quota.png'
        shot.write_bytes(png(256, 256, noisy=True))
        self.refresh_image()
        self.quit()
        with closing(sqlite3.connect(self.database.as_uri() + '?mode=ro', uri=True)) as db:
            live = (db.execute('PRAGMA page_count').fetchone()[0] - db.execute('PRAGMA freelist_count').fetchone()[0]) * db.execute('PRAGMA page_size').fetchone()[0]
        self.quota_kib = (live + 1023) // 1024 + 64
        self.launch()
        self.workspace()
        self.review_image(shot.name)
        self.session.send('\r')
        self.until(lambda: 'storage failed' in self.session.display().lower() or 'quota' in self.session.display().lower(), 'image quota refusal')
        assert not self.images() and not self.peer.snapshot('POST'), 'quota refusal saved/sent image'
        self.capture('quota-refused-no-image-selection-no-post')
        return {'quota_kib': self.quota_kib, 'compressed_bytes': shot.stat().st_size, 'selected': False, 'posts': 0}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', action='store_true', help='Explicitly opt in to disposable actual TUI journeys')
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not args.run:
        parser.error('--run is required; this driver launches a real disposable TUI')
    args.output.mkdir(parents=True, exist_ok=True)
    results = []
    for label, size, ascii_mode, quota in [('wide', (120, 36), False, False), ('compact', (60, 18), True, False), ('quota', (120, 36), False, True)]:
        with tempfile.TemporaryDirectory(prefix='ev-screenshot-') as temporary:
            peer = Peer('responses', 'unused-keyless-fixture', True, retain_body_bytes=True)
            peer.__enter__()
            journey = None
            row = {'case': label, 'status': 'failed', 'size': size}
            try:
                journey = ScreenshotJourney(args.binary.resolve(), args.output / label, Path(temporary), size, ascii_mode, [], peer)
                row.update(journey.run_quota() if quota else journey.run_images())
                peer.assert_clean()
                row['status'] = 'passed'
            except Exception:
                row['error'] = traceback.format_exc()
                raise
            finally:
                if journey:
                    for session in journey.sessions:
                        session.close()
                peer.__exit__(None, None, None)
                row['requests'] = peer.snapshot()
                results.append(row)
                (args.output / 'results.json').write_text(json.dumps({'scope': 'Controlled loopback Responses only; no live model interpretation', 'cases': results}, indent=2) + '\n')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
