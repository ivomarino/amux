#!/usr/bin/env python3
"""ATE-92: one Cargo cleanup guard for the builder and the embedded reclaim API.

The lifetime lease lives OUTSIDE target/ and survives exec into cargo and tests.
Reclaim also takes Cargo's native locks and probes processes for older/unwrapped
builds. Native lock files and their ancestors never move or disappear. Failed
probes/locks defer cleanup, regardless of free disk; the next idle tick retries.
"""
import argparse
import contextlib
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


class Deferred(Exception):
    def __init__(self, reason, measured=False, considered=0):
        super().__init__(reason)
        self.measured = measured
        self.considered = considered


def overlaps(left, right):
    return left == right or left in right.parents or right in left.parents


def lease_path(target):
    return target.parent / ('.' + target.name + '.reclaim.lock')


def open_lock(path, shared=False, wait=False):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(fd, (fcntl.LOCK_SH if shared else fcntl.LOCK_EX)
                    | (0 if wait else fcntl.LOCK_NB))
    except OSError:
        os.close(fd)
        raise Deferred('lock busy or unmeasured: ' + str(path))
    return fd


def native_locks(target):
    # Cargo profiles (debug/release/custom) and target triples. Do not traverse
    # deps/incremental/build: these can contain millions of artifacts, not locks.
    result = set()
    frontier = [(target, 0)]
    while frontier:
        directory, depth = frontier.pop()
        if not directory.exists():
            continue
        for entry in directory.iterdir():
            if entry.name in ('.cargo-lock', '.cargo-build-lock', '.cargo-artifact-lock'):
                result.add(entry)
            elif depth < 2 and entry.is_dir() and not entry.is_symlink() and entry.name not in (
                    'deps', 'incremental', 'build', 'examples', '.fingerprint', 'doc'):
                frontier.append((entry, depth + 1))
    # Create locks in existing standard profiles too, so a just-starting Cargo
    # cannot acquire a different inode after the process snapshot.
    for profile in ('debug', 'release'):
        if (target / profile).is_dir():
            result.add(target / profile / '.cargo-lock')
    return sorted(result)


def process_executable(pid, command, *, linux=None, proc_root=Path('/proc'),
                       readlink=None, which=None):
    """Resolve one process without treating an unreadable /proc link as a build.

    Hardened Linux runners can deny ``/proc/<pid>/exe`` for ordinary sibling
    processes.  ``ps comm`` is still measured evidence: when its complete name
    resolves through PATH (for example ``sleep`` or ``bash``), that executable
    cannot be a test artifact under a Cargo target.  Truncated/custom names do
    not resolve and therefore continue to fail closed.
    """
    executable = Path(command.removesuffix(' (deleted)'))
    linux = sys.platform.startswith('linux') if linux is None else linux
    if not linux:
        return executable
    readlink = os.readlink if readlink is None else readlink
    which = shutil.which if which is None else which
    proc = proc_root / pid
    try:
        if proc.stat().st_uid == os.getuid():
            executable = Path(readlink(proc / 'exe').removesuffix(' (deleted)'))
    except FileNotFoundError:
        return None  # process exited during the snapshot
    except PermissionError as error:
        known = which(executable.name)
        if known:
            known = Path(known).resolve()
            if known.name == executable.name:
                return known
        raise Deferred('process executable unmeasured: pid ' + pid) from error
    return executable


def active_processes(targets):
    try:
        output = subprocess.run(['ps', '-A', '-ww', '-o', 'pid=,comm='],
                                capture_output=True, text=True, check=True, timeout=5).stdout
    except (OSError, subprocess.SubprocessError) as error:
        raise Deferred('process probe unmeasured: ' + str(error)) from error
    rows = [line.split(None, 1) for line in output.splitlines() if line.strip()]
    if not rows or any(len(row) != 2 or not row[0].isdigit() for row in rows):
        raise Deferred('process probe unmeasured: empty or malformed ps output')
    active = []
    for pid, command in rows:
        # Linux ps comm is truncated. Resolve same-user executable paths so a
        # directly launched test binary is protected after its Cargo parent exits.
        executable = process_executable(pid, command)
        if executable is None:
            continue
        if executable.name in ('cargo', 'rustc', 'rustdoc', 'clippy-driver') or any(
                executable == root or root in executable.parents for root in targets):
            active.append(pid)
    return active, len(rows)


@contextlib.contextmanager
def exclusive(targets, process_probe=active_processes):
    descriptors = []
    locks = []
    try:
        for target in sorted(set(targets)):
            descriptors.append(open_lock(lease_path(target)))
            for lock in native_locks(target):
                descriptors.append(open_lock(lock))
                locks.append(lock)
        active, considered = process_probe(targets)
        if active:
            raise Deferred('active cargo/rustc/test process(es): ' + ','.join(active), True, considered)
        yield locks, considered
    finally:
        for descriptor in reversed(descriptors):
            os.close(descriptor)


def clear_preserving_locks(path, locks):
    if path in locks:
        return
    if any(path in lock.parents for lock in locks):
        for child in path.iterdir():
            clear_preserving_locks(child, locks)
    elif path.is_dir() and not path.is_symlink():
        shutil.rmtree(path)
    else:
        path.unlink(missing_ok=True)


def mutate(action, path, destination, targets, protected=(), dry_run=False,
           process_probe=active_processes):
    path = path.resolve()
    destination = destination.resolve() if destination else None
    touched = [root for root in targets if any(overlaps(root, candidate) for candidate in
               [path, *protected, *([destination] if destination else [])])]
    # A quarantine batch may contain artifacts moved by an older, unsafe server.
    # Probe those staged executable paths too, while locking their original roots.
    guard = exclusive(touched, lambda roots: process_probe([*roots, path])) if touched else contextlib.nullcontext(([], 0))
    with guard as (locks, considered):
        if action == 'move' and any(overlaps(path, lock) and (path == lock or path in lock.parents)
                                    for lock in locks):
            raise Deferred('Cargo lock directories cannot rotate; select obsolete artifact subdirectories')
        if any(path == lease_path(root) for root in targets):
            raise Deferred('Cargo lifetime lease cannot be reclaimed')
        if action == 'move' and destination.exists():
            raise Deferred('destination already exists')
        if action == 'move' and any(path == root or destination == root for root in touched):
            raise Deferred('Cargo target roots cannot rotate; select obsolete artifact subdirectories')
        if not dry_run:
            if action == 'clear':
                clear_preserving_locks(path, locks)
            elif action == 'move':
                destination.parent.mkdir(parents=True, exist_ok=True)
                path.rename(destination)
            elif action == 'purge':
                if path.exists():
                    clear_preserving_locks(path, locks)
        return {'verdict': 'dry_run' if dry_run else 'reclaimed',
                'measured': bool(touched), 'n_considered': considered}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['run', 'clear', 'move', 'purge'])
    parser.add_argument('--target', action='append', required=True)
    parser.add_argument('--path')
    parser.add_argument('--destination')
    parser.add_argument('--protected-path', action='append', default=[])
    parser.add_argument('--dry-run', action='store_true')
    # Parse command separately: argparse REMAINDER would swallow guard options.
    argsv = sys.argv[1:]
    command = []
    if '--' in argsv:
        split = argsv.index('--')
        argsv, command = argsv[:split], argsv[split + 1:]
    args = parser.parse_args(argsv)
    targets = sorted(set(Path(root).resolve() for root in args.target))
    try:
        if args.action == 'run':
            if not command:
                raise Deferred('no build command')
            # Acquired inside the systemd scope, and inherited by cargo and its
            # children. No parent polling window between build and test binaries.
            for target in targets:
                fd = open_lock(lease_path(target), shared=True, wait=True)
                os.set_inheritable(fd, True)
            os.execvp(command[0], command)
        if not args.path or (args.action == 'move' and not args.destination):
            raise Deferred('missing mutation path or destination')
        probe = active_processes
        fixture = os.environ.get('AMUX_CARGO_GUARD_TEST_FIXTURE')
        if fixture:
            fixture = Path(fixture).resolve()
            temp_roots = [Path(tempfile.gettempdir()).resolve(), Path('/tmp').resolve()]
            mutation_paths = [Path(args.path).resolve(),
                              *[Path(p).resolve() for p in args.protected_path],
                              *([Path(args.destination).resolve()] if args.destination else [])]
            if (os.environ.get('AMUX_RS_DISK_CLEAR_ONLY') != '1'
                    or not fixture.name.startswith('amux-cargo-guard-test.')
                    or not any(root in fixture.parents for root in temp_roots)
                    or not all(fixture in root.parents for root in targets)
                    or not all(fixture in path.parents for path in mutation_paths)):
                raise Deferred('invalid temporary fixture process-probe seam')
            # Tests may isolate host processes ONLY for throwaway target trees.
            # Locks still run; no real shared target can pass these constraints.
            probe = lambda _: ([], 1)
        result = mutate(args.action, Path(args.path),
                        Path(args.destination) if args.destination else None, targets,
                        [Path(p).resolve() for p in args.protected_path], args.dry_run, probe)
        print(json.dumps(result))
    except (Deferred, OSError) as error:
        print(json.dumps({'verdict': 'cargo_reclaim_deferred', 'measured': getattr(error, 'measured', False),
                          'n_considered': getattr(error, 'considered', 0), 'reason': str(error)}))
        return 75
    return 0


if __name__ == '__main__':
    sys.exit(main())
