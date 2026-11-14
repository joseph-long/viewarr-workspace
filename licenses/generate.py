#!/usr/bin/env python3
"""Write the third-party license files that ship with every viewarr build.

Every release artifact carries compiled copies of a few hundred Rust crates --
the WebAssembly viewer inside both Python wheels, the native app in the macOS
zip and Linux tarballs -- and their licenses (MIT, Apache-2.0, BSD, the fonts'
OFL, ...) require their notices to travel with those copies. This collects
them from `cargo metadata` and the crates' own sources:

    viewarr/THIRD_PARTY_LICENSES.txt         the WebAssembly build (both wheels)
    viewarr-native/THIRD_PARTY_LICENSES.txt  the native app (macOS and Linux)

Material that did not arrive as a crate (the colormap tables, the Phosphor
icon font) is added from licenses/extra/.

    python3 licenses/generate.py          regenerate both files
    python3 licenses/generate.py --check  fail if either is out of date

Uses only the standard library, so it runs anywhere cargo does.
"""

import argparse
import json
import re
import subprocess
import textwrap
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HERE = ROOT / "licenses"

# (output, manifest, targets whose dependency graphs are unioned, description)
PRODUCTS = [
    (
        ROOT / "viewarr" / "THIRD_PARTY_LICENSES.txt",
        ROOT / "viewarr" / "Cargo.toml",
        ["wasm32-unknown-unknown"],
        "the viewarr viewer's WebAssembly build, as shipped in the pyviewarr "
        "and fitsview Python packages",
    ),
    (
        ROOT / "viewarr-native" / "THIRD_PARTY_LICENSES.txt",
        ROOT / "viewarr-native" / "Cargo.toml",
        [
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
        ],
        "the viewarr desktop app, as shipped for macOS and Linux",
    ),
]

# When a crate offers a choice, take the first of these it allows. MIT leads
# because it is the shortest text that satisfies the obligation.
PREFERENCE = [
    "MIT",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Zlib",
    "0BSD",
    "Unlicense",
    "CC0-1.0",
    "BSL-1.0",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "Unicode-3.0",
    "OFL-1.1",
    "Ubuntu-font-1.0",
]

# Files a crate needs shipped beyond the ones at its top level, always
# included. Relative to the crate, or to licenses/ for "upstream/..." paths.
EXTRA_CRATE_FILES = {
    "epaint_default_fonts": [
        "fonts/emoji-icon-font-mit-license.txt",
        "fonts/Hack-Regular.txt",
        "fonts/OFL.txt",
        "fonts/UFL.txt",
    ],
}

# AccessKit is largely derived from Chromium and says so in its README, so
# Chromium's BSD license travels with every one of its crates.
ACCESSKIT = [
    "accesskit", "accesskit_atspi_common", "accesskit_consumer",
    "accesskit_macos", "accesskit_unix", "accesskit_windows", "accesskit_winit",
]
for _crate in ACCESSKIT:
    EXTRA_CRATE_FILES.setdefault(_crate, []).append("upstream/accesskit-LICENSE.chromium")
# Declared CC0 on crates.io, but its repository's own license file is this.
EXTRA_CRATE_FILES["hexf-parse"] = ["upstream/hexf-LICENSE"]

# Public-domain dedications ask for no notice, so a crate under one of these
# that ships no text needs nothing reproduced.
NO_NOTICE = {"CC0-1.0", "Unlicense", "0BSD"}

# Crates published to crates.io without their license files. These are the
# files from each one's upstream repository, kept in licenses/upstream/, and
# offered as candidates for the licenses the crate declares.
_EGUI = [
    "ecolor", "eframe", "egui", "egui-wgpu", "egui-winit", "egui_glow",
    "emath", "epaint", "epaint_default_fonts",
]
_OBJC2 = [
    "block2", "dispatch2", "objc-sys", "objc2", "objc2-app-kit",
    "objc2-core-data", "objc2-core-foundation", "objc2-core-graphics",
    "objc2-core-image", "objc2-core-services", "objc2-encode",
    "objc2-foundation", "objc2-io-surface", "objc2-metal",
    "objc2-quartz-core", "objc2-security",
]
UPSTREAM_TEXTS = {
    **{c: ["egui-LICENSE-MIT"] for c in _EGUI},
    **{c: ["accesskit-LICENSE-MIT", "accesskit-LICENSE-APACHE"] for c in ACCESSKIT},
    **{c: ["objc2-LICENSE-MIT"] for c in _OBJC2},
    "atspi-proxies": ["atspi-LICENSE-MIT"],
    "malloc_buf": ["malloc_buf-LICENSE"],
    "gpu-alloc": ["gpu-alloc-LICENSE-MIT"],
    "gpu-alloc-types": ["gpu-alloc-LICENSE-MIT"],
    "gpu-descriptor": ["gpu-descriptor-LICENSE-MIT"],
    "gpu-descriptor-types": ["gpu-descriptor-LICENSE-MIT"],
    "profiling": ["profiling-LICENSE-MIT"],
    "spirv": ["rspirv-LICENSE"],
}

LICENSE_FILE = re.compile(r"^(licen[cs]e|copying|copyright|notice|unlicense)", re.I)


# ---------------------------------------------------------------------------
# SPDX expressions
# ---------------------------------------------------------------------------


def parse_spdx(expr):
    """Parse an SPDX expression into nested ("or"|"and", [...]) / id tuples."""
    # Older crates write "MIT/Apache-2.0" for a choice.
    expr = expr.replace("/", " OR ")
    tokens = re.findall(r"\(|\)|[^\s()]+", expr)
    pos = 0

    def peek():
        return tokens[pos] if pos < len(tokens) else None

    def take():
        nonlocal pos
        pos += 1
        return tokens[pos - 1]

    def atom():
        if peek() == "(":
            take()
            node = disjunction()
            assert take() == ")", expr
            return node
        ident = take()
        if peek() == "WITH":
            take()
            ident = f"{ident} WITH {take()}"
        return ident

    def conjunction():
        parts = [atom()]
        while peek() == "AND":
            take()
            parts.append(atom())
        return parts[0] if len(parts) == 1 else ("and", parts)

    def disjunction():
        parts = [conjunction()]
        while peek() == "OR":
            take()
            parts.append(conjunction())
        return parts[0] if len(parts) == 1 else ("or", parts)

    node = disjunction()
    assert pos == len(tokens), expr
    return node


def rank(ident):
    return PREFERENCE.index(ident) if ident in PREFERENCE else len(PREFERENCE)


def choose(node):
    """The licenses to comply with: one branch of each OR, all of each AND."""
    if isinstance(node, str):
        return [node]
    op, parts = node
    if op == "and":
        return [ident for part in parts for ident in choose(part)]
    options = [choose(part) for part in parts]
    return min(options, key=lambda ids: (max(rank(i) for i in ids), len(ids)))


# ---------------------------------------------------------------------------
# License texts
# ---------------------------------------------------------------------------


def classify(path, text):
    """Which licenses a crate's license file holds, judged by name, then text."""
    name = path.name.lower()
    if "notice" in name:
        return {"NOTICE"}
    by_name = {
        "mit": "MIT",
        "apache": "Apache-2.0",
        "bsd": "BSD",
        "zlib": "Zlib",
        "unicode": "Unicode-3.0",
        "unlicense": "Unlicense",
        "isc": "ISC",
        "boost": "BSL-1.0",
        "cc0": "CC0-1.0",
        "ofl": "OFL-1.1",
        "ufl": "Ubuntu-font-1.0",
    }
    found = {spdx for key, spdx in by_name.items() if key in name}
    if found:
        return found

    sniffs = {
        "MIT": "Permission is hereby granted, free of charge",
        "Apache-2.0": "Apache License",
        "BSD": "Redistribution and use in source and binary forms",
        "Zlib": "This software is provided 'as-is'",
        "Unicode-3.0": "UNICODE LICENSE",
        "Unlicense": "This is free and unencumbered software",
        "ISC": "Permission to use, copy, modify, and/or distribute",
        "BSL-1.0": "Boost Software License",
        "CC0-1.0": "CC0",
        "OFL-1.1": "SIL OPEN FONT LICENSE",
        "Ubuntu-font-1.0": "UBUNTU FONT LICENCE",
    }
    flat = " ".join(text.split())
    return {spdx for spdx, needle in sniffs.items() if needle in flat}


def satisfies(kinds, ident):
    if ident.startswith("BSD-"):
        return "BSD" in kinds
    if ident.startswith("Apache-2.0"):
        return "Apache-2.0" in kinds
    return ident in kinds


def crate_texts(pkg):
    """[(heading, text)] for one crate, and the licenses nothing was found for."""
    crate_dir = Path(pkg["manifest_path"]).parent
    files = sorted(
        p for p in crate_dir.iterdir() if p.is_file() and LICENSE_FILE.match(p.name)
    )
    files += [HERE / "upstream" / name for name in UPSTREAM_TEXTS.get(pkg["name"], [])]
    always = [locate(crate_dir, rel) for rel in EXTRA_CRATE_FILES.get(pkg["name"], [])]
    files += [p for p in always if p not in files]
    found = [(p, read(p)) for p in files]
    classified = [(p, text, classify(p, text)) for p, text in found]

    chosen = choose(parse_spdx(pkg["license"])) if pkg.get("license") else []
    picked, missing = [], []
    for ident in chosen:
        matches = [(p, t) for p, t, kinds in classified if satisfies(kinds, ident)]
        if matches:
            picked += [m for m in matches if m not in picked]
        elif ident not in NO_NOTICE:
            missing.append(ident)

    # NOTICE files are a condition of Apache-2.0 whichever branch was taken,
    # and anything the override table names is there because it is needed.
    for p, t, kinds in classified:
        if ("NOTICE" in kinds or p in always) and (p, t) not in picked:
            picked.append((p, t))

    # A crate with no recognisable files at all but a license file of some
    # kind: ship what it has rather than nothing.
    if not picked and found and not chosen:
        picked = found

    texts = [(describe(crate_dir, p), t) for p, t in picked]
    for ident in missing:
        canonical = HERE / "texts" / f"{ident}.txt"
        if not canonical.exists():
            raise SystemExit(
                f"{pkg['name']} {pkg['version']} is {ident} but ships no text for "
                f"it; add the standard text as {canonical.relative_to(ROOT)}"
            )
        # Strip e-mail addresses: the holder's name is what the notice needs.
        authors = ", ".join(
            re.sub(r"\s*<[^>]*>", "", a) for a in pkg.get("authors") or []
        ) or f"the {pkg['name']} authors"
        text = read(canonical).replace("<year> <copyright holders>", authors)
        texts.append((f"standard {ident} text; neither the crate nor its repository ships one", text))
    return texts


def locate(crate_dir, rel):
    return HERE / rel if rel.startswith("upstream/") else crate_dir / rel


def describe(crate_dir, path):
    """Where a license text came from, for the heading above it."""
    if path.is_relative_to(HERE):
        return f"from its upstream repository (licenses/{path.relative_to(HERE).as_posix()})"
    return path.relative_to(crate_dir).as_posix()


def read(path):
    return path.read_text(encoding="utf-8", errors="replace").strip("\n") + "\n"


# ---------------------------------------------------------------------------
# Dependency graph
# ---------------------------------------------------------------------------


def shipped_packages(manifest, targets):
    """Registry packages compiled into the product on any of `targets`.

    Follows normal dependency edges only: build-dependencies run at compile
    time and do not end up in the binary.
    """
    shipped = {}
    for target in targets:
        meta = json.loads(
            subprocess.run(
                [
                    "cargo", "metadata", "--format-version", "1", "--locked",
                    "--manifest-path", str(manifest), "--filter-platform", target,
                ],
                check=True,
                capture_output=True,
                text=True,
            ).stdout
        )
        packages = {p["id"]: p for p in meta["packages"]}
        nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
        stack, seen = [meta["resolve"]["root"]], set()
        while stack:
            node = stack.pop()
            if node in seen:
                continue
            seen.add(node)
            for dep in nodes[node]["deps"]:
                if any(k["kind"] is None for k in dep["dep_kinds"]):
                    stack.append(dep["pkg"])
        for node in seen:
            pkg = packages[node]
            # Path dependencies are this repository's own code.
            if pkg["source"] is not None:
                shipped[(pkg["name"], pkg["version"])] = pkg
    return [shipped[k] for k in sorted(shipped)]


# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------

RULE = "=" * 78


def render(description, packages):
    intro = (
        f"This file accompanies {description}. It contains the third-party "
        "material listed below, each part under its own license, after "
        "viewarr's own license."
    )
    out = [
        "THIRD-PARTY LICENSES",
        "",
        *textwrap.wrap(intro, 78),
        "",
        "Generated by licenses/generate.py; do not edit by hand.",
        "",
        RULE,
        "viewarr",
        "",
        # Spelled out rather than pointed at: next to fitsview's own LICENSE,
        # "see LICENSE" would name the wrong one.
        read(ROOT / "viewarr" / "LICENSE"),
    ]

    for path in sorted((HERE / "extra").glob("*.txt")):
        out += [RULE, read(path)]

    # Many crates share a text verbatim (most Apache-2.0 copies, say), so
    # print each distinct text once under the list of crates that use it.
    groups = {}
    for pkg in packages:
        for heading, text in crate_texts(pkg):
            key = " ".join(text.split())
            entry = groups.setdefault(key, {"text": text, "users": []})
            entry["users"].append(
                f"{pkg['name']} {pkg['version']} ({pkg.get('license') or 'no license field'})"
                f" -- {heading}"
            )

    out += [
        RULE,
        f"Rust crates ({len(packages)})",
        "",
        "Each crate is listed with its declared license; where it offers a choice,",
        "the license reproduced is the one viewarr uses it under.",
        "",
    ]
    for entry in sorted(groups.values(), key=lambda e: e["users"][0]):
        out += [RULE]
        out += entry["users"]
        out += ["", entry["text"]]
    return "\n".join(out).rstrip("\n") + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--check", action="store_true", help="fail if a file is out of date"
    )
    args = parser.parse_args()

    stale = []
    for output, manifest, targets, description in PRODUCTS:
        text = render(description, shipped_packages(manifest, targets))
        current = output.read_text(encoding="utf-8") if output.exists() else None
        if text == current:
            continue
        if args.check:
            stale.append(output.relative_to(ROOT))
        else:
            output.write_text(text, encoding="utf-8")
            print(f"wrote {output.relative_to(ROOT)}")

    if stale:
        names = ", ".join(map(str, stale))
        print(
            f"out of date: {names}\nrun `python3 licenses/generate.py` and commit the result",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
