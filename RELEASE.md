# Releasing Kerf

Releases are **fully automatic** and driven by [Conventional Commits](https://www.conventionalcommits.org).
You never edit versions, merge a release PR, or push a tag.

## How it works

Every push to `main` runs `.github/workflows/release.yml`, which:

1. Computes the next version from the commit messages since the last release
   (`feat:` → minor, `fix:` / `perf:` → patch, `feat!:` or a `BREAKING CHANGE:` footer → major).
   Commits that warrant no release (`docs:`/`chore:`/`ci:`/`refactor:`/`test:`) do nothing.
2. If a release is warranted, bumps the version in `Cargo.toml`, `pyproject.toml`, the
   `kerf-cli` → `kerf-core` pin and `Cargo.lock`, prepends the changelog, commits it back as
   `chore(release): vX.Y.Z [skip ci]`, and creates the `vX.Y.Z` tag.
3. Publishes in the same run: **crates.io** (`kerf-core` then `kerf-cli`), **PyPI** (`pykerf` abi3
   wheels for Linux/macOS/Windows + sdist, via Trusted Publishing), and a **GitHub Release** with
   generated notes.

Just write conventional commits. The version bumps and publishes itself.

## One-time setup

- **crates.io token** — add it as the repo secret `CARGO_REGISTRY_TOKEN`:
  ```console
  gh secret set CARGO_REGISTRY_TOKEN -R Khushiyant/kerf
  ```
- **PyPI Trusted Publishing** — already configured (project `pykerf`, environment `pypi`).

Nothing else: the tag, container, and GitHub Release use the built-in `GITHUB_TOKEN`, and `main` is
not branch-protected so the workflow can push the release commit back. (If you ever protect `main`,
allow the `github-actions` bot to bypass it, or the release commit push will fail.)

## Validate a build locally (no publishing)

```console
cargo publish -p kerf-core --dry-run
maturin build --release --out dist
maturin sdist --out dist
```
