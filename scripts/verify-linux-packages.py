#!/usr/bin/env python3
"""Check shipped files and launch actual package payloads on isolated X11."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time


def run(args, **kwargs):
    return subprocess.run(args, check=True, capture_output=True, text=True, timeout=180, **kwargs).stdout


def launch(command, root, name):
    state = root / name
    state.mkdir()
    runtime = state / "runtime"
    runtime.mkdir(mode=0o700)
    env = dict(os.environ, LIBGL_ALWAYS_SOFTWARE="1", XDG_RUNTIME_DIR=str(runtime),
               XDG_CONFIG_HOME=str(state / "config"), XDG_DATA_HOME=str(state / "data"),
               XDG_CACHE_HOME=str(state / "cache"))
    env.pop("WAYLAND_DISPLAY", None)
    # A new X server for each payload prevents another instance satisfying a check.
    display_file = state / "display"
    with display_file.open("w") as display, (state / "xvfb.log").open("w") as xlog:
        xvfb = subprocess.Popen(["Xvfb", "-displayfd", str(display.fileno()), "-screen", "0",
                                 "1400x900x24", "-nolisten", "tcp"],
                                pass_fds=(display.fileno(),), stdout=xlog, stderr=xlog)
    process = None
    try:
        deadline = time.monotonic() + 20
        while not display_file.read_text().strip():
            assert xvfb.poll() is None and time.monotonic() < deadline, "Xvfb failed"
            time.sleep(0.1)
        env["DISPLAY"] = ":" + display_file.read_text().strip()
        log = state / "gui.log"
        with log.open("w") as output:
            process = subprocess.Popen(command, cwd=state, env=env, stdout=output, stderr=output,
                                       start_new_session=True)
        deadline = time.monotonic() + 90  # AppImage extraction includes corresponding source.
        while time.monotonic() < deadline:
            assert process.poll() is None, f"{name} exited: {log.read_text()}"
            tree = run(["xwininfo", "-root", "-tree"], env=env)
            if '"ez-sdr"' in tree:
                time.sleep(2)
                assert process.poll() is None, log.read_text()
                return {"payload": name, "native_window": True}
            time.sleep(0.25)
        raise AssertionError(f"{name}: no GUI window: {log.read_text()}")
    finally:
        if process is not None and process.poll() is None:
            # AppImage extract-and-run starts a child; terminate the process group.
            import signal
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=10)
        xvfb.terminate()
        xvfb.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dist", type=Path)
    parser.add_argument("--installed", action="store_true")
    args = parser.parse_args()
    dist = args.dist.resolve()
    deb, = dist.glob("ez-sdr_*_amd64.deb")
    appimage, = dist.glob("ez-sdr-*-x86_64.AppImage")
    appimage.chmod(0o755)  # workflow artifact downloads do not preserve modes.
    checksum, = dist.glob("*-linux-installers.sha256")
    run(["sha256sum", "--check", checksum.name], cwd=dist)
    checks = []
    with tempfile.TemporaryDirectory(prefix="ez-sdr-install-check-") as temp:
        root = Path(temp)
        extracted = root / "deb"
        run(["dpkg-deb", "--extract", str(deb), str(extracted)])
        doc = extracted / "usr/share/doc/ez-sdr"
        commit = (doc / "SOURCE-COMMIT.txt").read_text().strip()
        assert (doc / "SOURCE-CODE.tar.gz").stat().st_size > 0
        control = root / "control"
        run(["dpkg-deb", "--control", str(deb), str(control)])
        run(["md5sum", "--check", str(control / "md5sums")], cwd=extracted)
        run(["desktop-file-validate", str(extracted / "usr/share/applications/ez-sdr.desktop")])
        for binary in ("ez-daemon", "dump1090"):
            assert run([str(extracted / "usr/bin" / binary), "--help"]).strip()
        checks.append(launch([str(extracted / "usr/bin/ez-gui")], root, "debian-payload"))
        run([str(appimage), "--appimage-extract"], cwd=root)
        appdir = root / "squashfs-root"
        run(["sha256sum", "--check", "usr/share/doc/ez-sdr/SHA256SUMS"], cwd=appdir)
        assert (appdir / "usr/lib/librtlsdr.so.2").is_file()
        assert (appdir / "usr/lib/libpulse.so.0").is_file()
        library_env = dict(os.environ, LD_LIBRARY_PATH=str(appdir / "usr/lib"))
        for binary in (appdir / "usr/bin/ez-gui", appdir / "usr/lib/libpulse.so.0"):
            links = run(["ldd", str(binary)], env=library_env)
            assert "not found" not in links, links
            for library in ("librtlsdr.so.2", "libasound.so.2", "libpulsecommon-"):
                for line in links.splitlines():
                    if library in line:
                        assert str(appdir / "usr/lib") in line, line
        native_sources = appdir / "usr/share/doc/ez-sdr/native-source"
        assert list(native_sources.glob("*.dsc")), "Missing native corresponding source"
        assert (appdir / "usr/share/doc/ez-sdr/SOURCE-COMMIT.txt").read_text().strip() == commit
        checks.append(launch([str(appimage), "--appimage-extract-and-run"], root, "appimage"))
        if args.installed:
            assert run(["dpkg-query", "-W", "-f=${Status}", "ez-sdr"]) == "install ok installed"
            assert Path("/usr/share/applications/ez-sdr.desktop").exists()
            assert Path("/usr/share/doc/ez-sdr/SOURCE-COMMIT.txt").read_text().strip() == commit
            checks.append(launch(["/usr/bin/ez-gui"], root, "apt-installed"))
    report = {"source_commit": commit, "checks": checks,
              "package_checksums": True, "internal_manifests": True,
              "cli_help": True, "bundled_library_resolution": True,
              "sha256": {path.name: hashlib.file_digest(path.open("rb"), "sha256").hexdigest()
                         for path in (deb, appimage)},
              "scope": "Native GUI startup and CLI help; no physical radio/audio validation"}
    (dist / "linux-package-verification.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
