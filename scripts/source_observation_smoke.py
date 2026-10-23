#!/usr/bin/env python3
"""Actual ev PTY source observations through native HTTP, controlled Codex and ACP.

Disposable projects contain >64 KiB Unicode/CRLF source and an exhausted filename
inventory. Synthetic peers request tools but never read or write project files.
No private credential, paid model or live-provider conformance claim is involved.
"""
from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import shlex
import sqlite3
import sys
import tempfile
import traceback

SOURCE_PATH = 'src/central-cafe\u0301.rs'
SOURCE = ''.join(f'// actual central source {line:04}: café 界 dirty bytes\r\n'
                 for line in range(1, 3001))
SLICE = ''.join(SOURCE.splitlines(keepends=True)[2499:2502])
ANSWER = 'SOURCE_OBSERVATION_COMPLETE'


def acp_peer():
    """A protocol-only peer: all source bytes must come from client callbacks."""
    log = Path(os.environ['EV_SOURCE_PEER_LOG'])

    def record(direction, packet):
        with log.open('a', encoding='utf-8') as stream:
            stream.write(json.dumps(dict(direction=direction, packet=packet)) + '\n')

    def receive():
        line = sys.stdin.readline()
        if not line:
            raise SystemExit(0)
        packet = json.loads(line)
        record('client', packet)
        return packet

    def emit(**packet):
        value = dict(jsonrpc='2.0', **packet)
        record('peer', value)
        print(json.dumps(value), flush=True)

    root = None
    session = 'source-observation'
    while True:
        packet = receive()
        method, identity = packet.get('method'), packet.get('id')
        if method == 'initialize':
            assert packet['params']['clientCapabilities']['fs']['readTextFile']
            emit(id=identity, result=dict(protocolVersion=1, agentCapabilities={}))
        elif method == 'session/new':
            root = Path(packet['params']['cwd'])
            emit(id=identity, result=dict(sessionId=session))
        elif method == 'session/prompt':
            prompt_id = identity
            break
        else:
            raise AssertionError(packet)

    def call(identity, method, **params):
        emit(id=identity, method=method, params=dict(sessionId=session, **params))
        while True:
            packet = receive()
            if packet.get('id') == identity:
                return packet

    ranged = call('ranged', 'fs/read_text_file', path=str(root / SOURCE_PATH), line=2500, limit=3)
    assert ranged['result']['content'] == SLICE, ranged
    tail = call('tail', 'fs/read_text_file', path=str(root / SOURCE_PATH), line=3000, limit=50)
    assert tail['result']['content'] == SOURCE.splitlines(keepends=True)[-1], tail
    unbounded = call('unbounded', 'fs/read_text_file', path=str(root / SOURCE_PATH))
    assert 'error' in unbounded and 'narrower line/limit' in json.dumps(unbounded), unbounded
    denied = call('oversized-write', 'fs/write_text_file', path=str(root / SOURCE_PATH), content='must not replace\n')
    assert 'error' in denied, denied
    emit(method='session/update', params=dict(sessionId=session, update=dict(
        sessionUpdate='agent_message_chunk', content=dict(type='text', text=ANSWER))))
    emit(id=prompt_id, result=dict(stopReason='end_turn'))
    while sys.stdin.readline():
        pass


if '--acp-peer' in sys.argv:
    if '--version' in sys.argv or 'version' in sys.argv:
        print('1.18.0')
    else:
        acp_peer()
    raise SystemExit(0)

from file_creation_smoke import (CodexCreationJourney, CreationJourney, Peer,
                                 response_plan, save, tool_results)
from native_connection_smoke import until
from response_smoke import SurfaceSession, SURFACE
from terminal_smoke import confirm_exit, inspect_permission_scope, permission_run_available, terminal_environment
from workbench_smoke import wait_for

ESC = '\x1b'


def fixture(project, *, large_tree):
    target = project / SOURCE_PATH
    target.parent.mkdir(exist_ok=True)
    target.write_bytes(SOURCE.encode())
    assert target.stat().st_size > 64 * 1024
    if large_tree:
        inventory = project / 'inventory'
        inventory.mkdir()
        for index in range(10_001):
            (inventory / (f'{index:05}-' + 'a' * 80 + '.txt')).touch()


def calls():
    return [
        dict(id='root', name='eevee_list_files', arguments=dict(directory='', query='central', offset=0)),
        dict(id='scoped', name='eevee_list_files', arguments=dict(directory='src', query='', offset=0)),
        dict(id='read', name='eevee_read_file', arguments=dict(path=SOURCE_PATH, start_line=2500, end_line=2502)),
        dict(id='oversized', name='eevee_write_file', arguments=dict(path=SOURCE_PATH, content='must not replace\n')),
    ]


def assert_observations(root, scoped, read):
    assert root['status'] == 'refused' and 'narrower directory scope' in root['message'], root
    assert scoped['paths'] == [SOURCE_PATH] and scoped['total'] == 1 and scoped['complete'], scoped
    assert read['content'] == SLICE and read['bytes'] == len(SOURCE.encode()), read
    assert read['total_lines'] == 3000 and read['start_line'] == 2500 and read['end_line'] == 2502, read
    assert read['truncated'] and not read['editable'] and read['digest_scope'] == 'full_file', read
    assert read['sha256'] == hashlib.sha256(SOURCE.encode()).hexdigest(), read


def native(binary, output, size, language):
    output.mkdir(parents=True)
    journey = None
    result = dict(status='failed', route='native-responses', size=list(size), language=language)
    with tempfile.TemporaryDirectory(prefix='ev-source-native-') as temporary, \
            Peer('responses', 'sk-source-fixture-only', True, retain_body_bytes=True) as peer:
        try:
            journey = CreationJourney(binary, output, Path(temporary).resolve(), size, size[0] == 60,
                                      'compatible', 'responses', True, False, peer, language=language)
            wait_for(journey.session, 'Storage: saved')
            journey.setup_route()
            fixture(journey.project, large_tree=True)
            journey.session.command('role-implement')
            journey.workspace()
            journey.write_request('Implement the source change after reading the central range, but refuse an oversized replacement.'
                                  if language == 'en' else '중앙 소스 범위를 읽고 수정해줘. 단 편집 한도를 넘는 큰 파일 교체는 거절해줘.')
            before = len(peer.snapshot('POST'))
            for index, call in enumerate(calls()):
                peer.plans.put(response_plan(peer, calls=[call], number=index))
            peer.plans.put(response_plan(peer, text=ANSWER, number=4))
            journey.session.send('\r', settle=False)
            approved = False

            def dispatched():
                nonlocal approved
                if len(peer.snapshot('POST')) > before:
                    return True
                if not approved and permission_run_available(journey.session):
                    journey.session.send('y', settle=False)
                    approved = True
                elif not approved:
                    inspect_permission_scope(journey.session)
                return False

            until(journey.session, dispatched, 'source observation request did not dispatch', timeout=40)
            journey.settled(ANSWER, before + 5)
            observed = tool_results('responses', peer.snapshot('POST')[before + 4]['body'])
            values = {key: json.loads(value) for key, value in observed.items()}
            assert_observations(values['root'], values['scoped'], values['read'])
            assert values['oversized']['status'] == 'refused', values['oversized']
            assert (journey.project / SOURCE_PATH).read_bytes() == SOURCE.encode()
            assert not journey.snapshot()['claims'] and journey.snapshot()['executions'] == 0
            journey.capture('oversized-source-scoped-observation-and-mutation-refusal')
            journey.session.command('quit')
            confirm_exit(journey.session)
            assert journey.session.wait_exit() == 0
            peer.assert_clean()
            result['status'] = 'passed'
        except Exception as error:
            result.update(error=str(error), traceback=traceback.format_exc())
        finally:
            save(output / 'http.json', peer.snapshot())
            if journey:
                journey.session.close()
            save(output / 'results.json', result)
    return result


def codex(binary, output, size, language):
    output.mkdir(parents=True)
    journey = None
    result = dict(status='failed', route='codex', size=list(size), language=language)
    try:
        journey = CodexCreationJourney(binary, output, size, language)
        journey.setup(count=1)
        journey.session.command('eevee')
        journey.session.command('role-implement')
        journey.workspace()
        fixture(journey.project, large_tree=True)
        marker = 'COOP_SOURCE_OBSERVATION'
        journey.request(1, marker)
        journey.publish(marker + '.prompt', json.dumps([dict(name=call['name'], arguments=call['arguments'])
                                                       for call in calls()]))
        journey.until(lambda: len(journey.peers('answer', marker)) == 4,
                      'controlled source observations and refusal did not settle')
        answers = [row['result'] for row in journey.peers('answer', marker)]
        assert not answers[0]['success'] and 'narrower directory scope' in json.dumps(answers[0]), answers[0]
        assert answers[1]['success'] and answers[2]['success'] and not answers[3]['success'], answers
        scoped, read = [json.loads(row['contentItems'][0]['text']) for row in answers[1:3]]
        root = dict(status='success' if answers[0]['success'] else 'refused',
                    message=answers[0]['contentItems'][0]['text'])
        assert_observations(root, scoped, read)
        assert (journey.project / SOURCE_PATH).read_bytes() == SOURCE.encode()
        journey.finish(marker)
        journey.until(lambda: not journey.claims(), 'controlled observation ownership did not settle')
        journey.workspace()
        journey.session.capture('oversized-source-full-digest-scoped-inventory')
        journey.quit()
        result['status'] = 'passed'
    except Exception as error:
        result.update(error=str(error), traceback=traceback.format_exc())
    finally:
        if journey:
            journey.close()
        save(output / 'results.json', result)
    return result


def acp(binary, output, size, language):
    output.mkdir(parents=True)
    result = dict(status='failed', route='acp-opencode', size=list(size), language=language)
    session = None
    with tempfile.TemporaryDirectory(prefix='ev-source-acp-') as temporary:
        root = Path(temporary).resolve()
        project, runtime, home = (root / name for name in ('project', 'runtime', 'home'))
        for directory in (project, runtime, home):
            directory.mkdir()
        fixture(project, large_tree=False)
        data = project / '.eevee-data'
        database = data / 'workbench.sqlite3'
        log = root / 'peer.jsonl'
        wrapper = runtime / 'opencode'
        wrapper.write_text('#!/bin/sh\nexec ' + shlex.quote(sys.executable) + ' ' +
                           shlex.quote(str(Path(__file__).resolve())) + ' --acp-peer "$@"\n')
        wrapper.chmod(0o700)
        environment = terminal_environment('mono')
        for key in list(environment):
            if key.startswith(('EV_', 'OPENAI_', 'CODEX_', 'ANTHROPIC_', 'CLAUDE_', 'GOOGLE_', 'GEMINI_', 'DEVIN_')) or key.lower().endswith('_proxy'):
                environment.pop(key, None)
        environment.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / 'config'),
                           XDG_DATA_HOME=str(home / 'data'), XDG_CACHE_HOME=str(home / 'cache'),
                           PATH=str(runtime) + os.pathsep + environment.get('PATH', ''),
                           EV_PROVIDER='opencode', EV_SOURCE_PEER_LOG=str(log))

        def state():
            with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
                return db.execute('SELECT count(*) FROM active_work').fetchone()[0]

        try:
            argv = [str(binary), '--data-dir', str(data), '--images', 'off', '--color', 'mono']
            if size[0] == 60:
                argv.append('--ascii')
            session = SurfaceSession(argv, project, environment, size, output, language=language)
            wait_for(session, 'Storage: saved', timeout=40)
            session.command('role-implement')
            for _ in range(12):
                footer = session.display().rstrip().splitlines()[-1].strip()
                if 'Alt+A' in footer or footer.startswith(tuple(SURFACE[language][key] for key in ('home', 'edit', 'draft'))):
                    break
                session.send(ESC)
            if footer.startswith(SURFACE[language]['home']):
                session.send('\r')
            session.paste('Read the exact central source range and refuse an oversized replacement.'
                          if language == 'en' else '중앙 소스의 정확한 범위를 읽고 너무 큰 파일 교체는 거절해줘.')
            session.send('\x13')
            session.send('\r', settle=False)
            neutral = ('Enter to review (not approve)', 'Enter 검토 (승인 아님)')
            until(session, lambda: any(label in session.display() for label in neutral),
                  'ACP external-policy neutral review was not presented', timeout=40)
            session.send('\r')

            def review():
                if permission_run_available(session):
                    return True
                session.send('\x1b[6~')
                return False

            until(session, review, 'ACP external-policy Run decision missing', timeout=40)
            session.send('y', settle=False)
            until(session, lambda: ANSWER in session.display() and state() == 0,
                  'ACP ranged source callback did not settle', timeout=40)
            packets = [json.loads(line) for line in log.read_text().splitlines()]
            replies = {row['packet'].get('id'): row['packet'] for row in packets
                       if row['direction'] == 'client' and row['packet'].get('id') in ('ranged', 'tail', 'unbounded', 'oversized-write')}
            assert replies['ranged']['result']['content'] == SLICE
            assert 'error' in replies['unbounded'] and 'error' in replies['oversized-write']
            assert (project / SOURCE_PATH).read_bytes() == SOURCE.encode()
            save(output / 'peer.json', packets)
            session.capture('acp-range-beyond-write-bound-and-explicit-output-refusal')
            session.command('quit')
            confirm_exit(session)
            assert session.wait_exit() == 0
            result['status'] = 'passed'
        except Exception as error:
            result.update(error=str(error), traceback=traceback.format_exc())
        finally:
            if session:
                session.close()
            save(output / 'results.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--language', choices=('en', 'ko'), default='en')
    parser.add_argument('--mode', choices=('native', 'all'), default='all')
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    report = dict(status='failed', binary=str(binary), sha256=digest, journeys=[])
    routes = (('native', native),) if args.mode == 'native' else (('native', native), ('codex', codex), ('acp', acp))
    for size in ((120, 36), (60, 18)):
        for route, run in routes:
            report['journeys'].append(run(binary, output / f'{route}-{size[0]}-{args.language}', size, args.language))
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest
    report['status'] = 'passed' if all(row['status'] == 'passed' for row in report['journeys']) else 'failed'
    save(output / 'results.json', report)
    print(json.dumps(dict(status=report['status'], output=str(output))))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
