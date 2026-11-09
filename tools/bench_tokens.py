"""Count equivalent FluxR and Rust benchmark text payloads with o200k_base."""

import argparse
from importlib.metadata import version
from pathlib import Path
import tomllib


ROOT = Path(__file__).resolve().parents[1]


def main():
    cases = tomllib.loads((ROOT / "benchmarks/cases.toml").read_text(encoding="utf-8"))["case"]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cases", nargs="*", metavar="case", help="case names; defaults to all")
    args = parser.parse_args()
    for name in args.cases:
        if name not in cases:
            parser.error(f"unknown case {name!r}; available: {', '.join(cases)}")

    import tiktoken

    tokenizer = tiktoken.get_encoding("o200k_base")
    print(f"{tokenizer.name}, tiktoken {version('tiktoken')}, UTF-8 with LF line endings")

    def count(path):
        text = (ROOT / path).read_text(encoding="utf-8")
        return len(tokenizer.encode_ordinary(text)), len(text.encode("utf-8"))

    def show(label, size):
        print(f"  {label}: {size[0]} tokens, {size[1]} bytes")

    def compare(label, fluxr, rust):
        fluxr_size, rust_size = count(fluxr), count(rust)
        show(f"{label} FluxR", fluxr_size)
        show(f"{label} Rust", rust_size)
        if rust_size[0]:
            print(f"  FluxR/Rust: {fluxr_size[0] / rust_size[0]:.1%}")

    for name in args.cases or cases:
        case = cases[name]
        print(name)
        output = case.get("output")
        compare("source input" if output else "whole-file input/output", case["fluxr"], case["rust"])
        if output:
            compare("authored output", output["fluxr"], output["rust"])
        for key, label in (
            ("host", "Rust host binding"),
            ("generated", "generated native Rust"),
            ("context", "shared task context"),
        ):
            if key in case:
                show(label, count(case[key]))
        if "performance" in case:
            print(f"  Runtime: {case['performance']}")


if __name__ == "__main__":
    main()
