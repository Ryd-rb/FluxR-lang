#!/usr/bin/env python3
"""Offline exact-string tokenization for WK1 development measurements.

Derived from research-v14/word_audit.py, SHA256
0b8558981cea916f9212a68e588240e02e3b87b26ae5e554e09efadea4c85c27.
The research helper is retained unchanged. This version shares its loader with
bench_wordcode_tokens.py, validates the complete inherited six-profile panel,
and never acquires assets or creates tokenizer caches.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
from decimal import Decimal
import gc
import hashlib
from importlib.metadata import PackageNotFoundError, version
import json
from pathlib import Path
import re
import stat
import sys
import time
import tomllib
from typing import Callable
from unittest.mock import patch

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
ORIGIN_SHA256 = "0b8558981cea916f9212a68e588240e02e3b87b26ae5e554e09efadea4c85c27"
DEPENDENCIES = {"tiktoken": "0.14.0", "tokenizers": "0.23.3"}
PINNED_PANEL = (
    ("o200k_base", "OpenAI", "https://openaipublic.blob.core.windows.net/encodings/o200k_base.tiktoken",
     "446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d"),
    ("cl100k_base", "OpenAI", "https://openaipublic.blob.core.windows.net/encodings/cl100k_base.tiktoken",
     "223921b76ee99bde995b7ff738513eef100fb51d18c93597a113bcffe865b2a7"),
    ("qwen", "Qwen", "https://huggingface.co/Qwen/Qwen2.5-Coder-7B-Instruct/resolve/c03e6d358207e414f1eca0bb1891e29f1db0e242/tokenizer.json",
     "c0382117ea329cdf097041132f6d735924b697924d6f6fc3945713e96ce87539"),
    ("starcoder", "StarCoder", "https://huggingface.co/bigcode/starcoder2-3b/resolve/733247c55e3f73af49ce8e9c7949bf14af205928/tokenizer.json",
     "6190a6649d90a7fe35bfed8bff990c3e7aafc3cfa5beea185045f03f0890402b"),
    ("mistral", "Mistral", "https://huggingface.co/mistralai/Mistral-7B-v0.1/resolve/27d67f1b5f57dc0953326b2601d68371d40ea8da/tokenizer.json",
     "11c08db21487c885d8c792180f0be237f6a261b89a46f128a6a80a3aa4bd1720"),
    ("olmo", "OLMo", "https://huggingface.co/allenai/OLMo-2-1124-7B/resolve/7df9a82518afdecae4e8c026b27adccc8c1f0032/tokenizer.json",
     "73fd5254624f39a88e3faac6a8e11300fc3c735ed37880d4f4f08db898eaecca"),
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key: " + key)
        result[key] = value
    return result


def reject_constant(text):
    raise ValueError("nonfinite JSON constant: " + text)


def parse_json(raw):
    return json.loads(raw, object_pairs_hook=unique_object, parse_constant=reject_constant)


def read_bounded(path: Path, limit: int) -> bytes:
    path = Path(path)
    before = path.stat()
    if not stat.S_ISREG(before.st_mode) or before.st_size > limit:
        raise ValueError(f"regular input of at most {limit} bytes required: {path}")
    with path.open("rb") as stream:
        raw = stream.read(limit + 1)
    after = path.stat()
    if (len(raw) > limit or len(raw) != before.st_size
            or (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns)
            != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns)):
        raise ValueError("input changed or exceeded bound while reading: " + str(path))
    return raw


def hash_file(path: Path, limit: int) -> dict:
    """Bounded-memory hashing for assets and external binary identity only."""
    path = Path(path)
    before = path.stat()
    if not stat.S_ISREG(before.st_mode) or before.st_size > limit:
        raise ValueError(f"regular input of at most {limit} bytes required: {path}")
    sha = hashlib.sha256()
    size = 0
    with path.open("rb") as stream:
        while block := stream.read(min(1024 * 1024, limit + 1 - size)):
            size += len(block)
            if size > limit:
                raise ValueError("input grew beyond hashing bound")
            sha.update(block)
    after = path.stat()
    if (size != before.st_size or (before.st_dev, before.st_ino, before.st_mtime_ns)
            != (after.st_dev, after.st_ino, after.st_mtime_ns)):
        raise ValueError("input changed while hashing: " + str(path))
    return {"bytes": size, "sha256": sha.hexdigest()}


def identity(text: str) -> dict:
    raw = text.encode("utf-8")
    return {"bytes": len(raw), "sha256": digest(raw)}


def validate_panel(panel: dict) -> None:
    if not isinstance(panel, dict):
        raise ValueError("tokenizer panel must be a TOML object")
    limit = panel.get("limit")
    if type(limit) not in (Decimal, float):
        raise ValueError("numeric 0.10 source-token target required")
    limit = Decimal(str(limit))
    if (type(panel.get("schema")) is not int or panel["schema"] != 1
            or type(panel.get("required_passes")) is not int or panel["required_passes"] != 4
            or not limit.is_finite() or limit != Decimal("0.10")
            or any(panel.get(name) != pinned for name, pinned in DEPENDENCIES.items())):
        raise ValueError("pinned dependency versions, schema, four-of-six gate and 0.10 target required")
    entries = panel.get("panel")
    if not isinstance(entries, list) or len(entries) != len(PINNED_PANEL):
        raise ValueError("complete six-tokenizer population required")
    for observed, expected in zip(entries, PINNED_PANEL, strict=True):
        if (not isinstance(observed, dict) or set(observed) != {"name", "family", "url", "sha256"}
                or tuple(observed[key] for key in ("name", "family", "url", "sha256")) != expected):
            raise ValueError("tokenizer identity, order, family, acquisition URL or asset hash changed")


def read_panel(path: Path) -> tuple[dict, bytes]:
    raw = read_bounded(path, 1024 * 1024)
    panel = tomllib.loads(raw.decode("utf-8"), parse_float=Decimal)
    validate_panel(panel)
    return panel, raw


def preflight_assets(panel: dict, cache: Path) -> list[dict]:
    """No loader runs until every local asset and package version passes."""
    validate_panel(panel)
    cache = cache.resolve(strict=True)
    if not cache.is_dir() or cache.is_relative_to(ROOT):
        raise ValueError("existing tokenizer cache outside verified checkout required")
    for dependency, expected in DEPENDENCIES.items():
        try:
            installed = version(dependency)
        except PackageNotFoundError as error:
            raise ValueError(f"missing pinned dependency: {dependency}=={expected}") from error
        if installed != expected:
            raise ValueError(f"require {dependency}=={expected}")
    assets = []
    for entry in panel["panel"]:
        name, url = entry["name"], entry["url"]
        filename = hashlib.sha1(url.encode()).hexdigest() if entry["family"] == "OpenAI" else name + ".json"
        path = (cache / filename).resolve(strict=True)
        if not path.is_relative_to(cache):
            raise ValueError("tokenizer asset escapes the supplied cache")
        observed = hash_file(path, 64 * 1024 * 1024)
        if observed["sha256"] != entry["sha256"]:
            raise ValueError("tokenizer asset hash mismatch: " + name)
        assets.append({**entry, "path": path, "bytes": observed["bytes"]})
    return assets


@dataclass
class LoadedCodec:
    encode: Callable[[str], list[int]]
    metadata: dict


def load_codec(asset: dict) -> LoadedCodec:
    """Load one verified in-memory asset; neither library receives a fetch path.

    The official tiktoken factory/parser is retained, but its acquisition hook is
    temporarily replaced by a reader accepting only this verified byte buffer.
    Call sequentially; this module is not a concurrent tokenizer service.
    """
    started = time.perf_counter_ns()
    raw = read_bounded(asset["path"], 64 * 1024 * 1024)
    if digest(raw) != asset["sha256"]:
        raise ValueError("tokenizer changed after preflight: " + asset["name"])
    if asset["family"] == "OpenAI":
        import tiktoken
        import tiktoken.load
        import tiktoken_ext.openai_public as official

        def local_bytes(url, expected_hash=None):
            if url != asset["url"] or expected_hash != asset["sha256"]:
                raise ValueError("offline tokenizer attempted unpinned asset access")
            return raw

        with patch.object(tiktoken.load, "read_file_cached", local_bytes):
            spec = official.ENCODING_CONSTRUCTORS[asset["name"]]()
        codec = tiktoken.Encoding(**spec)
        encode = codec.encode_ordinary
        implementation = {
            "regex_sha256": digest(codec._pat_str.encode()),
            "mergeable_ranks_count": len(codec._mergeable_ranks),
            "official_factory_sha256": digest(read_bounded(Path(official.__file__), 1024 * 1024)),
            "special_token_policy": "encode_ordinary; special-looking text remains ordinary input",
        }
    else:
        from tokenizers import Tokenizer
        codec = Tokenizer.from_str(raw.decode("utf-8"))
        encode = lambda text: codec.encode(text, add_special_tokens=False).ids
        document = parse_json(raw)
        implementation = {"normalizer": document.get("normalizer"),
                          "pre_tokenizer": document.get("pre_tokenizer"),
                          "special_token_policy": "add_special_tokens=False; no chat template"}
    return LoadedCodec(encode, {
        **{key: asset[key] for key in ("name", "family", "url", "sha256", "bytes")},
        "implementation": implementation, "load_ns": time.perf_counter_ns() - started,
        "offline": True, "custom_vocabulary": False,
    })


def count_text(encode: Callable, text: str, *, include_ids=False) -> dict:
    if not isinstance(text, str) or len(text.encode("utf-8")) > 2 * 1024 * 1024:
        raise ValueError("bounded UTF-8 measurement string required")
    ids = encode(text)  # The entire string is one encoding call.
    if (not isinstance(ids, list) or any(type(i) is not int or not 0 <= i < 2**32 for i in ids)):
        raise ValueError("tokenizer returned malformed token IDs")
    packed = b"".join(i.to_bytes(4, "little") for i in ids)
    result = {**identity(text), "tokens": len(ids),
              "token_ids_u32le_sha256": digest(packed)}
    if include_ids:
        result["token_ids"] = ids
    return result


def count_with_json(encode: Callable, text: str) -> dict:
    return {**count_text(encode, text), "json_quoted_string": count_text(
        encode, json.dumps(text, ensure_ascii=False, allow_nan=False))}


def output_path(path: Path) -> Path:
    path = path.resolve()
    if path.is_relative_to(ROOT):
        raise ValueError("measurement outputs must be outside the verified checkout")
    if path.exists():
        raise ValueError("refusing to overwrite existing evidence: " + str(path))
    return path


def write_report(path: Path, report: dict) -> None:
    path = output_path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(report, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def contexts(word: str) -> dict[str, str]:
    return {"bare": word, "leading_space": " " + word, "newline": "\n" + word,
            "double_space": "  " + word, "pair": word + " " + word,
            "operator_adjacency": word + "+" + word, "identifier_adjacency": "a" + word + "b",
            "numeric_arguments": word + " 1 -2 3u", "numeric_adjacency": word + "1",
            "json_string": json.dumps(word + "\n", ensure_ascii=False),
            "json_packet": json.dumps({"source": word + " " + word + "\n"},
                                      ensure_ascii=False, separators=(",", ":"))}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--panel", type=Path, default=ROOT / "benchmarks/token-panel.toml")
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--words", type=Path, required=True)
    parser.add_argument("--texts", type=Path,
                        help="optional bounded array of {id,path,role}; paths relative to this manifest")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    output = output_path(args.output)
    started = time.perf_counter_ns()
    panel, panel_raw = read_panel(args.panel)
    words_raw = read_bounded(args.words, 1024 * 1024)
    word_document = parse_json(words_raw)
    if not isinstance(word_document, dict):
        raise ValueError("word candidate document must be an object")
    words = word_document.get("words")
    if (not isinstance(words, list) or not 1 <= len(words) <= 256
            or any(not isinstance(w, str) or re.fullmatch(r"[a-z]{1,32}", w) is None for w in words)
            or len(words) != len(set(words))):
        raise ValueError("require 1..256 distinct lowercase ASCII candidate words")
    texts = []
    if args.texts:
        manifest = parse_json(read_bounded(args.texts, 1024 * 1024))
        if not isinstance(manifest, list) or len(manifest) > 128:
            raise ValueError("at most 128 text records required")
        ids, size = set(), 0
        for entry in manifest:
            if (not isinstance(entry, dict) or set(entry) != {"id", "path", "role"}
                    or any(not isinstance(value, str) or not value for value in entry.values())
                    or entry["id"] in ids):
                raise ValueError("malformed or duplicate text identity")
            ids.add(entry["id"])
            raw = read_bounded((args.texts.parent / entry["path"]).resolve(strict=True), 512 * 1024)
            size += len(raw)
            if size > 2 * 1024 * 1024:
                raise ValueError("text manifest exceeds aggregate 2 MiB")
            texts.append((entry, raw.decode("utf-8")))
    assets = preflight_assets(panel, args.cache)
    report = {"schema": "fluxr-word-audit-v14-v2", "profiles": [],
              "panel_sha256": digest(panel_raw), "words_sha256": digest(words_raw),
              "tool_sha256": digest(read_bounded(Path(__file__), 1024 * 1024)),
              "research_helper_origin_sha256": ORIGIN_SHA256, "dependencies": DEPENDENCIES,
              "classification": "static development spelling audit; no model generation or billing",
              "most_models_claim": False, "model_trials": 0, "source_limit_retained": "1/10",
              "encoding_policy": "ordinary full strings; no added special tokens or chat template",
              "newline_policy": "exact UTF-8 bytes, including CRLF and terminal newlines",
              "word_count": len(words), "context_count": len(contexts(words[0]))}
    for asset in assets:
        codec = load_codec(asset)
        start = time.perf_counter_ns()
        rows = {word: {key: {"text": text, **count_text(codec.encode, text, include_ids=True)}
                       for key, text in contexts(word).items()} for word in words}
        report["profiles"].append({
            "tokenizer": codec.metadata, "words": rows,
            "complete_texts": [{**entry, "text": text, **count_text(codec.encode, text)}
                               for entry, text in texts],
            "bare_one_token_words": [w for w in words if rows[w]["bare"]["tokens"] == 1],
            "leading_space_one_token_words": [w for w in words if rows[w]["leading_space"]["tokens"] == 1],
            "encode_ns": time.perf_counter_ns() - start,
        })
        del codec
        gc.collect()
    report["elapsed_ns"] = time.perf_counter_ns() - started
    write_report(output, report)
    print(json.dumps({"output": str(output), "profiles": 6, "words": len(words),
                      "most_models_claim": False}))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, KeyError, TypeError, UnicodeError) as error:
        print(json.dumps({"status": "refused", "error": str(error)}), file=sys.stderr)
        raise SystemExit(1)
