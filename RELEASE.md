# Releasing Kerf

Releases are **automated** via [release-please](https://github.com/googleapis/release-please) driven by
[Conventional Commits](https://www.conventionalcommits.org). You never edit versions or push tags by hand.

## How it works

1. Land commits on `main` with conventional prefixes: `feat:` (→ minor), `fix:` / `perf:` (→ patch),
   `feat!:` or a `BREAKING CHANGE:` footer (→ major, or minor while < 1.0). `docs:`/`chore:`/`ci:`/
   `build:`/`refactor:`/`test:` don't trigger a release on their own.
2. `.github/workflows/release-please.yml` keeps a **release PR** open that bumps the version in
   `Cargo.toml`, `pyproject.toml`, and the `kerf-cli` → `kerf-core` pin (in lockstep), and regenerates
   `CHANGELOG.md`.
3. **Merge the release PR** when you want to cut the release. That creates the `vX.Y.Z` tag + GitHub
   Release and runs the publish jobs in the same workflow:
   - **crates.io** — `kerf-core`, then `kerf-cli` (dependency order, with index-retry).
   - **PyPI** — `pykerf` abi3 wheels (Linux/macOS/Windows) + sdist, via Trusted Publishing (OIDC).
   - **GHCR** — `ghcr.io/khushiyant/kerf` container, stamped with the release commit SHA.

Merging the PR is the only manual step, and it's the intended "cut a release now" gate.

## One-time setup (required before the first release publishes)

1. **crates.io token** — create one at <https://crates.io/settings/tokens> (scopes: publish-new,
   publish-update) and add it as the repo secret `CARGO_REGISTRY_TOKEN`:
   ```console
   gh secret set CARGO_REGISTRY_TOKEN -R Khushiyant/kerf
   ```
2. **PyPI Trusted Publishing** — `pykerf` doesn't exist on PyPI yet, so add a *pending* publisher at
   <https://pypi.org/manage/account/publishing/>: project `pykerf`, owner `Khushiyant`, repo `kerf`,
   workflow `release-please.yml`, environment `pypi`. (The `pypi` GitHub Environment already exists.)

Nothing else is needed — the container and GitHub Release use the built-in `GITHUB_TOKEN`.

## Validate a build locally (no publishing)

```console
cargo publish -p kerf-core --dry-run   # packages the library; never uploads
maturin build --release --out dist     # builds the pykerf wheel
maturin sdist --out dist
```
