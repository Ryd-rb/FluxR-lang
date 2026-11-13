#!/usr/bin/env python3
"""Run the pinned official 197-step suite on real PostgreSQL 18.

Creates a private temporary database cluster and seed copy. It never connects to
an existing database. Full seed is the default; --dev is an explicitly different
qualification. Run under the shared resource coordinator. This is conformance,
not the official eight-core score or a safety sandbox for unknown servers.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import socket
import subprocess
import time
import urllib.error
import urllib.request

import fluxr_native as native
from unified_tests.live_probe import LiveProbe

REFERENCE_MANIFEST = Path(__file__).resolve().parents[1] / "benchmarks/multicore-reference.json"


def invalid_json_constant(value):
    raise ValueError(f"non-JSON numeric constant: {value}")


def check_reference(reference: Path):
    manifest = json.loads(native.read_regular(REFERENCE_MANIFEST))
    if (manifest.get("format") != "fluxr-pinned-multicore-reference-v1"
            or manifest.get("revision") != "6145e7291a3625c184a58b63b86ed9d1f7304700"):
        raise ValueError("unsupported pinned reference manifest")
    expected = manifest["files"]
    actual = {}
    for path in sorted(reference.rglob("*")):
        if path.is_symlink():
            raise ValueError("reference contains symlink")
        if path.is_file():
            actual[path.relative_to(reference).as_posix()] = digest(path)
    if actual != expected:
        raise ValueError("reference files differ from pinned challenge source")
    return actual


def validate_conformance_result(stdout: str, command: dict, transcript: Path):
    """Fail closed on partial runs, timeout-success tricks, and missing evidence."""
    if (type(command.get("exit_code")) is not int or command["exit_code"] != 0
            or command.get("timed_out") is not False):
        raise ValueError("conformance command failed or timed out")
    counts = re.findall(r"(?m)^(\d+) steps, (\d+) passed, (\d+) failed \([0-9.]+s\)$", stdout)
    if counts != [("197", "197", "0")] or stdout.splitlines().count("RESULT: PASS") != 1 or "RESULT: FAIL" in stdout:
        raise ValueError("exact 197/197/0 conformance summary required")
    data = native.read_regular(transcript, 16 * 1024 * 1024)
    lines = data.splitlines()
    if len(lines) != 197:
        raise ValueError("exactly197 transcript records required")
    steps = set()
    for line in lines:
        row = json.loads(line, object_pairs_hook=native.no_duplicates, parse_constant=invalid_json_constant)
        if (not isinstance(row, dict) or set(row) != {"step", "method", "path", "status", "body"}
                or not isinstance(row["step"], str) or not row["step"] or row["step"] == "ABORTED"
                or row["step"] in steps or not isinstance(row["method"], str)
                or not isinstance(row["path"], str) or type(row["status"]) is not int
                or not 100 <= row["status"] <= 599):
            raise ValueError("invalid/duplicate conformance transcript record")
        steps.add(row["step"])
    return {"steps": [197, 197, 0], "transcript_records": 197, "transcript_sha256": native.sha(data)}


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def terminate(process):
    if process is None:
        return None
    before = process.poll()
    signalled = False
    # All child commands are launched in sessions we own. Signal the group even
    # if the direct leader has exited; the shared guard also audits descendants.
    try:
        os.killpg(process.pid, signal.SIGTERM)
        signalled = True
    except ProcessLookupError:
        pass
    if before is None:
        try:
            process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=10)
    return {"exit_before_shutdown": before, "group_signalled": signalled, "exit_code": process.poll()}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--server", action="append", required=True, help="LABEL=/absolute/server-binary")
    parser.add_argument("--work", type=Path, required=True, help="new disposable directory")
    parser.add_argument("--output", type=Path, required=True, help="new retained evidence directory")
    parser.add_argument("--dev", action="store_true")
    parser.add_argument("--runtime-server", action="append", default=[], help="LABEL requiring concurrent in-binary VM injection and HTTP traffic")
    parser.add_argument("--seed-timeout", type=int, default=1200)
    args = parser.parse_args(argv)
    reference, work, output = (p.resolve() for p in (args.reference, args.work, args.output))
    reference_files = check_reference(reference)
    qualifier_sha = digest(Path(__file__))
    servers = []
    for item in args.server:
        label, sep, value = item.partition("=")
        if not sep or not re.fullmatch(r"[a-z][a-z0-9_-]{0,30}", label) or any(row[0] == label for row in servers):
            parser.error("server needs a unique simple label and path")
        path = Path(value).resolve(strict=True)
        servers.append((label, path, digest(path)))
    if len(set(args.runtime_server)) != len(args.runtime_server) or not set(args.runtime_server) <= {row[0] for row in servers}:
        parser.error("--runtime-server must name unique declared server labels")
    if work.exists() or output.exists() or work.is_relative_to(reference) or output.is_relative_to(reference):
        parser.error("work/output must be new directories outside the reference")
    work.mkdir(parents=True)
    output.mkdir(parents=True)
    pg_bin = Path(os.environ.get("PG_BIN", "/usr/lib/postgresql/18/bin"))
    env = os.environ.copy()
    env.update({"PGHOST": "127.0.0.1", "PGPORT": str(free_port()), "PGUSER": "fluxrtest", "PGDATABASE": "postgres"})
    # This is a hash-verified source snapshot, not a Git checkout. Go must not
    # discover unrelated parent-workspace VCS metadata for binary stamping.
    env["GOFLAGS"] = "-p=2 -buildvcs=false"
    for key in ["PGSERVICE", "PGSERVICEFILE", "PGOPTIONS", "PGPASSWORD"]:
        env.pop(key, None)
    rows, transcript_hashes = [], {}
    report = {"format": "fluxr-multicore-conformance-v1", "passed": False,
              "scope": "official API conformance on local PostgreSQL; no leaderboard or speed claim",
              "seed": "dev-0.02" if args.dev else "full-1.0", "commands": rows, "servers": [],
              "go_flags": env["GOFLAGS"],
              "go_vcs_scope": "VCS stamping disabled for the pinned detached snapshot; official Go source unchanged",
              "reference_files": reference_files, "qualifier_sha256_before": qualifier_sha,
              "reference_manifest_sha256": digest(REFERENCE_MANIFEST),
              "started_at_unix": time.time()}
    probe_source = Path(__file__).resolve().parent / "unified_tests/live_probe.py"
    probe_sha = digest(probe_source)
    report.update(runtime_servers=args.runtime_server, runtime_probe_sha256_before=probe_sha)
    pg, app, pg_log, app_log, live_probe = None, None, None, None, None

    def save():
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")

    def run(label, command, cwd=work, timeout=300, extra=None, allow_failure=False):
        command = [str(x) for x in command]
        start = time.monotonic()
        stdout_path, stderr_path = output / f"{label}.stdout.log", output / f"{label}.stderr.log"
        command_env = env | (extra or {})
        with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
            process = subprocess.Popen(command, cwd=cwd, env=command_env, stdout=stdout, stderr=stderr, start_new_session=True)
            timed_out = False
            try:
                code = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
                terminate(process)
                code = process.returncode
            except BaseException:
                terminate(process)
                raise
        row = {"name": label, "argv": command, "cwd": str(cwd), "exit_code": code,
               "timed_out": timed_out, "wall_seconds": time.monotonic() - start,
               "stdout": stdout_path.name, "stderr": stderr_path.name,
               "stdout_sha256": digest(stdout_path), "stderr_sha256": digest(stderr_path)}
        rows.append(row)
        save()
        print(f"{label}: exit={code} elapsed={row['wall_seconds']:.2f}s", flush=True)
        if (code != 0 or timed_out) and not allow_failure:
            raise RuntimeError(f"{label} failed; inspect retained logs")
        return stdout_path.read_text(), row

    psql = [pg_bin / "psql", "-X", "-v", "ON_ERROR_STOP=1", "-At"]
    try:
        versions = {}
        for tool, command in {"postgres": [pg_bin / "postgres", "--version"], "go": ["go", "version"],
                              "rustc": ["rustc", "-vV"], "cargo": ["cargo", "--version"]}.items():
            versions[tool] = run("version-" + tool, command)[0].strip()
        report["toolchains"] = versions
        if not re.search(r"PostgreSQL\) 18\.", versions["postgres"]):
            raise RuntimeError("PostgreSQL major18 required")
        snapshot = work / "reference"
        shutil.copytree(reference, snapshot)
        check_reference(snapshot)
        cluster = work / "cluster"
        command = [pg_bin / "initdb", "-D", cluster, "-A", "trust", "-U", "fluxrtest", "--no-locale", "--encoding=UTF8"]
        if os.environ.get("PG_SHARE"):
            command += ["-L", os.environ["PG_SHARE"]]
        run("initdb", command)
        pg_log = (output / "postgres.log").open("wb")
        pg_command = [str(pg_bin / "postgres"), "-D", str(cluster), "-h", "127.0.0.1", "-p", env["PGPORT"],
                      "-c", "unix_socket_directories=", "-c", "jit=off", "-c", "fsync=on", "-c", "synchronous_commit=on"]
        pg = subprocess.Popen(pg_command, env=env, stdout=pg_log, stderr=pg_log, start_new_session=True)
        report["postgres_argv"] = pg_command
        for _ in range(100):
            if pg.poll() is not None:
                raise RuntimeError("PostgreSQL failed to start")
            ready = subprocess.run([str(pg_bin / "pg_isready")], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0
            if ready:
                break
            time.sleep(.1)
        else:
            raise RuntimeError("PostgreSQL startup timeout")
        config, _ = run("postgres-config", psql + ["-c", "SELECT version(); SHOW fsync; SHOW synchronous_commit; SHOW max_connections;"])
        report["postgres_configuration"] = config
        seed = ["bash", "seed/make-seed.sh"] + (["--dev"] if args.dev else [])
        run("official-seed", seed, cwd=snapshot, timeout=args.seed_timeout, extra={"SEED_DB": "fn1_seed"})
        # Exercise the application's actual password-authentication path. Keep
        # the private fixture administrator on local trust, but require SCRAM
        # for the challenge role created by the unmodified seed script.
        hba = cluster / "pg_hba.conf"
        hba.write_text("host all conduit 127.0.0.1/32 scram-sha-256\n" + hba.read_text())
        run("reload-scram-policy", psql + ["-c", "SELECT pg_reload_conf();"])
        authentication, _ = run("authentication-policy", psql + ["-c",
            "SELECT auth_method FROM pg_hba_file_rules WHERE user_name=ARRAY['conduit']; SELECT split_part(rolpassword,'$',1) FROM pg_authid WHERE rolname='conduit';"])
        if "scram-sha-256" not in authentication or "SCRAM-SHA-256" not in authentication:
            raise RuntimeError("challenge role SCRAM authentication policy not established")
        report["application_authentication"] = "PostgreSQL SCRAM-SHA-256 with fixture conduit password"
        rejected = False
        for attempt in range(10):
            _, auth_command = run(f"scram-reject-{attempt}", psql + ["-U", "conduit", "-d", "fn1_seed", "-c", "SELECT 1"],
                                  extra={"PGPASSWORD": "deliberately-wrong-fixture-password"}, allow_failure=True)
            diagnostic = (output / auth_command["stderr"]).read_text()
            if auth_command["exit_code"] != 0 and not auth_command["timed_out"] and "password authentication failed" in diagnostic:
                rejected = True
                break
            if auth_command["exit_code"] != 0:
                raise RuntimeError("unexpected error during SCRAM rejection probe")
            time.sleep(.1)
        if not rejected:
            raise RuntimeError("wrong PostgreSQL password was not rejected")
        run("scram-accept", psql + ["-U", "conduit", "-d", "fn1_seed", "-c", "SELECT 1"], extra={"PGPASSWORD": "conduit"})
        report["scram_password_controls_passed"] = True
        counts, _ = run("seed-counts", psql + ["-d", "fn1_seed", "-c",
            "SELECT 'users',count(*) FROM users UNION ALL SELECT 'articles',count(*) FROM articles UNION ALL SELECT 'favorites',count(*) FROM favorites UNION ALL SELECT 'comments',count(*) FROM comments UNION ALL SELECT 'follows',count(*) FROM follows;"])
        report["seed_counts"] = counts
        conformance = work / "conformance"
        run("build-official-conformance", ["go", "build", "-o", conformance, "./cmd/conformance"], cwd=snapshot / "bench")
        report["conformance_binary_sha256"] = digest(conformance)
        for label, binary, before in servers:
            database = "fn1_" + label.replace("-", "_")
            run(f"create-{label}", psql + ["-c", f"CREATE DATABASE {database} TEMPLATE fn1_seed STRATEGY FILE_COPY"])
            port = free_port()
            server_env = env | {"DATABASE_URL": f"postgres://conduit:conduit@127.0.0.1:{env['PGPORT']}/{database}",
                                "JWT_SECRET": "multicore-server-challenge", "HOST": "127.0.0.1", "PORT": str(port)}
            if label in args.runtime_server:
                # Execution needs neither a compiler nor Python. The harness
                # retains its normal environment for PostgreSQL and Go tools.
                compiler_free = work / (label + "-empty-path")
                compiler_free.mkdir()
                for key in list(server_env):
                    if key.startswith(("RUST", "CARGO", "FLUXR_CONTROL_")):
                        server_env.pop(key)
                token = secrets.token_hex(32)
                discovery = work / (label + "-runtime.json")
                server_env.update(PATH=str(compiler_free), FLUXR_CONTROL_ADDR="127.0.0.1:0",
                                  FLUXR_CONTROL_TOKEN=token, FLUXR_CONTROL_FILE=str(discovery))
            app_log = (output / f"{label}-server.log").open("wb")
            app = subprocess.Popen([str(binary)], env=server_env, stdout=app_log, stderr=app_log, start_new_session=True)
            base = f"http://127.0.0.1:{port}"
            for _ in range(600):
                if app.poll() is not None:
                    raise RuntimeError(f"{label} exited during startup")
                try:
                    with urllib.request.urlopen(base + "/api/tags", timeout=1) as response:
                        ready = response.status == 200
                    if ready:
                        break
                except (OSError, urllib.error.URLError):
                    pass
                time.sleep(.1)
            else:
                raise RuntimeError(f"{label} HTTP readiness timeout")
            transcript = output / f"{label}-transcript.jsonl"
            if label in args.runtime_server:
                live_probe = LiveProbe(discovery, token, app, base).start()
                live_probe.mark_conformance_start()
            result, row = run(f"conformance-{label}", [conformance, "-base", base, "-secret", server_env["JWT_SECRET"],
                                                     "-transcript", transcript, "-q"], timeout=180, allow_failure=True)
            runtime_observation = live_probe.finish() if live_probe else None
            live_probe = None
            if runtime_observation is not None:
                runtime_observation["compiler_free_path"] = str(compiler_free)
                runtime_observation["compiler_free_path_empty"] = not any(compiler_free.iterdir())
                (output / f"{label}-runtime-probe.json").write_text(json.dumps(runtime_observation, indent=2) + "\n")
                if not runtime_observation["passed"]:
                    raise RuntimeError(f"{label} in-binary VM/HTTP concurrency probe failed")
            observation = validate_conformance_result(result, row, transcript)
            if app.poll() is not None:
                raise RuntimeError(f"{label} crashed before intentional shutdown")
            connections, _ = run(f"connections-{label}", psql + ["-c", f"SELECT count(*) FROM pg_stat_activity WHERE datname='{database}' AND usename='conduit'"])
            count = int(connections.strip())
            entry = {"label": label, "binary_sha256": before, "unchanged_binary": digest(binary) == before,
                     **observation, "passed": True,
                     "observed_idle_database_connections": count,
                     "connection_scope": "post-conformance observation only, not peak under official load"}
            if runtime_observation is not None:
                entry["runtime_probe"] = runtime_observation
            transcript_hashes[label] = entry["transcript_sha256"]
            report["servers"].append(entry)
            entry["shutdown"] = terminate(app)
            if entry["shutdown"]["exit_before_shutdown"] is not None:
                raise RuntimeError(f"{label} exited unexpectedly after conformance")
            app = None
            app_log.close()
            app_log = None
            run(f"drop-{label}", psql + ["-c", f"DROP DATABASE {database} WITH (FORCE)"])
            if not entry["unchanged_binary"] or count > 95:
                raise RuntimeError(f"{label} conformance/integrity/connection check failed")
        report["transcript_equality"] = len(set(transcript_hashes.values())) == 1 if len(transcript_hashes) > 1 else None
        report["reference_unchanged"] = check_reference(reference) == reference_files
        report["snapshot_unchanged"] = check_reference(snapshot) == reference_files
        report["qualifier_sha256_after"] = digest(Path(__file__))
        report["runtime_probe_sha256_after"] = digest(probe_source)
        report["passed"] = (all(row["passed"] for row in report["servers"])
                            and len(report["servers"]) == len(servers)
                            and len(transcript_hashes) == len(servers)
                            and report["transcript_equality"] is not False
                            and report["reference_unchanged"] and report["snapshot_unchanged"]
                            and report["qualifier_sha256_after"] == qualifier_sha
                            and report["runtime_probe_sha256_after"] == probe_sha
                            and all(entry.get("runtime_probe", {}).get("passed") is True
                                    for entry in report["servers"] if entry["label"] in args.runtime_server))
    except BaseException as error:
        report["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        if live_probe:
            report["interrupted_runtime_probe"] = live_probe.finish()
        app_shutdown = terminate(app)
        pg_shutdown = terminate(pg)
        if app_log:
            app_log.close()
        if pg_log:
            pg_log.close()
        report["cleanup"] = {"app": app_shutdown, "postgres": pg_shutdown,
                             "session_leaders_reaped": (app is None or app.poll() is not None) and (pg is None or pg.poll() is not None),
                             "descendant_scope": "Owned process groups signalled; shared guard survivor receipt is authoritative for process-tree audit"}
        if (not report["cleanup"]["session_leaders_reaped"]
                or (pg_shutdown is not None and pg_shutdown["exit_before_shutdown"] is not None)):
            report["passed"] = False
        report["finished_at_unix"] = time.time()
        save()
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
