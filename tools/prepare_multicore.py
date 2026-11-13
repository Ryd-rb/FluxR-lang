#!/usr/bin/env python3
"""Import the pinned, MIT-licensed challenge reference into canonical FluxR 1.

Lossless mode is an interoperability/code-generation control, not a new language
implementation claim. Structured mode rewrites seven functions in FluxR syntax;
the remaining native Rust is counted and identified explicitly in provenance.
The caller supplies an already verified source checkout; this tool downloads
nothing and never modifies that reference.
"""
import argparse
import json
from pathlib import Path
import shutil
import tempfile

import fluxr as native

REVISION = "6145e7291a3625c184a58b63b86ed9d1f7304700"
SUBMISSION = "submissions/rust-epoll-theCatWisel"

PINNED_FILES = {'Cargo.lock': '98e2a5429b1f772c2f6c783a63e978fb4b31261ed1feaa7d7f2b05ff02f2225d', 'Cargo.toml': 'c721cd4f42bce417ac06fd633e8fbacca7983140c663c553a2efaaf758e7704f', 'build.rs': '3f1044695adb38448bcf33f498a19c57d4f8dfb0a74d46ac6d5903a568ecd444', 'src/auth.rs': '00afc4d7e311c2c9548f1c67555ac13c2fcea8ac9c5b3368618e2ab8d15e1ce0', 'src/codec.rs': '743f85152df26c2397e6dfad1b89a639cba41ab697edb8263db9fe231311b3ef', 'src/engine.rs': 'd0d82f86fe73041644a67c18205fc99428c66f82b4257faa5aaddda7692b8092', 'src/main.rs': '094f2f3d803e241f7e439cd49094c7c11122a4f603906be341f5302b2209ffc8', 'src/queries.rs': 'a89be4d3dcc3b99d077a6b8afecddc4dfeddb0225bbd0296fd87c7a111efc20d', 'src/reactor.rs': '904d3b9c9e325d25c902966d8c9e92c46df8a33093c6755cfe9de299570e3edb', 'src/router.rs': '48f76141a19dd21a8a4bde512ea0a01153271bc152abf3cde877b2a2028af9a1', 'src/wire.rs': 'a4d0c2501e9edf1120b55a8a62f46002c643afcaa462ba89af07fa47800ab33f'}
LICENSE_SHA256 = '1c809f52dd742570746a8ac00694ad4ddd9229e12accc7d5c598075861b478a0'

REWRITES = {
    "src/engine.rs": [
        ("fn mono_micros() -> usize {\n    auth::mono_micros()\n}",
         "fn mono_micros 0 usize 1 tail call auth::mono_micros 0\n"),
        ("pub fn read_op_lane(op: &ReadOp) -> usize {\n    lane_for_op(op)\n}",
         "pubfn read_op_lane 1 op &ReadOp usize 1 tail call lane_for_op 1 path op\n"),
        ("pub fn wake_read_dispatcher(lane: usize) {\n    if COALESCING_LANE[lane].load(Ordering::SeqCst) == 0 && WAITING_READ_WORKERS.load(Ordering::SeqCst) > 0 {\n        let _g = BATCH_LOCK.lock().unwrap();\n        READ_DISPATCH_CV.notify_one();\n    }\n}",
         "pubfn wake_read_dispatcher 1 lane usize unit 1\n"
         "if-stmt bin and\n"
         "  bin eq method index path COALESCING_LANE path lane load 1 path Ordering::SeqCst int 0\n"
         "  bin gt method path WAITING_READ_WORKERS load 1 path Ordering::SeqCst int 0\n"
         "2\nlet _g method method path BATCH_LOCK lock 0 unwrap 0\n"
         "do method path READ_DISPATCH_CV notify_one 0\n0\n"),
        ("pub fn enqueue_read_front(task: ReadTask) {\n    let lane = lane_for_op(&task.op);\n    READ_LANES[lane].push_front(task);\n    wake_read_dispatcher(lane);\n}",
         "pubfn enqueue_read_front 1 task ReadTask unit 3\n"
         "let lane call lane_for_op 1 ref field path task op\n"
         "do method index path READ_LANES path lane push_front 1 path task\n"
         "do call wake_read_dispatcher 1 path lane\n"),
        ("fn pack_pair(viewer: i64, target: i64) -> u64 {\n    ((viewer as u64) << 32) | (target as u32 as u64)\n}",
         "fn pack_pair 2 viewer i64 target i64 u64 1\n"
         "tail bin bitor bin shl cast path viewer u64 int 32 cast cast path target u32 u64\n"),
    ],
    "src/reactor.rs": [
        ("pub fn worker_outbox(worker: usize) -> &'static WorkerOutbox {\n    unsafe { &*OUTBOX_RING.load(Ordering::Relaxed).add(worker) }\n}",
         "pubfn worker_outbox 1 worker usize \"&'static WorkerOutbox\" 1\n"
         "tail unsafe ref deref method method path OUTBOX_RING load 1 path Ordering::Relaxed add 1 path worker\n"),
        ("pub fn is_addr_alive(addr: ReplyAddr) -> bool {\n    let ob = worker_outbox(addr.worker as usize);\n    ob.epochs\n        .get(addr.slot as usize)\n        .is_none_or(|e| e.load(Ordering::Relaxed) == addr.epoch)\n}",
         "pubfn is_addr_alive 1 addr ReplyAddr bool 2\n"
         "let ob call worker_outbox 1 cast field path addr worker usize\n"
         "tail method method field path ob epochs get 1 cast field path addr slot usize\n"
         "is_none_or 1 closure 1 e bin eq method path e load 1 path Ordering::Relaxed field path addr epoch\n"),
    ],
}


def translate(relative, original, structured):
    replacements = REWRITES.get(relative, []) if structured else []
    segments = []
    for old, new in replacements:
        needle = old.encode()
        if original.count(needle) != 1:
            raise native.NativeError(f"pinned source differs at {relative}: {old.splitlines()[0]}")
        start = original.index(needle)
        segments.append((start, start + len(needle), b"native-" + new.encode()))
    parts, cursor, raw_bytes = [native.HEADER], 0, 0
    for start, end, body in sorted(segments):
        chunk = original[cursor:start]
        parts.extend([b"rust " + str(len(chunk)).encode() + b"\n", chunk, b"\n", body])
        raw_bytes += len(chunk)
        cursor = end
    chunk = original[cursor:]
    parts.extend([b"rust " + str(len(chunk)).encode() + b"\n", chunk])
    raw_bytes += len(chunk)
    result = b"".join(parts)
    native.parse_source(result)  # Validate before publishing; Cargo derives entry roles.
    return result, {"structured_functions": len(segments), "native_rust_payload_bytes": raw_bytes,
                    "original_rust_bytes": len(original), "fluxr_source_bytes": len(result)}


def prepare(reference, output, mode):
    if output.exists() or output.is_symlink():
        raise native.NativeError("output already exists")
    root = reference / SUBMISSION
    if not root.is_dir():
        raise native.NativeError("reference must contain the pinned submission path")
    output.parent.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix=".fluxr-port-", dir=output.parent))
    files, rows = [], []
    seen = set()
    try:
        for path in sorted(root.rglob("*")):
            if path.is_symlink():
                raise native.NativeError("reference contains a symlink")
            if not path.is_file():
                continue
            relative = path.relative_to(root).as_posix()
            if path.suffix != ".rs" and relative not in {"Cargo.toml", "Cargo.lock"}:
                continue
            original = native.read_regular(path)
            if PINNED_FILES.get(relative) != native.sha(original):
                raise native.NativeError(f"source differs from pinned revision: {relative}")
            seen.add(relative)
            source_name = relative[:-3] + ".fluxr" if path.suffix == ".rs" else relative
            target = stage / source_name
            target.parent.mkdir(parents=True, exist_ok=True)
            if path.suffix == ".rs":
                data, counts = translate(relative, original, mode == "structured")
                filemode = "source"
            else:
                data, counts, filemode = original, {}, "copy"
            target.write_bytes(data)
            files.append({"source": source_name, "output": relative, "mode": filemode, "executable": False})
            rows.append({"original_path": relative, "original_sha256": native.sha(original),
                         "source_path": source_name, "source_sha256": native.sha(data), **counts})
        if seen != set(PINNED_FILES):
            raise native.NativeError("pinned reference source is incomplete")
        license_bytes = native.read_regular(reference / "LICENSE")
        if native.sha(license_bytes) != LICENSE_SHA256:
            raise native.NativeError("upstream license differs from pinned revision")
        # The license is part of the source redistribution, not a build input.
        (stage / "LICENSE.upstream").write_bytes(license_bytes)
        config = {"format": "fluxr-project-v1", "rustflags": ["-C", "target-cpu=native"], "files": files}
        (stage / "fluxr.json").write_text(json.dumps(config, indent=2) + "\n")
        provenance = {"upstream": "https://github.com/arjaythedev/multicore-server-challenge", "revision": REVISION,
                      "submission": SUBMISSION, "mode": mode, "files": rows,
                      "scope": "Attributed reference port and capability fixture; no new leaderboard result or language-eligibility claim"}
        (stage / "IMPORT_PROVENANCE.json").write_text(json.dumps(provenance, indent=2) + "\n")
        if output.exists() or output.is_symlink():
            raise native.NativeError("output appeared during preparation")
        stage.rename(output)
    finally:
        if stage.exists():
            shutil.rmtree(stage)
    return provenance


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--mode", choices=["lossless", "structured"], default="structured")
    args = parser.parse_args()
    print(json.dumps(prepare(args.reference, args.output, args.mode), indent=2))


if __name__ == "__main__":
    main()
