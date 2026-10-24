#!/usr/bin/env python3
"""Opt-in ACP-v1 protocol-only peer. Never writes project sources or launches commands.

Every source/compiler/program effect is requested from the actual Eevee binary.
The only peer filesystem writes are its isolated wire-evidence log.
"""
import json
import os
from pathlib import Path
import sys
import time

FIXED = '''fn main() {
    let answer = 42;
    if std::env::args().any(|arg| arg == "--slow-marker") {
        println!("ACP-SLOW-START");
        std::thread::sleep(std::time::Duration::from_secs(30));
        std::fs::write("must-not-exist", "uncancelled command effect").unwrap();
    }
    println!("AGENT-CODING:{}", answer);
}
'''
BROKEN = FIXED.replace('let answer = 42;', 'let answer = still_missing;')


def record(direction, packet):
    path = os.environ.get('EEVEE_ACP_TERMINAL_LOG')
    if path:
        with open(path, 'a', encoding='utf-8') as stream:
            stream.write(json.dumps(dict(direction=direction, packet=packet)) + '\n')


def emit(**packet):
    packet = dict(jsonrpc='2.0', **packet)
    record('peer', packet)
    print(json.dumps(packet), flush=True)


def receive():
    line = sys.stdin.readline()
    if not line:
        raise SystemExit(0)
    packet = json.loads(line)
    record('client', packet)
    return packet


def main():
    if '--version' in sys.argv or 'version' in sys.argv:
        print('devin 3000.11.3 (012345abcdef)' if Path(sys.argv[0]).name == 'devin' else '1.18.0')
        return
    if os.environ.get('EEVEE_ACP_TERMINAL_CHILD') != '1':
        raise SystemExit('This protocol peer is only for the opt-in isolated PTY driver')
    mode = os.environ['EEVEE_ACP_TERMINAL_MODE']
    compiler = os.environ['EEVEE_ACP_TERMINAL_COMPILER']
    session = 'owned-acp-terminal-session'
    root = None
    while True:
        packet = receive()
        method, identity = packet.get('method'), packet.get('id')
        if method == 'initialize':
            assert packet['params']['clientCapabilities']['terminal'] is (mode != 'readonly'), packet
            emit(id=identity, result=dict(protocolVersion=1, agentCapabilities={}))
        elif method == 'session/new':
            root = Path(packet['params']['cwd'])
            emit(id=identity, result=dict(sessionId=session))
        elif method == 'session/prompt':
            prompt = identity
            break
        else:
            raise AssertionError(packet)
    serial = 0
    answers = {}

    def request(method, **params):
        nonlocal serial
        serial += 1
        identity = 'terminal-callback-' + str(serial)
        emit(id=identity, method=method, params=dict(sessionId=session, **params))
        return identity

    def response(identity):
        while identity not in answers:
            answer = receive()
            if answer.get('method') == 'session/cancel':
                emit(id=prompt, result=dict(stopReason='cancelled'))
                raise SystemExit(0)
            if 'id' in answer:
                answers[answer['id']] = answer
        return answers.pop(identity)

    def call(method, **params):
        return response(request(method, **params))

    def create(command, args=None, **params):
        packet = call('terminal/create', command=str(command), args=args or [], cwd=str(root), **params)
        assert 'result' in packet, packet
        return packet['result']['terminalId']

    def finish(terminal, code):
        status = call('terminal/wait_for_exit', terminalId=terminal)
        assert status['result']['exitCode'] == code, status
        output = call('terminal/output', terminalId=terminal)
        assert output['result']['exitStatus']['exitCode'] == code, output
        evidence = output['result']['_meta']['eevee.dev/terminal']
        assert evidence['run']['run_id'] and evidence['artifact']['revision'], evidence
        assert call('terminal/release', terminalId=terminal).get('result') is not None
        assert 'error' in call('terminal/output', terminalId=terminal), 'released ID remained valid'
        return output['result']['output']

    if mode == 'readonly':
        assert 'error' in call('terminal/create', command=compiler, args=['--version'], cwd=str(root))
        assert 'error' in call('terminal/kill', terminalId='foreign')
        emit(id=prompt, result=dict(stopReason='end_turn'))
        while sys.stdin.readline():
            pass
        return

    # Reject unsupported environment/PTY and foreign cwd before allocating anything.
    for extension in (dict(env=[dict(name='SPECIAL', value='1')]), dict(pty=True),
                      dict(_meta=dict(pty=True)), dict(cwd=str(root.parent))):
        params = dict(command=compiler, args=['--version'], cwd=str(root))
        params.update(extension)
        assert 'error' in call('terminal/create', **params), extension
    assert call('fs/write_text_file', path=str(root / 'main.rs'), content=BROKEN).get('result') == {}
    assert call('fs/read_text_file', path=str(root / 'main.rs'))['result']['content'] == BROKEN
    arguments = ['--edition=2021', '--color=never', 'main.rs', '-o', 'agent-program']
    terminal = create(compiler, arguments)
    # create is returned before approval/run, and peer frames continue to be handled.
    pending = call('terminal/output', terminalId=terminal)['result']
    assert pending['output'] == '' and 'exitStatus' not in pending, pending
    assert pending['_meta']['eevee.dev/terminal']['state'] == 'pending'
    assert 'error' in call('fs/write_text_file', path=str(root / 'main.rs'), content='must not overwrite during run\n')
    assert 'error' in call('terminal/create', command=compiler, args=arguments, cwd=str(root))
    assert 'error' in call('terminal/output', terminalId='wrong-generation-or-task')
    foreign = request('terminal/output', terminalId=terminal)
    # Explicitly send a second, wrong-session frame with an independent callback ID.
    serial += 1
    wrong_session = 'terminal-callback-' + str(serial)
    emit(id=wrong_session, method='terminal/output', params=dict(sessionId='foreign-session', terminalId=terminal))
    assert 'result' in response(foreign)
    assert 'error' in response(wrong_session)
    assert 'error' in call('terminal/input', terminalId=terminal, text='not supported')
    failed = finish(terminal, 1)
    assert 'still_missing' in failed and 'error' in failed, failed
    assert call('fs/read_text_file', path=str(root / 'main.rs'))['result']['content'] == BROKEN
    assert call('fs/write_text_file', path=str(root / 'main.rs'), content=FIXED).get('result') == {}
    assert call('fs/read_text_file', path=str(root / 'main.rs'))['result']['content'] == FIXED
    terminal = create(compiler, arguments)
    if mode in ('deny', 'cancel'):
        assert 'error' in call('terminal/wait_for_exit', terminalId=terminal)
        call('terminal/release', terminalId=terminal)  # Denial is not a fabricated exit or successful kill.
        emit(id=prompt, result=dict(stopReason='end_turn'))
    else:
        finish(terminal, 0)
        terminal = create(root / 'agent-program', ['--slow-marker'] if mode in ('kill', 'release') else [])
        if mode in ('kill', 'release'):
            # The driver signals only after the normal UI has dispatched this exact run.
            # This control file is outside the project and carries no command/approval.
            control = Path(os.environ['EEVEE_ACP_TERMINAL_CONTROL'])
            deadline = time.monotonic() + 60
            while not control.exists():
                if time.monotonic() >= deadline:
                    raise AssertionError('driver did not observe owned running process')
                time.sleep(0.02)
            wait = request('terminal/wait_for_exit', terminalId=terminal)
            stopped = call('terminal/' + mode, terminalId=terminal)
            assert 'result' in stopped, stopped
            assert response(wait)['result']['signal'] is not None
            if mode == 'kill':
                output = call('terminal/output', terminalId=terminal)
                assert output['result']['exitStatus']['signal'] is not None, output
                assert call('terminal/release', terminalId=terminal).get('result') is not None
            assert 'error' in call('terminal/output', terminalId=terminal)
        else:
            assert 'AGENT-CODING:42' in finish(terminal, 0)
        emit(id=prompt, result=dict(stopReason='end_turn'))
    while sys.stdin.readline():
        pass


if __name__ == '__main__':
    main()
