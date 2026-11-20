#!/usr/bin/env python3
"""Reject private/generated paths in the index and all reachable Git history.

This is a publication guard, not a secret scanner. Exact local master-content
matching also detects renamed copies when the private local document is available.
"""
import hashlib
import subprocess
import sys
from pathlib import Path, PurePosixPath

PRIVATE_DIRS = {'.mother-private', '.mother', '.mother-state', 'runtime-state',
                'auth-profiles', '.codex', '.claude', '.gh', 'gh-config'}
GENERATED_DIRS = {'target', 'coverage', 'artifacts', '__pycache__'}
PRIVATE_SUFFIXES = ('.db', '.db-wal', '.db-shm', '.sqlite', '.sqlite-wal', '.sqlite-shm',
                    '.sqlite3', '.sqlite3-wal', '.sqlite3-shm', '.key', '.pem', '.p12',
                    '.pfx', '.log', '.dump', '.profraw', '.profdata', '.pyc')


def git(*args):
    return subprocess.run(['git', *args], check=True, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE).stdout


def forbidden(path):
    parts = tuple(part.casefold() for part in PurePosixPath(path).parts)
    name = parts[-1] if parts else ''
    return (name in {'mother.md', '.git-credentials', 'id_rsa', 'id_ed25519'}
            or any(part in PRIVATE_DIRS | GENERATED_DIRS for part in parts[:-1])
            or (name.startswith('.env') and name not in {'.env.example', '.env.sample'})
            or name.endswith(PRIVATE_SUFFIXES))


def check():
    root = Path(git('rev-parse', '--show-toplevel').decode().strip())
    master = root / 'MOTHER.md'
    master_hash = hashlib.sha256(master.read_bytes()).digest() if master.is_file() else None
    blobs = {}
    failures = set()

    def inspect(path, oid, scope):
        if forbidden(path):
            failures.add(f'{scope}: private/generated path {path!r}')
        if master_hash is not None:
            if oid not in blobs:
                blobs[oid] = hashlib.sha256(git('cat-file', 'blob', oid)).digest()
            if blobs[oid] == master_hash:
                failures.add(f'{scope}: local private directive content at {path!r}')

    for record in git('ls-files', '--stage', '-z').split(b'\0'):
        if not record:
            continue
        metadata, path = record.split(b'\t', 1)
        mode, oid, stage = metadata.decode().split()
        if stage != '0':
            failures.add('index: unresolved merge entry')
        elif mode == '160000':
            failures.add('index: submodules require separate publication review')
        else:
            inspect(path.decode('utf-8', 'surrogateescape'), oid, 'index')

    commits = git('rev-list', '--all').decode().splitlines()
    trees = set()
    for commit in commits:
        tree = git('rev-parse', f'{commit}^{{tree}}').strip().decode()
        if tree in trees:
            continue
        trees.add(tree)
        for record in git('ls-tree', '-r', '-z', tree).split(b'\0'):
            if not record:
                continue
            metadata, path = record.split(b'\t', 1)
            _, kind, oid = metadata.decode().split()
            if kind != 'blob':
                failures.add(f'history {commit[:12]}: non-blob entry requires review')
            else:
                inspect(path.decode('utf-8', 'surrogateescape'), oid, f'history {commit[:12]}')

    if failures:
        for failure in sorted(failures):
            print(f'BLOCKED: {failure}', file=sys.stderr)
        return 1
    print(f'Public-tree guard passed: index and {len(commits)} reachable commits; '
          f'local private-content comparison {"enabled" if master_hash else "unavailable"}.')
    return 0


if __name__ == '__main__':
    try:
        sys.exit(check())
    except (OSError, subprocess.CalledProcessError):
        print('BLOCKED: Git/publication inspection failed; no publication is safe.', file=sys.stderr)
        sys.exit(1)
