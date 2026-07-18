# Version Spec

We adhere to [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html).

## Format

```
MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]
```

| Increment | When                                      | Example |
|-----------|-------------------------------------------|---------|
| MAJOR     | Incompatible API changes                  | `1.0.0` → `2.0.0` |
| MINOR     | Backward-compatible new functionality     | `1.0.0` → `1.1.0` |
| PATCH     | Backward-compatible bug fixes             | `1.0.0` → `1.0.1` |

## Pre-release

- Pre-release identifiers use a numeric suffix: `-alpha.1`, `-beta.2`, `-rc.1`.
- Pre-releases have lower precedence than the normal version.

## Zero-version (0.x)

- `0.1.0` is the first public release.
- `0.MINOR` bumps indicate substantial new capability.
- `0.0.PATCH` fixes are rare; prefer `0.1.0` for any meaningful change.
- Breaking changes within `0.x` only bump MINOR (per semver §4).

## Source of truth

- `version` in `Cargo.toml` is the single source of truth.
- Git tags mirror the version with a `v` prefix (e.g. `v0.1.0`).
- CHANGELOG.md headers reference the version number without prefix.
