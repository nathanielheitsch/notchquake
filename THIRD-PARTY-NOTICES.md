# Third-party notices — NotchQuake

NotchQuake is a standalone macOS application by Barnstorm LLC whose own source is
MIT-licensed (see [LICENSE](./LICENSE)). It links Rust crates and, through them, Apple
system frameworks. This file records the attribution and licence texts that the
distribution owes, per dependency, with the evidence used to derive it.

**Not legal advice.** This is a mechanical attribution audit produced by
`scripts/gen-notices.sh`. It reports what each dependency's own manifest and shipped
licence files say; it does not interpret the licences. Unresolved questions are listed
in [Open questions](#open-questions) rather than answered.

---

## How this file is produced and checked

```sh
scripts/gen-notices.sh gen      # regenerate THIRD-PARTY-NOTICES.md
scripts/gen-notices.sh check    # fail if the committed file is stale or hand-edited
scripts/gen-notices.sh deny     # cargo-deny licence/source policy check
scripts/gen-notices.sh linkage  # re-verify the Apple framework snapshot below
```

The generator is offline and deterministic: it reads `Cargo.lock`,
`cargo metadata --locked --offline`, the crate sources already in
`~/.cargo/registry/src`, and the committed texts in `licenses/`. It fetches nothing,
embeds no timestamps, and sorts everything, so the same inputs always produce the same
bytes. If a crate's source is absent locally it stops with an instruction to run
`cargo fetch --locked` instead of downloading.

Machine-readable inputs to this exact rendering:

- `Cargo.lock` sha256 `fdb917f80554cd79…`
- `Cargo.toml` sha256 `9f1be5b693f8ddd0…`
- `licenses/pins.json` sha256 `92160d715f4ace5a…`
- cargo metadata: `--locked --offline --filter-platform aarch64-apple-darwin`

---

## Scope: what ships, and on which target

Only `aarch64-apple-darwin` ships. The tables below were resolved with
`cargo metadata --filter-platform aarch64-apple-darwin`, which drops the dependency edges that
Cargo.lock records for other targets. Four scopes follow:

- **direct** — 11 crates declared in `Cargo.toml`.
- **transitive runtime** — 33 crates reachable from the root
  through non-`proc-macro` edges, i.e. code linked into the shipped binary.
- **compile-time only** — 5 crates: the one `proc-macro` crate in the
  graph (`serde_derive`) plus the dependency closure only it pulls in. They run during
  `cargo build` and contribute no runtime code to the shipped binary, but their licences
  still travel with the source distribution and are attributed below.
- **out of scope** — 23 crates present in `Cargo.lock` for other
  targets only (Windows, Linux, Redox, hermit). Listed at the end; not shipped and not
  attributed.

### Direct dependencies (declared in Cargo.toml)

| crate | version | licence (SPDX, per manifest) | required by | source | licence file in crate |
| --- | --- | --- | --- | --- | --- |
| `alacritty_terminal` | 0.26.0 | Apache-2.0 | 0.1.0 | [crates.io](https://crates.io/crates/alacritty_terminal/0.26.0) · [repo](https://github.com/alacritty/alacritty) | LICENSE-APACHE |
| `block2` | 0.6.2 | MIT | 0.1.0, objc2-app-kit, objc2-core-data, objc2-core-foundation, objc2-core-graphics, objc2-core-text, objc2-foundation | [crates.io](https://crates.io/crates/block2/0.6.2) · [repo](https://github.com/madsmtm/objc2) | none shipped — see Open questions |
| `libc` | 0.2.190 | MIT OR Apache-2.0 | 0.1.0, alacritty_terminal, errno, objc2-app-kit, objc2-core-foundation, objc2-core-graphics, objc2-core-text, objc2-foundation, parking_lot_core, rustix, rustix-openpty, signal-hook, signal-hook-registry | [crates.io](https://crates.io/crates/libc/0.2.190) · [repo](https://github.com/rust-lang/libc) | LICENSE-APACHE, LICENSE-MIT |
| `objc2` | 0.6.5 | MIT | 0.1.0, block2, dispatch2, objc2-app-kit, objc2-cloud-kit, objc2-core-data, objc2-core-foundation, objc2-core-graphics, objc2-core-image, objc2-core-text, objc2-core-video, objc2-foundation, objc2-io-surface, objc2-metal, objc2-quartz-core | [crates.io](https://crates.io/crates/objc2/0.6.5) · [repo](https://github.com/madsmtm/objc2) | none shipped — see Open questions |
| `objc2-app-kit` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | 0.1.0 | [crates.io](https://crates.io/crates/objc2-app-kit/0.3.2) · [repo](https://github.com/madsmtm/objc2) | none shipped — see Open questions |
| `objc2-core-data` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | 0.1.0, objc2-app-kit | [crates.io](https://crates.io/crates/objc2-core-data/0.3.2) · [repo](https://github.com/madsmtm/objc2) | none shipped — see Open questions |
| `objc2-core-foundation` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | 0.1.0, objc2-app-kit, objc2-core-graphics, objc2-core-text, objc2-core-video, objc2-foundation, objc2-io-surface | [crates.io](https://crates.io/crates/objc2-core-foundation/0.3.2) · [repo](https://github.com/madsmtm/objc2) | none shipped — see Open questions |
| `objc2-core-graphics` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | 0.1.0, objc2-app-kit, objc2-core-text, objc2-core-video | [crates.io](https://crates.io/crates/objc2-core-graphics/0.3.2) · [repo](https://github.com/madsmtm/objc2) | none shipped — see Open questions |
| `objc2-core-text` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | 0.1.0, objc2-app-kit | [crates.io](https://crates.io/crates/objc2-core-text/0.3.2) · [repo](https://github.com/madsmtm/objc2) | none shipped — see Open questions |
| `objc2-foundation` | 0.3.2 | MIT | 0.1.0, objc2-app-kit, objc2-cloud-kit, objc2-core-data, objc2-core-image, objc2-metal, objc2-quartz-core | [crates.io](https://crates.io/crates/objc2-foundation/0.3.2) · [repo](https://github.com/madsmtm/objc2) | none shipped — see Open questions |
| `polling` | 3.11.0 | Apache-2.0 OR MIT | 0.1.0, alacritty_terminal | [crates.io](https://crates.io/crates/polling/3.11.0) · [repo](https://github.com/smol-rs/polling) | LICENSE-APACHE, LICENSE-MIT |

### Transitive runtime dependencies (linked into the binary)

| crate | version | licence (SPDX, per manifest) | required by | source | licence file in crate |
| --- | --- | --- | --- | --- | --- |
| `aho-corasick` | 1.1.5 | Unlicense OR MIT | regex-automata | [crates.io](https://crates.io/crates/aho-corasick/1.1.5) · [repo](https://github.com/BurntSushi/aho-corasick) | LICENSE-MIT |
| `arrayvec` | 0.7.8 | MIT OR Apache-2.0 | vte | [crates.io](https://crates.io/crates/arrayvec/0.7.8) · [repo](https://github.com/bluss/arrayvec) | LICENSE-APACHE, LICENSE-MIT |
| `base64` | 0.22.1 | MIT OR Apache-2.0 | alacritty_terminal | [crates.io](https://crates.io/crates/base64/0.22.1) · [repo](https://github.com/marshallpierce/rust-base64) | LICENSE-APACHE, LICENSE-MIT |
| `bitflags` | 2.13.2 | MIT OR Apache-2.0 | alacritty_terminal, dispatch2, objc2-app-kit, objc2-cloud-kit, objc2-core-data, objc2-core-foundation, objc2-core-graphics, objc2-core-text, objc2-core-video, objc2-foundation, objc2-io-surface, objc2-metal, objc2-quartz-core, rustix, vte | [crates.io](https://crates.io/crates/bitflags/2.13.2) · [repo](https://github.com/bitflags/bitflags) | LICENSE-APACHE, LICENSE-MIT |
| `cfg-if` | 1.0.5 | MIT OR Apache-2.0 | parking_lot_core, polling | [crates.io](https://crates.io/crates/cfg-if/1.0.5) · [repo](https://github.com/rust-lang/cfg-if) | LICENSE-APACHE, LICENSE-MIT |
| `cursor-icon` | 1.2.0 | MIT OR Apache-2.0 OR Zlib | vte | [crates.io](https://crates.io/crates/cursor-icon/1.2.0) · [repo](https://github.com/rust-windowing/cursor-icon) | LICENSE-APACHE, LICENSE-MIT, LICENSE-ZLIB |
| `dispatch2` | 0.3.1 | Zlib OR Apache-2.0 OR MIT | objc2-core-foundation, objc2-core-graphics | [crates.io](https://crates.io/crates/dispatch2/0.3.1) · [repo](https://github.com/madsmtm/objc2) | none shipped — see Open questions |
| `errno` | 0.3.14 | MIT OR Apache-2.0 | rustix, rustix-openpty, signal-hook-registry | [crates.io](https://crates.io/crates/errno/0.3.14) · [repo](https://github.com/lambda-fairy/rust-errno) | LICENSE-APACHE, LICENSE-MIT |
| `home` | 0.5.12 | MIT OR Apache-2.0 | alacritty_terminal | [crates.io](https://crates.io/crates/home/0.5.12) · [repo](https://github.com/rust-lang/cargo) | LICENSE-APACHE, LICENSE-MIT |
| `lock_api` | 0.4.14 | MIT OR Apache-2.0 | parking_lot | [crates.io](https://crates.io/crates/lock_api/0.4.14) · [repo](https://github.com/Amanieu/parking_lot) | LICENSE-APACHE, LICENSE-MIT |
| `log` | 0.4.34 | MIT OR Apache-2.0 | alacritty_terminal, vte | [crates.io](https://crates.io/crates/log/0.4.34) · [repo](https://github.com/rust-lang/log) | LICENSE-APACHE, LICENSE-MIT |
| `memchr` | 2.8.3 | Unlicense OR MIT | aho-corasick, regex-automata, vte | [crates.io](https://crates.io/crates/memchr/2.8.3) · [repo](https://github.com/BurntSushi/memchr) | LICENSE-MIT |
| `objc2-cloud-kit` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | objc2-app-kit, objc2-core-data | [crates.io](https://crates.io/crates/objc2-cloud-kit/0.3.2) · [repo](https://github.com/madsmtm/objc2) | none shipped — see Open questions |
| `objc2-core-image` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | objc2-app-kit | [crates.io](https://crates.io/crates/objc2-core-image/0.3.2) · [repo](https://github.com/madsmtm/objc2) | none shipped — see Open questions |
| `objc2-core-video` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | objc2-app-kit | [crates.io](https://crates.io/crates/objc2-core-video/0.3.2) · [repo](https://github.com/madsmtm/objc2) | none shipped — see Open questions |
| `objc2-encode` | 4.1.0 | MIT | objc2 | [crates.io](https://crates.io/crates/objc2-encode/4.1.0) · [repo](https://github.com/madsmtm/objc2) | none shipped — see Open questions |
| `objc2-io-surface` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | objc2-core-graphics, objc2-core-video | [crates.io](https://crates.io/crates/objc2-io-surface/0.3.2) · [repo](https://github.com/madsmtm/objc2) | none shipped — see Open questions |
| `objc2-metal` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | objc2-core-graphics | [crates.io](https://crates.io/crates/objc2-metal/0.3.2) · [repo](https://github.com/madsmtm/objc2) | none shipped — see Open questions |
| `objc2-quartz-core` | 0.3.2 | Zlib OR Apache-2.0 OR MIT | objc2-app-kit | [crates.io](https://crates.io/crates/objc2-quartz-core/0.3.2) · [repo](https://github.com/madsmtm/objc2) | none shipped — see Open questions |
| `parking_lot` | 0.12.5 | MIT OR Apache-2.0 | alacritty_terminal | [crates.io](https://crates.io/crates/parking_lot/0.12.5) · [repo](https://github.com/Amanieu/parking_lot) | LICENSE-APACHE, LICENSE-MIT |
| `parking_lot_core` | 0.9.12 | MIT OR Apache-2.0 | parking_lot | [crates.io](https://crates.io/crates/parking_lot_core/0.9.12) · [repo](https://github.com/Amanieu/parking_lot) | LICENSE-APACHE, LICENSE-MIT |
| `regex-automata` | 0.4.18 | MIT OR Apache-2.0 | alacritty_terminal | [crates.io](https://crates.io/crates/regex-automata/0.4.18) · [repo](https://github.com/rust-lang/regex) | LICENSE-APACHE, LICENSE-MIT |
| `regex-syntax` | 0.8.11 | MIT OR Apache-2.0 | regex-automata | [crates.io](https://crates.io/crates/regex-syntax/0.8.11) · [repo](https://github.com/rust-lang/regex) | LICENSE-APACHE, LICENSE-MIT |
| `rustix` | 1.1.5 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | alacritty_terminal, polling, rustix-openpty | [crates.io](https://crates.io/crates/rustix/1.1.5) · [repo](https://github.com/bytecodealliance/rustix) | LICENSE-APACHE, LICENSE-Apache-2.0_WITH_LLVM-exception, LICENSE-MIT |
| `rustix-openpty` | 0.2.0 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | alacritty_terminal | [crates.io](https://crates.io/crates/rustix-openpty/0.2.0) · [repo](https://github.com/sunfishcode/rustix-openpty) | LICENSE-APACHE, LICENSE-Apache-2.0_WITH_LLVM-exception, LICENSE-MIT |
| `scopeguard` | 1.2.0 | MIT OR Apache-2.0 | lock_api | [crates.io](https://crates.io/crates/scopeguard/1.2.0) · [repo](https://github.com/bluss/scopeguard) | LICENSE-APACHE, LICENSE-MIT |
| `serde` | 1.0.229 | MIT OR Apache-2.0 | alacritty_terminal, vte | [crates.io](https://crates.io/crates/serde/1.0.229) · [repo](https://github.com/serde-rs/serde) | LICENSE-APACHE, LICENSE-MIT |
| `serde_core` | 1.0.229 | MIT OR Apache-2.0 | bitflags, serde | [crates.io](https://crates.io/crates/serde_core/1.0.229) · [repo](https://github.com/serde-rs/serde) | LICENSE-APACHE, LICENSE-MIT |
| `signal-hook` | 0.4.5 | MIT OR Apache-2.0 | alacritty_terminal | [crates.io](https://crates.io/crates/signal-hook/0.4.5) · [repo](https://github.com/vorner/signal-hook) | LICENSE-APACHE, LICENSE-MIT |
| `signal-hook-registry` | 1.4.8 | MIT OR Apache-2.0 | signal-hook | [crates.io](https://crates.io/crates/signal-hook-registry/1.4.8) · [repo](https://github.com/vorner/signal-hook) | LICENSE-APACHE, LICENSE-MIT |
| `smallvec` | 1.16.2 | MIT OR Apache-2.0 | parking_lot_core | [crates.io](https://crates.io/crates/smallvec/1.16.2) · [repo](https://github.com/servo/rust-smallvec) | LICENSE-APACHE, LICENSE-MIT |
| `unicode-width` | 0.2.2 | MIT OR Apache-2.0 | alacritty_terminal | [crates.io](https://crates.io/crates/unicode-width/0.2.2) · [repo](https://github.com/unicode-rs/unicode-width) | LICENSE-APACHE, LICENSE-MIT |
| `vte` | 0.15.0 | Apache-2.0 OR MIT | alacritty_terminal | [crates.io](https://crates.io/crates/vte/0.15.0) · [repo](https://github.com/alacritty/vte) | LICENSE-APACHE, LICENSE-MIT |

### Compile-time-only dependencies (proc-macro closure)

| crate | version | licence (SPDX, per manifest) | required by | source | licence file in crate |
| --- | --- | --- | --- | --- | --- |
| `proc-macro2` | 1.0.107 | MIT OR Apache-2.0 | quote, serde_derive, syn | [crates.io](https://crates.io/crates/proc-macro2/1.0.107) · [repo](https://github.com/dtolnay/proc-macro2) | LICENSE-APACHE, LICENSE-MIT |
| `quote` | 1.0.47 | MIT OR Apache-2.0 | serde_derive, syn | [crates.io](https://crates.io/crates/quote/1.0.47) · [repo](https://github.com/dtolnay/quote) | LICENSE-APACHE, LICENSE-MIT |
| `serde_derive` | 1.0.229 | MIT OR Apache-2.0 | serde | [crates.io](https://crates.io/crates/serde_derive/1.0.229) · [repo](https://github.com/serde-rs/serde) | LICENSE-APACHE, LICENSE-MIT |
| `syn` | 3.0.6 | MIT OR Apache-2.0 | serde_derive | [crates.io](https://crates.io/crates/syn/3.0.6) · [repo](https://github.com/dtolnay/syn) | LICENSE-APACHE, LICENSE-MIT |
| `unicode-ident` | 1.0.26 | (MIT OR Apache-2.0) AND Unicode-3.0 | proc-macro2, syn | [crates.io](https://crates.io/crates/unicode-ident/1.0.26) · [repo](https://github.com/dtolnay/unicode-ident) | LICENSE-APACHE, LICENSE-MIT, LICENSE-UNICODE |

---

## Apple system frameworks (no third-party licence)

The binary links Apple's own frameworks, which are not third-party open-source
dependencies: they are supplied by the operating system and are covered by the Apple
software licence the user accepted with macOS / Xcode, not by anything in `Cargo.lock`.

Recorded from `otool -L` on the built arm64 binary and committed as a snapshot in
`licenses/apple-framework-linkage.txt`; `scripts/gen-notices.sh linkage` re-checks a
fresh build against it:

- `/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit`
- `/System/Library/Frameworks/Carbon.framework/Versions/A/Carbon`
- `/System/Library/Frameworks/CloudKit.framework/Versions/A/CloudKit`
- `/System/Library/Frameworks/CoreData.framework/Versions/A/CoreData`
- `/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation`
- `/System/Library/Frameworks/CoreGraphics.framework/Versions/A/CoreGraphics`
- `/System/Library/Frameworks/CoreImage.framework/Versions/A/CoreImage`
- `/System/Library/Frameworks/CoreText.framework/Versions/A/CoreText`
- `/System/Library/Frameworks/CoreVideo.framework/Versions/A/CoreVideo`
- `/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation`
- `/System/Library/Frameworks/Metal.framework/Versions/A/Metal`
- `/System/Library/Frameworks/QuartzCore.framework/Versions/A/QuartzCore`
- `/usr/lib/libSystem.B.dylib`
- `/usr/lib/libiconv.2.dylib`
- `/usr/lib/libobjc.A.dylib`

The `objc2`-family crates are Rust bindings *derived from Apple SDK headers*; see
[the objc2 licensing section](#the-objc2-family) for what that affects.

---

## Licence groups and texts

For each group below the crates are the ones whose shipped licence file matches the
canonical text committed under `licenses/` (compared with whitespace normalisation, and
with the licence's own copyright line excluded, since authors fill it in per project).
Anything that does not match is reproduced verbatim in the last section instead of being
abstracted away.

| SPDX | shipped-text matches | canonical text |
| --- | --- | --- |
| Unlicense | 0 | [`licenses/Unlicense.txt`](licenses/Unlicense.txt) |
| MIT | 32 | [`licenses/MIT.txt`](licenses/MIT.txt) |
| Apache-2.0 | 31 | [`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt) |
| Zlib | 1 | [`licenses/Zlib.txt`](licenses/Zlib.txt) |
| Apache-2.0 WITH LLVM-exception | 2 | [`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt) + [`LLVM-exception.txt`](licenses/LLVM-exception.txt) |
| Unicode-3.0 | 1 | [`licenses/Unicode-3.0.txt`](licenses/Unicode-3.0.txt) |

`licenses/Unlicense.txt` is committed even though no crate ships that file: it is the
other half of `aho-corasick` and `memchr`'s `Unlicense OR MIT` expression, quoted so the
dual licence can be read without a second source.

### Unlicense

Canonical text: [`licenses/Unlicense.txt`](licenses/Unlicense.txt)

No crate ships a file matching this text; it is committed only as the other
option of a dual-licensed dependency.

No crate ships an `Unlicense` text. It is the other option of `aho-corasick` and
`memchr`'s `Unlicense OR MIT` expression, committed here so that dual licence can
be read without a second source. Those two crates ship a `COPYING` file that only
states the dual-licensing:

- `aho-corasick 1.1.5: COPYING`
- `memchr 2.8.3: COPYING`

### MIT

Canonical text: [`licenses/MIT.txt`](licenses/MIT.txt)

Copyright notices as written in the shipped licence files:

- `Copyright (c) 2015 Andrew Gallant` — aho-corasick, memchr
- `Copyright (c) Ulrik Sverdrup "bluss" 2015-2023` — arrayvec
- `Copyright (c) 2015 Alice Maz` — base64
- `Copyright (c) 2014 The Rust Project Developers` — bitflags, log, regex-automata, regex-syntax
- `Copyright (c) 2014 Alex Crichton` — cfg-if
- `Copyright (c) 2023 Kirill Chibisov` — cursor-icon
- `Copyright (c) 2014 Chris Wong` — errno
- `Copyright (c) The Rust Project Developers` — libc
- `Copyright (c) 2016 The Rust Project Developers` — lock_api, parking_lot, parking_lot_core
- `Copyright (c) 2016-2019 Ulrik Sverdrup "bluss" and scopeguard developers` — scopeguard
- `Copyright (c) 2017 tokio-jsonrpc developers` — signal-hook, signal-hook-registry
- `Copyright (c) 2018 The Servo Project Developers` — smallvec
- `Copyright (c) 2015 The Rust Project Developers` — unicode-width
- `Copyright (c) 2016 Joe Wilm` — vte

Crates whose shipped licence file matches:

- `aho-corasick 1.1.5` — `LICENSE-MIT` (sha256 `0f96a83840e1…)
- `arrayvec 0.7.8` — `LICENSE-MIT` (sha256 `4da95ec4ecb6…)
- `base64 0.22.1` — `LICENSE-MIT` (sha256 `0dd882e53de1…)
- `bitflags 2.13.2` — `LICENSE-MIT` (sha256 `6485b8ed310d…)
- `cfg-if 1.0.5` — `LICENSE-MIT` (sha256 `378f5840b258…)
- `cursor-icon 1.2.0` — `LICENSE-MIT` (sha256 `c138726a726a…)
- `errno 0.3.14` — `LICENSE-MIT` (sha256 `8764a5976757…)
- `home 0.5.12` — `LICENSE-MIT` (sha256 `23f18e03dc49…)
- `libc 0.2.190` — `LICENSE-MIT` (sha256 `123a331b5dbf…)
- `lock_api 0.4.14` — `LICENSE-MIT` (sha256 `c9a75f18b9ab…)
- `log 0.4.34` — `LICENSE-MIT` (sha256 `6485b8ed310d…)
- `memchr 2.8.3` — `LICENSE-MIT` (sha256 `0f96a83840e1…)
- `parking_lot 0.12.5` — `LICENSE-MIT` (sha256 `c9a75f18b9ab…)
- `parking_lot_core 0.9.12` — `LICENSE-MIT` (sha256 `c9a75f18b9ab…)
- `polling 3.11.0` — `LICENSE-MIT` (sha256 `23f18e03dc49…)
- `proc-macro2 1.0.107` — `LICENSE-MIT` (sha256 `23f18e03dc49…)
- `quote 1.0.47` — `LICENSE-MIT` (sha256 `23f18e03dc49…)
- `regex-automata 0.4.18` — `LICENSE-MIT` (sha256 `6485b8ed310d…)
- `regex-syntax 0.8.11` — `LICENSE-MIT` (sha256 `6485b8ed310d…)
- `rustix 1.1.5` — `LICENSE-MIT` (sha256 `23f18e03dc49…)
- `rustix-openpty 0.2.0` — `LICENSE-MIT` (sha256 `23f18e03dc49…)
- `scopeguard 1.2.0` — `LICENSE-MIT` (sha256 `fb77f0a9c53e…)
- `serde 1.0.229` — `LICENSE-MIT` (sha256 `23f18e03dc49…)
- `serde_core 1.0.229` — `LICENSE-MIT` (sha256 `23f18e03dc49…)
- `serde_derive 1.0.229` — `LICENSE-MIT` (sha256 `23f18e03dc49…)
- `signal-hook 0.4.5` — `LICENSE-MIT` (sha256 `503558bfefe6…)
- `signal-hook-registry 1.4.8` — `LICENSE-MIT` (sha256 `503558bfefe6…)
- `smallvec 1.16.2` — `LICENSE-MIT` (sha256 `0b28172679e0…)
- `syn 3.0.6` — `LICENSE-MIT` (sha256 `23f18e03dc49…)
- `unicode-ident 1.0.26` — `LICENSE-MIT` (sha256 `23f18e03dc49…)
- `unicode-width 0.2.2` — `LICENSE-MIT` (sha256 `7b63ecd5f190…)
- `vte 0.15.0` — `LICENSE-MIT` (sha256 `e4c9b06fa850…)

### Apache-2.0

Canonical text: [`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt)

Copyright notices as written in the shipped licence files:

- `Copyright 2020 The Alacritty Project` — alacritty_terminal
- `Copyright 2023 Kirill Chibisov` — cursor-icon

Crates whose shipped licence file matches:

- `alacritty_terminal 0.26.0` — `LICENSE-APACHE` (sha256 `8884289e3969…)
- `arrayvec 0.7.8` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `base64 0.22.1` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `bitflags 2.13.2` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `cfg-if 1.0.5` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `cursor-icon 1.2.0` — `LICENSE-APACHE` (sha256 `b59affd015e5…)
- `errno 0.3.14` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `home 0.5.12` — `LICENSE-APACHE` (sha256 `8ada45cd9f84…)
- `libc 0.2.190` — `LICENSE-APACHE` (sha256 `62c7a1e35f56…)
- `lock_api 0.4.14` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `log 0.4.34` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `parking_lot 0.12.5` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `parking_lot_core 0.9.12` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `polling 3.11.0` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `proc-macro2 1.0.107` — `LICENSE-APACHE` (sha256 `62c7a1e35f56…)
- `quote 1.0.47` — `LICENSE-APACHE` (sha256 `62c7a1e35f56…)
- `regex-automata 0.4.18` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `regex-syntax 0.8.11` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `rustix 1.1.5` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `rustix-openpty 0.2.0` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `scopeguard 1.2.0` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `serde 1.0.229` — `LICENSE-APACHE` (sha256 `62c7a1e35f56…)
- `serde_core 1.0.229` — `LICENSE-APACHE` (sha256 `62c7a1e35f56…)
- `serde_derive 1.0.229` — `LICENSE-APACHE` (sha256 `62c7a1e35f56…)
- `signal-hook 0.4.5` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `signal-hook-registry 1.4.8` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `smallvec 1.16.2` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `syn 3.0.6` — `LICENSE-APACHE` (sha256 `62c7a1e35f56…)
- `unicode-ident 1.0.26` — `LICENSE-APACHE` (sha256 `62c7a1e35f56…)
- `unicode-width 0.2.2` — `LICENSE-APACHE` (sha256 `a60eea817514…)
- `vte 0.15.0` — `LICENSE-APACHE` (sha256 `62c7a1e35f56…)

### Zlib

Canonical text: [`licenses/Zlib.txt`](licenses/Zlib.txt)

Copyright notices as written in the shipped licence files:

- `Copyright (c) 2023 Kirill Chibisov` — cursor-icon

Crates whose shipped licence file matches:

- `cursor-icon 1.2.0` — `LICENSE-ZLIB` (sha256 `01e533bd94e8…)

### Apache-2.0 WITH LLVM-exception

Canonical text: [`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt) + [`licenses/LLVM-exception.txt`](licenses/LLVM-exception.txt)

Crates whose shipped licence file matches:

- `rustix 1.1.5` — `LICENSE-Apache-2.0_WITH_LLVM-exception` (sha256 `268872b9816f…)
- `rustix-openpty 0.2.0` — `LICENSE-Apache-2.0_WITH_LLVM-exception` (sha256 `268872b9816f…)

### Unicode-3.0

Canonical text: [`licenses/Unicode-3.0.txt`](licenses/Unicode-3.0.txt)

Copyright notices as written in the shipped licence files:

- `COPYRIGHT AND PERMISSION NOTICE` — unicode-ident
- `Copyright © 1991-2023 Unicode, Inc.` — unicode-ident

Crates whose shipped licence file matches:

- `unicode-ident 1.0.26` — `LICENSE-UNICODE` (sha256 `f7db81051789…)

---

## The objc2 family

The `objc2` family is the one place that needs care beyond the tables above: the
crates.io manifest states a licence expression, but the published crates ship no text.

- Manifest expressions in this build: `MIT` for `block2`, `objc2`, `objc2-encode`, `objc2-foundation`;
  `Zlib OR Apache-2.0 OR MIT` for `dispatch2`, `objc2-app-kit`, `objc2-cloud-kit`, `objc2-core-data`, `objc2-core-foundation`, `objc2-core-graphics`, `objc2-core-image`, `objc2-core-text`, `objc2-core-video`, `objc2-io-surface`, `objc2-metal`, `objc2-quartz-core`.
- **No licence text is shipped inside those published crates.** `cargo metadata` reports
  a licence expression, but the `.crate` archives contain no `LICENSE*` file at all, so
  there is nothing in the vendored source to quote. The texts below are therefore taken
  from the upstream repository, pinned to a specific commit, and committed here so the
  release does not depend on GitHub being reachable.

Pinned upstream evidence (see `licenses/pins.json`, whose sha256 is quoted at the top of
this file):

```
MIT.txt
  https://raw.githubusercontent.com/spdx/license-list-data/31846481de7ea17d92f2f46fad76abe58b2e9bb0/text/MIT.txt
  sha256 b05785f9f18e6716bab63424b11454513b9943a222595b70411009202fc592b5
Apache-2.0.txt
  https://raw.githubusercontent.com/spdx/license-list-data/31846481de7ea17d92f2f46fad76abe58b2e9bb0/text/Apache-2.0.txt
  sha256 074e6e32c86a4c0ef8b3ed25b721ca23aca83df277cd88106ef7177c354615ff
Zlib.txt
  https://raw.githubusercontent.com/spdx/license-list-data/31846481de7ea17d92f2f46fad76abe58b2e9bb0/text/Zlib.txt
  sha256 bfb1112d49db5b1daecdfef24bd7e2f3ea0bafb33aa67aa0ab51e2bf8407c03d
Unlicense.txt
  https://raw.githubusercontent.com/spdx/license-list-data/31846481de7ea17d92f2f46fad76abe58b2e9bb0/text/Unlicense.txt
  sha256 0bdebfeda07d45dada625ae1317c6f833186e798b171d0db640bcf32e92a8240
Unicode-3.0.txt
  https://raw.githubusercontent.com/spdx/license-list-data/31846481de7ea17d92f2f46fad76abe58b2e9bb0/text/Unicode-3.0.txt
  sha256 f7db81051789b729fea528a63ec4c938fdcb93d9d61d97dc8cc2e9df6d47f2a1
LLVM-exception.txt
  https://raw.githubusercontent.com/spdx/license-list-data/31846481de7ea17d92f2f46fad76abe58b2e9bb0/text/LLVM-exception.txt
  sha256 e34c58338bd89d43e709e226610d8f32b3e3c47f4ad9a99a8dc1d4ac7842488e
objc2-LICENSE.md
  https://raw.githubusercontent.com/madsmtm/objc2/refs/tags/objc2-0.6.5/LICENSE.md
  sha256 7f976f7e9cb2d87df7230606feb932c3f21ac0e664045a775b600046ff850c54
objc2-LICENSE-MIT.txt
  https://raw.githubusercontent.com/madsmtm/objc2/8a5abacd90d5403fc0463459a35826f892b9142e/LICENSE-MIT.txt
  sha256 f6a2e783669a0dd7a2c3b8a4f76db40ae49033e66e1bdc30f3aa25fe5577b070
objc2-LICENSE-ZLIB.txt
  https://raw.githubusercontent.com/madsmtm/objc2/8a5abacd90d5403fc0463459a35826f892b9142e/LICENSE-ZLIB.txt
  sha256 fced3e7fedc235fa83ed0108b0df74d86b435d06436356f46917a903cbadd80a
objc2-LICENSE-APACHE.txt
  https://raw.githubusercontent.com/madsmtm/objc2/8a5abacd90d5403fc0463459a35826f892b9142e/LICENSE-APACHE.txt
  sha256 0c9a5f6fe28e27015e12506ffe58c9e007cead169af9670ec15c1df0d14e4f2a
```

What that document says, quoted from the pinned `licenses/objc2-LICENSE.md`:

> The crates `objc2`, `block2`, `objc2-foundation` and `objc2-encode` are
> [currently][#23] licensed under [the MIT license][MIT]. All other crates are
> trio-licensed under the [Zlib], [Apache-2.0] or [MIT] license, at your option.

Two consequences we record rather than resolve:

1. **A licence transition is in flight.** The upstream note links issue `#23` and its
   own manifest comment for `objc2-foundation` reads `All future contributions are
   licensed under "Zlib OR Apache-2.0 OR MIT"` — so the MIT label on the crates in
   this build is the *current* licence of the code as published at those versions, not a
   statement about later ones. Choosing MIT for these versions is supported by the
   crates.io metadata plus the pinned text; anyone redistributing newer versions must
   re-read that document rather than reuse this section.
2. **Apple SDK derivation.** The same document states the crates are derived from Apple
   SDKs shipped with Xcode and that the Xcode end-user licence agreement
   (`xcodebuild -license`) may bear on distributing derived works. NotchQuake links
   Apple's own frameworks on the user's machine rather than redistributing Apple code,
   and the bindings themselves are small Rust declarations, but that is a description of
   the situation, not a legal conclusion — see [Open questions](#open-questions).

Attribution to use for these crates:

```
block2 0.6.2
objc2 0.6.5
objc2-encode 4.1.0
objc2-foundation 0.3.2
  Copyright Mads Marquart and the objc2 project contributors
  MIT licence — licenses/objc2-LICENSE-MIT.txt (pinned from upstream)

dispatch2 0.3.1
objc2-app-kit 0.3.2
objc2-cloud-kit 0.3.2
objc2-core-data 0.3.2
objc2-core-foundation 0.3.2
objc2-core-graphics 0.3.2
objc2-core-image 0.3.2
objc2-core-text 0.3.2
objc2-core-video 0.3.2
objc2-io-surface 0.3.2
objc2-metal 0.3.2
objc2-quartz-core 0.3.2
  Copyright Mads Marquart and the objc2 project contributors
  Zlib OR Apache-2.0 OR MIT, at your option
  — licenses/objc2-LICENSE-ZLIB.txt / objc2-LICENSE-APACHE.txt / objc2-LICENSE-MIT.txt
```

The family pulls in more than the crate names suggest: `objc2-app-kit` and
`objc2-core-graphics` enable `objc2-cloud-kit`, `objc2-core-image`, `objc2-metal`,
`objc2-quartz-core`, `objc2-core-video`, `objc2-io-surface` and `dispatch2` through
their default features, and `otool -L` on the built binary confirms the resulting
frameworks (CloudKit, CoreData, CoreImage, Metal, QuartzCore, CoreVideo) are linked.
That is why those framework crates appear in the runtime table even though no source
file in `src/` mentions them.

---

## Verbatim licence texts

None. Every vendored licence file matched its committed canonical text.

---

## Out of scope for this build

Present in `Cargo.lock` but resolved only for targets NotchQuake does not ship;
`cargo metadata --filter-platform aarch64-apple-darwin` excludes them. They are not linked, not
attributed, and listed here so a reader can see the difference between "in the lock
file" and "in the app":

- `atomic-waker 1.1.2`
- `concurrent-queue 2.5.0`
- `crossbeam-utils 0.8.23`
- `fastrand 2.5.0`
- `futures-io 0.3.34`
- `hermit-abi 0.5.3`
- `linux-raw-sys 0.12.1`
- `miow 0.6.1`
- `pin-project-lite 0.2.17`
- `piper 0.2.5`
- `redox_syscall 0.5.18`
- `windows-link 0.2.1`
- `windows-sys 0.59.0`
- `windows-sys 0.61.2`
- `windows-targets 0.52.6`
- `windows_aarch64_gnullvm 0.52.6`
- `windows_aarch64_msvc 0.52.6`
- `windows_i686_gnu 0.52.6`
- `windows_i686_gnullvm 0.52.6`
- `windows_i686_msvc 0.52.6`
- `windows_x86_64_gnu 0.52.6`
- `windows_x86_64_gnullvm 0.52.6`
- `windows_x86_64_msvc 0.52.6`

`cargo-deny` reports the two `windows-sys` entries in `Cargo.lock` as a duplicate-version
warning. That is expected: the two versions are gated to different target cfgs and only
one is ever built for `aarch64-apple-darwin`. `deny.toml` keeps the check at `warn`.

---

## Open questions

Recorded, not resolved. None of these blocks a build; each is a question for whoever
owns the release decision (and, where it is genuinely legal, for a lawyer).

1. **objc2 ↔ Xcode SLA.** Upstream itself calls the distribution position "unclear"
   while still using standard SPDX identifiers. NotchQuake distributes only its own
   compiled binary plus Apple's system frameworks, but whether the generated bindings
   count as a derived work of the SDK is not answered by anything in the crate metadata.
2. **Crates that ship no licence text.** 16 crates, all of them
   in the objc2 family (the generator fails if that ever stops being true):
   `block2 0.6.2`, `dispatch2 0.3.1`, `objc2 0.6.5`, `objc2-app-kit 0.3.2`, `objc2-cloud-kit 0.3.2`, `objc2-core-data 0.3.2`, `objc2-core-foundation 0.3.2`, `objc2-core-graphics 0.3.2`, `objc2-core-image 0.3.2`, `objc2-core-text 0.3.2`, `objc2-core-video 0.3.2`, `objc2-encode 4.1.0`, `objc2-foundation 0.3.2`, `objc2-io-surface 0.3.2`, `objc2-metal 0.3.2`, `objc2-quartz-core 0.3.2`
   The SPDX expression is present in the manifest, the text is absent from the published
   crate. The texts quoted above are pinned from upstream and committed in `licenses/`;
   if a future crate has no text, pin it the same way — never fetch it at release time.
3. **Copyright lines missing from some shipped licence files.** 10
   crates ship a licence file with no copyright line at all: `home`, `polling`, `proc-macro2`, `quote`, `rustix`, `rustix-openpty`, `serde`, `serde_core`, `serde_derive`, `syn`.
   Their manifest `authors` field is reported in the tables above, but no attribution
   line can be quoted from the licence file itself, so the exact copyright holder for
   those is left unstated here rather than guessed.
4. **Fira Code is not bundled.** NotchQuake resolves monospace fonts through CoreText at
   runtime and ships no font files; if a font were ever embedded in the bundle, its own
   licence (Fira Code is OFL-1.1) would have to be added here and in `NOTICE.md`.
5. **No NOTICE files in this dependency graph.** None found.
   Verified across all 49 vendored crate roots. Apache-2.0 section 4.4's
   notices requirement therefore has nothing to carry forward, and NOTICE.md is
   Barnstorm's own attribution file, not a pass-through of upstream notices.

---

## Bundle these files in the release archive

A distribution archive should contain:

- `NOTICE.md` — the short attribution summary
- `THIRD-PARTY-NOTICES.md` — this file
- `LICENSE` — the NotchQuake MIT licence
- `licenses/` — every canonical and pinned licence text referenced above

`scripts/gen-notices.sh check` must pass on the commit the release is cut from.

