#!/usr/bin/env python3
"""Assemble only the five verified native workflow packages for a version tag."""
from __future__ import annotations

import argparse
from pathlib import Path
import shutil
import tarfile
import tomllib
import zipfile

from package_binary import digest_file

TARGETS = ('darwin-arm64', 'darwin-x86_64', 'linux-x86_64', 'linux-arm64', 'windows-x86_64')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tag', required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    version = tomllib.loads((root / 'Cargo.toml').read_text())['package']['version']
    if args.tag != 'v' + version:
        raise ValueError('Release tag must exactly match Cargo.toml version')
    artifacts = Path(args.artifacts).resolve()
    verified = []
    for target in TARGETS:
        directory = artifacts / ('ev-' + target)
        filename = f'ev-{target}' + ('.zip' if target.startswith('windows-') else '.tar.gz')
        archive = directory / filename
        if not archive.is_file() or archive.is_symlink():
            raise ValueError(f'Missing regular native archive: {filename}')
        if not 0 < archive.stat().st_size <= 64 * 1024 * 1024:
            raise ValueError(f'Archive size exceeds package contract: {filename}')
        checksum = digest_file(archive)
        if (directory / 'SHA256SUMS').read_text(encoding='ascii') != f'{checksum}  {filename}\n':
            raise ValueError(f'Native checksum mismatch: {filename}')
        expected = {'ev.exe' if filename.endswith('.zip') else 'ev', 'LICENSE', 'THIRD_PARTY_NOTICES'}
        if filename.endswith('.zip'):
            with zipfile.ZipFile(archive) as package:
                names = package.namelist()
        else:
            with tarfile.open(archive) as package:
                members = package.getmembers()
                if not all(member.isfile() for member in members):
                    raise ValueError(f'Non-file native archive entry: {filename}')
                names = [member.name for member in members]
        if len(names) != len(expected) or set(names) != expected:
            raise ValueError(f'Unexpected or duplicated native archive members: {filename}')
        for installer in ('install.sh', 'install.ps1'):
            if (directory / installer).read_bytes() != (root / installer).read_bytes():
                raise ValueError(f'Installer differs from tagged source: {target}/{installer}')
        verified.append((archive, checksum))
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    sums = []
    for archive, checksum in verified:
        shutil.copyfile(archive, output / archive.name)
        sums.append(f'{checksum}  {archive.name}\n')
    for installer in ('install.sh', 'install.ps1'):
        shutil.copyfile(root / installer, output / installer)
        sums.append(f'{digest_file(output / installer)}  {installer}\n')
    (output / 'SHA256SUMS').write_text(''.join(sums), encoding='ascii')
    print(f'Assembled {args.tag}: five verified native archives, two installers, SHA256SUMS')


if __name__ == '__main__':
    main()
