# Changelog Spec

We adhere to [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) v1.1.0.

## Format

```
# Changelog

## [<version>] — <YYYY-MM-DD>

### Added      — new features
### Changed    — changes to existing functionality
### Deprecated — soon-to-be-removed features
### Removed    — removed features
### Fixed      — bug fixes
### Security   — vulnerability fixes
```

## Rules

- The `Unreleased` section tracks changes not yet released.
- Each release links back to the diff on the repository.
- Entries use present tense, imperative mood ("Add", not "Added" or "Adds").
- Group related entries under the same section header.
- Empty sections are omitted on release (but may be kept during development for discoverability).
- Release `0.0.0` is never published; first release is `0.1.0`.

## Linking

```
[unreleased]: https://codeberg.org/EdwardJoke/cage/compare/v0.1.0...HEAD
[0.1.0]: https://codeberg.org/EdwardJoke/cage/releases/tag/v0.1.0
```
