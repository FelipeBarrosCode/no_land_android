# Android CI and releases

- Pull requests run shared TypeScript/localization, Rust, security, script and
  restore-engine checks, plus native arm64 and x86_64 APK builds.
- `npm run test:scripts` discovers all script regressions instead of a hand-picked
  list. Linux and macOS both run it.
- `android.yml` owns APK publication. Main/tag/manual releases must first pass
  reusable `release.yml` validation and the full test suite. A failed prerequisite
  prevents publication; no branch-protection or security gate is bypassed.
- Nightly uses the same native build with `publish: false`. Manual nightly runs
  also cannot publish. The inherited six-desktop release workflow is not called.
- The public feedback endpoint and immutable runtime pins remain compile-time
  build inputs. Runtime signing private keys never belong in the client.

Shared desktop fixes carried forward: bounded authenticated restore streaming,
adaptive provider transfer limits, runtime app discovery/backup, diagnostic-log
exclusion, and startup failure handling that preserves the original state file.
Mobile entry points and protected credential storage remain platform-specific.

Local checks: `npm run test:scripts`, `npm run i18n:check`, `npm run build`,
`npm run check:android`, and `actionlint`. Native APK builds run on hosted Linux.
