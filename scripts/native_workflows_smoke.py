#!/usr/bin/env python3
"""Run explicit native acceptance coverage against one immutable, already-built binary.

Each driver owns isolated homes/projects and its native process cleanup. This runner
bounds driver concurrency and duration, retains stdout/stderr on disk, and never
turns a failed or timed-out selected driver into passing qualification evidence.
--plan-only prints the exact coverage/argv manifest without touching the filesystem
or launching a driver. No build, unit test, private account or live call is implied.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

from file_creation_smoke import SIZES, selected_variants
from terminal_smoke import WindowsJob

PROFILES = ('full', 'windows', 'macos', 'architecture')
DRIVER_TIMEOUT = 20 * 60
TOOLS = (
    ('receipts', 'receipts_smoke.py', ()),
    ('context', 'context_optimizer_smoke.py', ()),
    ('catalog', 'tool_catalog_smoke.py', ()),
    ('artifacts', 'artifacts_smoke.py', ()),
    ('sources', 'source_observation_smoke.py', ('--mode', 'native')),
    ('conversation', 'conversation_flow_smoke.py', ('--mode', 'native')),
)


def job_count(value):
    count = int(value)
    if not 1 <= count <= 4:
        raise argparse.ArgumentTypeError('--jobs must be between 1 and 4')
    return count


def make_plan(binary, output, profile, jobs=2):
    if profile not in PROFILES:
        raise ValueError('unknown native coverage profile: ' + profile)
    job_count(jobs)
    binary, output = Path(binary).resolve(), Path(output).resolve()
    scripts = Path(__file__).resolve().parent
    file_profile = 'full' if profile == 'full' else 'platform'
    variants = selected_variants(file_profile)
    controlled = file_profile == 'full' and os.name == 'posix'
    sizes = [{'width': size[0], 'height': size[1], 'ascii': ascii_mode}
             for size, ascii_mode in SIZES]
    commands = []

    def add(name, script, language, extra, coverage):
        destination = output / name
        commands.append({
            'name': name, 'output': str(destination),
            'argv': [sys.executable, str(scripts / script), '--binary', str(binary),
                     '--output', str(destination), '--language', language, *extra],
            'timeout_seconds': DRIVER_TIMEOUT,
            'coverage': {'language': language, 'sizes': sizes, **coverage},
        })

    for language in ('en', 'ko'):
        add('platform-files-' + language, 'file_creation_smoke.py', language,
            ('--profile', file_profile), {
                'suite': 'files', 'profile': file_profile,
                'routes': [{'provider': variant[0], 'protocol': variant[1],
                            'keyless': variant[2]}
                           for variant in variants],
                'controlled_codex': controlled,
            })
    languages = () if profile == 'architecture' else (('ko',) if profile == 'macos' else ('en', 'ko'))
    for language in languages:
        for suite, script, extra in TOOLS:
            add('platform-tools-' + suite + '-' + language, script, language, extra,
                {'suite': suite, 'mode': 'native'})
    native_files = 2 * len(SIZES) * len(variants)
    codex_files = 2 * len(SIZES) if controlled else 0
    tool_runs = len(languages) * len(TOOLS) * len(SIZES)
    return {
        'schema_version': 1, 'profile': profile, 'jobs': jobs,
        'binary': str(binary), 'output': str(output),
        'counts': {'commands': len(commands), 'native_file_journeys': native_files,
                   'controlled_codex_journeys': codex_files, 'tool_journeys': tool_runs,
                   'total_journeys': native_files + codex_files + tool_runs},
        'commands': commands,
    }


def stop_process(process, job):
    if os.name == 'nt':
        if job is not None:
            job.close()  # Existing kill-on-close job owns the driver and every native descendant.
        elif process.poll() is None:
            process.kill()  # Startup handshake has not released any descendant yet.
        process.wait(timeout=10)
        return
    # Drivers own their detached PTY sessions. Give their finally blocks time to
    # close those sessions before escalating against this owned process group.
    process.send_signal(signal.SIGINT)
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        # A timed-out wait has not reaped the group leader, so its ID cannot be
        # reused. Never signal a saved PID/group after a successful wait.
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=5)


def run_command(command, logs):
    started = time.monotonic()
    stdout_path, stderr_path = (logs / (command['name'] + suffix)
                                for suffix in ('.stdout.log', '.stderr.log'))
    result = {'name': command['name'], 'status': 'failed', 'returncode': None,
              'stdout': str(stdout_path), 'stderr': str(stderr_path)}
    process = job = None
    try:
        argv = command['argv']
        if os.name == 'nt':
            # Gate startup until the existing Windows ownership job is attached.
            argv = [sys.executable, str(Path(__file__).with_name('terminal_smoke.py')),
                    '--owned-command', '--', *argv]
        with stdout_path.open('wb') as stdout, stderr_path.open('wb') as stderr:
            process = subprocess.Popen(argv, stdin=subprocess.PIPE if os.name == 'nt' else subprocess.DEVNULL,
                                       stdout=stdout, stderr=stderr, start_new_session=os.name != 'nt')
            try:
                if os.name == 'nt':
                    job = WindowsJob(process.pid)
                    process.stdin.write(b'EEVEE_SMOKE_START\n')
                    process.stdin.close()
                result['returncode'] = process.wait(timeout=command['timeout_seconds'])
                result['status'] = 'passed' if result['returncode'] == 0 else 'failed'
            except subprocess.TimeoutExpired:
                result['timed_out'] = True
                raise
            finally:
                if process.poll() is None:
                    stop_process(process, job)
                if job is not None:
                    job.close()
    except Exception as error:
        result['status'] = 'failed'
        result['error'] = str(error)
        if process is not None:
            result['returncode'] = process.returncode
    result['duration_seconds'] = round(time.monotonic() - started, 3)
    return result


def digest(binary):
    with Path(binary).open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def run_plan(plan):
    output = Path(plan['output'])
    output.mkdir(parents=True, exist_ok=False)
    logs = output / 'logs'
    logs.mkdir()
    save(output / 'coverage-manifest.json', plan)
    report = {'status': 'failed', 'profile': plan['profile'], 'counts': plan['counts'],
              'binary': plan['binary'], 'results': []}
    started = time.monotonic()
    try:
        report['binary_sha256'] = digest(plan['binary'])
        with ThreadPoolExecutor(max_workers=plan['jobs']) as pool:
            futures = [pool.submit(run_command, command, logs) for command in plan['commands']]
            # Consume every selected result, including later work after an early failure.
            for command, future in zip(plan['commands'], futures):
                try:
                    report['results'].append(future.result())
                except Exception as error:
                    report['results'].append({'name': command['name'], 'status': 'failed',
                                              'error': str(error)})
        report['binary_sha256_after'] = digest(plan['binary'])
        if report['binary_sha256'] != report['binary_sha256_after']:
            raise RuntimeError('supplied binary changed during qualification')
        if (len(report['results']) == len(plan['commands']) and report['results']
                and all(result['status'] == 'passed' for result in report['results'])):
            report['status'] = 'passed'
    except Exception as error:
        report['error'] = str(error)
    finally:
        report['duration_seconds'] = round(time.monotonic() - started, 3)
        save(output / 'results.json', report)
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--profile', choices=PROFILES, default='full')
    parser.add_argument('--jobs', type=job_count, default=2)
    parser.add_argument('--plan-only', action='store_true')
    args = parser.parse_args(argv)
    plan = make_plan(args.binary, args.output, args.profile, args.jobs)
    if args.plan_only:
        print(json.dumps(plan, ensure_ascii=False, indent=2))
        return 0
    if not args.binary.is_file():
        parser.error('--binary must be an existing, already-built executable')
    report = run_plan(plan)
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
