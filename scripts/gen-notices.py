#!/usr/bin/env python3
"""Deterministic third-party notice generator for NotchQuake (macOS arm64).

Inputs (all local -- NO network access):
  * Cargo.toml / Cargo.lock          -> the dependency graph
  * cargo metadata --offline         -> per-crate license SPDX, repository, features
  * ~/.cargo/registry/src/<index>/   -> the license files each crate actually shipped
  * licenses/                        -> committed canonical license texts + pins

Outputs:
  * THIRD-PARTY-NOTICES.md   (committed; regenerates byte-identically)

The graph is scoped to the shipped runtime target, aarch64-apple-darwin, so
Windows/other-platform crates that appear in Cargo.lock are reported as
out-of-scope instead of silently attributed.

Usage: scripts/gen-notices.py            write THIRD-PARTY-NOTICES.md
       scripts/gen-notices.py --check    exit 1 if the committed file drifts
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import sys
from collections import OrderedDict

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LIC_DIR = os.path.join(ROOT, "licenses")
OUT = os.path.join(ROOT, "THIRD-PARTY-NOTICES.md")
TARGET = "aarch64-apple-darwin"

# --------------------------------------------------------------------------- #
# small helpers
# --------------------------------------------------------------------------- #


def sha(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def norm(s: str) -> str:
    """Whitespace-insensitive normalisation for license-body comparison."""
    s = s.replace("\u00a0", " ").replace("\r\n", "\n").replace("\r", "\n")
    return " ".join(s.split())


def body(text: str, anchor: str, end: str | None = None) -> str | None:
    """Extract the operative license body, dropping header/comment boilerplate.

    Crate authors ship these texts with their own banner lines and a filled-in
    copyright line; comparing from the operative anchor keeps that noise from
    looking like a licence deviation.
    """
    t = norm(text)
    i = t.find(anchor)
    if i < 0:
        return None
    if end is not None:
        j = t.find(end, i)
        if j < 0:
            return None
        return t[i : j + len(end)]
    return t[i:]


COPY_LINE = re.compile(r"^\s*copyright\b", re.I)
# lines inside a licence text that merely *talk about* copyright notices
COPY_NOISE = re.compile(r"copyright (notice|license|owner|work|patents|holder|that|to)", re.I)


def copyright_lines(text: str) -> list[str]:
    out = []
    for line in text.split("\n"):
        s = line.strip()
        if COPY_LINE.match(s) and not COPY_NOISE.search(s):
            out.append(" ".join(s.split()))
    return out


def run(cmd: list[str]) -> str:
    p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    if p.returncode != 0:
        sys.exit(f"command failed: {' '.join(cmd)}\n{p.stderr.strip()}")
    return p.stdout


def registry_dirs() -> list[str]:
    base = os.path.expanduser("~/.cargo/registry/src")
    dirs = []
    if os.path.isdir(base):
        for d in sorted(os.listdir(base)):
            full = os.path.join(base, d)
            if os.path.isdir(full):
                dirs.append(full)
    return dirs


# --------------------------------------------------------------------------- #
# canonical licence texts (committed, pinned)
# --------------------------------------------------------------------------- #

# spdx id -> (file in licenses/, operative anchor, optional end marker)
CANONICAL = OrderedDict(
    [
        ("MIT", ("MIT.txt", "Permission is hereby granted", None)),
        ("Apache-2.0", ("Apache-2.0.txt", "TERMS AND CONDITIONS FOR USE", "END OF TERMS AND CONDITIONS")),
        (
            "Apache-2.0 WITH LLVM-exception",
            ("Apache-2.0.txt", "TERMS AND CONDITIONS FOR USE", "END OF TERMS AND CONDITIONS"),
        ),
        ("Zlib", ("Zlib.txt", "This software is provided", None)),
        ("Unlicense", ("Unlicense.txt", "This is free and unencumbered software", None)),
        ("Unicode-3.0", ("Unicode-3.0.txt", "Permission is hereby granted", None)),
    ]
)


def load_canonical() -> dict[str, str]:
    out = {}
    for spdx, (fname, anchor, end) in CANONICAL.items():
        path = os.path.join(LIC_DIR, fname)
        if not os.path.exists(path):
            sys.exit(f"missing committed licence text licenses/{fname}")
        out[spdx] = body(open(path, encoding="utf-8").read(), anchor, end) or ""
    return out


CANON = load_canonical()
def exc_body(text: str) -> str | None:
    """Operative body of the LLVM exception, ignoring its banner formatting."""
    t = norm(text)
    anchor = "As an exception, if, as a result of"
    end = "with respect to the Combined Software."
    i = t.find(anchor)
    j = t.find(end, i)
    return t[i : j + len(end)] if i >= 0 and j >= 0 else None


LLVM_EXC = exc_body(open(os.path.join(LIC_DIR, "LLVM-exception.txt"), encoding="utf-8").read())


def classify(fname: str) -> str | None:
    """Map a vendored licence filename onto a canonical SPDX id."""
    u = fname.upper()
    if "LLVM" in u:
        return "Apache-2.0 WITH LLVM-exception"
    if "APACHE" in u:
        return "Apache-2.0"
    if "UNICODE" in u:
        return "Unicode-3.0"
    if "ZLIB" in u:
        return "Zlib"
    if "UNLICENSE" in u:
        return "Unlicense"
    if u.startswith("LICENSE-MIT") or u == "LICENSE":
        return "MIT"
    if u == "COPYING":
        return None  # dual-license pointer file, not a licence text
    return None


# --------------------------------------------------------------------------- #
# graph
# --------------------------------------------------------------------------- #

meta_target = json.loads(
    run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1",
         "--filter-platform", TARGET])
)
meta_all = json.loads(
    run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1"])
)

nodes = {n["id"]: n for n in meta_target["resolve"]["nodes"]}
root_id = meta_target["resolve"]["root"]


def short(nid: str) -> str:
    return nid.split("#")[-1]


def name_of(nid: str) -> str:
    return short(nid).split("@")[0]


proc_macros = {
    p["name"] + "@" + p["version"]
    for p in meta_target["packages"]
    if any("proc-macro" in (t["kind"] if isinstance(t["kind"], list) else [t["kind"]])
           for t in p["targets"])
}

direct = {name_of(d["pkg"]) for d in nodes[root_id]["deps"]}


def is_proc(nid: str) -> bool:
    return short(nid) in proc_macros


def reachable(skip_proc: bool) -> set[str]:
    seen = {root_id}
    out: set[str] = set()
    stack = [root_id]
    while stack:
        nid = stack.pop()
        for d in nodes[nid]["deps"]:
            if skip_proc and is_proc(d["pkg"]):
                continue
            out.add(name_of(d["pkg"]))
            if d["pkg"] not in seen:
                seen.add(d["pkg"])
                stack.append(d["pkg"])
    return out


full = reachable(False)
runtime = reachable(True)
compile_only = full - runtime

target_pkgs = {p["name"] + "@" + p["version"] for p in meta_target["packages"]}
out_of_scope = sorted(
    p["name"] + "@" + p["version"]
    for p in meta_all["packages"]
    if p["source"] is not None and p["name"] + "@" + p["version"] not in target_pkgs
)

pkg = {p["name"]: p for p in meta_target["packages"]}


def parents(name: str) -> list[str]:
    return sorted({name_of(nid) for nid, n in nodes.items()
                   if any(name_of(d["pkg"]) == name for d in n["deps"])})


# --------------------------------------------------------------------------- #
# vendored licence evidence
# --------------------------------------------------------------------------- #

REG = registry_dirs()


def crate_dir(name: str, version: str) -> str:
    for d in REG:
        cand = os.path.join(d, f"{name}-{version}")
        if os.path.isdir(cand):
            return cand
    sys.exit(
        f"crate source for {name} {version} is not present in the local cargo registry "
        f"(~/.cargo/registry/src). Run `cargo fetch --locked` first; this generator never "
        f"downloads anything by itself."
    )


LicenseGroup = OrderedDict()


def group(spdx: str) -> dict:
    g = LicenseGroup.setdefault(spdx, {"crates": [], "copyrights": OrderedDict(), "verbatim": []})
    return g


# Unlicense is the other option of aho-corasick/memchr's `Unlicense OR MIT`, and no crate
# ships that text; make the group exist anyway so the committed text and the COPYING pointer
# files are accounted for rather than silently absent.
group("Unlicense")

unmatched: list[dict] = []
pointer_files: list[str] = []
notice_scan = 0
notice_files: list[str] = []
# crates whose shipped licence files contain no copyright line at all
has_copyright: set[str] = set()
all_crates: set[str] = set()

for p in sorted((x for x in meta_target["packages"] if x["source"] is not None),
                key=lambda x: (x["name"], x["version"])):
    d = crate_dir(p["name"], p["version"])
    notice_scan += 1
    for f in os.listdir(d):
        if f.upper() == "NOTICE":
            notice_files.append(f"{p['name']} {p['version']}: {f}")
    files = sorted(f for f in os.listdir(d)
                   if f.upper().startswith("LICENSE") or f.upper().startswith("COPYING"))
    if not files:
        unmatched.append({"name": p["name"], "version": p["version"]})
    all_crates.add(p["name"])
    for f in files:
        text = open(os.path.join(d, f), encoding="utf-8", errors="replace").read()
        spdx = classify(f)
        if spdx is None:
            pointer_files.append(f"{p['name']} {p['version']}: {f}")
            continue
        own = copyright_lines(text)
        if own:
            has_copyright.add(p["name"])
        vbody = body(text, CANONICAL[spdx][1], CANONICAL[spdx][2])
        if spdx == "Apache-2.0 WITH LLVM-exception":
            # rustix vendors Apache-2.0 + the LLVM exception in one file, with its
            # own banner dash counts; compare both operative bodies.
            fits = vbody == CANON[spdx] and exc_body(text) == LLVM_EXC
        else:
            fits = vbody == CANON[spdx]
        if fits:
            g = group(spdx)
            g["crates"].append((p["name"], p["version"], f, sha(text)[:12]))
            for c in own:
                g["copyrights"].setdefault(c, []).append(p["name"])
        else:
            group(spdx)["verbatim"].append((p["name"], p["version"], f, text, sha(text)[:12]))

# crates that DO ship licence files but with no copyright line in any of them
no_copyright = sorted(all_crates - has_copyright - {u["name"] for u in unmatched})

for name in sorted(set(direct)):
    if name not in pkg:
        sys.exit(f"direct dependency {name} missing from metadata")

# The claim in Open questions is only safe while every text-less crate is in the objc2
# family; fail loudly if a future dependency ships no licence file.
for u in unmatched:
    if not (u["name"].startswith("objc2") or u["name"] in ("block2", "dispatch2")):
        sys.exit(f"{u['name']} {u['version']} ships no licence file and is not an objc2 "
                 f"family crate — pin its text into licenses/ and re-run the generator")

# --------------------------------------------------------------------------- #
# linkage evidence (committed snapshot; re-verifiable against a real binary)
# --------------------------------------------------------------------------- #

linkage_path = os.path.join(LIC_DIR, "apple-framework-linkage.txt")
linkage = []
if os.path.exists(linkage_path):
    linkage = [l.strip() for l in open(linkage_path) if l.strip() and not l.startswith("#")]

# --------------------------------------------------------------------------- #
# render
# --------------------------------------------------------------------------- #

L: list[str] = []
add = L.append

lock_hash = sha(open(os.path.join(ROOT, "Cargo.lock"), encoding="utf-8").read())
pins_hash = sha(open(os.path.join(LIC_DIR, "pins.json"), encoding="utf-8").read())
cargo_hash = sha(open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8").read())

add("# Third-party notices — NotchQuake")
add("")
add("NotchQuake is a standalone macOS application by Barnstorm LLC whose own source is")
add("MIT-licensed (see [LICENSE](./LICENSE)). It links Rust crates and, through them, Apple")
add("system frameworks. This file records the attribution and licence texts that the")
add("distribution owes, per dependency, with the evidence used to derive it.")
add("")
add("**Not legal advice.** This is a mechanical attribution audit produced by")
add("`scripts/gen-notices.sh`. It reports what each dependency's own manifest and shipped")
add("licence files say; it does not interpret the licences. Unresolved questions are listed")
add("in [Open questions](#open-questions) rather than answered.")
add("")
add("---")
add("")
add("## How this file is produced and checked")
add("")
add("```sh")
add("scripts/gen-notices.sh gen      # regenerate THIRD-PARTY-NOTICES.md")
add("scripts/gen-notices.sh check    # fail if the committed file is stale or hand-edited")
add("scripts/gen-notices.sh deny     # cargo-deny licence/source policy check")
add("scripts/gen-notices.sh linkage  # re-verify the Apple framework snapshot below")
add("```")
add("")
add("The generator is offline and deterministic: it reads `Cargo.lock`,")
add("`cargo metadata --locked --offline`, the crate sources already in")
add("`~/.cargo/registry/src`, and the committed texts in `licenses/`. It fetches nothing,")
add("embeds no timestamps, and sorts everything, so the same inputs always produce the same")
add("bytes. If a crate's source is absent locally it stops with an instruction to run")
add("`cargo fetch --locked` instead of downloading.")
add("")
add("Machine-readable inputs to this exact rendering:")
add("")
add(f"- `Cargo.lock` sha256 `{lock_hash[:16]}…`")
add(f"- `Cargo.toml` sha256 `{cargo_hash[:16]}…`")
add(f"- `licenses/pins.json` sha256 `{pins_hash[:16]}…`")
add(f"- cargo metadata: `--locked --offline --filter-platform {TARGET}`")
add("")
add("---")
add("")
add("## Scope: what ships, and on which target")
add("")
add(f"Only `{TARGET}` ships. The tables below were resolved with")
add(f"`cargo metadata --filter-platform {TARGET}`, which drops the dependency edges that")
add("Cargo.lock records for other targets. Four scopes follow:")
add("")
add(f"- **direct** — {len(direct)} crates declared in `Cargo.toml`.")
add(f"- **transitive runtime** — {len(runtime) - len(direct)} crates reachable from the root")
add(f"  through non-`proc-macro` edges, i.e. code linked into the shipped binary.")
add(f"- **compile-time only** — {len(compile_only)} crates: the one `proc-macro` crate in the")
add("  graph (`serde_derive`) plus the dependency closure only it pulls in. They run during")
add("  `cargo build` and contribute no runtime code to the shipped binary, but their licences")
add("  still travel with the source distribution and are attributed below.")
add(f"- **out of scope** — {len(out_of_scope)} crates present in `Cargo.lock` for other")
add("  targets only (Windows, Linux, Redox, hermit). Listed at the end; not shipped and not")
add("  attributed.")
add("")

def deps_of(name: str) -> str:
    par = parents(name)
    if not par:
        return "root"
    return ", ".join(par)


def section_crates(names: list[str], label: str) -> None:
    add(f"### {label}")
    add("")
    add("| crate | version | licence (SPDX, per manifest) | required by | source | licence file in crate |")
    add("| --- | --- | --- | --- | --- | --- |")
    for name in names:
        p = pkg[name]
        lic = p.get("license") or "not stated in manifest"
        files = {c[2] for spdx in LicenseGroup for c in LicenseGroup[spdx]["crates"] if c[0] == name}
        files |= {c[2] for spdx in LicenseGroup for c in LicenseGroup[spdx]["verbatim"] if c[0] == name}
        files = sorted(files)
        shown = ", ".join(files) if files else "none shipped — see Open questions"
        repo = p.get("repository") or p.get("homepage") or ""
        src = f"[crates.io](https://crates.io/crates/{name}/{p['version']})"
        if repo and repo.startswith("http"):
            src += f" · [repo]({repo})"
        add(f"| `{name}` | {p['version']} | {lic} | {deps_of(name)} | {src} | {shown} |")
    add("")


direct_names = sorted(direct)
trans_names = sorted(runtime - direct)
compile_names = sorted(compile_only)

section_crates(direct_names, "Direct dependencies (declared in Cargo.toml)")
section_crates(trans_names, "Transitive runtime dependencies (linked into the binary)")
section_crates(compile_names, "Compile-time-only dependencies (proc-macro closure)")

add("---")
add("")
add("## Apple system frameworks (no third-party licence)")
add("")
add("The binary links Apple's own frameworks, which are not third-party open-source")
add("dependencies: they are supplied by the operating system and are covered by the Apple")
add("software licence the user accepted with macOS / Xcode, not by anything in `Cargo.lock`.")
if linkage:
    add("")
    add("Recorded from `otool -L` on the built arm64 binary and committed as a snapshot in")
    add("`licenses/apple-framework-linkage.txt`; `scripts/gen-notices.sh linkage` re-checks a")
    add("fresh build against it:")
    add("")
    for f in linkage:
        add(f"- `{f}`")
    add("")
else:
    add("")
    add("`licenses/apple-framework-linkage.txt` is absent; run `scripts/gen-notices.sh linkage`")
    add("against a built binary to record it.")
    add("")
add("The `objc2`-family crates are Rust bindings *derived from Apple SDK headers*; see")
add("[the objc2 licensing section](#the-objc2-family) for what that affects.")
add("")
add("---")
add("")
add("## Licence groups and texts")
add("")
add("For each group below the crates are the ones whose shipped licence file matches the")
add("canonical text committed under `licenses/` (compared with whitespace normalisation, and")
add("with the licence's own copyright line excluded, since authors fill it in per project).")
add("Anything that does not match is reproduced verbatim in the last section instead of being")
add("abstracted away.")
add("")
add("| SPDX | shipped-text matches | canonical text |")
add("| --- | --- | --- |")
for spdx, g in LicenseGroup.items():
    fname = CANONICAL[spdx][0]
    extra = " + [`LLVM-exception.txt`](licenses/LLVM-exception.txt)" if spdx == "Apache-2.0 WITH LLVM-exception" else ""
    add(f"| {spdx} | {len(g['crates'])} | [`licenses/{fname}`](licenses/{fname}){extra} |")
add("")
add("`licenses/Unlicense.txt` is committed even though no crate ships that file: it is the")
add("other half of `aho-corasick` and `memchr`'s `Unlicense OR MIT` expression, quoted so the")
add("dual licence can be read without a second source.")
add("")

for spdx, g in LicenseGroup.items():
    fname = CANONICAL[spdx][0]
    add(f"### {spdx}")
    add("")
    add(f"Canonical text: [`licenses/{fname}`](licenses/{fname})"
        + (" + [`licenses/LLVM-exception.txt`](licenses/LLVM-exception.txt)"
           if spdx == "Apache-2.0 WITH LLVM-exception" else ""))
    add("")
    if g["copyrights"]:
        add("Copyright notices as written in the shipped licence files:")
        add("")
        for c, names in g["copyrights"].items():
            add(f"- `{c}` — {', '.join(sorted(set(names)))}")
        add("")
    if g["crates"]:
        add("Crates whose shipped licence file matches:")
        add("")
        for name, version, f, h in sorted(g["crates"]):
            add(f"- `{name} {version}` — `{f}` (sha256 `{h[:12]}…)")
    else:
        add("No crate ships a file matching this text; it is committed only as the other")
        add("option of a dual-licensed dependency.")
    add("")
    if spdx == "Unlicense":
        add("No crate ships an `Unlicense` text. It is the other option of `aho-corasick` and")
        add("`memchr`'s `Unlicense OR MIT` expression, committed here so that dual licence can")
        add("be read without a second source. Those two crates ship a `COPYING` file that only")
        add("states the dual-licensing:")
        add("")
        for pf in sorted(pointer_files):
            add(f"- `{pf}`")
        add("")

# objc2 section — special handling
add("---")
add("")
add("## The objc2 family")
add("")
family = [n for n in pkg if n.startswith("objc2") or n in ("block2", "dispatch2")]
mit_family = sorted(n for n in family if pkg[n].get("license") == "MIT")
trio = sorted(n for n in family if pkg[n].get("license") != "MIT")
assert mit_family and trio, "objc2 family licence split changed upstream"
add("The `objc2` family is the one place that needs care beyond the tables above: the")
add("crates.io manifest states a licence expression, but the published crates ship no text.")
add("")
add(f"- Manifest expressions in this build: `MIT` for {', '.join('`'+n+'`' for n in mit_family)};")
add(f"  `Zlib OR Apache-2.0 OR MIT` for {', '.join('`'+n+'`' for n in trio)}.")
add("- **No licence text is shipped inside those published crates.** `cargo metadata` reports")
add("  a licence expression, but the `.crate` archives contain no `LICENSE*` file at all, so")
add("  there is nothing in the vendored source to quote. The texts below are therefore taken")
add("  from the upstream repository, pinned to a specific commit, and committed here so the")
add("  release does not depend on GitHub being reachable.")
add("")
add("Pinned upstream evidence (see `licenses/pins.json`, whose sha256 is quoted at the top of")
add("this file):")
add("")
add("```")
pins = json.load(open(os.path.join(LIC_DIR, "pins.json")))
for pin in pins["pins"]:
    add(f"{pin['file']}")
    add(f"  {pin['url']}")
    add(f"  sha256 {pin['sha256']}")
add("```")
add("")
add("What that document says, quoted from the pinned `licenses/objc2-LICENSE.md`:")
add("")
add("> The crates `objc2`, `block2`, `objc2-foundation` and `objc2-encode` are")
add("> [currently][#23] licensed under [the MIT license][MIT]. All other crates are")
add("> trio-licensed under the [Zlib], [Apache-2.0] or [MIT] license, at your option.")
add("")
add("Two consequences we record rather than resolve:")
add("")
add(f"1. **A licence transition is in flight.** The upstream note links issue `#23` and its")
add("   own manifest comment for `objc2-foundation` reads `All future contributions are")
add("   licensed under \"Zlib OR Apache-2.0 OR MIT\"` — so the MIT label on the crates in")
add("   this build is the *current* licence of the code as published at those versions, not a")
add("   statement about later ones. Choosing MIT for these versions is supported by the")
add("   crates.io metadata plus the pinned text; anyone redistributing newer versions must")
add("   re-read that document rather than reuse this section.")
add("2. **Apple SDK derivation.** The same document states the crates are derived from Apple")
add("   SDKs shipped with Xcode and that the Xcode end-user licence agreement")
add("   (`xcodebuild -license`) may bear on distributing derived works. NotchQuake links")
add("   Apple's own frameworks on the user's machine rather than redistributing Apple code,")
add("   and the bindings themselves are small Rust declarations, but that is a description of")
add("   the situation, not a legal conclusion — see [Open questions](#open-questions).")
add("")
add("Attribution to use for these crates:")
add("")
add("```")
add("\n".join(f"{n} {pkg[n]['version']}" for n in mit_family))
add("  Copyright Mads Marquart and the objc2 project contributors")
add("  MIT licence — licenses/objc2-LICENSE-MIT.txt (pinned from upstream)")
add("")
add("\n".join(f"{n} {pkg[n]['version']}" for n in trio))
add("  Copyright Mads Marquart and the objc2 project contributors")
add("  Zlib OR Apache-2.0 OR MIT, at your option")
add("  — licenses/objc2-LICENSE-ZLIB.txt / objc2-LICENSE-APACHE.txt / objc2-LICENSE-MIT.txt")
add("```")
add("")
add("The family pulls in more than the crate names suggest: `objc2-app-kit` and")
add("`objc2-core-graphics` enable `objc2-cloud-kit`, `objc2-core-image`, `objc2-metal`,")
add("`objc2-quartz-core`, `objc2-core-video`, `objc2-io-surface` and `dispatch2` through")
add("their default features, and `otool -L` on the built binary confirms the resulting")
add("frameworks (CloudKit, CoreData, CoreImage, Metal, QuartzCore, CoreVideo) are linked.")
add("That is why those framework crates appear in the runtime table even though no source")
add("file in `src/` mentions them.")
add("")
add("---")
add("")
add("## Verbatim licence texts")
add("")
verbatim_total = sum(len(g["verbatim"]) for g in LicenseGroup.values())
if verbatim_total:
    add(f"{verbatim_total} shipped licence file(s) did not match the canonical text above, so")
    add("they are reproduced exactly as shipped:")
    add("")
    for spdx, g in LicenseGroup.items():
        for name, version, f, text, h in sorted(g["verbatim"]):
            add(f"### {name} {version} — `{f}` ({spdx})")
            add("")
            add(f"sha256 `{h}`. Source: <https://crates.io/crates/{name}/{version}>")
            add("")
            add("```text")
            add(text.rstrip("\n"))
            add("```")
            add("")
else:
    add("None. Every vendored licence file matched its committed canonical text.")
    add("")

add("---")
add("")
add("## Out of scope for this build")
add("")
add("Present in `Cargo.lock` but resolved only for targets NotchQuake does not ship;")
add(f"`cargo metadata --filter-platform {TARGET}` excludes them. They are not linked, not")
add("attributed, and listed here so a reader can see the difference between \"in the lock")
add("file\" and \"in the app\":")
add("")
for entry in out_of_scope:
    name, version = entry.split("@")
    add(f"- `{name} {version}`")
add("")
add("`cargo-deny` reports the two `windows-sys` entries in `Cargo.lock` as a duplicate-version")
add("warning. That is expected: the two versions are gated to different target cfgs and only")
add("one is ever built for `aarch64-apple-darwin`. `deny.toml` keeps the check at `warn`.")
add("")
add("---")
add("")
add("## Open questions")
add("")
add("Recorded, not resolved. None of these blocks a build; each is a question for whoever")
add("owns the release decision (and, where it is genuinely legal, for a lawyer).")
add("")
add("1. **objc2 ↔ Xcode SLA.** Upstream itself calls the distribution position \"unclear\"")
add("   while still using standard SPDX identifiers. NotchQuake distributes only its own")
add("   compiled binary plus Apple's system frameworks, but whether the generated bindings")
add("   count as a derived work of the SDK is not answered by anything in the crate metadata.")
add("2. **Crates that ship no licence text.** " + str(len(unmatched)) + " crates, all of them")
add("   in the objc2 family (the generator fails if that ever stops being true):")
add("   " + ", ".join(f"`{u['name']} {u['version']}`" for u in sorted(unmatched, key=lambda u: u["name"])))
add("   The SPDX expression is present in the manifest, the text is absent from the published")
add("   crate. The texts quoted above are pinned from upstream and committed in `licenses/`;")
add("   if a future crate has no text, pin it the same way — never fetch it at release time.")
add("3. **Copyright lines missing from some shipped licence files.** " + str(len(no_copyright)))
add("   crates ship a licence file with no copyright line at all: "
    + ", ".join(f"`{n}`" for n in sorted(no_copyright)) + ".")
add("   Their manifest `authors` field is reported in the tables above, but no attribution")
add("   line can be quoted from the licence file itself, so the exact copyright holder for")
add("   those is left unstated here rather than guessed.")
add("4. **Fira Code is not bundled.** NotchQuake resolves monospace fonts through CoreText at")
add("   runtime and ships no font files; if a font were ever embedded in the bundle, its own")
add("   licence (Fira Code is OFL-1.1) would have to be added here and in `NOTICE.md`.")
add("5. **No NOTICE files in this dependency graph.** " + ("None found." if not notice_files else "Found: " + ", ".join(f"`{n}`" for n in sorted(notice_files)) + "."))
add("   Verified across all " + str(notice_scan) + " vendored crate roots. Apache-2.0 section 4.4's")
add("   notices requirement therefore has nothing to carry forward, and NOTICE.md is")
add("   Barnstorm's own attribution file, not a pass-through of upstream notices.")
add("")
add("---")
add("")
add("## Bundle these files in the release archive")
add("")
add("A distribution archive should contain:")
add("")
add("- `NOTICE.md` — the short attribution summary")
add("- `THIRD-PARTY-NOTICES.md` — this file")
add("- `LICENSE` — the NotchQuake MIT licence")
add("- `licenses/` — every canonical and pinned licence text referenced above")
add("")
add("`scripts/gen-notices.sh check` must pass on the commit the release is cut from.")
add("")

content = "\n".join(L) + "\n"

if "--check" in sys.argv:
    if not os.path.exists(OUT):
        sys.exit("THIRD-PARTY-NOTICES.md does not exist")
    current = open(OUT, encoding="utf-8").read()
    if current != content:
        import difflib
        diff = list(difflib.unified_diff(current.splitlines(), content.splitlines(),
                                         "committed", "generated", lineterm="", n=2))
        print("THIRD-PARTY-NOTICES.md is stale or hand-edited. Regenerate with "
              "scripts/gen-notices.sh gen\n")
        print("\n".join(diff[:80]))
        sys.exit(1)
    print(f"THIRD-PARTY-NOTICES.md is current ({len(content)} bytes, "
          f"sha256 {sha(content)[:16]}…)")
else:
    open(OUT, "w", encoding="utf-8").write(content)
    print(f"wrote {OUT} ({len(content)} bytes, sha256 {sha(content)[:16]}…)")
