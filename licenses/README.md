# licenses/

Licence texts referenced by `NOTICE.md` and `THIRD-PARTY-NOTICES.md`.

Two kinds of file live here:

- **Canonical texts** — `MIT.txt`, `Apache-2.0.txt`, `Zlib.txt`, `Unlicense.txt`,
  `Unicode-3.0.txt`, `LLVM-exception.txt`. Taken verbatim from the SPDX license-list-data
  repository at a pinned commit (`licenses/pins.json` records the commit and each file's
  sha256). Crates that ship these texts ship them with their own banner lines and a
  filled-in copyright line; the per-crate copyright lines are reported in
  `THIRD-PARTY-NOTICES.md` instead of being baked into these files.
- **Pinned upstream texts** — `objc2-*`. The `objc2` family publishes a licence expression in
  its crate manifest but ships no licence file inside the `.crate` archive, so these texts are
  pinned from the upstream repository at a specific commit. `objc2-LICENSE.md` is upstream's
  own licensing overview and explains why the family splits between MIT and
  `Zlib OR Apache-2.0 OR MIT`, plus its note about being derived from Apple SDK headers.

`apple-framework-linkage.txt` is not a licence: it is an `otool -L` snapshot of the Apple
system frameworks the arm64 binary links, kept so `scripts/gen-notices.sh linkage` can detect
a new framework appearing without anyone noticing.

`scripts/gen-notices.py` reads only these files — a release never fetches a licence text from
the network. Add a new text by pinning it (URL + commit + sha256 in `pins.json`), then
regenerate the notices.
