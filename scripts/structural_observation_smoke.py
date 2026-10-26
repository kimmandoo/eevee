#!/usr/bin/env python3
"""Opt-in real ev PTY RX03/TC04 observations; synthetic loopback/Codex peers only.

This driver runs no model, paid service, credential import or provider shell. It
records exact binary identity, actual correlated tool results, original times,
query executions avoided and real fingerprint/lookup/store costs, not estimates
of tests/builds/billing avoided. Nothing adds this matrix to default CI.
The native fixture exposes Git without restoring the host executable path;
queued response gates stop with their peer so cleanup preserves failure evidence.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import tempfile
import time
import traceback
from unittest.mock import patch

import response_smoke

from file_creation_smoke import (CodexCreationJourney, CreationJourney, Peer,
                                 response_plan, save, tool_results)
from native_connection_smoke import until
from terminal_smoke import confirm_exit, inspect_permission_scope, permission_run_available
from workbench_smoke import wait_for

ANSWER = 'STRUCTURAL_OBSERVATION_COMPLETE'
SOURCE = ('fn actual() {\r\n send(資料 + 1,\r\n 資料+1);\r\n' +
          ' send(value,value);\r\n' * 39 +
          ' send(left,right);\r\n let _ = "send(a,a)";\r\n // send(a,a)\r\n'
          ' opaque! { send(a,a) }\r\n}\r\n')


def fixture(project):
    subprocess.run(['git', 'init', '-q', str(project)], check=True, capture_output=True)
    (project / 'src').mkdir(exist_ok=True)
    (project / '.gitignore').write_text('src/ignored.rs\n')
    (project / 'src/actual.rs').write_bytes(SOURCE.encode())
    (project / 'src/ignored.rs').write_text('fn hidden() { send(hidden,hidden); }\n')
    (project / '.env').write_text('fn excluded() { send(excluded,excluded); }\n')
    (project / 'secrets').mkdir(exist_ok=True)
    (project / 'secrets/private.rs').write_text('fn private() { send(private,private); }\n')
    try:
        (project / 'src/link.rs').symlink_to('ignored.rs')
    except OSError:
        pass


def search(**changes):
    arguments = dict(directory='src', language='rust', pattern='send($VALUE,$VALUE)', offset=0, snapshot='')
    arguments.update(changes)
    return 'eevee_structural_search', arguments


def read():
    return 'eevee_read_file', dict(path='src/actual.rs', start_line=1, end_line=2)


def check_matches(result, source, count=32):
    assert result['status'] == 'success' and result['complete'], result
    assert len(result['matches']) == count, result
    digest = hashlib.sha256(source.encode()).hexdigest()
    for found in result['matches']:
        assert found['path'] == 'src/actual.rs' and found['sha256'] == digest, found
        bounds = found['range']
        text = source.encode()[bounds['start_byte']:bounds['end_byte']].decode()
        assert text.startswith('send(') and text.endswith(')'), found
        for captured in found['captures'].values():
            bounds = captured['range']
            assert source.encode()[bounds['start_byte']:bounds['end_byte']].decode() == captured['text'], captured


def check_hit(cold, warm):
    metadata = warm['observation_cache']
    assert metadata['status'] == 'hit' and metadata['result_computations'] == 0, warm
    assert metadata['observation_executions_avoided'] == 1, warm
    assert metadata['original_observed_at_unix_ms'] == cold['observation_cache']['original_observed_at_unix_ms'], warm
    assert metadata['fingerprinted_bytes'] > 0 and metadata['fingerprint_us'] >= 0, warm
    for key in ('cache_lookup_us', 'cache_store_us', 'original_execution_us'):
        assert metadata[key] >= 0, metadata


def cache_cost_samples(observations):
    samples = []
    for label, value in observations.items():
        metadata = value.get('observation_cache', {})
        if metadata.get('status') != 'hit':
            continue
        current = sum(metadata[key] for key in ('fingerprint_us', 'cache_lookup_us', 'cache_store_us'))
        samples.append(dict(label=label, avoided=metadata['observation_executions_avoided'],
                            historical_result_computation_us=metadata['original_execution_us'],
                            current_fingerprint_lookup_store_us=current,
                            historical_minus_current_us=metadata['original_execution_us'] - current,
                            meaning='Historical result computation versus actual current cache costs; not an end-to-end speedup, test result or billing saving.'))
    return samples


def scenarios(call, project):
    results = {}

    def observe(label, tool):
        result = call(label, *tool)
        results[label] = result
        assert isinstance(result.get('status'), str), result
        assert isinstance(result.get('message'), str) and result['message'], result
        assert isinstance(result.get('path'), str), result
        if tool[0] == 'eevee_structural_search' and result['status'] != 'refused':
            assert result['path'] == tool[1]['directory'], result
        return result

    cold_read = observe('read-cold', read())
    assert cold_read.get('observation_cache', {}).get('status') == 'miss', cold_read
    check_hit(cold_read, observe('read-warm', read()))
    cold = observe('shape-cold', search())
    check_matches(cold, SOURCE)
    assert cold['total_retained_matches'] == 40, cold
    warm = observe('shape-warm', search())
    check_hit(cold, warm)
    assert warm['matches'] == cold['matches'], warm
    continuation = search(offset=cold['next_offset'], snapshot=cold['snapshot'])
    tail = observe('shape-next', continuation)
    check_matches(tail, SOURCE, 8)
    assert tail['next_offset'] is None, tail
    private = observe('root-private-exclusions', search(directory=''))
    check_matches(private, SOURCE)
    assert 'private.rs' not in json.dumps(private) and 'ignored.rs' not in json.dumps(private), private
    assert observe('query-stale', search(pattern='other($VALUE)', offset=32,
                                        snapshot=cold['snapshot']))['status'] == 'refused'
    (project / '.gitignore').write_text('src/ignored.rs\n# changed rule identity\n')
    assert observe('policy-stale', continuation)['status'] == 'refused'
    policy = observe('policy-cold', search())
    assert policy['observation_cache']['status'] == 'miss', policy
    changed = SOURCE.replace('value,value', 'changed,changed')
    (project / 'src/actual.rs').write_bytes(changed.encode())
    assert observe('source-stale', search(offset=32, snapshot=policy['snapshot']))['status'] == 'refused'
    content = observe('content-cold', search())
    check_matches(content, changed)
    assert content['observation_cache']['status'] == 'miss', content
    assert observe('no-match', search(pattern='absent($VALUE)'))['status'] == 'no_match'
    assert observe('invalid-pattern', search(pattern='let x = 1'))['status'] == 'invalid_pattern'
    assert observe('unsupported-language', search(language='python'))['status'] == 'unsupported_language'
    (project / 'src/actual.rs').write_text('fn broken(')
    assert observe('parse-failure', search())['status'] == 'parse_failure'
    (project / 'src/valid.rs').write_text('fn valid() {}\n')
    mixed = observe('parse-incomplete', search())
    assert mixed['status'] == 'incomplete' and not mixed['complete'], mixed
    (project / 'src/valid.rs').unlink()
    (project / 'src/actual.rs').write_text('fn adversarial() { let _ = ' + '!' * 300 + 'true; }')
    deep = observe('bounded-expression-chain', search())
    assert deep['status'] == 'incomplete', deep
    assert any(row['reason'] == 'parser_lexical_limit' for row in deep['omissions']), deep
    for label, prefix in (('bom-source', '\ufeff'), ('shebang-source', '#!/usr/bin/env rust-script\n'),
                          ('bom-shebang-source', '\ufeff#!/usr/bin/env rust-script\r\n')):
        prefixed = prefix + SOURCE
        (project / 'src/actual.rs').write_bytes(prefixed.encode())
        observed = observe(label, search())
        check_matches(observed, prefixed)
        first = observed['matches'][0]['range']
        before = prefixed.encode()[:first['start_byte']].decode()
        assert first['start_line'] == before.count('\n') + 1, observed
        assert first['start_column'] == len(before.rsplit('\n', 1)[-1]) + 1, observed
    (project / 'src/actual.rs').write_bytes(SOURCE.encode())
    observe('read-rewarmed', read())
    actual, retained = project / 'src/actual.rs', project / 'retained.rs'
    actual.rename(retained)
    try:
        actual.symlink_to('../retained.rs')
    except OSError:
        retained.rename(actual)
    else:
        refused = observe('link-boundary', read())
        assert refused['status'] == 'refused' and 'content' not in refused, refused
        actual.unlink()
        retained.rename(actual)
    limits = project / 'limits'
    limits.mkdir()
    for index in range(129):
        (limits / f'{index:03}.rs').write_text('fn unrelated() {}\n')
    bounded = observe('candidate-limit', search(directory='limits', pattern='absent($VALUE)'))
    assert bounded['status'] == 'incomplete' and not bounded['complete'], bounded
    assert any(row['reason'] == 'candidate_limit' for row in bounded['omissions']), bounded
    assert all(len(json.dumps(row, ensure_ascii=False).encode()) <= 64 * 1024 for row in results.values())
    return results


def native(binary, output, size, ):
    output.mkdir(parents=True)
    journey = None
    result = dict(status='failed', route='native-responses', size=list(size), )
    with tempfile.TemporaryDirectory(prefix='ev-structural-native-') as temporary, \
            Peer('responses', 'sk-structural-fixture-only', True, retain_body_bytes=True) as peer:
        replies = queue.Queue()

        def blocked_reply(handler, event, row):
            deadline = time.monotonic() + 60
            while not peer.stopped.is_set():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise AssertionError('structural response gate exceeded its deadline')
                try:
                    reply = replies.get(timeout=min(.05, remaining))
                except queue.Empty:
                    continue
                if not peer.stopped.is_set():
                    reply(handler, event, row)
                return

        try:
            git = shutil.which('git')
            assert git, 'Git is required for actual policy-bound source observations'
            session = response_smoke.SurfaceSession

            def observation_session(argv, cwd, environment, dimensions, directory, **options):
                # Do not expose installed providers just to satisfy Git-backed
                # ignore-policy checks in the source observation tools.
                if os.name == 'nt':
                    environment = environment | {'PATH': str(Path(git).resolve().parent)}
                else:
                    (Path(environment['PATH']) / 'git').symlink_to(Path(git).resolve())
                return session(argv, cwd, environment, dimensions, directory, **options)

            with patch.object(response_smoke, 'SurfaceSession', observation_session):
                journey = CreationJourney(binary, output, Path(temporary).resolve(), size, size[0] == 60,
                                          'compatible', 'responses', True, False, peer, )
            wait_for(journey.session, 'Storage: saved')
            journey.setup_route()
            fixture(journey.project)
            journey.session.command('role-plan')
            journey.workspace()
            journey.write_request('Find repeated send expression shapes in Rust. Read only; do not modify files.')
            before = len(peer.snapshot('POST'))
            peer.plans.put(('custom', blocked_reply))
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

            until(journey.session, dispatched, 'structural request did not dispatch', timeout=40)
            tools = peer.snapshot('POST')[before]['body']['tools']
            assert any(tool.get('name') == 'eevee_structural_search' for tool in tools), tools
            assert not any(tool.get('name') == 'eevee_write_file' for tool in tools), tools
            ordinal = 0

            def raw_call(label, name, arguments):
                nonlocal ordinal
                post_count = len(peer.snapshot('POST'))
                peer.plans.put(('custom', blocked_reply))
                replies.put(response_plan(peer, calls=[dict(id=label, name=name, arguments=arguments)], number=ordinal)[1])
                ordinal += 1
                until(journey.session, lambda: len(peer.snapshot('POST')) > post_count,
                      f'{label} did not produce a correlated actual result', timeout=45)
                values = tool_results('responses', peer.snapshot('POST')[-1]['body'])
                return json.loads(values[label])

            def call(label, name, arguments):
                value = raw_call(label, name, arguments)
                if value.get('artifact_encoding', '').startswith('application/json'):
                    assert value['artifact_omitted_bytes'] == 0, value
                    handle, offset, chunks = value['artifact'], 0, []
                    while True:
                        retained = raw_call(label + f'-retained-{offset}', 'eevee_artifact_read',
                                            dict(id=handle['id'], revision=handle['revision'], offset=offset, limit=65536))
                        page = retained['observation']['data']['page']
                        assert page['handle'] == handle and page['offset'] == offset, retained
                        chunks.append(page['text'])
                        if page['next_offset'] is None:
                            break
                        offset = page['next_offset']
                    value = json.loads(''.join(chunks))
                return value

            observations = scenarios(call, journey.project)
            replies.put(response_plan(peer, text=ANSWER, number=ordinal)[1])
            journey.settled(ANSWER, before + ordinal + 1)
            assert not journey.snapshot()['claims'] and journey.snapshot()['executions'] == 0
            journey.capture('structural-shapes-cache-and-stale-boundaries')
            journey.session.command('quit')
            confirm_exit(journey.session)
            assert journey.session.wait_exit() == 0
            peer.assert_clean()
            save(output / 'observations.json', observations)
            result['cache_cost_samples'] = cache_cost_samples(observations)
            result['status'] = 'passed'
        except Exception as error:
            result.update(error=str(error), traceback=traceback.format_exc())
        finally:
            save(output / 'http.json', peer.snapshot())
            if journey:
                journey.session.close()
            save(output / 'results.json', result)
    return result


def codex(binary, output, size, ):
    output.mkdir(parents=True)
    journey = None
    result = dict(status='failed', route='controlled-codex', size=list(size), )
    try:
        journey = CodexCreationJourney(binary, output, size, )
        journey.setup(count=1)
        journey.session.command('role-plan')
        journey.workspace()
        fixture(journey.project)
        marker = 'COOP_STRUCTURAL_CACHE'
        journey.request(1, marker)
        calls = [read(), read(), search(), search(), search(pattern='absent($VALUE)'),
                 search(pattern='let x = 1'), search(language='python')]
        journey.publish(marker + '.prompt', json.dumps([dict(name=name, arguments=arguments) for name, arguments in calls]))
        journey.until(lambda: len(journey.peers('answer', marker)) == len(calls),
                      'controlled structural observations did not settle')
        answers = [row['result'] for row in journey.peers('answer', marker)]
        assert all(row['success'] for row in answers), answers
        values = [json.loads(row['contentItems'][0]['text']) for row in answers]
        check_hit(values[0], values[1])
        check_matches(values[2], SOURCE)
        check_hit(values[2], values[3])
        assert [value['status'] for value in values[4:]] == ['no_match', 'invalid_pattern', 'unsupported_language']
        assert (journey.project / 'src/actual.rs').read_bytes() == SOURCE.encode()
        journey.finish(marker)
        journey.until(lambda: not journey.claims(), 'controlled observation ownership did not settle')
        journey.workspace()
        journey.session.capture('controlled-codex-structural-cache')
        journey.quit()
        save(output / 'observations.json', values)
        result['cache_cost_samples'] = cache_cost_samples(dict(enumerate(values)))
        result['status'] = 'passed'
    except Exception as error:
        result.update(error=str(error), traceback=traceback.format_exc())
    finally:
        if journey:
            journey.close()
        save(output / 'results.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--mode', choices=('native', 'codex', 'all'), default='all')
    parser.add_argument('--size', choices=('wide', 'compact'), default='wide')
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    report = dict(status='failed', binary=str(binary), sha256=digest, journeys=[])
    size = (120, 36) if args.size == 'wide' else (60, 18)
    for route, run in (('native', native), ('codex', codex)):
        if args.mode in (route, 'all'):
            report['journeys'].append(run(binary, output / route, size, ))
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest
    report['status'] = 'passed' if all(row['status'] == 'passed' for row in report['journeys']) else 'failed'
    save(output / 'results.json', report)
    print(json.dumps(dict(status=report['status'], output=str(output))))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
