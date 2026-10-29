#!/usr/bin/env python3
"""Deterministic stdio MCP fixture, compatible with POSIX PTY and Windows ConPTY.

Only writes fixture note/log/pid files beneath --root. No provider/network calls.
"""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time


def serve(root: Path):
    sys.stdin.reconfigure(encoding='utf-8', errors='strict')
    sys.stdout.reconfigure(encoding='utf-8', errors='strict', newline='\n')
    root.mkdir(parents=True, exist_ok=True)
    (root / 'mcp.pid').write_text(str(os.getpid()))

    def log(value):
        with (root / 'mcp-events.jsonl').open('a', encoding='utf-8') as stream:
            stream.write(json.dumps(value, ensure_ascii=False) + '\n')

    def send(value):
        sys.stdout.write(json.dumps(value, ensure_ascii=False, separators=(',', ':')) + '\n')
        sys.stdout.flush()

    log({'event': 'launch', 'pid': os.getpid(), 'cwd': os.getcwd(), 'env_names': sorted(os.environ)})
    pending = {}
    for line in sys.stdin:
        request = json.loads(line)
        method = request.get('method')
        if method is None:
            log({'event': 'server-request-reply', 'reply': request})
            if request.get('id') in pending:
                original = pending.pop(request['id'])
                send({'jsonrpc': '2.0', 'id': original, 'result': {'content': [{'type': 'text', 'text': json.dumps(request)}]}})
            continue
        log({'event': method, 'id': request.get('id'), 'params': request.get('params', {})})
        if 'id' not in request:
            continue
        result = None
        if method == 'initialize':
            result = {'protocolVersion': '2024-11-05', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'unmanaged-fixture', 'version': '1'}}
        elif method == 'tools/list':
            schema = {'type': 'object', 'properties': {'text': {'type': 'string'}, 'mode': {'type': 'string'}}, 'required': ['text'], 'additionalProperties': False}
            result = {'tools': [
                {'name': 'echo', 'description': 'Untrusted echo fixture; annotations do NOT grant.', 'inputSchema': schema, 'annotations': {'readOnlyHint': True}},
                {'name': 'write_note', 'description': 'Writes the fixture note.txt; unmanaged effect.', 'inputSchema': {'type': 'object', 'properties': {'text': {'type': 'string'}}, 'required': ['text'], 'additionalProperties': False}},
                {'name': 'unselected_secret', 'description': 'UNSELECTED_SCHEMA_SENTINEL_93ca', 'inputSchema': {'type': 'object', 'properties': {'UNSELECTED_SCHEMA_SENTINEL_93ca': {'type': 'string'}}}},
            ]}
        elif method == 'tools/call':
            params = request['params']
            arguments = params['arguments']
            name = params['name']
            mode = arguments.get('mode', '')
            if mode == 'crash':
                os._exit(23)
            if mode == 'hang':
                child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(600)'])
                (root / 'mcp-child.pid').write_text(str(child.pid))
                log({'event': 'hanging', 'pid': os.getpid(), 'child': child.pid})
                time.sleep(600)
            if mode in ('roots', 'sampling', 'elicitation', 'resources', 'prompts'):
                server_id = 'server-' + mode
                pending[server_id] = request['id']
                method = {'roots': 'roots/list', 'sampling': 'sampling/createMessage', 'elicitation': 'elicitation/create', 'resources': 'resources/list', 'prompts': 'prompts/list'}[mode]
                send({'jsonrpc': '2.0', 'id': server_id, 'method': method, 'params': {}})
                continue
            if name == 'write_note':
                (root / 'note.txt').write_text(arguments['text'], encoding='utf-8')
            text = 'x' * (70 * 1024) if mode == 'oversize' else arguments.get('text', '')
            result = {'content': [{'type': 'text', 'text': text}]}
        if result is None:
            send({'jsonrpc': '2.0', 'id': request['id'], 'error': {'code': -32601, 'message': 'unsupported fixture method'}})
        else:
            send({'jsonrpc': '2.0', 'id': request['id'], 'result': result})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', required=True, type=Path)
    args = parser.parse_args()
    serve(args.root.resolve())


if __name__ == '__main__':
    main()
