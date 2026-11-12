# NOTICE

NotchQuake — Copyright 2026 Barnstorm LLC. NotchQuake's own source is MIT-licensed;
see [LICENSE](./LICENSE).

This distribution includes third-party open-source software. The complete, per-dependency
attribution — licence expression, version, source link, and the licence text each crate
actually shipped — is in **[THIRD-PARTY-NOTICES.md](./THIRD-PARTY-NOTICES.md)**, with the
licence texts themselves in **[licenses/](./licenses/)** and their pinned provenance in
`licenses/pins.json`. Bundle all four with a release archive.

## Headline attributions

- **`alacritty_terminal` 0.26.0** — Apache-2.0, Copyright 2020 The Alacritty Project
  (Christian Duerr, Joe Wilm). <https://github.com/alacritty/alacritty>
  Text: [`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt). NotchQuake uses it for the
  terminal grid, PTY plumbing, and the escape-sequence parser, and modifies nothing in it.
- **`vte` 0.15.0** — Apache-2.0 OR MIT, Copyright (c) 2016 Joe Wilm.
  <https://github.com/alacritty/vte>
- **`objc2` family and `dispatch2`** (`objc2`, `objc2-app-kit`, `objc2-core-foundation`,
  `objc2-core-graphics`, `objc2-core-text`, `objc2-foundation`, `objc2-core-data`,
  `objc2-cloud-kit`, `objc2-core-image`, `objc2-core-video`, `objc2-io-surface`,
  `objc2-metal`, `objc2-quartz-core`, `objc2-encode`, `block2`) — Copyright Mads Marquart
  and the objc2 project contributors. `MIT`, or `Zlib OR Apache-2.0 OR MIT` at your option
  depending on the crate; **the published crates ship no licence text**, so the texts are
  pinned from upstream and committed in `licenses/`. Upstream also notes these crates are
  derived from Apple SDK headers — see the objc2 section of THIRD-PARTY-NOTICES.md.
  <https://github.com/madsmtm/objc2>
- **`rustix` 1.1.5 / `rustix-openpty` 0.2.0** — Apache-2.0 WITH LLVM-exception OR
  Apache-2.0 OR MIT (manifest authors: Dan Gohman, Jakub Konka; their shipped licence files
  carry no copyright line). The LLVM exception is reproduced in
  [`licenses/LLVM-exception.txt`](licenses/LLVM-exception.txt).
- **`unicode-ident` 1.0.26** — `(MIT OR Apache-2.0) AND Unicode-3.0`; the Unicode-3.0
  licence (Copyright © 1991-2023 Unicode, Inc.) is **not** optional and is reproduced in
  [`licenses/Unicode-3.0.txt`](licenses/Unicode-3.0.txt).
- **Everything else** — MIT and/or Apache-2.0 (plus Zlib for `cursor-icon`, and
  Unlicense-or-MIT for `aho-corasick` and `memchr`), with each copyright line listed in
  THIRD-PARTY-NOTICES.md.

## What is *not* a third-party dependency

Apple's system frameworks (AppKit, CoreFoundation, CoreGraphics, CoreText, Foundation,
Carbon, CloudKit, CoreData, CoreImage, CoreVideo, Metal, QuartzCore, `libSystem`,
`libobjc`, `libiconv`) are supplied by macOS on the user's machine and are covered by the
Apple software licence the user already accepted, not by anything in `Cargo.lock`.
The recorded linkage snapshot is `licenses/apple-framework-linkage.txt`.

No fonts are bundled: NotchQuake resolves monospace families through CoreText at runtime,
so no font licence (e.g. Fira Code's OFL-1.1) applies to the shipped bundle today. If a
font is ever embedded, add it here and regenerate the notices.

## Regenerating and checking

```sh
scripts/gen-notices.sh gen     # regenerate THIRD-PARTY-NOTICES.md
scripts/gen-notices.sh check   # fail if the committed file is stale or hand-edited
scripts/gen-notices.sh deny    # cargo-deny licence/version/source policy (deny.toml)
scripts/gen-notices.sh linkage # re-verify the Apple framework snapshot against a build
```

The generator is offline and deterministic. `deny.toml` fails the build on any licence
outside the allow-list, and on any dependency source other than crates.io.

This file is an attribution record, not legal advice; open questions it does not resolve
are listed in THIRD-PARTY-NOTICES.md.
