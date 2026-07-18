# Branch Naming Spec

## Main branches

| Branch   | Purpose                     | Source    |
|----------|-----------------------------|-----------|
| `main`   | Production-ready releases   | —         |
| `dev`    | Integration branch          | `main`    |

## Feature branches

```
<type>/<short-description>
```

Types:

| Type        | When                        |
|-------------|-----------------------------|
| `feat`      | New feature                 |
| `fix`       | Bug fix                     |
| `docs`      | Documentation               |
| `refactor`  | Code restructuring          |
| `perf`      | Performance                 |
| `test`      | Test additions or changes   |
| `chore`     | Maintenance, CI, tooling    |

Examples:

```
feat/http-client-timeout
fix/sandbox-fuel-overflow
docs/api-usage
refactor/router-patterns
perf/serialization-bytes
test/orchestrator-lifecycle
chore/update-wasmtime-48
```

## Rules

- Use lowercase kebab-case (hyphen-separated).
- Branch off `dev`, PR into `dev`.
- Delete branch after merge.
- Do not include issue tracker IDs in branch names.
