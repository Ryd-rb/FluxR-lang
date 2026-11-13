#!/usr/bin/env python3
"""Compatibility command name for the single FluxR 1 compiler.

The historical FN1 source reader survives only as ``import-native``. Emission,
project export and builds use tools/fluxr.py and always retain the Rust Runtime.
There is no independent native-only compiler or execution profile here.
"""
from fluxr import *  # Compatibility for importers; one implementation owns behavior.

if __name__ == "__main__":
    raise SystemExit(main())
