# Commit Spec

We use [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) v1.0.0.

## Format

```text
<type>(<scope>): <description>

[optional body]

[optional footer(s)]
```

## Types

| Type       | Usage                                   |
|------------|-----------------------------------------|
| `feat`     | New feature                             |
| `fix`      | Bug fix                                 |
| `docs`     | Documentation only                      |
| `style`    | Formatting, whitespace (no logic change)|
| `refactor` | Code change that neither fixes nor adds |
| `perf`     | Performance improvement                 |
| `test`     | Adding or fixing tests                  |
| `build`    | Build system or dependencies            |
| `ci`       | CI configuration                        |
| `chore`    | Maintenance, tooling, minor tasks       |
| `revert`   | Revert a previous commit                |

## Scope

- Scopes are optional but encouraged. Common scopes: `cli`, `orchestrator`, `sandbox`, `ffi`, `router`, `dlq`.
- Must be a single noun; no spaces allowed.

## Description

- Imperative mood, lowercase, no trailing period.
- Max 72 characters.

## Body

- Blank line after description.
- Wrap at 72 characters.
- Explain *why* the change was made, not *what*.

## Breaking changes

- Append `!` after the type/scope: `feat(orchestrator)!: remove spawn timeout`.
- Or add `BREAKING CHANGE:` footer.

## Examples

```text
feat(sandbox): add fuel metering to WASM execution

fix(orchestrator): handle deadlock on agent kill during inbox flush

docs: add architecture overview to README

refactor(router): extract pattern matching into separate module

ci: pin rust-toolchain to nightly-2026-06-01
```
