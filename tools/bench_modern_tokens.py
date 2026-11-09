"""Count the authored bodies used by bench_modern. Python 3.11+."""
import argparse
from hashlib import sha256
from importlib.metadata import version
from pathlib import Path
import tomllib

import tiktoken
from bench_tokens import source


def sizes(count, source):
    return f"{count(source)}t/{len(source.encode('utf-8'))}B"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tokenizer", action="append", type=Path, default=[],
                        help="optional Hugging Face tokenizer.json file")
    parser.add_argument("--counter-compact", type=Path,
                        help="check counter source freshly emitted by migrate-structured")
    args = parser.parse_args()
    print(f"tiktoken {version('tiktoken')}; UTF-8/LF; no special tokens")
    encodings = []
    for name in ("o200k_base", "cl100k_base"):
        codec = tiktoken.get_encoding(name)
        encodings.append((name, lambda s, codec=codec: len(codec.encode_ordinary(s))))
    for path in args.tokenizer:
        from tokenizers import Tokenizer
        codec = Tokenizer.from_file(str(path))
        print(f"{path.name} SHA-256 {sha256(path.read_bytes()).hexdigest()}; tokenizers {version('tokenizers')}")
        encodings.append((path.stem, lambda s, codec=codec: len(codec.encode(s, add_special_tokens=False).ids)))
    manifest = tomllib.loads(source("benchmarks/cases.toml"))["case"]
    if args.counter_compact:
        compact = args.counter_compact.read_text(encoding="utf-8")
        if compact != source(manifest["modern_counter"]["fluxr"]):
            parser.error("migrated counter does not match the frozen case")
    host = source("examples/bench_modern.rs", "fn leaf")
    timeline_v1 = "v1 1000" + "".join(f"\n{i} 1 1 0:{i} 500:{i+500} 1000:{i}" for i in range(128))
    for tokenizer, count in encodings:
        print(tokenizer)
        for name, case in manifest.items():
            if not name.startswith("modern_"):
                continue
            flux = source(case["fluxr"])
            rust = source(case["rust"], case["rust_item"])
            old = source(case["legacy"]) if "legacy" in case else None
            if name == "modern_timeline":
                old = timeline_v1
            ratio = count(flux) / count(rust)
            legacy = f", legacy {sizes(count, old)}, retained {count(flux) / count(old):.2%}" if old else ""
            print(f"  {name}: FluxR {sizes(count, flux)}, Rust {sizes(count, rust)}, {ratio:.1%}{legacy}")
        print(f"  scalar invocation helper only: {sizes(count, host)}")
        print(f"  task context: no model prompt executed; host bindings excluded from source ratios")


if __name__ == "__main__":
    main()
