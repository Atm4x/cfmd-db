#!/usr/bin/env python3
"""Build the repository manifest from staged Git bytes, excluding the manifest itself.

Stage source changes first, run this script, then stage REPOSITORY_MANIFEST.sha256.
Use --check to verify the manifest against the Git index without changing files.
"""

import argparse
import hashlib
from pathlib import Path
import subprocess
import sys


def build_manifest(root: Path) -> bytes:
    records = subprocess.check_output(['git', 'ls-files', '--stage', '-z'], cwd=root)
    entries = {}
    portable_paths = {}
    for record in records.split(b'\0'):
        if not record:
            continue
        metadata, raw_path = record.split(b'\t', 1)
        _, oid, stage = metadata.split()
        path = raw_path.decode('utf-8')
        if stage != b'0':
            raise ValueError(f'unmerged Git index entry: {path}')
        if path == 'REPOSITORY_MANIFEST.sha256':
            continue
        folded = path.casefold()
        if folded in portable_paths and portable_paths[folded] != path:
            raise ValueError(f'case-colliding Git paths: {portable_paths[folded]} and {path}')
        portable_paths[folded] = path
        entries[path] = oid

    hashes = {}
    lines = []
    with subprocess.Popen(
        ['git', 'cat-file', '--batch'], cwd=root,
        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
    ) as git:
        for path, oid in sorted(entries.items()):
            if oid not in hashes:
                git.stdin.write(oid + b'\n')
                git.stdin.flush()
                header = git.stdout.readline().split()
                if len(header) != 3 or header[1] != b'blob':
                    raise ValueError(f'Git blob unavailable: {path}')
                remaining = int(header[2])
                digest = hashlib.sha256()
                while remaining:
                    chunk = git.stdout.read(min(remaining, 1024 * 1024))
                    if not chunk:
                        raise ValueError(f'truncated Git blob: {path}')
                    digest.update(chunk)
                    remaining -= len(chunk)
                if git.stdout.read(1) != b'\n':
                    raise ValueError(f'invalid Git blob boundary: {path}')
                hashes[oid] = digest.hexdigest()
            lines.append(f'{hashes[oid]}  ./{path}\n')
        git.stdin.close()
        if git.wait() != 0:
            raise ValueError('git cat-file failed')
    return ''.join(lines).encode('utf-8')


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    manifest = root / 'REPOSITORY_MANIFEST.sha256'
    expected = build_manifest(root)
    if args.check:
        if manifest.read_bytes() != expected:
            print(
                'repository manifest differs from staged Git contents; stage the intended files, '
                'run python3 scripts/update-repository-manifest.py, then stage the manifest',
                file=sys.stderr,
            )
            return 1
        print('repository manifest matches Git index: PASS')
    else:
        manifest.write_bytes(expected)
        print(f'updated repository manifest: {len(expected.splitlines())} files')
    return 0


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (ValueError, subprocess.CalledProcessError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1) from error
