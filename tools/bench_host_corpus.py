"""Build and record the frozen host corpus in A/B/B/A process order."""

import argparse
from contextlib import contextmanager
import ctypes
from datetime import datetime, timezone
from hashlib import sha256
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess


ORDER = ("A1", "B1", "B2", "A2")
CHECKSUMS = {
    "volume": ("c086d204d748df10", 20_000),
    "counter": ("33762119da5ca390", 20_000),
    "scan16": ("ccf572a672f91bf6", 2_048),
    "scan256": ("5ce7852c6e608f80", 2_048),
    "decision_uniform": ("18885e81fa059ce0", 20_000),
    "decision_skewed": ("1311c541ff92feec", 20_000),
    "leaf": ("237871883635de4c", 20_000),
}
CHECKS = 35


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


@contextmanager
def affinity(cpu):
    if os.name == "nt":
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.GetCurrentProcess.restype = ctypes.c_void_p
        kernel.GetProcessAffinityMask.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_size_t),
            ctypes.POINTER(ctypes.c_size_t),
        ]
        kernel.SetProcessAffinityMask.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
        handle = kernel.GetCurrentProcess()
        process_mask, system_mask = ctypes.c_size_t(), ctypes.c_size_t()
        if not kernel.GetProcessAffinityMask(handle, ctypes.byref(process_mask),
                                             ctypes.byref(system_mask)):
            raise ctypes.WinError(ctypes.get_last_error())
        allowed = [i for i in range(8 * ctypes.sizeof(process_mask)) if process_mask.value >> i & 1]

        def set_mask(mask):
            if not kernel.SetProcessAffinityMask(handle, mask):
                raise ctypes.WinError(ctypes.get_last_error())

        chosen = allowed[0] if cpu is None else cpu
        require(chosen in allowed, f"CPU {chosen} is outside process affinity {allowed}")
        set_mask(1 << chosen)
        try:
            yield chosen, allowed
        finally:
            set_mask(process_mask.value)
    else:
        original = os.sched_getaffinity(0)
        chosen = min(original) if cpu is None else cpu
        os.sched_setaffinity(0, {chosen})
        try:
            yield chosen, sorted(original)
        finally:
            os.sched_setaffinity(0, original)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path, help="New directory for binaries, logs and manifest")
    parser.add_argument("--cpu", type=int, help="Logical CPU (default: lowest allowed)")
    parser.add_argument("--wasm", action="store_true", help="Run the same conformance checks under Node")
    parser.add_argument("--check-target", action="append", default=[],
                        help="Additional installed Rust target to check without linking or running")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True)
    env = {**os.environ, "FLUXR_TELEMETRY": "off"}
    commands = []

    def run(argv, *, timeout=90, log=None):
        argv = list(map(str, argv))
        commands.append(argv)
        result = subprocess.run(argv, cwd=root, env=env, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, timeout=timeout)
        if log:
            (evidence / log).write_bytes(result.stdout)
        result.check_returncode()
        return result.stdout.decode("utf-8", errors="replace").strip()

    source = run(["git", "rev-parse", "HEAD"])
    require(not run(["git", "status", "--short", "--untracked-files=no"]),
            "Commit tracked source before measuring")
    version = run(["rustc", "-Vv"])
    host = next(line.removeprefix("host: ") for line in version.splitlines()
                if line.startswith("host: "))
    cargo = run(["cargo", "-V"])
    target = Path(json.loads(run(["cargo", "metadata", "--locked", "--format-version", "1",
                                 "--no-deps"]))["target_directory"])
    ext = ".exe" if os.name == "nt" else ""
    build = ["cargo", "build", "--locked", "-j", "4", "--release", "--example", "bench_typed"]
    binaries = {}
    for label, feature in (("A", "native-jit"), ("B", "typed-jit")):
        run([*build, "--target", host, "--features", feature], timeout=360,
            log=f"build-{label}.txt")
        folder = target / host / "release" / "examples"
        binaries[label] = evidence / f"bench-{label}{ext}"
        shutil.copy2(folder / f"bench_typed{ext}", binaries[label])
        if (folder / "bench_typed.pdb").is_file():
            shutil.copy2(folder / "bench_typed.pdb", evidence / f"bench-{label}.pdb")

    with affinity(args.cpu) as (cpu, allowed):
        for label in ORDER:
            output = run([binaries[label[0]]], log=f"{label}.txt")
            expected = [f"host_corpus.checks={CHECKS}"]
            expected += [f"{name}.checksum={value} samples={samples}"
                         for name, (value, samples) in CHECKSUMS.items()]
            windows_x64 = host == "x86_64-pc-windows-msvc"
            native = str(windows_x64 and label.startswith("B")).lower()
            expected.extend(f"{name}.prepared_native={native}"
                            for name in ("volume", "tick", "decide", "pick"))
            missing = set(expected).difference(output.splitlines())
            require(not missing, f"{label} missing expected output: {sorted(missing)}")
            print(f"PASS {label} (CPU {cpu})", flush=True)

    portable = {}
    for triple in args.check_target:
        run(["cargo", "check", "--locked", "-j", "4", "--release", "--target", triple,
             "--example", "bench_typed", "--features", "typed-jit"],
            timeout=360, log=f"check-{triple}.txt")
        portable[triple] = "check only"
    if args.wasm:
        triple = "wasm32-unknown-unknown"
        run([*build, "--target", triple], timeout=360, log="build-wasm.txt")
        portable["wasm-bindgen"] = run(["wasm-bindgen", "--version"])
        portable["node"] = run(["node", "--version"])
        wasm = target / triple / "release/examples/bench_typed.wasm"
        run(["wasm-bindgen", "--target", "nodejs", "--out-dir", evidence, wasm],
            log="wasm-bindgen.txt")
        output = run(["node", "-e",
                      "console.log(require(process.argv[1]).host_corpus_check());",
                      evidence / "bench_typed.js"], log="node.txt")
        require(output == str(CHECKS), f"Unexpected Wasm conformance result: {output}")
        portable[triple] = f"{CHECKS} conformance cases passed under Node"

    counters = {}
    if os.name == "nt" and shutil.which("wpr"):
        run(["wpr", "-pmcsources"], log="wpr-sources.txt")
        counters["wpr_elevated"] = bool(ctypes.windll.shell32.IsUserAnAdmin())
    counters["recorded"] = False

    require(source == run(["git", "rev-parse", "HEAD"])
            and not run(["git", "status", "--short", "--untracked-files=no"]),
            "Tracked source changed during measurement")
    manifest = {
        "source": source,
        "utc": datetime.now(timezone.utc).isoformat(),
        "host": platform.platform(),
        "cpu": platform.processor(),
        "affinity": {"selected": cpu, "original": allowed},
        "rustc": version,
        "cargo": cargo,
        "profile": "release, telemetry off, no profiling during timing",
        "environment": {name: env[name] for name in (
            "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_PROFILE_RELEASE_LTO",
            "CARGO_PROFILE_RELEASE_OPT_LEVEL", "CARGO_PROFILE_RELEASE_CODEGEN_UNITS",
        ) if name in env},
        "process_order": ORDER,
        "checksums": CHECKSUMS,
        "portable": portable,
        "hardware_counters": counters,
        "commands": commands,
        "files": {path.name: digest(path) for path in sorted(evidence.iterdir()) if path.is_file()},
    }
    (evidence / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
