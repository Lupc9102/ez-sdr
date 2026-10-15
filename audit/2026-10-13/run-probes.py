"""Run local-only audit reproductions against already-built workspace libraries."""
from pathlib import Path
import json
from itertools import product
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
AUDIT = Path(__file__).resolve().parent
DEPS = ROOT / "target/debug/deps"


def fingerprint(library, crate):
    suffix = library.stem.rsplit("-", 1)[1]
    path = ROOT / "target/debug/.fingerprint" / f"{crate.replace('_', '-')}-{suffix}" / f"lib-{crate}.json"
    return json.loads(path.read_text()) if path.exists() else {}


helper_files = sorted(DEPS.glob("libez_gui-*.rlib"), key=lambda p: p.stat().st_mtime_ns, reverse=True)
helper_metadata = [fingerprint(p, "ez_gui") for p in helper_files]
compiler = next((d["rustc"] for d in helper_metadata if "test-helpers" in d.get("features", "")), None)


def libraries(crate):
    candidates = [p for p in DEPS.glob(f"lib{crate}-*.rlib") if fingerprint(p, crate).get("rustc") == compiler]
    if crate == "ez_gui":
        matched = []
        for library in candidates:
            if "test-helpers" in fingerprint(library, crate).get("features", ""):
                matched.append(library)
        if not matched:
            raise RuntimeError("Build ez-gui with --no-default-features --features test-helpers --lib first")
        candidates = matched
    return sorted(candidates, key=lambda p: p.stat().st_mtime_ns, reverse=True)


def rust_probe(name, crates):
    with tempfile.TemporaryDirectory(prefix="ez-sdr-audit-") as folder:
        executable = Path(folder) / name
        args = ["rustc", "--edition=2021", str(AUDIT / f"{name}.rs"), "-L", f"dependency={DEPS}", "-L", "native=/usr/local/lib", "-o", str(executable)]
        for crate in crates:
            args += ["--extern", f"{crate}={libraries(crate)[0]}"]
        for output in (ROOT / "target/debug/build").glob("*/output"):
            for line in output.read_text(errors="replace").splitlines():
                if line.startswith("cargo:rustc-link-search="):
                    path = line.split("=", 1)[1].removeprefix("native=")
                    if Path(path).is_dir():
                        args += ["-L", f"native={path}"]
        candidates = product(libraries("ez_proto"), libraries("num_complex")) if name == "core-probe" else [(None, None)]
        for proto, complex_library in candidates:
            extra = ["--extern", f"ez_proto={proto}", "--extern", f"num_complex={complex_library}"] if proto else []
            built = subprocess.run(args + extra, cwd=ROOT, capture_output=True, text=True)
            if built.returncode == 0:
                break
        else:
            raise RuntimeError(built.stderr)
        result = subprocess.run([str(executable)], cwd=ROOT, capture_output=True, text=True, timeout=30)
        (AUDIT / f"{name}.log").write_text(result.stdout + result.stderr)
        print(f"{name}:\n{result.stdout}{result.stderr}", flush=True)
        result.check_returncode()


rust_probe("core-probe", ["ez_daemon", "anyhow", "tokio"])
rust_probe("side-services-probe", ["ez_gui"])
rust_probe("rigctl-lifecycle-probe", ["ez_gui"])
rust_probe("popup-ppm-probe", ["ez_gui"])
for name in ["browser-lifecycle-probe", "desktop-remote-input-probe"]:
    result = subprocess.run(["node", str(AUDIT / f"{name}.js")], cwd=ROOT, capture_output=True, text=True, check=True)
    (AUDIT / f"{name}.log").write_text(result.stdout + result.stderr)
    print(f"{name}:\n{result.stdout}", flush=True)
