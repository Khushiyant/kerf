# Releasing Kerf

Everything here is **set up but not triggered** — a release happens only when *you* push a version tag.
Nothing publishes on an ordinary push.

## What ships where

| Artifact | Registry | How |
|---|---|---|
| `kerf-core` (library) | crates.io | `cargo publish -p kerf-core` |
| `kerf-cli` (the `kerf` binary) | crates.io | `cargo publish -p kerf-cli` (after `kerf-core` is indexed) |
| `kerf` (Python, abi3 wheels + sdist) | PyPI | maturin, built for Linux/macOS/Windows |
| Source + wheels | GitHub Releases | attached automatically |

`kerf-py` is `publish = false` — it is a Python extension crate, distributed to PyPI as the `kerf`
wheel, never to crates.io.

## One-time setup (before the first release)

1. **crates.io token** — create a token at <https://crates.io/settings/tokens> and add it as the repo
   secret `CARGO_REGISTRY_TOKEN` (Settings → Secrets and variables → Actions).
2. **PyPI Trusted Publishing** — on PyPI, add a trusted publisher for this repo:
   workflow `release.yml`, environment `pypi`. No token is stored (OIDC). Then create a GitHub
   Actions *Environment* named `pypi`. (Fallback: skip trusted publishing and instead set the
   `PYPI_API_TOKEN` secret and add `password: ${{ secrets.PYPI_API_TOKEN }}` to the publish step.)
3. That's it — `release.yml` already requests the right permissions (`id-token: write`,
   `contents: write`).

## Cutting a release

1. Bump the version in **`Cargo.toml`** (`[workspace.package] version`) and **`pyproject.toml`**
   (`[project] version`) — keep them in lockstep.
2. Move the `## [Unreleased]` notes in `CHANGELOG.md` under a new `## [X.Y.Z]` heading.
3. Commit (`chore(release): vX.Y.Z`), then tag and push the tag:
   ```console
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```
4. The `release` workflow publishes to crates.io + PyPI and creates the GitHub Release. Watch it in the
   Actions tab. You can also run it manually (`workflow_dispatch`) for a dry run — the crates/PyPI
   publish steps are guarded to the tag ref.

## Validate locally first (no publishing)

```console
# Rust: package + a dry-run publish of the library (never uploads)
cargo package -p kerf-core
cargo publish -p kerf-core --dry-run

# Python: build a wheel and an sdist into ./dist (never uploads)
maturin build --release --out dist
maturin sdist --out dist
```

## Notes

- **Order matters on crates.io:** `kerf-cli` depends on `kerf-core` by version, so `kerf-core` must be
  indexed first. `release.yml` publishes `kerf-core`, then retries `kerf-cli` until the index catches up.
- **Later:** when `kerf-server` (the enterprise service) lands, add a container-image build/push job to
  `release.yml` (e.g. GHCR) — the workflow is structured to extend with one more job.
