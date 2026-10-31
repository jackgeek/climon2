#!/usr/bin/env python3
"""Fail closed if the pinned Handler surface and adapter signatures diverge."""

import json
from pathlib import Path
import re
import subprocess


ROOT = Path(__file__).resolve().parent
metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--locked", "--format-version", "1",
     "--manifest-path", str(ROOT / "probe/Cargo.toml")],
    text=True,
))
vte, = [package for package in metadata["packages"] if package["name"] == "vte"]
if vte["version"] != "0.15.0":
    raise SystemExit("FAIL: review the changed vte version before qualification")

upstream = (Path(vte["manifest_path"]).parent / "src/ansi.rs").read_text()
trait = upstream.split("pub trait Handler {", 1)[1].split("\n}", 1)[0]
adapter = (ROOT / "probe/src/lib.rs").read_text().split(
    "impl<T: EventListener> Handler for Observed<'_, T> {", 1
)[1]


def signatures(source):
    methods = {}
    for name, parameters in re.findall(r"^\s*fn (\w+)\(([^)]*)\)", source, re.M):
        types = []
        for parameter in parameters.split(","):
            parameter = parameter.strip()
            if not parameter or parameter == "&mut self":
                continue
            _, separator, type_name = parameter.partition(":")
            if not separator:
                raise SystemExit(f"FAIL: unsupported signature for {name}")
            types.append("".join(type_name.split()))
        if name in methods:
            raise SystemExit(f"FAIL: duplicate forwarding method {name}")
        methods[name] = tuple(types)
    if not methods:
        raise SystemExit("FAIL: no methods found; source scanner must be reviewed")
    return methods


expected = signatures(trait)
actual = signatures(adapter)
if expected != actual:
    missing = expected.keys() - actual.keys()
    extra = actual.keys() - expected.keys()
    changed = [
        name for name in expected.keys() & actual.keys()
        if expected[name] != actual[name]
    ]
    raise SystemExit(
        f"FAIL: Handler surface mismatch: missing={sorted(missing)}, "
        f"extra={sorted(extra)}, changed={sorted(changed)}"
    )
print(f"PASS: all {len(expected)} pinned vte Handler signatures forwarded explicitly")
