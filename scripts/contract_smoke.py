#!/usr/bin/env python3
"""Run the reviewed-contract capability drivers concurrently.

Each driver owns its temporary project, data directory, loopback peers and PTY,
so they are independent. Output for each lands in `<output>/<name>` with its
console log; a failure prints that driver's log tail and fails the run.
"""
from __future__ import annotations

import argparse
import os
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

# Longest first so it never starts last. similar_moves keeps the fixed
# per-key window: its move-review flow races quiet-redraw settling.
DRIVERS = ('similar_moves', 'mcp', 'contract_profiles', 'delegation', 'steering')
FIXED_KEYS = {'similar_moves'}


def run(name: str, binary: Path, output: Path) -> tuple[str, int, float, Path]:
    script = Path(__file__).with_name(f'{name}_smoke.py')
    log = output / f'{name}.log'
    started = time.monotonic()
    environment = dict(os.environ)
    if name in FIXED_KEYS:
        environment.pop('EV_SMOKE_KEY_QUIET', None)
    with log.open('w', encoding='utf-8') as handle:
        code = subprocess.run(
            [sys.executable, str(script), '--binary', str(binary), '--output', str(output / name)],
            stdout=handle, stderr=subprocess.STDOUT, timeout=45 * 60, env=environment,
        ).returncode
    return name, code, time.monotonic() - started, log


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--jobs', type=int, default=3)
    parser.add_argument('--sizes', help='Comma-separated WxH subset, e.g. 60x18 (default: every size)')
    args = parser.parse_args()
    if args.sizes:
        os.environ['EV_SMOKE_SIZES'] = args.sizes
    # Keys settle once the redraw is quiet instead of a fixed 0.25 s each.
    os.environ.setdefault('EV_SMOKE_KEY_QUIET', '0.08')
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    failed = []
    with ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        for name, code, seconds, log in pool.map(lambda name: run(name, binary, output), DRIVERS):
            print(f'{"passed" if code == 0 else "FAILED"} {name} in {seconds:.0f}s', flush=True)
            if code:
                failed.append(name)
                print(log.read_text(encoding='utf-8', errors='replace')[-6000:], flush=True)
    return 1 if failed else 0


if __name__ == '__main__':
    raise SystemExit(main())
