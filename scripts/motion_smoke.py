#!/usr/bin/env python3
"""Real PTY motion preference/receipt journeys; deterministic loopback, no live model.

Runs both supported sizes and languages. No assistive-technology or native-pixel
qualification is inferred from the terminal-cell captures.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import time
from unittest.mock import patch

import file_creation_smoke as files
import response_smoke as response
from terminal_smoke import confirm_exit
from workbench_smoke import wait_for

ESC = '\x1b'
TITLES = ('Evolution!', '진화했어요!')


def raw(session, keys):
    session.events.append({'input': repr(keys), 'bounded_transition_input': True})
    if os.name == 'nt':
        session.process.write(keys)
    else:
        pending = keys.encode()
        while pending:
            pending = pending[os.write(session.master, pending):]


def preference(data):
    with sqlite3.connect((data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True) as db:
        row = db.execute('SELECT record FROM provider_preferences WHERE id=1').fetchone()
    return json.loads(row[0]) if row else {}


def still(session, duration=.5):
    session.pump(.8)
    before = len(session.text)
    session.pump(duration)
    assert len(session.text) == before, 'static or idle surface kept redrawing'


class Journey(response.SurfaceJourney):
    def __init__(self, *args, **kwargs):
        original = response.SurfaceSession
        def remember(argv, cwd, environment, size, output, **options):
            environment['NO_COLOR'] = '1'  # Color is not motion intent.
            argv = [*argv, '--images', 'cells']
            self.argv, self.environment, self.size = argv, dict(environment), size
            return original(argv, cwd, environment, size, output, **options)
        with patch.object(response, 'SurfaceSession', remember):
            super().__init__(*args, **kwargs)

    def restart(self, name, extra=()):
        self.session.command('quit')
        confirm_exit(self.session)
        assert self.session.wait_exit() == 0
        self.session.close()
        self.session = response.SurfaceSession([*self.argv, *extra], self.project,
            self.environment, self.size, self.output / name, language=self.language)
        wait_for(self.session, 'Storage: saved')
        self.session.pump(.8)

    def picker(self, right=1, animate=True, skip=None, paste=True):
        session = self.session
        session.command('forms')
        session.send((ESC + '[C') * right)
        raw(session, '\r')
        frames = set()
        started = time.monotonic()
        skipped = False
        while time.monotonic() - started < .45:
            session.pump(.012)
            screen = session.display()
            if any(title in screen for title in TITLES):
                rows = screen.splitlines()
                frames.add(tuple(row for row in rows if '>' in row and any(
                    name in row for name in ('Eevee', 'Vaporeon', 'Jolteon', 'Flareon', 'Espeon', 'Umbreon', 'Leafeon', 'Glaceon', 'Sylveon'))))
                if skip is not None and not skipped:
                    raw(session, ESC + '[200~' + skip + ESC + '[201~' if paste else skip)
                    skipped = True
        if animate and skip is None:
            assert len({row for row in frames if row}) >= 2, 'picker did not show bounded visible motion with NO_COLOR'
        if not animate:
            assert not any(frames), 'static route rendered an animated picker arrow'
        if skip is not None:
            assert skipped and not any(title in session.display() for title in TITLES)
        return frames


def run(binary, output, root, size, language, previous_binary=None):
    root.mkdir()
    output.mkdir(parents=True)
    cases = []
    with response.StreamingPeer() as peer:
        journey = Journey(binary, output, root, size, False, 'compatible', 'chat_completions', True, False, peer, language=language)
        session = journey.session
        try:
            wait_for(session, 'Storage: saved')
            journey.setup(initial=True)
            journey.write_request('Motion fixture question')
            session.send('\r')
            files.until(session, peer.started.is_set, 'real provider activity never started')
            wait_for(session, 'R079w19')
            screens = set()
            for _ in range(5):
                session.pump(.14)
                screens.add(session.display())
            assert len(screens) >= 2, 'NO_COLOR suppressed real provider motion'
            session.command('motion-off')
            files.until(session, lambda: preference(journey.data).get('reduced_motion') is True,
                        'persistent motion preference did not commit')
            still(session)
            session.capture('persistent-static-real-provider-activity')
            assert len(peer.snapshot('POST')) == 1
            peer.release_append.set()
            peer.release_finish.set()
            files.until(session, lambda: journey.surface['complete'] in session.display(), 'stream did not finish')
            cases.append('real activity animates with NO_COLOR, persistent optout freezes it without changing owned work')
            journey.restart('restart-reduced')
            # The loopback connection was deliberately session-only. Restore it
            # explicitly; motion persistence must not restore connection authority.
            journey.setup(initial=True)
            peer.stream_used = False
            peer.started.clear()
            peer.release_append.clear()
            peer.release_finish.clear()
            journey.write_request('Restarted static activity fixture')
            journey.session.send('\r')
            files.until(journey.session, peer.started.is_set, 'restarted preference was not exercised against real activity')
            wait_for(journey.session, 'R079w19')
            still(journey.session)
            journey.session.capture('restarted-preference-real-static-activity')
            peer.release_append.set()
            peer.release_finish.set()
            files.until(journey.session, lambda: journey.surface['complete'] in journey.session.display(), 'restarted stream did not finish')
            journey.workspace()
            journey.write_request('draft界e\u0301')
            session = journey.session
            session.send(ESC + '[D')
            journey.picker(right=3, animate=False)
            session.capture('static-complete-receipt-after-restart')
            still(session)
            journey.restart('explicit-animation', ['--animation'])
            journey.write_request('exact future draft')
            journey.picker(right=1, animate=True)
            still(journey.session)
            journey.session.capture('bounded-picker-static-result')
            # Same-form Enter must not manufacture another receipt or timer.
            journey.session.command('forms')
            journey.session.send('\r')
            assert not any(title in journey.session.display() for title in TITLES)
            still(journey.session)
            journey.session.send(ESC + '[D' + ESC + '[D')
            journey.picker(right=1, animate=True, skip='!')
            journey.session.send('\x13')
            files.until(journey.session, lambda: response.saved_request(journey, 'exact future dra!ft', len('exact future dra!')),
                        'skip paste was lost, duplicated, or sent to another editor')
            assert len(peer.snapshot('POST')) == 2, 'picker skip input granted or submitted work'
            journey.picker(right=1, animate=True, skip='x', paste=False)
            journey.session.send('\x13')
            files.until(journey.session, lambda: response.saved_request(journey, 'exact future dra!xft', len('exact future dra!x')),
                        'skip key was lost, duplicated, or sent to another editor')
            assert len(peer.snapshot('POST')) == 2, 'picker skip key granted or submitted work'
            cases.append('CLI animation overrides saved reduction, picker is bounded, same-form is silent, key and paste forward exactly once')
            # A receipt hidden immediately by another surface returns only as
            # its complete static result, never queued motion.
            raw(journey.session, '\x0bforms\r' + ESC + '[C\r\x0bhelp\r')
            journey.session.pump(.5)
            journey.session.send(ESC)
            journey.session.pump(.4)
            still(journey.session)
            journey.session.capture('unseen-transition-no-replay')
            # Rapid deliberate commits replace the one receipt/timer.
            raw(journey.session, '\x0bforms\r' + ESC + '[C\r\x0bforms\r' + ESC + '[C\r')
            journey.session.pump(.5)
            still(journey.session)
            journey.session.capture('rapid-switches-one-static-result')
            journey.session.command('forms')
            journey.session.send(ESC + '[H')
            raw(journey.session, '\r')
            journey.session.pump(.03)
            journey.session.resize((size[0] + 1, size[1]))
            journey.session.resize(size)
            journey.session.pump(.4)
            assert 'Eevee' in journey.session.display()
            still(journey.session)
            journey.restart('explicit-static', ['--animation', '--no-animation'])
            journey.picker(right=3, animate=False)
            still(journey.session)
            assert preference(journey.data).get('reduced_motion') is True, 'CLI override mutated saved preference'
            cases.append('neutral return and resize retain a complete static receipt; last CLI choice wins without preference mutation')
            journey.session.command('quit')
            confirm_exit(journey.session)
            assert journey.session.wait_exit() == 0
            if previous_binary:
                journey.session.close()
                def stored_bytes():
                    return {str(path.relative_to(journey.data)): hashlib.sha256(path.read_bytes()).hexdigest()
                            for path in journey.data.rglob('*') if path.is_file()}
                before = stored_bytes()
                previous = response.SurfaceSession([str(previous_binary), *journey.argv[1:]],
                    journey.project, journey.environment, journey.size, output / 'previous-writer',
                    language=language)
                try:
                    wait_for(previous, 'Storage: save failed', timeout=35)
                    previous.command('inspect')
                    previous.capture('unsupported-motion-reader-refused')
                    assert stored_bytes() == before, 'older writer changed the store on preference refusal'
                finally:
                    previous.close()
                assert stored_bytes() == before, 'older writer changed data on exit'
                journey.session = response.SurfaceSession(journey.argv, journey.project,
                    journey.environment, journey.size, output / 'current-resume', language=language)
                wait_for(journey.session, 'Storage: saved')
                assert preference(journey.data).get('reduced_motion') is True
                journey.session.command('quit')
                confirm_exit(journey.session)
                assert journey.session.wait_exit() == 0
                cases.append('actual previous writer refused the new preference without changing bytes; current writer resumed')
            return {'status': 'passed', 'size': list(size), 'language': language, 'cases': cases}
        finally:
            peer.release_append.set()
            peer.release_finish.set()
            journey.session.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--previous-binary', type=Path)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    results = {'status': 'failed', 'binary': str(binary), 'sha256': digest, 'journeys': [],
               'limits': 'Loopback model fixture and actual PTYs; no assistive setup, physical terminal, or native image acceptance claimed.'}
    try:
        for language in ('en', 'ko'):
            for size in ((120, 36), (60, 18)):
                with tempfile.TemporaryDirectory(prefix='ev-motion-界-') as temporary:
                    results['journeys'].append(run(binary, output / f'{language}-{size[0]}', Path(temporary) / 'journey', size, language,
                                                  args.previous_binary.resolve() if args.previous_binary else None))
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest
        results['status'] = 'passed'
    finally:
        files.save(output / 'results.json', results)
    print(json.dumps(results, ensure_ascii=False))


if __name__ == '__main__':
    main()
