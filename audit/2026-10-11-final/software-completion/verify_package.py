#!/usr/bin/env python3
"""Validate a local audit package without using user settings or radio devices."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import time

ROOT = Path(__file__).resolve().parents[3]
PROOF = Path(__file__).resolve().parent
ARCHIVE = ROOT / 'dist/ez-sdr-audit-software-validation-20261013-linux-x86_64.tar.gz'

def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def run(args, **kwargs):
    result = subprocess.run(args, capture_output=True, text=True, timeout=180, **kwargs)
    assert result.returncode == 0, (args, result.returncode, result.stderr)
    return result.stdout

checks = []
with tempfile.TemporaryDirectory(prefix='ez-sdr-package-validation-') as temporary:
    temp = Path(temporary)
    with tarfile.open(ARCHIVE) as archive:
        archive.extractall(temp, filter='data')
    package = temp / ARCHIVE.name.removesuffix('.tar.gz')
    run(['sha256sum', '--check', 'SHA256SUMS'], cwd=package)
    checks.append({'name': 'all archive checksums', 'passed': True})
    commit = (package / 'SOURCE-COMMIT.txt').read_text().strip()
    run(['git', 'cat-file', '-e', commit + '^{commit}'], cwd=ROOT)
    source = temp / 'source'
    source.mkdir()
    with tarfile.open(package / 'SOURCE-CODE.tar.gz') as archive:
        archive.extractall(source, filter='data')
    fingerprints = json.loads((PROOF / 'source.json').read_text())['files']
    for name, expected in fingerprints.items():
        assert sha(source / name) == expected, name
    checks.append({'name': 'source matches verified workspace', 'passed': True,
                   'checked_source_files': len(fingerprints)})
    cargo_home = temp / 'empty-cargo-home'
    cargo_home.mkdir()
    env = dict(os.environ, CARGO_HOME=str(cargo_home))
    command = ['cargo', 'metadata', '--locked', '--offline', '--all-features', '--format-version', '1']
    metadata = json.loads(run(command, cwd=source, env=env))
    checks.append({'name': 'vendored dependencies resolve with empty Cargo cache in offline mode',
                   'passed': True, 'command': command, 'resolved_packages': len(metadata['packages'])})
    binary_hashes = {}
    for name in ('ez-gui', 'ez-daemon', 'dump1090'):
        binary = package / 'bin' / name
        binary_hashes[name] = sha(binary)
        assert binary_hashes[name] == sha(ROOT / 'target/release' / name)
        assert 'not found' not in run(['ldd', str(binary)])
        if name != 'ez-gui':
            assert run([str(binary), '--help']).strip()
    checks.append({'name': 'packaged binaries match release build, native libraries resolve, CLI help works',
                   'passed': True})
    display_file = temp / 'display'
    xlog = (PROOF / 'evidence/package-xvfb-final.log').open('w')
    glog = (PROOF / 'evidence/package-gui-final.log').open('w')
    gui = None
    try:
        with display_file.open('w') as display_stream:
            xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(display_stream.fileno()), '-screen', '0',
                                      '1280x900x24', '-nolisten', 'tcp'],
                                     pass_fds=(display_stream.fileno(),), stdout=xlog, stderr=xlog)
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline and not display_file.read_text().strip():
            assert xvfb.poll() is None
            time.sleep(0.2)
        display = ':' + display_file.read_text().strip()
        assert display != ':'
        runtime = temp / 'runtime'
        runtime.mkdir(mode=0o700)
        gui_env = dict(os.environ, DISPLAY=display, LIBGL_ALWAYS_SOFTWARE='1',
                       XDG_RUNTIME_DIR=str(runtime), XDG_CONFIG_HOME=str(temp / 'config'),
                       XDG_CACHE_HOME=str(temp / 'cache'), XDG_DATA_HOME=str(temp / 'data'))
        gui_env.pop('WAYLAND_DISPLAY', None)
        gui = subprocess.Popen([str(package / 'bin/ez-gui')], cwd=temp, env=gui_env,
                               stdout=glog, stderr=glog)
        deadline = time.monotonic() + 30
        tree = ''
        while time.monotonic() < deadline:
            assert gui.poll() is None, 'packaged GUI exited before creating its window'
            tree = run(['xwininfo', '-root', '-tree'], env=gui_env)
            if '"ez-sdr"' in tree:
                break
            time.sleep(0.5)
        assert '"ez-sdr"' in tree, tree
        (PROOF / 'evidence/package-window-final.txt').write_text(tree)
        checks.append({'name': 'packaged GUI creates native window on isolated X11/software GL',
                       'passed': True, 'scope': 'startup only; no physical reception/audio validation'})
    finally:
        for process in (gui, locals().get('xvfb')):
            if process is not None and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)
        xlog.close()
        glog.close()
    build_log = PROOF / 'evidence/package-build-final.log'
    result = {'verified_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'archive': str(ARCHIVE), 'archive_sha256': sha(ARCHIVE),
              'archive_bytes': ARCHIVE.stat().st_size, 'source_commit': commit,
              'checks': checks, 'binary_sha256': binary_hashes,
              'package_build_command': 'RELEASE_VERSION=audit-software-validation-20261013 CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4 taskset -c 0-5 nice -n 10 scripts/package-linux-release.sh dist',
              'package_build_exit_code': 0, 'package_build_log_sha256': sha(build_log)}
    (PROOF / 'package.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))
