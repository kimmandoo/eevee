#!/usr/bin/env python3
"""XC05 real-binary PTY/ConPTY journey; synthetic stdio MCP + loopback inference only.

python3 scripts/mcp_smoke.py --binary target/debug/ev --output /tmp/ev-mcp-proof
Runs both 120x36 and 60x18 unless --width/--height are supplied.
"""
from __future__ import annotations
import smoke_sizes
import argparse
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import traceback

import file_creation_smoke as files
from tool_catalog_smoke import Journey as CatalogJourney
from terminal_smoke import inspect_permission_scope
from workbench_smoke import wait_for

ESC = '\x1b'
DOWN = ESC + '[B'
PAGE_DOWN = ESC + '[6~'
WIRE = 'mcp__fixture__echo'


class Journey(CatalogJourney):
    def events(self):
        path = self.project / 'mcp-events.jsonl'
        return [json.loads(line) for line in path.read_text(encoding='utf-8').splitlines()] if path.exists() else []

    def review(self, label):
        wait_for(self.session, 'Enter Inspect', timeout=35)
        before = len(self.events())
        self.session.paste('a')
        self.session.send('a')
        assert len(self.events()) == before, 'paste or an unengaged key launched MCP'
        self.session.send('\r')
        for _ in range(120):
            view = self.session.display()
            self.capture(label + '-scope-' + str(_)) if _ in (0, 5, 10, 20) else None
            if 'a Register only' in view or 'a Launch exact' in view:
                break
            self.session.send(DOWN)
        else:
            raise AssertionError('complete MCP scope never became exposed')
        self.capture(label + '-complete-scope')
        self.session.send('a')

    def connect_mcp(self):
        self.workspace()
        self.session.command('mcp connect fixture')
        self.review('launch-review')
        files.until(self.session, lambda: len([row for row in self.events() if row['event'] == 'tools/list']) >= 1,
                    'reviewed stdio MCP tools/list never completed', timeout=35)
        self.session.pump(.5)
        self.capture('connected-tools-not-sent')
        self.session.send(ESC)
        self.workspace()

    def select(self, tool, *, read=False):
        if read:
            self.session.command('mcp read fixture ' + tool)
            self.session.send(ESC)
        self.session.command('mcp select fixture ' + tool)
        self.capture('explicit-selection-' + tool)
        self.session.send(ESC)
        self.workspace()

    def inspect_call_scope(self, request_id):
        def inspected():
            if 'y Send selected' in self.session.display():
                return True
            if not inspect_permission_scope(self.session):
                self.session.send(PAGE_DOWN)
            return False
        files.until(self.session, inspected, 'MCP exact review scope was not fully exposed', timeout=35)
        self.capture('complete-call-scope-' + request_id)

    def submit_call(self, arguments, *, tool='echo', rule=False, create_rule=False):
        request_id = 'mcp-' + str(len(self.peer.snapshot('POST')))
        before = self.submit('Explain using the selected unmanaged fixture tool; do not invoke any host operation. ' + request_id, [
            files.response_plan(self.peer, calls=[{'id': request_id, 'name': 'mcp__fixture__' + tool, 'arguments': arguments}]),
            files.response_plan(self.peer, text='MCP_CALL_CONTINUED_' + request_id),
        ], expected_posts=1)
        body = self.peer.snapshot('POST')[before]['body']
        names = files.advertised_tools(self.protocol, body)
        assert 'mcp__fixture__' + tool in names and 'mcp__fixture__unselected_secret' not in names
        assert 'UNSELECTED_SCHEMA_SENTINEL_93ca' not in json.dumps(body), 'unselected MCP schema/description leaked'
        if not rule:
            def pending():
                return 'UNMANAGED MCP call' in self.session.display() or 'Allow needs the complete visible scope' in self.session.display() or 'y Send selected' in self.session.display()
            files.until(self.session, pending, 'MCP tools/call did not request a separate review', timeout=35)
            self.capture('call-review-' + request_id)
            assert not any(row['event'] == 'tools/call' and row.get('params', {}).get('arguments') == arguments for row in self.events()), 'MCP call ran before consent'
            if create_rule:
                # The asynchronously presented pane intentionally swallows
                # palette keys until Enter engages it. Expose the exact call
                # before explicitly creating its server/tool rule.
                self.inspect_call_scope(request_id)
                self.session.command('rule-allow launch')
            else:
                def approve():
                    view = self.session.display()
                    if 'y Send selected' in view:
                        self.session.send(ESC + '[C')
                        self.capture('selected-allow-once-' + request_id)
                        self.session.send('y', settle=False)
                        return True
                    if not inspect_permission_scope(self.session):
                        self.session.send(PAGE_DOWN)
                    return False
                files.until(self.session, approve, 'MCP exact review never allowed one-shot decision', timeout=35)
        return before, request_id

    def settled_call(self, before, request_id):
        files.until(self.session, lambda: len(self.peer.snapshot('POST')) >= before + 2 and not self.snapshot()['claims'],
                    'MCP tool error/result did not continue the lead request', timeout=40)
        requests = self.peer.snapshot('POST')[before:]
        assert len(requests) == 2, 'MCP call caused hidden model resend'
        result = json.loads(files.tool_results(self.protocol, requests[1]['body'])[request_id])
        assert result.get('unmanaged') is True
        self.workspace()
        return result

    def native_effect_floor(self, command=False):
        path = 'native-command-floor.txt' if command else 'native-write-floor.txt'
        marker = 'MCP_UNKNOWN_REQUIRES_MANUAL_' + path
        call_id = 'native-floor-' + str(len(self.peer.snapshot('POST')))
        if command:
            tool = 'eevee_run_command'
            arguments = {'cwd': '', 'command': {'Direct': {'program': sys.executable, 'args': [
                '-c', 'from pathlib import Path; Path(' + repr(path) + ').write_text(' + repr(marker) + ')']}}}
        else:
            tool = 'eevee_write_file'
            arguments = {'path': path, 'content': marker}
        before = self.submit('Perform this native fixture operation only after its exact fresh manual review. ' + call_id, [
            files.response_plan(self.peer, calls=[{'id': call_id, 'name': tool, 'arguments': arguments}]),
            files.response_plan(self.peer, text='NATIVE_FLOOR_REVIEW_OBSERVED_' + call_id),
        ], expected_posts=1)
        if command:
            files.until(self.session, lambda: 'Review one-shot execution permission' in self.session.display(),
                        'native command did not remain under review', timeout=40)
        else:
            files.until(self.session, lambda: files.file_review_pending(self.session),
                        'native write did not remain under review', timeout=40)
        # Engage the actual review before opening the palette. For files,
        # Enter opens the proposed bytes; inspect them, then return to summary.
        if not command:
            self.session.send('\r')
            files.until(self.session, lambda: 'd Back' in self.session.display(),
                        'native file proposal did not open its exact detail review', timeout=35)
            files.inspect_to_bottom(self.session)
            self.session.send('d')
            files.until(self.session, lambda: files.file_review_pending(self.session),
                        'native write inspection did not return to the decision summary', timeout=40)
        elif 'Enter to review (not approve)' in self.session.display():
            self.session.send('\r')
        self.capture('native-rule-source-' + ('command' if command else 'write'))
        self.session.command('rule-allow launch')
        rule_id = 3 if command else 2
        files.until(self.session, lambda: ('Rule #' + str(rule_id) + ':') in self.session.display(),
                    'explicit native rule creation receipt missing', timeout=35)
        self.capture('unknown-native-' + ('command' if command else 'write') + '-rule-created')
        self.session.command('auto-approve')
        wait_for(self.session, 'AUTO-APPROVE', timeout=35)
        self.session.pump(.5)
        assert not (self.project / path).exists(), 'unknown MCP effects allowed an automatic native effect'
        assert len(self.peer.snapshot('POST')) == before + 1, 'unknown MCP effects bypassed fresh native review'
        self.capture('unknown-blocks-native-' + ('command' if command else 'write') + '-rule-and-task-auto')
        if command:
            from agent_coding_smoke import command_decision
            command_decision(self)
        else:
            self.session.command('approvals')
            files.until(self.session, lambda: files.file_review_pending(self.session),
                        'fresh manual native write review was not reachable', timeout=40)
            self.session.send('\r')
            files.inspect_to_bottom(self.session)
            self.session.send('d')
            files.until(self.session, lambda: files.file_review_pending(self.session),
                        'fresh manual native write summary was not exposed', timeout=40)
            self.capture('fresh-manual-native-write')
            self.session.send(ESC + '[C' + '\r', settle=False)
        files.until(self.session, lambda: len(self.peer.snapshot('POST')) == before + 2 and not self.snapshot()['claims'],
                    'fresh manual native effect did not settle', timeout=60)
        result = json.loads(files.tool_results(self.protocol, self.peer.snapshot('POST')[before + 1]['body'])[call_id])
        assert result['status'] == 'success', result
        assert (self.project / path).read_text() == marker
        self.workspace()
        self.session.command('rules')
        files.until(self.session, lambda: 'Approval rules' in self.session.display(),
                    'settled native rule was not available for inspection', timeout=35)
        rows = self.session.display().splitlines()
        assert any(('#' + str(rule_id)) in row and 'now' in row and 'Allow' in row
                   and (command or 'native-write' in row) for row in rows), self.session.display()
        self.capture('native-' + ('command' if command else 'write') + '-rule-still-enabled')
        self.session.send(ESC)
        self.session.command('auto-approve-off')
        self.workspace()


def alive(pid):
    if os.name == 'nt':
        import ctypes
        handle = ctypes.windll.kernel32.OpenProcess(0x1000, False, pid)
        if not handle:
            return False
        code = ctypes.c_ulong()
        ctypes.windll.kernel32.GetExitCodeProcess(handle, ctypes.byref(code))
        ctypes.windll.kernel32.CloseHandle(handle)
        return code.value == 259
    try:
        os.kill(pid, 0)
        stat = Path('/proc') / str(pid) / 'stat'
        return not stat.exists() or stat.read_text().split()[2] != 'Z'
    except ProcessLookupError:
        return False


def run(binary, output, root, size):
    output.mkdir(parents=True, exist_ok=False)
    root.mkdir(parents=True, exist_ok=False)
    result = {'status': 'failed', 'size': list(size), 'cases': []}
    journey = None
    with files.Peer('chat_completions', 'synthetic-mcp-key-unused', True, retain_body_bytes=True) as peer:
        try:
            journey = Journey(binary, output, root, size, True, 'compatible', 'chat_completions', True, False, peer)
            wait_for(journey.session, 'Storage: saved', timeout=35)
            journey.session.command('goal')
            journey.session.paste('XC05 unmanaged MCP fixture acceptance')
            journey.session.send('\r')
            wait_for(journey.session, 'Storage: saved', timeout=35)
            journey.setup_route()
            journey.session.command('role-plan')
            journey.workspace()
            registration = {'name': 'fixture', 'program': sys.executable, 'args': [str(Path(__file__).with_name('mcp_stdio_fixture.py').resolve()), '--root', str(journey.project)], 'cwd': '.', 'env': []}
            journey.session.command('mcp add ' + json.dumps(registration, separators=(',', ':')))
            journey.review('registration-review')
            assert not journey.events(), 'registration launched server'
            stored = json.loads((journey.data / 'mcp.json').read_text())
            assert list(stored['projects'].values())[0][0]['name'] == 'fixture'
            journey.session.send(ESC)
            journey.connect_mcp()
            launches = [row for row in journey.events() if row['event'] == 'launch']
            assert len(launches) == 1
            assert 'OPENAI_API_KEY' not in launches[0]['env_names']
            journey.select('echo', read=True)
            before, call_id = journey.submit_call({'text': 'MCP_ECHO_ACTUAL_丁_한글'})
            answer = journey.settled_call(before, call_id)
            assert answer['status'] == 'success' and answer['result']['content'][0]['text'] == 'MCP_ECHO_ACTUAL_丁_한글'
            assert len([row for row in journey.events() if row['event'] == 'initialize']) == 1
            assert len([row for row in journey.events() if row['event'] == 'tools/list']) == 1
            result['cases'] += ['registration-inert-masked-direct-launch-review', 'selection-explicit-unselected-schema-not-sent', 'reviewed-call-actual-stdio-result', 'initialize-list-once']
            before, call_id = journey.submit_call({'text': 'RULE_CREATED'}, create_rule=True)
            answer = journey.settled_call(before, call_id)
            assert 'rule #1' in answer.get('approval_cause', ''), answer
            assert '`fixture / echo` with any JSON arguments in this project until ev exits' in answer['approval_cause'], answer
            before, call_id = journey.submit_call({'text': 'RULE_ALLOWED'}, rule=True)
            answer = journey.settled_call(before, call_id)
            assert 'rule #1' in answer.get('approval_cause', ''), answer
            assert '`fixture / echo` with any JSON arguments in this project until ev exits' in answer['approval_cause'], answer
            journey.session.command('rule-revoke 1')
            journey.workspace()
            result['cases'].append('user-created-MCP-rule-allowed-with-receipt-cause')
            before, call_id = journey.submit_call({'text': '', 'mode': 'crash'}, create_rule=True)
            answer = journey.settled_call(before, call_id)
            assert answer['status'] == 'error' and 'crashed' in answer['message']
            assert answer['effects_unknown'] is True
            marker = json.loads((journey.data / 'mcp.json').read_text())
            # Windows stores the verbatim canonical form (\\?\D:\...); compare identities.
            def same(entry):
                try:
                    return os.path.samefile(entry.removeprefix('\\\\?\\'), journey.project)
                except OSError:
                    return False
            assert any(same(entry) for entry in marker['uncertain_projects']), marker
            assert 'uncertain_projects' in marker['critical']
            result['cases'].append('server-crash-tool-error-lead-continues')
            journey.connect_mcp()
            journey.select('echo', read=True)
            # The echo rule still exists, but retained uncertainty requires a fresh review.
            before, call_id = journey.submit_call({'text': 'AFTER_CRASH_MANUAL_REVIEW'})
            answer = journey.settled_call(before, call_id)
            assert answer['status'] == 'success' and answer['effects_unknown'] is True
            assert answer['approval_cause'] == 'Fresh explicit user review'
            marker = json.loads((journey.data / 'mcp.json').read_text())
            assert str(journey.project.resolve()) in marker['uncertain_projects']
            assert 'uncertain_projects' in marker['critical']
            result['cases'].append('retained-unknown-disables-rule-later-success-does-not-clear')
            before, call_id = journey.submit_call({'text': '', 'mode': 'hang'})
            files.until(journey.session, lambda: (journey.project / 'mcp-child.pid').exists(), 'MCP hanging child did not start', timeout=35)
            parent_pid = int((journey.project / 'mcp.pid').read_text())
            child_pid = int((journey.project / 'mcp-child.pid').read_text())
            journey.session.command('cancel')
            files.until(journey.session, lambda: not alive(parent_pid) and not alive(child_pid) and not journey.snapshot()['claims'], 'cancel did not terminate owned MCP process tree', timeout=35)
            assert len(peer.snapshot('POST')) == before + 1, 'cancelled MCP call continued inference'
            journey.capture('cancel-owned-server-and-child-tree')
            # The controlled peer has one deliberately unused continuation plan after cancellation.
            peer.plans.get_nowait()
            result['cases'].append('cancel-kills-owned-tree-no-continuation')
            journey.connect_mcp()
            journey.select('write_note')
            journey.session.command('role-implement')
            journey.workspace()
            before, call_id = journey.submit_call({'text': 'MCP_REVIEWED_NOTE_한글'}, tool='write_note')
            answer = journey.settled_call(before, call_id)
            assert answer['status'] == 'success'
            assert answer['effects_unknown'] is True
            assert answer['approval_cause'] == 'Fresh explicit user review'
            assert (journey.project / 'note.txt').read_text(encoding='utf-8') == 'MCP_REVIEWED_NOTE_한글'
            result['cases'].append('reviewed-unmanaged-write-note-actual-bytes')
            journey.native_effect_floor()
            journey.native_effect_floor(command=True)
            result['cases'] += ['unknown-blocks-native-write-rule-and-task-auto', 'unknown-blocks-native-command-rule-and-task-auto']
            journey.finish()
            from response_smoke import SurfaceSession
            journey.session = SurfaceSession(journey.restart_argv, journey.project, journey.restart_environment,
                                             journey.restart_size, output / 'restart')
            wait_for(journey.session, 'Storage: saved', timeout=35)
            journey.session.command('mcp')
            wait_for(journey.session, 'UNMANAGED MCP EFFECTS UNKNOWN', timeout=35)
            journey.capture('restart-retains-unmanaged-uncertainty')
            marker = json.loads((journey.data / 'mcp.json').read_text())
            assert str(journey.project.resolve()) in marker['uncertain_projects']
            assert 'uncertain_projects' in marker['critical']
            result['cases'].append('restart-loads-retained-unknown-before-any-effect-grant')
            journey.finish()
            peer.assert_clean()
            result['status'] = 'passed'
        except BaseException:
            result['error'] = traceback.format_exc()
            if journey is not None:
                journey.capture('failure')
            raise
        finally:
            if journey is not None:
                journey.session.close()
            files.save(output / 'results.json', result)
            files.save(output / 'http.json', peer.snapshot())
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--width', type=int)
    parser.add_argument('--height', type=int)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    sizes = [(args.width, args.height)] if args.width and args.height else smoke_sizes.selected([(120, 36), (60, 18)])
    with tempfile.TemporaryDirectory(prefix='eevee-mcp-smoke-') as directory:
        for size in sizes:
            name = f'{size[0]}x{size[1]}'
            run(args.binary.resolve(), output / name, Path(directory) / name, size)


if __name__ == '__main__':
    main()
