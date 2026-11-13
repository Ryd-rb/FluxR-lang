"""Audit existing FR1 codewords and FA1 spellings on the pinned tokenizer panel.

This measures text encoding only. A codeword that costs one token is not evidence
that an untrained language model understands the bit pattern it represents.
"""
from collections import Counter
import argparse
import gzip
from hashlib import sha256
import json
from pathlib import Path
import tomllib

from bench_heldout_tokens import codecs

ROOT = Path(__file__).resolve().parents[1]
ALPHABET = ROOT / "tools/fr1/o200k-fr1.txt.gz"
ALPHABET_SHA256 = "ad5ec04db8fd979a5a8c14c8329284fe27d7253173c268c076253ea212418ecd"
WORDS = "fa1 pub fn method inputs rec enum int uint float record list set put push pop if while break continue ret add sub mul div rem eq lt le and or not abs tof i64tof ftoi64 sqrt local input new field tag wrap is payload empty last at minidx call m".split()


def audit(cache):
    payload = gzip.decompress(ALPHABET.read_bytes())
    if sha256(payload).hexdigest() != ALPHABET_SHA256:
        raise ValueError("FR1 alphabet changed; review and bind the new alphabet first")
    alphabet = payload.decode("utf-8").splitlines()
    if len(alphabet) != 65536 or len(set(alphabet)) != 65536:
        raise ValueError("FR1 alphabet must contain exactly 65,536 distinct codewords")
    panel = tomllib.loads((ROOT / "benchmarks/token-panel.toml").read_text())
    results = []
    for name, family, count in codecs(panel, cache):
        measured = [count(word) for word in alphabet]
        sizes = Counter(measured)
        results.append({"name": name, "family": family,
            "isolated_codeword_token_histogram": dict(sorted(sizes.items())),
            "non_single_codeword_examples": [{"index": i, "text": alphabet[i], "tokens": n} for i, n in enumerate(measured) if n != 1][:5],
            "alphabet_in_recorded_order_tokens": count("".join(alphabet)),
            "fa1_keyword_tokens_with_leading_space": {word: count(" " + word) for word in WORDS}})
    return {"schema":"fluxr-agent-vocabulary-audit-v1", "alphabet_sha256":ALPHABET_SHA256,
            "codewords":len(alphabet), "uncompressed_bytes":len(payload), "gzip_bytes":ALPHABET.stat().st_size,
            "results":results, "panel":panel,
            "scope":"Literal tokenizer measurements. Isolated or concatenated codeword compactness does not establish semantic learnability, edit reliability, or inference speed. No model vocabulary is modified."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", type=Path, default=ROOT / "target/token-cache")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    report = audit(args.cache)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"codewords":report["codewords"], "histograms": {r["name"]: r["isolated_codeword_token_histogram"] for r in report["results"]}}, indent=2))


if __name__ == "__main__":
    main()
