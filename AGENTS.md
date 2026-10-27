# Agent guidelines

- **GitHub communication is English-only**: commit messages, release notes,
  PR titles/descriptions, README, CHANGELOG and all repo documentation
  (`docs/`) must be written in English.
- UI-facing strings are multilingual by design (see `crates/*/src/i18n.rs`)
  — that is product content, not GitHub communication.
- Versioning on `next`: dev builds only, `1.99.x`, tags + workflow artifacts
  (prereleases allowed); never a non-prerelease GitHub Release and never
  touch the stable 1.0.x channel of the Electron edition on `main`.
