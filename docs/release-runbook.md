# NotchQuake release runbook

Status: **stage 1 (build/package/CI) is implemented and locally verified; stage 2
(signing + notarization) is BLOCKED — no Developer ID certificate or notary
credentials exist, and nothing in this repository has ever been signed with a
Developer ID or notarized.** See `docs/project-state.md` for the authoritative
blocker list. This document is the plan, not a record of a working pipeline.

NotchQuake is macOS-only, arm64-only. There is no Intel, Linux, or Windows build.

---

## Stage 1 — build, package, CI (implemented)

| Piece | File | Verified locally |
| --- | --- | --- |
| CI gate (push/PR on `main`) | `.github/workflows/gate.yml` | steps mirror the local commands; YAML validated |
| Release build + gated publish | `.github/workflows/release.yml` | build job validated; publish job needs credentials |
| Bundle assembly | `scripts/build-bundle.sh` | yes — plist templating, version/minos agreement, arm64 check |
| Deterministic archive + checksums | `scripts/package-artifact.sh` | yes — runs both packers twice, compares tar.gz and zip sha256, writes SHA256SUMS only after both pass |
| Release prep (gates → archive → signed tag) | `scripts/release.sh` | gates + archive path; refuses to publish |
| Bundle metadata template | `packaging/Info.plist` | tracked; `plutil -lint` clean |

Runner label: **`macos-26`** (arm64). `macos-14` was retired from GitHub Actions
on 2026-11-02 (actions/runner-images#15226) and is not usable; `macos-26` is the
arm64 label behind `macos-latest`. Do not use `macos-26-large`/`macos-26-intel`
or `macos-15-large` — those are Intel, and an Intel artifact is not a NotchQuake
build. `macos-15` is still a valid arm64 label if `macos-26` ever becomes
unavailable.

Version plumbing (one source of truth, `Cargo.toml` `[package] version`):

- `CFBundleShortVersionString` ← Cargo version, substituted into
  `packaging/Info.plist` at bundle-assembly time.
- `CFBundleVersion` ← `<version>+<8-char commit>`: deterministic, so rebuilding
  the same commit yields the same build number.
- `LSMinimumSystemVersion` ← `13.0`, which is also `MACOSX_DEPLOYMENT_TARGET`
  used for the compile. `build-bundle.sh` reads `minos` out of the binary's
  `LC_BUILD_VERSION` and **fails if it disagrees with the plist** — a plist that
  lies about the minimum OS produces "app is damaged" on too-old machines instead
  of a clean refusal.
- `NSHumanReadableCopyright` ← `Copyright © 2026 Barnstorm LLC` plus the
  MIT/notice pointer. No secrets, no team ID, no certificate names in the plist.
- `LSUIElement` mirrors the existing `setActivationPolicy(.Accessory)` call.

The bundle is **deleted and rebuilt from scratch** every time (`rm -rf "$APP"`
first, `NQ_APP` overrides the destination), so a stale gitignored `NotchQuake.app/`
— or any leftover member of one, including a stale `Contents/_CodeSignature` or an
old `Contents/Resources/` file — can never survive into the tested or shipped
artifact. It then seals the bundle with an **ad-hoc** signature, because a
bundle with `Contents/Resources` and no `_CodeSignature` fails
`codesign --verify --strict` with "code has no resources but signature indicates
they must be present" — the linker's implicit signature seals the Mach-O only.
That is why the previous bundle's checks were meaningless: its plist had 4 keys and
its signature sealed **0** files. Ad-hoc is a local signature only: `spctl` rejects
it and notarization will never accept it.

The old `launch.sh` failure message ("stale bundle signature") was actually launchd
refusing a **regular file** named `NotchQuake.app` (spawn error 162/163);
`build-bundle.sh` removes a non-directory at the bundle path and `launch.sh` now
reports the real `open -n` error instead of asserting a cause.

Artifact determinism: mtimes **and atimes** pinned to 1980-01-01, uid/gid 0, no
AppleDouble (`._*`) or `.DS_Store` entries, `COPYFILE_DISABLE=1`, `gzip -n` (no
embedded name/mtime), xattrs cleared from the staging tree, and
`bsdtar --format ustar`. The ustar choice is a measured fix, not a style
preference: bsdtar's default pax format writes atime/ctime as extended records,
so simply reading a file between two archive runs changed the sha256. With ustar,
two archive runs after a read produce identical bytes (verified). ustar's 100-char
path limit is not a constraint here — the longest entry is ~55 chars.

One claim to keep honest: the **tar** archive is not "sorted" — bsdtar has no
`--sort` option, so entry order is directory-traversal order. Only the **zip**
writer sorts its entries explicitly. Determinism therefore rests on the double
build, not on an ordering guarantee: the script runs **both** packers twice over
the same staged tree and compares sha256 for the tar.gz and the zip, and writes
`SHA256SUMS` only after both comparisons pass. A failed comparison exits non-zero
with no checksums file, so checksums can never describe bytes the script did not
certify.

Determinism is therefore a checked property, not a claim. Verified 2026-11-12: two
independent bundle builds into different `NQ_APP` directories produced the same
tar.gz sha256 (`ca09ef19…`) and the same zip sha256 (`19f5d5b9…`); extracted
bundles pass `codesign --verify --strict`. Re-verified 2026-11-13 against a
from-scratch bundle: both packers byte-identical across runs, and injecting a
deliberate run-to-run perturbation made the script exit non-zero **without**
writing `SHA256SUMS` — the guard, not just the happy path, was exercised.

Licence artefacts ship inside the bundle (`Contents/Resources/`: `NOTICE.md`,
`THIRD-PARTY-NOTICES.md`, `LICENSE`, `licenses/`), and `scripts/release.sh` plus
the release workflow assert their presence **inside the archive**. The notices
themselves stay generated by `scripts/gen-notices.sh` and are not hand-edited.

There is no tag-triggered release workflow on purpose. Release publication
requires an explicit `workflow_dispatch` with `publish: true`; the tag itself is
created locally with `git tag -s` (see stage 3).

Where the tag checks actually live: **both** jobs of `release.yml` re-run them —
the build job verifies the tag type and signature, detaches to
`refs/tags/<tag>^{commit}`, and only then runs the gates, bundle, and archive; the
publish job repeats the same verification and detach before it signs anything, and
its `gh release create --target` is the tag's commit captured in that step
(`TAG_COMMIT`), not `$GITHUB_SHA`. So the bytes built, the bytes signed, and the
release's target commit all come from the verified signed tag. The version-match
check runs after the detach in both jobs, i.e. against the tagged `Cargo.toml`.

### GitHub Environment `release` (must be created — it does not exist yet)

`publish` declares `environment: release`. That declaration gates nothing by
itself: GitHub **creates the environment silently on first run**, and an
environment with no required reviewers and no protection rules auto-approves.
Before stage 2 is ever dispatched with `publish: true`, configure it in the
repository:

```bash
# Settings → Actions → Environments → New environment, or via gh:
gh api -X PUT "repos/<org>/<repo>/environments/release" -f name=release
# then add required reviewers (users or teams) in the Settings UI; the API
# route for reviewers is environment-specific and requires the reviewer's
# id/login — do it in the UI unless you verify the exact payload first.
```

Required reviewers are the human approval gate: a publish run waits for an
approval from someone other than the person who dispatched it. Until that exists,
the honest protection is simply never dispatching `publish: true` — which is the
current state, since the secrets check fails closed anyway.

---

## Stage 2 — Developer ID signing and notarization (blocked)

**Nothing here works yet.** It requires credentials Barnstorm LLC does not have.
Do not run `scripts/sign-notarize.sh` expecting success, and do not treat its
existence as evidence that signing is verified. Its job is to refuse loudly
until the inputs exist.

### Credentials to obtain

1. **Developer ID Application certificate** for Barnstorm LLC, with its private
   key. Requires the "App Development with Distribution" capability on the Apple
   Developer account. Apple issues the certificate; you install it in a keychain
   so the private key sits beside it, then export **identity + key** as a
   passphrase-protected `.p12`.
   - A self-signed certificate is **not** a Developer ID certificate.
   - An ad-hoc signature (`codesign -s -`) is not distributable: Gatekeeper
     rejects it on another machine, and notarization will never accept it.
   - Verify the identity chain, never `Signature=`:
     `security find-identity -v -p codesigning`, then
     `codesign -dvv <bundle>` and read `Authority=Developer ID Application: …`
     and `TeamIdentifier=…`. The old scripts' `grep -qE '^Signature=Developer'`
     check was wrong — it does not distinguish a real Developer ID chain from an
     ad-hoc or self-signed identity.
2. **Notary Service API key** (App Store Connect → Users and Access →
   Integrations → Keys → Mobile Account Secrets; "Notary Service" access;
   download a `.p8`). Gives you `NOTARY_API_KEY` (the `.p8`), `NOTARY_KEY_ID`,
   and `NOTARY_ISSUER`. `xcrun notarytool` works with it; the earlier claim that
   submission was web-portal-only was incorrect.
   - **Justified alternative:** per-user Apple ID credentials
     (`--apple-id/--password/--team-id` with an app-specific password). It works,
     but it puts a person's identity into CI, breaks when that person leaves or
     turns on 2FA, and cannot be scoped or rotated independently — so the API key
     is the right choice and is what the workflow expects.
3. An Apple Developer **Team ID** (appears in the identity line) — needed only to
   confirm the chain, never stored in the repo.

### GitHub Actions secrets (and variables)

Required, all **secrets** (never repository variables, never committed, never
echoed):

| Secret | Contents | Notes |
| --- | --- | --- |
| `DEVID_P12_B64` | `base64` of the `.p12` | single line; decode into `$RUNNER_TEMP`, delete in an `if: always()` step |
| `DEVID_P12_PASS` | the `.p12` passphrase | never printed |
| `NOTARY_API_KEY` | the `.p8` text (or a runner-local path) | staged at `0600`, deleted on exit |
| `NOTARY_KEY_ID` | Key ID UUID | not secret in practice, but keep it with the others |
| `NOTARY_ISSUER` | Issuer ID UUID | ditto |

Repository **variable** (optional, non-secret): `NQ_SIGNING_IDENTITY`, default
`Developer ID Application: Barnstorm LLC`. Keep the identity string out of
secrets so a rename is not a secret rotation.

Set them with:

```bash
# macOS `base64` has no `-A`; use `-i`, which writes ONE line (what a secret wants).
gh secret set DEVID_P12_B64 < <(base64 -i .signing/BarnstormLLC.p12)
gh secret set DEVID_P12_PASS            # prompts, never on the command line
gh secret set NOTARY_API_KEY   < NotaryService.p8
gh secret set NOTARY_KEY_ID
gh secret set NOTARY_ISSUER
```

Sanity-check the round trip before trusting the secret (macOS `base64 --decode`
does fail loudly on corrupt input, and the workflow also refuses an empty decode):

```bash
base64 -i .signing/BarnstormLLC.p12 | base64 --decode | cmp - .signing/BarnstormLLC.p12 && echo round-trip OK
security find-identity -v -p codesigning        # confirm the identity string and Team ID
```

`.signing/` is gitignored. Never commit a `.p12`, a `.p8`, a passphrase, or a
`security find-identity` dump into the repo, a log, or a commit message.

### Ephemeral keychain discipline

`scripts/sign-notarize.sh` does the keychain work, and CI must never touch the
login keychain:

1. `security create-keychain` in a private temp dir (`$RUNNER_TEMP` on CI,
   `mktemp -d` locally) with a random password from `uuidgen`.
2. `security set-keychain-settings -lut 21600` + `security unlock-keychain`.
3. `security import <p12> -k <ephemeral> -P "$pass" -T /usr/bin/codesign -A`.
4. `security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k <kcpass> <ephemeral>`
   — without this, `codesign` blocks on an interactive ACL prompt that a CI
   runner can never answer.
5. Prepend it to `security list-keychains -d user`, and **restore the previous
   list** before `security delete-keychain` in the exit trap.

The runner is ephemeral anyway, but the trap makes local runs safe too: the
keychain and the decoded `.p12` are removed even on failure, and no secret value
is ever echoed (`set -x` must stay off in these steps).

### Signing order and options

```bash
codesign --force --options runtime --timestamp --sign "$SHA" NotchQuake.app/Contents/MacOS/notchquake
codesign --force --options runtime --timestamp --sign "$SHA" NotchQuake.app
```

- Inner executable first, then the bundle: `codesign` on a directory does not
  re-sign the embedded binary.
- `--options runtime` (hardened runtime) and `--timestamp` (secure timestamp) are
  notarization prerequisites.
- **No entitlements.** The Carbon hotkey and the PTY need none, and the app is
  deliberately unsandboxed. If a future feature needs one, add
  `packaging/notchquake.entitlements` and pass `--entitlements` — and re-check
  that the hotkey still registers.

### Notarize, staple, verify

```bash
ditto -c -k --keepParent NotchQuake.app notarize-input.zip
xcrun notarytool submit notarize-input.zip \
  --key "$KEY" --key-id "$KEY_ID" --issuer "$ISSUER" --wait --json   # require status == Accepted
xcrun stapler staple NotchQuake.app
xcrun stapler validate NotchQuake.app
codesign --verify --strict --verbose=2 NotchQuake.app
spctl -a -vv -t exec NotchQuake.app
```

- Staple **before** the final archive, so the ticket is inside what users
  download; re-run `scripts/package-artifact.sh` after stapling.
- `Accepted` is the only acceptable status. `Invalid` → read the notary log
  (runtime-violation issues usually mean a missing timestamp or a hardened-runtime
  violation); do not staple, do not publish.
- Stapling matters offline: without the ticket a first launch on a fresh machine
  requires an online check.

---

## Stage 3 — tag, publish, validate

1. Bump `Cargo.toml` version and add a `CHANGELOG.md` section for the tag;
   `scripts/release.sh v0.2.0` runs the gates and produces `dist/` (no tag).
2. Create a **signed annotated** tag: `git tag -s v0.2.0 -m "NotchQuake v0.2.0"`.
   Verify before pushing. A tag object's signature is a `-----BEGIN PGP
   SIGNATURE-----` block appended **after** the message, not a `gpgsig` header —
   `gpgsig` is only how signed *commit* objects store it, and
   `git for-each-ref %(signature)` is empty for tag objects. So:
   ```bash
   git verify-tag v0.2.0                                   # local, has your key
   git cat-file tag v0.2.0 | grep -qE '^-----BEGIN (PGP|SSH) SIGNATURE-----'
   ```
   Never fall back to an unsigned or lightweight tag — an unsigned release tag is
   not attributable. Push it deliberately: `git push origin refs/tags/v0.2.0`.
3. Run `release-build` with `tag: v0.2.0`, `publish: true`. The publish job
   waits for a GitHub Environment (`release`) approval if required reviewers are
   configured (see stage 1), then re-checks the tag type and signature, detaches
   to the tag's commit, re-checks version agreement against the tagged
   `Cargo.toml`, and checks the presence of all five secrets — then signs,
   notarizes, staples, verifies, and uploads `NotchQuake-<v>.tar.gz`,
   `NotchQuake-<v>.zip`, `SHA256SUMS`, `NOTICE.md`, `THIRD-PARTY-NOTICES.md`, and
   `LICENSE` to a release whose `--target` is that tag commit.
4. **Clean-Mac Gatekeeper validation** — mandatory before announcing anything.
   On a Mac that never had this app and never had the certificate:
   ```bash
   shasum -a 256 -c SHA256SUMS        # checksums must match the release assets
   tar -xzf NotchQuake-<v>.tar.gz     # tar.gz preserves the .app tree exactly
   xcrun stapler validate NotchQuake.app
   spctl -a -vv -t exec NotchQuake.app      # expected: accepted, source=Notarized Developer ID
   open -n NotchQuake.app             # launches, pill appears, Option+Space toggles
   ```
   `source=Notarized Developer ID` is the pass condition. `rejected`,
   `unsigned`, or a quarantine dialog means the release is not done.
5. Record the release in `docs/project-state.md` with the commit, tag, artifact
   sha256, notary request UUID, and the clean-Mac result.

---

## Future: Barnstorm Homebrew cask (does not exist yet)

**There is no Barnstorm tap and no cask today, and none of the prerequisites
below are in place.** This is a checklist for later, not a claim of availability
or an invitation to add a `Casks/` directory now.

Prerequisites, in order:

1. A public GitHub repository serving the tap, e.g. `barnstorm/homebrew-tap`,
   containing `Casks/notchquake.rb`. Homebrew taps are repos; the cask name comes
   from the path.
2. A stable, **permanent** download URL with a fixed filename per version
   (`https://github.com/<org>/<repo>/releases/download/v<version>/NotchQuake-<version>.tar.gz`)
   and the `sha256` of that exact asset. Checksums must be stable, which is why
   the artifact is deterministic.
3. A signed **and notarized** bundle with a stapled ticket. A cask install runs
   Gatekeeper; an unsigned or unstapled app fails there, so stage 2 is a hard
   prerequisite.
4. Cask metadata honesty: `version`, `sha256`, `name "NotchQuake"`,
   `desc`, `homepage`, `url`, `app "NotchQuake.app"`. macOS arm64 only, and the
   supported way to say that in a cask is:
   ```ruby
   depends_on arch: :arm64            # hard requirement: refuses Intel machines
   depends_on macos: :ventura         # matches LSMinimumSystemVersion (13.0)
   ```
   `depends_on arch:` accepts `:arm64` and `:intel`/`:x86_64`; `depends_on
   :macos` (bare) and `depends_on macos: <symbol>` (Homebrew uses `:sonoma`,
   `:ventura`, etc. as OS version symbols) are both valid, and `on_arm`/
   `on_intel` blocks exist in the cask DSL as well. Use the `depends_on` form:
   a *requirement* is what actually refuses an Intel machine, while an `on_arm`
   block only conditionally wraps values — and per Homebrew's own cask DSL
   comment, `on_arm`/`on_intel` blocks are evaluated on EVERY OS, so a macOS
   dependency written inside one applies everywhere. Do not claim Intel
   support.
5. `brew audit --strict --new notchquake` and `brew style` pass before the cask
   is merged. There is **no `--new-cask` flag** — `brew audit --new` is the one
   flag, it applies to whichever formula or cask is named, and it implies
   `--strict` and `--online`. The cask must also be verifiable from a clean
   machine (`brew install --cask barnstorm/tap/notchquake`).
6. Version bumps stay in one place. Today that place is `Cargo.toml`; a cask adds
   a second version field plus a checksum. A release bump must update
   `Cargo.toml`, `CHANGELOG.md`, and the cask together — automate it in the
   release workflow rather than trusting a human to remember.
7. Licence obligations survive distribution: the notices are already inside
   `Contents/Resources/`, so a cask install carries them. Keep that true — do not
   slim the bundle to shave bytes.
8. Uninstall correctness: `brew uninstall --cask notchquake` must leave no
   orphaned state that the app depends on. Note that preferences live in
   `~/Library/Preferences/local.notchquake.plist` and are intentionally **not**
   removed by an uninstall; document that in the cask stanza if it matters.

Do not add the cask until 1–4 are real and a notarized artifact has passed
clean-Mac validation.
