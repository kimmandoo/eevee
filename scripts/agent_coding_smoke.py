#!/usr/bin/env python3
"""Opt-in real agent coding acceptance against an already-built Eevee binary.

Deterministic loopback/JSONL peers ONLY request tools. Eevee creates/edits the
source, launches a real Rust compiler, receives its real failure, repairs the
source, compiles/runs an executable and downloads an inert local HTTP resource
after observing a real 404. No model, credential, SQL write, compiler invocation
or download by the peer is used. Default coverage is four representative journeys.
"""
from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading
import traceback

import file_creation_smoke as files
from native_connection_smoke import Peer
from response_smoke import saved_request
from terminal_smoke import confirm_exit
from workbench_smoke import wait_for

ESC = '\x1b'
SOURCE = 'main.rs'
PRODUCT = 'agent-program.exe' if os.name == 'nt' else 'agent-program'
INITIAL = 'fn main() {\n    let answer = not_defined;\n    println!("draft:{}", answer);\n}\n'
BROKEN = INITIAL.replace('not_defined', 'still_missing').replace('draft:{}', 'AGENT-CODING:{}')
FIXED = BROKEN.replace('still_missing', '42')
OUTPUT = 'AGENT-CODING:42'
FUTURE = 'Unsent owner draft / 아직 보내지 않은 초안 界'
COMMAND = 'eevee_run_command'
RESOURCE = 'downloaded-resource-界.bin'
RESOURCE_BYTES = b'\x00\x01\xfe\xff\r\n' + 'Inert resource / 한글 界 café\n'.encode('utf-8')
RESOURCE_SHA = hashlib.sha256(RESOURCE_BYTES).hexdigest()
DOWNLOAD_OUTPUT = 'DOWNLOAD-SHA256:' + RESOURCE_SHA
DOWNLOAD_SCRIPT = (
    'import hashlib, pathlib, sys, urllib.error, urllib.request\n'
    'opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))\n'
    'try:\n'
    '    with opener.open(sys.argv[1], timeout=5) as response:\n'
    '        data = response.read(1048577)\n'
    'except urllib.error.HTTPError as error:\n'
    '    print("HTTP Error " + str(error.code), file=sys.stderr)\n'
    '    raise SystemExit(1)\n'
    'assert len(data) <= 1048576, "resource exceeds download bound"\n'
    'with pathlib.Path(sys.argv[2]).open("xb") as destination:\n'
    '    destination.write(data)\n'
    'print("DOWNLOAD-SHA256:" + hashlib.sha256(data).hexdigest())\n'
)
PROFILES = {
    'representative': (('native', (120, 36), True),
                       ('native', (60, 18), False),
                       ('codex', (60, 18), False),
                       ('codex', (120, 36), True)),
    'native': (('native', (120, 36), True), ('native', (60, 18), False)),
    'codex': (('codex', (60, 18), False), ('codex', (120, 36), True)),
}


def sha(text):
    return hashlib.sha256(text.encode()).hexdigest()


def direct(program, *arguments):
    return {'cwd': '', 'command': {'Direct': {'program': str(program), 'args': list(arguments)}}}


class ResourceFixture:
    """Owned, bounded loopback HTTP server; never fetches or executes anything."""
    def __init__(self):
        self.server = None
        self.thread = None
        self.requests = []
        self.lock = threading.Lock()

    def start(self):
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def setup(self):
                super().setup()
                self.connection.settimeout(5)

            def do_GET(self):
                status = 200 if self.path == '/resource.bin' else 404
                body = RESOURCE_BYTES if status == 200 else b'resource not found\n'
                with fixture.lock:
                    fixture.requests.append({'method': 'GET', 'path': self.path, 'status': status})
                self.send_response(status)
                self.send_header('Content-Type', 'application/octet-stream')
                self.send_header('Content-Length', str(len(body)))
                self.send_header('Connection', 'close')
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *arguments):
                pass

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.server.daemon_threads = False
        self.thread = threading.Thread(target=self.server.serve_forever,
                                       kwargs={'poll_interval': .05}, name='agent-resource-http')
        self.thread.start()

    def url(self, path):
        return 'http://127.0.0.1:{}{}'.format(self.server.server_port, path)

    def snapshot(self):
        with self.lock:
            return list(self.requests)

    def assert_complete(self):
        assert self.snapshot() == [
            {'method': 'GET', 'path': '/missing-resource.bin', 'status': 404},
            {'method': 'GET', 'path': '/resource.bin', 'status': 200},
        ], self.snapshot()

    def close(self):
        if self.server:
            if self.thread and self.thread.is_alive():
                self.server.shutdown()
            self.server.server_close()
        if self.thread and self.thread.ident is not None:
            self.thread.join(timeout=6)
            assert not self.thread.is_alive(), 'loopback resource server did not stop'


def chain(compiler, project, resource):
    build = direct(compiler, '--edition=2021', '--color=never', SOURCE, '-o', PRODUCT)
    return [
        {'id': 'create', 'name': 'eevee_write_file', 'arguments': {'path': SOURCE, 'content': INITIAL}},
        {'id': 'read-original', 'name': 'eevee_read_file', 'arguments': {'path': SOURCE, 'start_line': 1, 'end_line': 0}},
        {'id': 'edit-original', 'name': 'eevee_edit_file', 'arguments': {'path': SOURCE, 'sha256': sha(INITIAL), 'replacements': [
            {'old_text': 'not_defined', 'new_text': 'still_missing'},
            {'old_text': 'draft:{}', 'new_text': 'AGENT-CODING:{}'}]}},
        {'id': 'compile-failure', 'name': COMMAND, 'arguments': build},
        {'id': 'read-failure-source', 'name': 'eevee_read_file', 'arguments': {'path': SOURCE, 'start_line': 1, 'end_line': 0}},
        {'id': 'fix', 'name': 'eevee_edit_file', 'arguments': {'path': SOURCE, 'sha256': sha(BROKEN), 'replacements': [
            {'old_text': 'still_missing', 'new_text': '42'}]}},
        {'id': 'compile-success', 'name': COMMAND, 'arguments': build},
        {'id': 'run-program', 'name': COMMAND, 'arguments': direct(project / PRODUCT)},
        {'id': 'download-failure', 'name': COMMAND, 'arguments': direct(
            sys.executable, '-I', '-c', DOWNLOAD_SCRIPT, resource.url('/missing-resource.bin'), RESOURCE)},
        {'id': 'download-success', 'name': COMMAND, 'arguments': direct(
            sys.executable, '-I', '-c', DOWNLOAD_SCRIPT, resource.url('/resource.bin'), RESOURCE)},
    ]


def decode_result(value):
    if isinstance(value, str):
        return json.loads(value)
    # Controlled Codex returns the actual host result in a text content block.
    if ('contentItems' in value or 'content' in value) and 'status' not in value:
        content = value.get('contentItems', value.get('content', []))
        text = ''.join(part.get('text', '') for part in content if isinstance(part, dict))
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            # File mutations return the actual protocol success receipt as
            # human-readable text; commands/read observations return JSON.
            # Exact filesystem effects are asserted independently below.
            if isinstance(value.get('success'), bool):
                return {'status': 'success' if value['success'] else 'refused', 'message': text}
            raise
    return value


def assert_result(call, value, project):
    value = decode_result(value)
    identity = call['id']
    if identity == 'compile-failure':
        assert value['status'] == 'error', value
        assert value['run']['exit_code'] not in (None, 0), value
        assert 'still_missing' in value['stderr'] and 'error' in value['stderr'], value
        assert not (project / PRODUCT).exists(), 'failed compiler fabricated a build product'
    elif identity == 'download-failure':
        assert value['status'] == 'error', value
        assert value['run']['exit_code'] not in (None, 0), value
        assert 'HTTP Error 404' in value['stderr'], value
        assert not value['stdout'].strip(), value
        assert not (project / RESOURCE).exists(), '404 created a destination before a successful response'
    else:
        assert value['status'] == 'success', value
    if identity == 'read-original':
        assert value['content'] == INITIAL and value['sha256'] == sha(INITIAL), value
    elif identity == 'read-failure-source':
        assert value['content'] == BROKEN and value['sha256'] == sha(BROKEN), value
    elif identity == 'compile-success':
        assert value['run']['exit_code'] == 0 and (project / PRODUCT).is_file(), value
    elif identity == 'run-program':
        assert value['run']['exit_code'] == 0 and OUTPUT in value['stdout'], value
    elif identity == 'download-success':
        assert value['run']['exit_code'] == 0, value
        assert value['stdout'].strip() == DOWNLOAD_OUTPUT and not value['stderr'].strip(), value
        content = (project / RESOURCE).read_bytes()
        assert content == RESOURCE_BYTES, 'downloaded resource bytes changed'
        assert hashlib.sha256(content).hexdigest() == RESOURCE_SHA
    return value


def final_artifacts(project):
    assert (project / SOURCE).read_bytes() == FIXED.encode(), 'download changed authored source'
    assert (project / PRODUCT).is_file(), 'compiled program disappeared'
    content = (project / RESOURCE).read_bytes()
    assert content == RESOURCE_BYTES, 'downloaded resource bytes changed'
    digest = hashlib.sha256(content).hexdigest()
    assert digest == RESOURCE_SHA, 'downloaded resource digest changed'
    return {'path': RESOURCE, 'bytes': len(content), 'sha256': digest,
            'origin': 'owned-loopback-http', 'executed': False}


def database(journey):
    return journey.data / 'workbench.sqlite3'


def runs(journey):
    with closing(sqlite3.connect(database(journey).as_uri() + '?mode=ro', uri=True)) as db:
        return [json.loads(row[0]) for row in db.execute('SELECT record FROM executions ORDER BY rowid')]


def draft_present(journey):
    return saved_request(journey, FUTURE, len(FUTURE.encode()))


def save_future(journey):
    journey.workspace()
    # Open only the visible Request owner; never dispatch the later draft.
    footer = journey.session.display().rstrip().splitlines()[-1].strip()
    if footer.startswith(('Type Request |', journey.surface['home'])):
        journey.session.send('\r')
    journey.session.send((ESC + '[A') * 100 + ESC + '[H' + (ESC + '[3~') * 512)
    journey.session.paste(FUTURE)
    journey.session.send('\x13')
    files.until(journey.session, lambda: draft_present(journey), 'later owner draft was not saved')


def file_decision(journey, allow=True):
    session = journey.session
    def ready():
        if files.file_review_pending(session):
            return True
        session.command('approvals')
        return files.file_review_pending(session)
    files.until(session, ready, 'exact file review did not open')
    assert SOURCE in session.display(), session.display()
    session.send('\r')  # Inspect complete actual content before returning to summary.
    files.inspect_to_bottom(session)
    session.send('d')
    files.until(session, lambda: files.file_review_pending(session), 'file decision summary')
    session.capture('exact-file-' + str(len(runs(journey))))
    session.send(ESC + ('[C' if allow else '[D'))
    if allow:
        files.expose_file_scope(session)
    session.send('\r', settle=False)


def command_decision(journey, *, decision='allow', retain_draft=False):
    session = journey.session
    titles = ('Review one-shot execution permission', )
    def ready():
        if any(title in session.display() for title in titles):
            return True
        # Commands join the shared FIFO instead of stealing focus. Select the
        # pending exact command once preparation has produced its immutable ID.
        session.command('approvals')
        return any(title in session.display() for title in titles)
    files.until(session, ready, 'actual host command one-shot review did not open', timeout=60)
    count = len(runs(journey))
    session.capture('command-neutral-' + str(count) + '-' + decision)
    if not draft_present(journey):
        session.send('\r')
        assert len(runs(journey)) == count, 'neutral Enter executed a command'
        files.until(session, ready, 'neutral Stay did not retain the pending command for review')
        # Pasted text grants nothing, but typing-safe permission arrival may
        # correctly put it in Request. Exercise this before staging FUTURE;
        # do not then mistake our own later paste for provider draft corruption.
        session.paste('y')
        assert len(runs(journey)) == count, 'pasted text executed a command'
        files.until(session, ready, 'pasted input lost the pending command review')
    if retain_draft:
        session.send(ESC)
        save_future(journey)
        session.command('approvals')
        files.until(session, ready, 'deferred exact command review did not reopen')
        assert draft_present(journey)
    if decision == 'cancel':
        session.send(ESC)
        journey.workspace()
        session.command('cancel')
    elif decision == 'deny':
        session.command('deny')
    else:
        # An automatically presented review must be engaged before choosing.
        if any(hint in session.display() for hint in ('Enter to review (not approve)', )):
            session.send('\r')
        session.send(ESC + '[C' + '\r', settle=False)


def source_effects(journey, step):
    expected = INITIAL if step < 2 else BROKEN if step < 5 else FIXED
    assert (journey.project / SOURCE).read_bytes() == expected.encode()


def natural_request(marker):
    return ('Create main.rs, read and edit the code, then build the project. Read the real error, fix it, '
            'rebuild and run the program. Download an inert resource over HTTP to ' + RESOURCE
            + '; observe the first 404, correct the URL and download the exact bytes without executing them. ' + marker)


def visible_outcomes(journey, calls):
    journey.session.command('response')
    journey.session.send(ESC + '[1;3H')
    observed = []
    for page in range(8 * len(calls)):
        view = journey.session.display()
        observed.append(view)
        if page in (0, 3, 7, 15):
            journey.session.capture('actual-tool-evidence-' + str(page))
        journey.session.send(ESC + '[6~')
        if journey.session.display() == view:
            break
    visible = '\n'.join(observed)
    rows = visible.splitlines()
    assert any('[!!]' in row and 'exit 1' in row for row in rows), 'failed command outcome was not visible in the transcript'
    assert any('[ok]' in row and 'exit 0' in row for row in rows), 'successful command outcome was not visible in the transcript'
    assert OUTPUT in visible, 'actual program output was hidden by the tools side index'
    assert 'HTTP Error 404' in visible, 'actual HTTP failure was hidden by the tools side index'
    assert DOWNLOAD_OUTPUT[:32] in visible, 'actual download digest output was hidden by the tools side index'
    journey.workspace()


class NativeJourney(files.CreationJourney):
    def settled_coding(self, count):
        files.until(self.session, lambda: len(self.peer.snapshot('POST')) == count
                    and not self.snapshot()['claims'] and 'Storage: saved' in self.session.display()
                    and '/cancel' not in self.session.display().rstrip().splitlines()[-1],
                    'coding turn did not settle', timeout=90)
        self.workspace()


def native_case(binary, compiler, output, size, manual):
    result = {'status': 'failed', 'route': 'native-chat-completions', 'size': size, 'manual_appearance': manual, 'tool_results': []}
    journey = None
    resource = ResourceFixture()
    with tempfile.TemporaryDirectory(prefix='ev-agent-coding-界-') as temporary, Peer(
            'chat_completions', 'public-unused-fixture', True, retain_body_bytes=True) as peer:
        try:
            resource.start()
            journey = NativeJourney(binary, output, Path(temporary).resolve(), size, size[0] == 60,
                                    'compatible', 'chat_completions', True, False, peer, )
            # Native connection fixtures deliberately start with an empty PATH.
            # Expose only installed linker helpers needed by the genuine rustc;
            # the peer still cannot compile or write the authored source.
            for name in ('cc', 'clang', 'gcc', 'ld', 'as', 'ar', 'xcrun'):
                executable = shutil.which(name)
                if executable:
                    (journey.root / 'empty-bin' / name).symlink_to(Path(executable).resolve())
            wait_for(journey.session, 'Storage: saved')
            journey.setup_route()
            if manual:
                journey.session.command('vaporeon')
                journey.workspace()
            calls = chain(compiler, journey.project, resource)
            run_count = sum(call['name'] == COMMAND for call in calls)
            continuation = next(index + 1 for index, call in enumerate(calls) if call['id'] == 'compile-failure')
            todo_label = 'Build program and download resource'
            def plan(index, *, pause=False, resumed=False):
                def reply(handler, event, row):
                    if index and not resumed:
                        previous = calls[index - 1]
                        results = files.tool_results('chat_completions', row['body'])
                        value = assert_result(previous, results[previous['id']], journey.project)
                        result['tool_results'].append({'call': previous['id'], 'result': value})
                        if index == 1:
                            progress = decode_result(results['coding-progress'])
                            assert progress['status'] == 'success' and progress['path'] == '', progress
                            result['todo_before_reviewed_file'] = progress
                    if pause:
                        # A valid model stop after a definite failed compiler
                        # must not abandon this request's still-active build.
                        return files.response_plan(peer, text='The compiler failed; the build is still unfinished.',
                                                   number=index)[1](handler, event, row)
                    selected = ([calls[index]] if index < len(calls) else [{
                        'id': 'coding-progress-done', 'name': 'todo', 'arguments': {
                            'operation': 'done', 'label': todo_label, 'steps': []}}])
                    if index == 0:
                        selected.insert(0, {'id': 'coding-progress', 'name': 'todo', 'arguments': {
                            'operation': 'init', 'label': '', 'steps': [
                                {'label': todo_label, 'phase': 'implement', 'status': 'active'}
                            ]}})
                    return files.response_plan(peer, calls=selected, number=index)[1](handler, event, row)
                return ('custom', reply)
            def final_reply(handler, event, row):
                progress = decode_result(files.tool_results('chat_completions', row['body'])['coding-progress-done'])
                assert progress['status'] == 'success', progress
                result['todo_after_download'] = progress
                return files.response_plan(peer, text='Actual compiler/program and downloaded resource evidence received.',
                                           number=len(calls) + 2)[1](handler, event, row)
            plans = [plan(i) for i in range(continuation)]
            plans += [plan(continuation, pause=True), plan(continuation, resumed=True)]
            plans += [plan(i) for i in range(continuation + 1, len(calls) + 1)]
            plans.append(('custom', final_reply))
            before = journey.begin(natural_request('COOP_CODING_NATIVE'), plans)
            for index, call in enumerate(calls):
                if call['name'] in ('eevee_write_file', 'eevee_edit_file'):
                    file_decision(journey)
                elif call['name'] == COMMAND:
                    command_decision(journey, retain_draft=call['id'] == 'compile-failure')
                files.until(journey.session, lambda i=index: len(peer.snapshot('POST')) >= before + i + 2 + int(i >= continuation - 1),
                            'actual tool result did not reach next provider round: ' + call['id'], timeout=90)
                source_effects(journey, index)
            journey.settled_coding(before + len(calls) + 3)
            assert [row['call'] for row in result['tool_results']] == [call['id'] for call in calls]
            result['continued_after_failed_build_answer'] = True
            assert draft_present(journey), 'coding overwrote later owner draft'
            partner = journey.snapshot()['partners'][0]
            if manual:
                assert partner['form'] == 1 and not partner['auto_form'], partner
            assert len(runs(journey)) == run_count
            assert (journey.project / SOURCE).read_bytes() == FIXED.encode()
            assert (journey.project / PRODUCT).is_file()
            resource.assert_complete()
            visible_outcomes(journey, calls)
            result['actual_tool_outcomes_and_output_visible'] = True
            # Every additional command requires a fresh approval; denial/cancel
            # cannot create the proposed compiler output or reserve a real run.
            for decision in ('deny', 'cancel'):
                destination = 'never-built-' + decision
                call = {'id': decision, 'name': COMMAND, 'arguments': direct(compiler,
                    '--edition=2021', '--color=never', SOURCE, '-o', destination)}
                literal = '`rustc --edition=2021 --color=never main.rs -o ' + destination + '`'
                request = 'Run ' + literal + '.'
                before = journey.begin(request, [
                    files.response_plan(peer, calls=[call]), files.response_plan(peer, text='No execution approved.')])
                command_decision(journey, decision=decision)
                journey.settled_coding(before + 2)
                value = decode_result(files.tool_results('chat_completions', peer.snapshot('POST')[-1]['body'])[decision])
                assert value['status'] != 'success', value
                assert not (journey.project / destination).exists() and len(runs(journey)) == run_count
            journey.session.command('role-plan')
            journey.workspace()
            gate = files.Gate()
            try:
                before = journey.begin('Build the project and run the program.', [
                    files.response_plan(peer, calls=[{'id': 'plan-command', 'name': COMMAND,
                        'arguments': direct(compiler, SOURCE, '-o', 'plan-must-not-build')}], gate=gate),
                    files.response_plan(peer, text='Plan has no execution authority.')], writing=False)
            finally:
                gate.release.set()
            journey.settled_coding(before + 2)
            value = decode_result(files.tool_results('chat_completions', peer.snapshot('POST')[-1]['body'])['plan-command'])
            assert value['status'] != 'success', value
            assert not (journey.project / 'plan-must-not-build').exists() and len(runs(journey)) == run_count
            resource.assert_complete()
            result['download'] = final_artifacts(journey.project)
            journey.session.command('quit')
            confirm_exit(journey.session)
            assert journey.session.wait_exit() == 0
            peer.assert_clean()
            result['runs'] = runs(journey)
            result['status'] = 'passed'
        except Exception as error:
            result.update(error=str(error), traceback=traceback.format_exc())
            if journey:
                journey.session.capture('failed')
        finally:
            resource.close()
            files.save(output / 'resource-http-requests.json', resource.snapshot())
            files.save(output / 'http-requests.json', peer.snapshot())
            if journey:
                journey.session.close()
            files.save(output / 'results.json', result)
    return result


def codex_case(binary, compiler, output, size, manual):
    result = {'status': 'failed', 'route': 'controlled-codex', 'size': size, 'manual_appearance': manual, 'tool_results': []}
    journey = None
    resource = ResourceFixture()
    try:
        resource.start()
        journey = files.CodexCreationJourney(binary, output, size, )
        journey.setup(count=1)
        journey.session.command('role-implement')  # Remove setup's explicit Plan; not a sticky grant.
        if not manual:
            journey.session.command('eevee')
        journey.workspace()
        marker = 'COOP_CODING_CODEX'
        text = natural_request(marker)
        journey.request(1, text, started=False)
        journey.until(lambda: bool(journey.peers('started', marker)), 'ordinary coding request did not start', approve_preview=True)
        assert json.loads(journey.peers('started', marker)[-1]['text'])['request'] == text
        assert all(row['writing'] for row in journey.snapshot()[2])
        assert COMMAND in journey.peers('session')[-1]['tools']
        calls = chain(compiler, journey.project, resource)
        run_count = sum(call['name'] == COMMAND for call in calls)
        journey.publish(marker + '.prompt', json.dumps(calls))
        for index, call in enumerate(calls):
            if call['name'] in ('eevee_write_file', 'eevee_edit_file'):
                file_decision(journey)
            elif call['name'] == COMMAND:
                command_decision(journey, retain_draft=call['id'] == 'compile-failure')
            journey.until(lambda i=index: len(journey.peers('answer', marker)) >= i + 1,
                          'actual controlled tool result: ' + call['id'], timeout=90)
            answer = journey.peers('answer', marker)[index]
            assert not answer.get('error'), answer
            value = assert_result(call, answer['result'], journey.project)
            result['tool_results'].append({'call': call['id'], 'result': value})
            source_effects(journey, index)
        journey.finish(marker)
        journey.until(lambda: not journey.claims(), 'controlled coding owner settlement')
        journey.workspace()
        assert draft_present(journey), 'controlled coding overwrote later owner draft'
        if manual:
            partner = journey.snapshot()[1][0]
            assert partner['form'] == 1 and not partner['auto_form'], partner
        assert len(journey.peers('answer', marker)) == len(calls), 'controlled tools repeated an effect'
        assert len(runs(journey)) == run_count and (journey.project / PRODUCT).is_file()
        resource.assert_complete()
        visible_outcomes(journey, calls)
        result['actual_tool_outcomes_and_output_visible'] = True
        for decision in ('deny', 'cancel'):
            marker = 'COOP_CODING_' + decision.upper()
            destination = 'never-built-' + decision
            literal = '`rustc --edition=2021 --color=never main.rs -o ' + destination + '`'
            text = 'Run ' + literal + '.' + ' ' + marker
            journey.request(1, text, started=False)
            journey.until(lambda: bool(journey.peers('started', marker)), 'controlled command request', approve_preview=True)
            journey.publish(marker + '.prompt', json.dumps([{'name': COMMAND, 'arguments': direct(
                compiler, '--edition=2021', '--color=never', SOURCE, '-o', destination)}]))
            command_decision(journey, decision=decision)
            journey.until(lambda: bool(journey.peers('answer', marker)), 'controlled command refusal callback')
            answer = journey.peers('answer', marker)[-1]
            assert not (answer.get('result') or {}).get('success'), answer
            journey.finish(marker)
            journey.until(lambda: not journey.claims(), 'denied/cancelled controlled owner settlement')
            journey.workspace()
            assert not (journey.project / destination).exists() and len(runs(journey)) == run_count
        journey.until(lambda: '/cancel' not in journey.session.display().rstrip().splitlines()[-1],
                      'controlled provider terminal event did not reach the policy owner')
        journey.session.command('role-plan')
        journey.workspace()
        marker = 'COOP_CODING_PLAN'
        journey.request(1, 'Build the project and run the program. ' + marker, started=False)
        journey.until(lambda: bool(journey.peers('started', marker)), 'controlled Plan request', approve_preview=True)
        assert COMMAND not in journey.peers('session')[-1]['tools']
        assert all(not row['writing'] for row in journey.snapshot()[2])
        journey.finish(marker)
        journey.until(lambda: not journey.claims(), 'controlled Plan settlement')
        assert len(runs(journey)) == run_count and not (journey.project / 'plan-must-not-build').exists()
        resource.assert_complete()
        result['download'] = final_artifacts(journey.project)
        result['runs'] = runs(journey)
        journey.quit()
        result['status'] = 'passed'
    except Exception as error:
        result.update(error=str(error), traceback=traceback.format_exc())
        if journey:
            journey.session.capture('failed')
    finally:
        resource.close()
        files.save(output / 'resource-http-requests.json', resource.snapshot())
        if journey:
            files.save(output / 'controlled-peer.json', journey.peers())
            journey.close()
        files.save(output / 'results.json', result)
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--compiler', type=Path, help='real installed rustc, not a protocol/compiler fixture')
    parser.add_argument('--profile', choices=tuple(PROFILES), default='representative')
    args = parser.parse_args(argv)
    binary, output = args.binary.resolve(), args.output.resolve()
    if not binary.is_file():
        parser.error('--binary must already exist')
    if args.profile != 'native' and os.name != 'posix':
        parser.error('controlled Codex fixture requires POSIX; use --profile native on Windows')
    compiler = args.compiler
    if compiler is None:
        rustup = shutil.which('rustup')
        if not rustup:
            parser.error('installed Rust 1.90.0 required, or provide --compiler')
        compiler = Path(subprocess.check_output([rustup, 'which', '--toolchain', '1.90.0', 'rustc'], text=True).strip())
    compiler = compiler.resolve()
    if not compiler.is_file():
        parser.error('--compiler must be a genuine installed rustc executable')
    output.mkdir(parents=True, exist_ok=False)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    report = {'status': 'failed', 'binary': str(binary), 'binary_sha256': digest,
              'compiler': str(compiler), 'profile': args.profile, 'journeys': [],
              'limits': ['Deterministic peers are not live model/billing conformance.',
                         'No paid models, private credentials or SQL writes.',
                         'Compiler, built program and Python downloader run only through Eevee host command approval.',
                         'HTTP download acceptance uses owned loopback HTTP, not public-network conformance; fetched bytes are never executed.',
                         'PTY evidence is not physical GUI/mouse certification.']}
    for route, size, manual in PROFILES[args.profile]:
        destination = output / '{}-{}'.format(route, size[0])
        destination.mkdir()
        function = native_case if route == 'native' else codex_case
        report['journeys'].append(function(binary, compiler, destination, size, manual))
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest, 'supplied binary changed'
    report['status'] = 'passed' if all(row['status'] == 'passed' for row in report['journeys']) else 'failed'
    files.save(output / 'results.json', report)
    print(json.dumps({'status': report['status'], 'output': str(output)}, ensure_ascii=False))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
