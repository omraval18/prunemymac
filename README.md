# prunemymac

A fast, lightweight Mac cleaner and health monitor written in Rust with [GPUI](https://gpui.rs) (Zed's GPU-accelerated UI framework). Single 18 MB binary, ~75 MB RAM, ~2% CPU idle.

```sh
cargo run --release -p prunemymac        # run
scripts/bundle.sh                      # build target/prunemymac.app
scripts/release.sh                     # build a universal Mac DMG and checksum
cargo test -p macscrap-core              # engine tests
cargo run --release -p macscrap-core --example smoke   # read-only scan of this Mac, prints everything
```

Needs Rust only. No Xcode required (GPUI's `runtime_shaders` feature compiles Metal shaders at runtime).

To build a universal release, install the Rust target for the other Mac architecture with `rustup target add x86_64-apple-darwin` on Apple Silicon or `rustup target add aarch64-apple-darwin` on Intel. `scripts/release.sh` builds both architectures and packages them in a DMG. Builds are ad-hoc signed and are not notarized.

For complete results, give `prunemymac.app` **Full Disk Access** (System Settings → Privacy & Security). Without it, Mail, Safari and other apps' containers are skipped.

## Features

| Page | What it does | How |
|---|---|---|
| **Health** | Disk free, memory pressure, CPU load + temperature, battery, network. Flags heat, memory pressure and low disk; sends a macOS notification if the CPU stays ≥ 85 °C for 30 s | `sysinfo` (Apple Silicon die sensors via IOHID), `sysctl kern.memorystatus_level`, `statfs`, `pmset` |
| **Cleanup** | Caches, logs, Xcode/simulator junk, package-manager caches, Electron app caches, old installers, iOS backups, local AI models | Rule file `rules/junk.toml` (globs, age, safe/review, delete/trash) |
| **Large & Old Files** | Files ≥ 100 MB / 500 MB / 1 GB in your home folder, flagged when unused for 6+ months | Parallel walker with pruning (≈ 800k files in ~6 s) |
| **Duplicates** | Byte-identical files ≥ 1 MB; keeps the oldest copy by default | size → xxh3 of first 16 KB → full BLAKE3; hard links ignored |
| **Uninstaller** | App + caches, prefs, containers, group containers, launch agents/daemons, privileged helpers, crash reports | Matches bundle ID / team-prefixed IDs / exact app name across ~20 locations |
| **Performance** | Processes grouped by app, third-party launch agents/daemons with Running / Broken / Unsigned badges (Remove), plus maintenance: flush DNS, free RAM, reindex Spotlight, thin Time Machine snapshots | `launchctl`, `codesign`; maintenance asks for your admin password via the standard macOS prompt |
| **Protection** | Malware quick scan + persistence audit; optional ClamAV deep scan | See below |
| **Network** | Sampled established TCP connections and local listening TCP ports / bound UDP sockets visible to the current user, with review indicators for processes running from Downloads or temporary folders | `lsof`; refreshes while the Network page is open |

The Connections view is a snapshot, not a complete request log: it may miss short-lived connections and other users' processes. It does not show UDP connections, connection direction, HTTPS URLs, or request contents. Review indicators are clues, not malware verdicts.

The Ports view can request a graceful stop only for verified third-party app processes owned by the current user on ports 1024 or above. System, privileged, and unverified processes are protected. Command-line programs and scripts remain protected because their importance cannot be determined reliably. Even a verified app can have unsaved work or provide a service, so the app asks for confirmation before stopping a port owner. The process and port are checked again at confirmation.

### Malware detection (all open source / built in)

- **YARA via [`yara-x`](https://github.com/VirusTotal/yara-x)** (VirusTotal's pure-Rust engine, BSD-3) embedded in the app.
- **Apple's XProtect signatures** (~500 rules) are loaded at runtime from `/Library/Apple/System/Library/CoreServices/XProtect.bundle`. They stay up to date through macOS updates, and prunemymac never bundles or redistributes them.
- **Your own rules:** drop `.yar` files in `~/Library/Application Support/prunemymac/yara/`. Good sources: [Elastic protections-artifacts](https://github.com/elastic/protections-artifacts) (Elastic License 2.0) and [YARA-Forge](https://github.com/YARAHQ/yara-forge).
- **Persistence audit:** flags unsigned or broken launch agents and daemons. Homebrew services are recognised as benign.
- **ClamAV** (GPL): when you `brew install clamav && freshclam`, a "Deep scan" button runs `clamscan` as a separate process. It isn't linked, so the GPL doesn't spread to prunemymac.

Quick scan covers Downloads, Desktop, Applications, Application Support, launch agents/daemons, privileged helpers, /Users/Shared and /tmp. A magic-number prefilter only feeds Mach-O, scripts, archives and disk images to YARA (≈ 180k files in ~11 s).

## Safety

- All removal goes through `macscrap_core::clean::remove`. It refuses protected paths (`/`, `/System`, `~`, `~/Library`, `~/Documents`, …).
- Every clean shows a confirmation with item count, size, and what's permanent vs. recoverable.
- Caches and logs are deleted (they're rebuilt). Everything else (installers, archives, duplicates, apps, threats) goes to the Trash.
- prunemymac never modifies app bundles. It doesn't strip languages or thin universal binaries, because that breaks code signatures.

## Layout

```
crates/macscrap-core/   engine, no UI deps
  fsutil.rs           parallel walker, sizes, protected paths
  junk.rs             rule loader + scanner
  files.rs            large files, duplicates
  apps.rs             uninstaller
  background.rs       launch items, heavy processes
  health.rs           live system metrics
  malware.rs          yara-x + XProtect, persistence audit, ClamAV
  clean.rs            the only code that removes files
crates/macscrap/        GPUI app (app.rs = state/jobs, pages.rs = screens, theme.rs)
rules/junk.toml       cleanup rules (override in ~/Library/Application Support/prunemymac/rules.toml)
```

## Not yet

Menu-bar mini dashboard (GPUI has no status-item API yet; needs a small AppKit shim), privileged helper for root-owned junk, browser privacy cleaning, scheduled scans.
