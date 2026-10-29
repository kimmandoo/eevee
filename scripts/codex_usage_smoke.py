#!/usr/bin/env python3
"""Explicit-opt-in, exactly-three-turn real Codex TUI usage qualification.

Uses the supplied candidate, the actual installed Codex app-server and a private
transparent protocol guard. No provider replies, usage rows or sessions are
fabricated. Only sanitized reconciliation evidence survives temporary cleanup.
The third turn explicitly resumes the owned session in the same TUI lifecycle;
cross-process/external-session baselines are outside this paid qualification.

--preflight-only exercises metadata and the actual first-turn local preview,
then cancels it with a zero-turn protocol permit. It does not authorize a turn.
"""
from __future__ import annotations

import argparse
from contextlib import ExitStack, closing
import fcntl
import hashlib
import json
import os
import re
from pathlib import Path
import secrets
import shlex
import sqlite3
import stat
import subprocess
import sys
import tempfile
import threading
import time

sys.dont_write_bytecode = True

from terminal_smoke import Session, confirm_exit, terminal_environment


class QualificationSession(Session):
    def command(self, name):
        # A nonempty Request treats slash as literal text, not an action.
        # Always use the global chooser without touching the authored bytes.
        self.send("\x0b")
        self.send(name)
        self.send("\r")

FIELDS = {'input': 'inputTokens', 'output': 'outputTokens',
          'cache_read': 'cachedInputTokens', 'cache_write': 'cacheWriteInputTokens',
          'reasoning': 'reasoningOutputTokens'}
ANSWERS = ('EV_USAGE_AMBER_713', 'EV_USAGE_CEDAR_829', 'EV_USAGE_QUARTZ_137')
PROMPTS = tuple('Reply exactly ' + answer + '; no tools.' for answer in ANSWERS)
RPC_STAGES = ('initialize', 'initialized', 'account/read', 'config/read', 'model/list',
              'thread/start', 'thread/resume', 'turn/start')
PREFLIGHT_STAGES = ('workspace_saved', 'goal_saved', 'planning_form_saved',
                    'metadata_ready', 'exact_model_committed', 'request_saved',
                    'local_preview_ready', 'preview_cancelled')


def screen_flags(screen):
    """Never return screen content, account/config values, IDs or private paths."""
    markers = {
        'storage_saved': 'Storage: saved',
        'command_chooser': '/ Commands / type to search',
        'pending_approvals': 'pending approvals',
        'run_available': 'y Run',
        'preview_blocked': 'Run blocked',
        'metadata_only_scope': 'Scope: model catalogue only; no task/model turn.',
        'metadata_loading': 'Loading metadata',
        'metadata_failed': 'Fetch failed',
        'repository_wait': 'Waiting for repository evidence',
        'saved_input_refused': 'Save task/context first and wait',
        'saved_ack_refused': 'Wait for saved task/context acknowledgement',
        'local_work_refused': 'Wait for task/context/journal storage acknowledgement',
        'preferences_loading': 'Restoring the last provider/model preference',
        'connection_review': 'Review this project-supplied runtime or unmanaged connection',
        'protocol_failure': 'Protocol',
        'authentication_failure': 'Authentication layer',
        'model_failure': 'Model catalogue',
        'executable_prerequisite': 'executable or project identity unavailable',
        'identity_changed': 'identity changed; review a fresh',
        'reasoning_picker': 'Reasoning level',
        'model_picker': '> Focus · * Current',
        'request_unavailable': '/new is unavailable in the current provider context.',
        'pending_local_or_provider': 'Wait for pending local/provider work.',
        'empty_goal_refused': 'Save a nonempty goal in an active task before planning.',
        'journal_storage_refused': 'Retry failed journal storage before preparing',
        'preparing_local_preview': 'Preparing local preview.',
        'waiting_owned_reservation': 'owned reservation',
    }
    return {name: marker in screen for name, marker in markers.items()}


class QualificationError(Exception):
    """Messages must be static, nonprivate prerequisite/contract descriptions."""


def require(condition, message):
    if not condition:
        raise QualificationError(message)


def private_json(path, value):
    with path.open('w', encoding='utf-8') as stream:
        json.dump(value, stream)
    path.chmod(0o600)


def ledger_update(path, callback):
    # One common lock covers concurrent relay readers/writers and the TUI driver.
    with path.open('r+', encoding='utf-8') as stream:
        fcntl.flock(stream, fcntl.LOCK_EX)
        value = json.load(stream)
        result = callback(value)
        stream.seek(0)
        json.dump(value, stream)
        stream.truncate()
        stream.flush()
        os.fsync(stream.fileno())
        return result


def relay():
    """Forward untouched protocol bytes; retain a strictly whitelisted projection."""
    path = Path(os.environ['EV_USAGE_LEDGER'])
    real = os.environ['EV_USAGE_CODEX']
    argv = sys.argv[2:]
    if argv == ['--version']:
        completed = subprocess.run([real, *argv], stdout=subprocess.PIPE,
                                   stderr=subprocess.DEVNULL, check=False)
        # A wrapper's arbitrary stdout is not public version evidence.
        match = re.fullmatch(
            rb'codex-cli ([0-9]{1,5}\.[0-9]{1,5}\.[0-9]{1,5}(?:-(?:alpha|beta|rc)\.[0-9]{1,5})?)\s*',
            completed.stdout)
        ledger_update(path, lambda value: value.update(
            public_version=match.group(1).decode('ascii') if match else None,
            version_checked=True))
        sys.stdout.buffer.write(completed.stdout)
        sys.stdout.buffer.flush()
        return completed.returncode
    if not argv or argv[0] != 'app-server':
        ledger_update(path, lambda value: value['violations'].append('Unexpected Codex invocation'))
        return 90
    # Provider launch flags pass through unchanged, never substitute a model.
    process = subprocess.Popen([real, *argv], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL)
    calls, local = {}, {'ordinal': None, 'thread_requested': False}
    stopped = threading.Event()

    def violation(message):
        ledger_update(path, lambda value: value['violations'].append(message))
        stopped.set()
        process.terminate()

    def outgoing(frame):
        method, params = frame.get('method'), frame.get('params') or {}
        require(method in RPC_STAGES,
                'Unexpected RPC; no extra model, tool, login or approval call authorized')
        ledger_update(path, lambda value: value['protocol_sent'].__setitem__(method, True))
        if 'id' in frame:
            calls[str(frame['id'])] = method
        if method == 'account/read':
            require(params.get('refreshToken') is False, 'Authentication refresh requested')
        if method == 'config/read':
            require(params.get('includeLayers') is False, 'Private configuration layers requested')
        if method == 'model/list':
            require(params.get('includeHidden') is False and params.get('limit') == 100,
                    'Unbounded or hidden model catalogue requested')
        if method in ('thread/start', 'thread/resume'):
            require(not local['thread_requested'], 'Repeated thread request in one submission')
            local['thread_requested'] = True
            def check(value):
                ordinal = value['sent'] + 1
                require(ordinal == value['permit'] and ordinal <= 3,
                        'Unpermitted thread request or hidden retry')
                require(params.get('model') == value['model'], 'Exact selected model changed')
                require(params.get('sandbox') == 'read-only' and
                        params.get('approvalPolicy') == 'on-request' and
                        params.get('approvalsReviewer') == 'user', 'Thread read-only policy changed')
                config = params.get('config') or {}
                require(config.get('features.multi_agent') is False and
                        config.get('web_search') == 'disabled' and
                        config.get('allow_login_shell') is False, 'Thread tool policy changed')
                require('dynamicTools' not in params, 'Dynamic write tools are not authorized')
                if ordinal == 1:
                    require(method == 'thread/start' and params.get('allowProviderModelFallback') is False,
                            'First turn must start without model fallback')
                else:
                    require(method == 'thread/resume' and params.get('threadId') == value.get('session'),
                            'Continuation must actually resume the exact owned session')
                local['ordinal'] = ordinal
                value['threads'].append({'ordinal': ordinal, 'method': method, 'pre_usage': []})
            ledger_update(path, check)
        if method == 'turn/start':
            def check(value):
                ordinal = local['ordinal']
                require(ordinal == value['permit'] == value['sent'] + 1 and ordinal <= 3,
                        'More than one model dispatch per explicit authorization')
                require(params.get('threadId') == value.get('session'), 'Turn session identity changed')
                require(params.get('model') == value['model'], 'Turn selected model changed')
                require(params.get('sandboxPolicy') == {'type': 'readOnly', 'networkAccess': False}
                        and params.get('approvalPolicy') == 'on-request'
                        and params.get('approvalsReviewer') == 'user', 'Turn read-only policy changed')
                settings = (params.get('collaborationMode') or {}).get('settings') or {}
                require(settings.get('model') == value['model'], 'Collaboration model changed')
                inputs = params.get('input')
                require(isinstance(inputs, list) and len(inputs) == 1 and inputs[0].get('type') == 'text',
                        'Unexpected prompt envelope')
                payload = json.loads(inputs[0]['text'])
                require(payload.get('request') == PROMPTS[ordinal - 1], 'Unexpected or repeated authored prompt')
                # Conservatively consume authorization before forwarding any turn bytes.
                value['sent'] += 1
                value['turns'].append({'ordinal': ordinal, 'session': value['session'],
                                       'usage': [], 'completed': False})
            ledger_update(path, check)

    def incoming(frame):
        method, params = frame.get('method'), frame.get('params') or {}
        require(not (method and 'id' in frame), 'Provider requested tools, approval or user interaction')
        call = calls.get(str(frame.get('id')))
        result = frame.get('result') or {}
        if call in RPC_STAGES:
            ledger_update(path, lambda value: value['protocol_received'].__setitem__(call, True))
        if 'error' in frame:
            # Never retain provider error text, codes, request IDs or error data.
            ledger_update(path, lambda value: value.update(protocol_blocker='rpc_error:' + (call or 'unknown')))
            return
        if call == 'initialize' and not isinstance(result.get('userAgent'), str):
            ledger_update(path, lambda value: value.update(protocol_blocker='initialize_shape'))
        if call == 'config/read' and not isinstance(result.get('config'), dict):
            ledger_update(path, lambda value: value.update(protocol_blocker='config_shape'))
        if call == 'model/list':
            valid = isinstance(result.get('data'), list)
            ledger_update(path, lambda value: value.update(
                metadata_complete=valid and result.get('nextCursor') is None,
                protocol_blocker=None if valid else 'model_list_shape'))
        if call == 'account/read':
            account_type = (result.get('account') or {}).get('type')
            ledger_update(path, lambda value: value.update(
                account_usable=account_type in ('chatgpt', 'apiKey', 'amazonBedrock')))
            require(account_type in ('chatgpt', 'apiKey', 'amazonBedrock'), 'Usable copied account not reported')
            ledger_update(path, lambda value: value['account_types'].append(account_type))
        if call in ('thread/start', 'thread/resume'):
            session_id = (result.get('thread') or {}).get('id')
            def note(value):
                require(isinstance(session_id, str) and session_id, 'Actual thread identity missing')
                require(value.get('session') in (None, session_id), 'Provider replaced the owned session')
                value['session'] = session_id
                require(result.get('model') == value['model'] and
                        result.get('approvalPolicy') == 'on-request' and
                        result.get('approvalsReviewer') == 'user' and
                        (result.get('sandbox') or {}).get('type') == 'readOnly' and
                        (result.get('sandbox') or {}).get('networkAccess') is not True,
                        'Provider failed exact model and read-only acknowledgement')
            ledger_update(path, note)
        if method in ('item/started', 'item/completed'):
            item = params.get('item') or {}
            require(item.get('type') in ('userMessage', 'agentMessage', 'reasoning', 'plan'),
                    'Provider used a tool despite tool-free request')
        if method == 'thread/tokenUsage/updated':
            # Retain IDs and counter fields only, never account objects or raw bodies.
            usage = params.get('tokenUsage') or {}
            projection = {'session': params.get('threadId'), 'turn': params.get('turnId')}
            for boundary in ('last', 'total'):
                source = usage.get(boundary)
                projection[boundary] = ({name: source.get(wire) for name, wire in FIELDS.items()}
                                        if isinstance(source, dict) else None)
            def note(value):
                ordinal = local['ordinal']
                if ordinal:
                    if len(value['turns']) >= ordinal:
                        value['turns'][ordinal - 1]['usage'].append(projection)
                    else:
                        value['threads'][ordinal - 1]['pre_usage'].append(projection)
            ledger_update(path, note)
        turn = result.get('turn') if call == 'turn/start' else params.get('turn')
        if isinstance(turn, dict) and local['ordinal']:
            def note(value):
                ordinal = local['ordinal']
                if len(value['turns']) >= ordinal:
                    entry = value['turns'][ordinal - 1]
                    turn_id = turn.get('id')
                    require(entry.get('turn') in (None, turn_id), 'Provider turn identity changed')
                    entry['turn'] = turn_id
                    if turn.get('status') == 'completed':
                        entry['completed'] = True
            ledger_update(path, note)

    def read_provider():
        try:
            for raw in process.stdout:
                require(len(raw) <= 2 * 1024 * 1024, 'Provider frame exceeded protocol limit')
                incoming(json.loads(raw))
                sys.stdout.buffer.write(raw)
                sys.stdout.buffer.flush()
        except QualificationError as error:
            violation(str(error))
        except Exception:
            violation('Unreadable actual provider protocol')
        finally:
            stopped.set()

    reader = threading.Thread(target=read_provider, daemon=True)
    reader.start()
    try:
        for raw in sys.stdin.buffer:
            if stopped.is_set():
                break
            outgoing(json.loads(raw))
            process.stdin.write(raw)
            process.stdin.flush()
    except QualificationError as error:
        violation(str(error))
    except Exception:
        violation('Unreadable actual application protocol')
    finally:
        if process.poll() is None:
            process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        reader.join(timeout=2)
    return 0


def source_fingerprint(path):
    if not path.exists() and not path.is_symlink():
        return None
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1,
            'Original auth/config must be regular nonlinked files; no keychain fallback is authorized')
    # These private digests remain memory-only and are never included in evidence.
    return stat.S_IMODE(info.st_mode), hashlib.sha256(path.read_bytes()).digest()


def reconcile(records, wire, threads, digest):
    require(len(records) == len(wire) == 3, 'Expected exactly three persisted actual request records')
    require(len({row['request_id'] for row in records}) == 3, 'Request identity reused')
    require(all(isinstance(turn.get('turn'), str) and turn['turn'] for turn in wire)
            and len({turn['turn'] for turn in wire}) == 3, 'Actual turn identity missing or reused')
    require(len({turn['session'] for turn in wire}) == 1, 'Owned turns did not share one actual session')
    owner = records[0]['owner']
    task = records[0]['task_id']
    previous = None
    evidence = []
    for number, (record, turn) in enumerate(zip(records, wire), 1):
        require(record['agent'] == 'Codex' and record['owner'] == owner and record['task_id'] == task,
                'Persisted usage not bound to the same actual task and partner owner')
        require(turn['completed'], 'Actual correlated turn did not complete')
        if number > 1:
            for update in threads[number - 1]['pre_usage']:
                if (update['session'] == turn['session'] and
                        update['turn'] == wire[number - 2]['turn'] and update['total'] is not None):
                    previous = {key: (update['total'][key] if update['total'][key] is not None
                                      else previous[key]) for key in FIELDS}
        expected = {}
        for update in turn['usage']:
            if update['session'] != turn['session'] or update['turn'] != turn['turn']:
                continue  # Unknown/historical notifications do not establish current ownership.
            for boundary, scope in (('last', 'Response'), ('total', 'Session')):
                if update[boundary] is not None and any(value is not None for value in update[boundary].values()):
                    target = expected.setdefault(scope, dict.fromkeys(FIELDS))
                    for key, value in update[boundary].items():
                        if value is not None:
                            require(isinstance(value, int) and not isinstance(value, bool) and value >= 0,
                                    'Provider counter is not a nonnegative integer')
                            target[key] = value
        require(set(expected) == {'Response', 'Session'}, 'Actual provider did not supply both last and cumulative usage')
        measurements = record['measurements']
        require(len(measurements) == 2 and {m['report']['scope'] for m in measurements} == set(expected),
                'Repeated usage snapshots duplicated or lost persisted boundaries')
        public = {'ordinal': number, 'request_digest': digest(record['request_id']),
                  'session_digest': digest(turn['session']), 'turn_digest': digest(turn['turn']),
                  'owner_digest': digest(json.dumps(owner, sort_keys=True)),
                  'task_digest': digest(str(task)),
                  'selected_model_acknowledged': True, 'boundaries': []}
        for measurement in measurements:
            report = measurement['report']
            scope = report['scope']
            identity = report['identity']
            require(identity['session'] == turn['session'] and identity['turn'] == turn['turn']
                    and identity.get('response') is None and identity.get('event') is None,
                    'Persisted identity differs from the actual reported usage identity')
            require(report['counters'] == expected[scope] and report['kind'] == 'Snapshot'
                    and report['complete'] is True and measurement['delta_events'] == [],
                    'Persisted counters/completion differ from merged actual protocol snapshots')
            require(report['model'] is None and report['tokenizer'] is None,
                    'Unreported usage model/tokenizer was inferred from selected settings')
            require(report['semantics'] == {'cache_read_in_input': True, 'cache_write_in_input': None,
                                             'reasoning_in_output': True}, 'Usage subset semantics changed')
            baseline = previous if scope == 'Session' else None
            require(report['baseline'] == baseline, 'Cumulative baseline not the preceding completed owned turn')
            attributable = (expected[scope] if scope == 'Response' else
                            {key: (expected[scope][key] - baseline[key]
                                   if baseline is not None and expected[scope][key] is not None
                                   and baseline[key] is not None and expected[scope][key] >= baseline[key]
                                   else None) for key in FIELDS})
            require(measurement['attributable'] == attributable, 'Persisted cumulative attribution is incorrect')
            public['boundaries'].append({'scope': scope, 'reported': report['counters'],
                'baseline': baseline, 'attributable': attributable, 'model': None, 'tokenizer': None,
                'semantics': report['semantics']})
        previous = expected['Session']
        evidence.append(public)
    return evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--live', action='store_true', help='Authorize exactly THREE actual model turns; may consume paid usage')
    parser.add_argument('--preflight-only', action='store_true',
                        help='Inspect metadata, prepare and cancel the actual preview; authorize ZERO turns')
    parser.add_argument('--binary', required=True, type=Path, help='Final already-built candidate')
    parser.add_argument('--codex', required=True, type=Path, help='Actual installed Codex executable, not a fixture')
    parser.add_argument('--model', required=True, help='Exact selected model ID; no fallback')
    parser.add_argument('--source-codex-home', type=Path, default=Path(os.environ.get('CODEX_HOME', Path.home() / '.codex')))
    parser.add_argument('--output', required=True, type=Path, help='New sanitized evidence directory')
    args = parser.parse_args()
    if args.live == args.preflight_only:
        parser.error('Choose exactly one of --live (THREE turns) or --preflight-only (ZERO turns)')
    print('BILLING WARNING: exactly THREE real turns authorized.' if args.live else
          'ZERO-TURN PREFLIGHT: metadata access only; actual request preview is cancelled.', flush=True)
    require(os.name == 'posix', 'This private PTY qualification requires POSIX')
    binary, codex = args.binary.resolve(), args.codex.resolve()
    require(binary.is_file() and os.access(binary, os.X_OK), 'Final candidate executable is missing')
    require(codex.is_file() and os.access(codex, os.X_OK), 'Actual Codex executable is missing')
    require(args.model and not any(ch.isspace() or ord(ch) < 32 for ch in args.model), 'Exact model ID must be one token')
    require(not args.output.exists(), 'Evidence directory must be new; do not overwrite earlier live results')
    args.output.mkdir(parents=True, mode=0o700)
    original = args.source_codex_home.expanduser().resolve()
    originals = {name: source_fingerprint(original / name) for name in ('auth.json', 'config.toml')}
    require(originals['auth.json'] is not None,
            'A file-backed existing Codex auth.json is required; login, refresh and keychain discovery are not authorized')
    result = {'status': 'failed', 'authorized_turn_limit': 3 if args.live else 0,
              'actual_dispatched_turns': 0, 'mode': 'live' if args.live else 'preflight-only',
              'authorizations_consumed_before_wire_write': 0, 'dispatch_outcome_unknown_count': 0,
              'billing': 'Provider-owned; no cost estimate', 'scope': 'same-TUI owned-session explicit resume',
              'external_or_restarted_session_baseline': 'not exercised; no attribution inferred',
              'original_auth_config_unchanged': False, 'credential_copies_removed': False}
    stages = PREFLIGHT_STAGES if args.preflight_only else PREFLIGHT_STAGES[:-1]
    result['preflight'] = {'expected_stages': list(stages),
                           'reached': dict.fromkeys(stages, False),
                           'zero_dispatch_before_authorization': False}
    salt = secrets.token_bytes(32)
    digest = lambda value: hashlib.sha256(salt + value.encode()).hexdigest()[:24]
    session = None
    root_path = None
    try:
        with tempfile.TemporaryDirectory(prefix='ev-codex-usage-') as temporary, ExitStack() as cleanup:
            root = Path(temporary).resolve()
            root_path = root
            root.chmod(0o700)
            home, project, data, private_codex, bins = [root / name for name in ('home', 'project', 'data', 'codex', 'bin')]
            for directory in (home, project, data, private_codex, bins):
                directory.mkdir(mode=0o700)
            auth = private_codex / 'auth.json'
            descriptor = os.open(auth, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, 'wb') as stream:
                stream.write((original / 'auth.json').read_bytes())
            # No original config, hooks, MCP servers, skills, projects or history are copied.
            config = private_codex / 'config.toml'
            config.write_text('model = ' + json.dumps(args.model) + '\nweb_search = "disabled"\n'
                              'allow_login_shell = false\n[features]\nmulti_agent = false\n', encoding='utf-8')
            config.chmod(0o600)
            ledger = root / 'wire.json'
            private_json(ledger, {'model': args.model, 'permit': 0, 'sent': 0,
                                 'threads': [], 'turns': [], 'account_types': [], 'violations': [],
                                 'protocol_sent': dict.fromkeys(RPC_STAGES, False),
                                 'protocol_received': dict.fromkeys(RPC_STAGES, False),
                                 'metadata_complete': False, 'account_usable': False,
                                 'public_version': None, 'version_checked': False,
                                 'protocol_blocker': None})
            wrapper = bins / 'codex'
            wrapper.write_text('#!/bin/sh\nexec ' + shlex.quote(sys.executable) + ' ' +
                               shlex.quote(str(Path(__file__).resolve())) + ' --relay "$@"\n', encoding='utf-8')
            wrapper.chmod(0o700)
            # Do not inherit provider keys, config overrides, plugins or user startup files.
            environment = {key: value for key, value in terminal_environment('mono').items()
                           if key in ('PATH', 'TERM', 'COLORTERM', 'LANG', 'LC_ALL', 'LC_CTYPE',
                                      'PYTHONUTF8', 'SYSTEMROOT', 'SSL_CERT_FILE', 'SSL_CERT_DIR')}
            environment.update(HOME=str(home), XDG_CONFIG_HOME=str(home / 'config'),
                XDG_DATA_HOME=str(home / 'data'), XDG_CACHE_HOME=str(home / 'cache'),
                TMPDIR=str(root), CODEX_HOME=str(private_codex),
                EV_USAGE_CODEX=str(codex), EV_USAGE_LEDGER=str(ledger),
                PATH=str(bins) + os.pathsep + environment.get('PATH', ''))
            session = QualificationSession([str(binary), '--data-dir', str(data), '--images', 'off',
                               '--color', 'mono', ], project, environment,
                              (120, 36), root / 'terminal', controlling_tty=True)

            def stop_session():
                nonlocal session
                try:
                    observed = ledger_update(ledger, lambda value: json.loads(json.dumps(value)))
                    confirmed = sum(bool(turn.get('turn')) for turn in observed['turns'])
                    result['actual_dispatched_turns'] = confirmed
                    result['authorizations_consumed_before_wire_write'] = observed['sent']
                    result['dispatch_outcome_unknown_count'] = observed['sent'] - confirmed
                    result['preflight'].update(
                        protocol_sent=observed['protocol_sent'],
                        protocol_received=observed['protocol_received'],
                        metadata_complete=observed['metadata_complete'],
                        account_usable=observed['account_usable'],
                        public_version=observed['public_version'],
                        version_checked=observed['version_checked'],
                        protocol_blocker=observed['protocol_blocker'],
                        protocol_guard_blocked=bool(observed['violations']))
                    if session is not None:
                        result['preflight']['observed'] = screen_flags(session.display())
                finally:
                    if session is not None:
                        # Session.close normally persists raw PTY/events. This
                        # qualification never writes those private bytes, even temporarily.
                        session.text = ''
                        session.events = []
                        session.close()
                        session = None

            cleanup.callback(stop_session)

            def wire():
                return ledger_update(ledger, lambda value: json.loads(json.dumps(value)))

            def until(predicate, message, timeout=30):
                deadline = time.monotonic() + timeout
                while not predicate():
                    state = wire()
                    result['preflight']['observed'] = screen_flags(session.display())
                    require(not state['violations'], state['violations'][0] if state['violations'] else '')
                    require(state['protocol_blocker'] is None,
                            'Provider metadata protocol prerequisite failed; see redacted protocol blocker')
                    require(session.alive(), 'Candidate exited before qualification completed')
                    require(time.monotonic() < deadline, message)
                    session.pump(.05)

            def visible(text, timeout=30):
                until(lambda: text in session.display(), 'TUI prerequisite unavailable: ' + text, timeout)

            def rows(table):
                require(table in ('tasks', 'provider_usage', 'active_work'), 'Unexpected SQL table')
                with closing(sqlite3.connect((data / 'workbench.sqlite3').as_uri() + '?mode=ro', uri=True)) as db:
                    db.execute('PRAGMA query_only=ON')
                    db.row_factory = sqlite3.Row
                    return [dict(row) for row in db.execute('SELECT * FROM "' + table + '" ORDER BY rowid')]

            def usage():
                return [json.loads(row['record']) for row in rows('provider_usage')]

            def workspace():
                for _ in range(8):
                    footer = session.display().rstrip().splitlines()[-1].strip()
                    if footer.startswith(('Type Request |', 'Enter Send |')):
                        return footer
                    require(not any(marker in session.display() for marker in ('y Run', 'Confirm exit')),
                            'Unresolved consent blocks Request editing')
                    session.send('\x1b')
                raise QualificationError('TUI Request editor is unavailable')

            visible('Storage: saved')
            result['preflight']['reached']['workspace_saved'] = True
            session.command('goal')
            visible('Task fact / local')
            goal = 'Read-only provider usage qualification. Follow the current tiny reply request; no tools.'
            session.paste(goal)
            session.send('\r')
            until(lambda: any(task['goal'] == goal for task in rows('tasks')), 'TUI-created goal did not persist')
            visible('Storage: saved')
            result['preflight']['reached']['goal_saved'] = True
            session.command('forms')
            visible('Choose evolution')
            session.send('\x1b[H' + '\x1b[C' * 5 + '\r')
            session.command('role-plan')
            until(lambda: any(task['form'] == 4 for task in rows('tasks')),
                  'Read-only planning form did not persist')
            visible('Espeon')
            visible('Storage: saved')
            result['preflight']['reached']['planning_form_saved'] = True
            # Select the actual CLI transport explicitly; an evolution does
            # not imply its provider/connection method.
            session.command('provider-codex')
            workspace()
            visible('Storage: saved')
            session.command('models-refresh')
            metadata_consent_given = False

            def metadata_ready():
                nonlocal metadata_consent_given
                state = wire()
                if state['metadata_complete'] and '> Focus' in session.display():
                    return True
                flags = screen_flags(session.display())
                if flags['run_available'] and not flags['command_chooser'] and not metadata_consent_given:
                    require(flags['metadata_only_scope'],
                            'Refusing nonmetadata launch during zero-permit inspection')
                    # This is metadata consent, never model-turn consent. The
                    # permit remains zero and rejects thread/start and turn/start.
                    metadata_consent_given = True
                    session.send('y')
                return False

            until(metadata_ready, 'Metadata prerequisite unavailable; no turn authorized', 60)
            result['preflight']['reached']['metadata_ready'] = True
            require(wire()['permit'] == 0 and wire()['sent'] == 0 and not wire()['threads'],
                    'Zero-permit metadata inspection attempted provider work')
            workspace()
            session.command('model ' + args.model)
            until(lambda: screen_flags(session.display())['reasoning_picker']
                  or args.model in session.display().splitlines()[0],
                  'Exact model selection did not reach its reasoning review or committed header')
            if screen_flags(session.display())['reasoning_picker']:
                # Commit the visible model+reasoning transaction at Default.
                # Global commands do not own input while this picker is editing.
                session.send('\x1b[H\r')
            workspace()
            visible('Storage: saved')
            until(lambda: args.model in session.display().splitlines()[0],
                  'Exact model selection did not commit')
            result['preflight']['reached']['exact_model_committed'] = True
            persisted = []
            for number, prompt in enumerate(PROMPTS, 1):
                footer = workspace()
                if footer.startswith('Type Request |'):
                    session.send('\r')
                session.send('\x1b[A' * 16 + '\x1b[H' + '\x1b[3~' * 512)
                session.paste(prompt)
                session.send('\x13')
                until(lambda: any(draft.get('field') == 'Request' and draft.get('text') == prompt
                    for task in rows('tasks')
                    for state in json.loads(task.get('partner_drafts') or '{}').values()
                    for draft in state.get('drafts', [])), 'Exact authored Request did not persist')
                # SQL visibility precedes App's saved-revision acknowledgement.
                # Do not open a one-shot slash action until the UI observes it.
                visible('Storage: saved')
                result['preflight']['reached']['request_saved'] = True
                before = {record['request_id'] for record in usage()}
                require(wire()['permit'] == number - 1 and wire()['sent'] == number - 1,
                        'Prior authorization/dispatch count changed before preview')
                session.command('session-new' if number == 1 else 'session-resume')
                until(lambda: 'y Run' in session.display()
                      or 'pending approvals' in session.display(),
                      'Prepared request did not reach its explicit approval queue')
                if 'y Run' not in session.display():
                    session.command('approvals')
                visible('y Run')
                require(args.model in session.display() and 'Codex' in session.display()
                        and 'Plan' in session.display() and prompt in session.display()
                        and not screen_flags(session.display())['command_chooser'],
                        'Actual scope did not acknowledge the exact model, planning role and authored request')
                result['preflight']['reached']['exact_model_committed'] = True
                if number == 1:
                    result['preflight']['zero_dispatch_before_authorization'] = (
                        not wire()['threads'] and wire()['sent'] == 0)
                result['preflight']['reached']['local_preview_ready'] = True
                if args.preflight_only:
                    require(wire()['permit'] == 0 and wire()['sent'] == 0 and not wire()['threads'],
                            'Preflight unexpectedly consumed model authorization')
                    session.send('n')
                    until(lambda: 'y Run' not in session.display(),
                          'Actual request preview did not cancel')
                    result['preflight']['reached']['preview_cancelled'] = True
                    break
                # One explicit authorization, never resend on timeout/failure.
                ledger_update(ledger, lambda value: value.update(permit=number))
                session.send('y')
                until(lambda: wire()['sent'] == number, 'Authorized turn was not dispatched; no retry performed', 60)
                until(lambda: len(usage()) == number and wire()['turns'][number - 1]['completed'],
                      'Actual completed turn usage did not persist; no retry performed', 240)
                until(lambda: not rows('active_work'), 'Committed usage did not release its actual work owner', 30)
                new = [record for record in usage() if record['request_id'] not in before]
                require(len(new) == 1, 'More than one persisted request from one authorized dispatch')
                persisted.append(new[0])
            observed = wire()
            require(not observed['violations'], 'Protocol guard observed an unauthorized operation')
            if args.preflight_only:
                require(observed['permit'] == observed['sent'] == 0
                        and not observed['threads'] and not observed['turns'] and not usage(),
                        'Zero-turn preflight produced provider work or usage')
            else:
                require([row['method'] for row in observed['threads']] == ['thread/start', 'thread/resume', 'thread/resume'],
                        'Actual start/resume RPC sequence differed')
                result['turns'] = reconcile(persisted, observed['turns'], observed['threads'], digest)
                require(len(set(observed['account_types'])) == 1 and len(observed['account_types']) == 4,
                        'Account type changed or metadata/turn account acknowledgement was missing')
                result.update(actual_dispatched_turns=observed['sent'], account_type=observed['account_types'][0],
                              actual_thread_resume_calls=2, explicit_final_resume=True,
                              no_overlapping_scope_sum=True, usage_model='unknown', tokenizer='unknown')
            workspace()
            session.command('quit')
            confirm_exit(session)
            require(session.wait_exit() == 0, 'Candidate did not exit cleanly after persisted usage')
            session.text = ''
            session.events = []
            session.close()
            session = None
            result['status'] = 'passed'
    except QualificationError as error:
        result['blocker'] = str(error)
    except Exception as error:
        # No raw exception text/traceback: provider/TUI failures may contain private paths or account data.
        result['blocker'] = 'Qualification interrupted by ' + type(error).__name__ + '; no automatic retry'
    finally:
        result['credential_copies_removed'] = root_path is not None and not root_path.exists()
        try:
            result['original_auth_config_unchanged'] = all(
                source_fingerprint(original / name) == expected for name, expected in originals.items())
        except Exception:
            result['original_auth_config_unchanged'] = False
        if not result['original_auth_config_unchanged'] or not result['credential_copies_removed']:
            result['status'] = 'failed'
            result['blocker'] = 'Original credential/config preservation or temporary credential cleanup failed'
        private_json(args.output / 'results.json', result)
    print(json.dumps(result, indent=2), flush=True)
    return 0 if result['status'] == 'passed' else 1


if __name__ == '__main__':
    if len(sys.argv) > 1 and sys.argv[1] == '--relay':
        try:
            raise SystemExit(relay())
        except Exception:
            raise SystemExit(91)
    try:
        raise SystemExit(main())
    except QualificationError as error:
        print('BLOCKED: ' + str(error), file=sys.stderr)
        raise SystemExit(1)
    except Exception as error:
        # Setup can fail before the cleanup/evidence scope is entered. Do not
        # expose OS exception text containing credential/configuration paths.
        print('BLOCKED: private prerequisite interrupted by ' + type(error).__name__,
              file=sys.stderr)
        raise SystemExit(1)
