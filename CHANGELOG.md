# Changelog

## 0.1.0 (2026-07-04)


### Features

* **api:** add kerf-api + kerf-serve — the demoable Phase-1 spine ([47b7b1a](https://github.com/Khushiyant/kerf/commit/47b7b1a3ce5a536c9b2e03e74d7f343e73333cf5))
* **api:** dashboard history + regression-alerts views (Phase 3) ([e98b8e2](https://github.com/Khushiyant/kerf/commit/e98b8e2c9803f8718a622da88da6ba150d2073ae))
* **api:** OpenTelemetry OTLP tracing (feature otel) + Jaeger in compose ([13fa75b](https://github.com/Khushiyant/kerf/commit/13fa75be1f33021edb5712f270c0bf8f4b067bd3))
* **api:** RBAC roles + /metrics observability (Phase 4) ([762f655](https://github.com/Khushiyant/kerf/commit/762f655ea8bee04707925fe842770ea83ab7dbfa))
* **api:** serve per-layer visual diff as PNG (GET /v1/results/{id}/diff.png) ([391ad68](https://github.com/Khushiyant/kerf/commit/391ad68fb692daf28f1041c034e77fc00024d2a5))
* **engine:** add kerf-engine — deterministic, versioned verdict envelope ([9f4bdcc](https://github.com/Khushiyant/kerf/commit/9f4bdcc8d7b3ce50496ba1325f74afd9fbd3e92d))
* **ingest:** add kerf-ingest — watch-folder submission agent ([c73b52c](https://github.com/Khushiyant/kerf/commit/c73b52cc46655e57d4c073c49cc952ba04d6fa1b))
* **parser:** support Simplify3D vocabulary and add semantics doc ([d661344](https://github.com/Khushiyant/kerf/commit/d661344c5629bd693e8a71990ccaf77692b84ceb))
* **proofs:** machine-checked Lean proof of P1–P4 (no sorry) ([c19bbf6](https://github.com/Khushiyant/kerf/commit/c19bbf6ed66784a73b7f05deaff74859208e80d3))
* **queue:** add kerf-queue — leased job queue with retry + dead-letter ([c6b0557](https://github.com/Khushiyant/kerf/commit/c6b0557e24a3e341552d8fa1bb2979a9b8998e00))
* **render:** add kerf-render — occupancy + per-layer visual diff to PNG ([c93ae71](https://github.com/Khushiyant/kerf/commit/c93ae71998b4904e889403e484ca16ead9e65ccd))
* **store,worker,api:** baselines + regression alerts (Phase 2) ([4901421](https://github.com/Khushiyant/kerf/commit/4901421d0c88e684bbf159864d3cde75bbc497ac))
* **store:** add kerf-store — content-addressed blobs, immutable results, audit chain ([f1dc49c](https://github.com/Khushiyant/kerf/commit/f1dc49c2810c8ceedcecb2d3b027150409f3a80a))
* **store:** durable Postgres backend, verified via Docker Compose ([270709d](https://github.com/Khushiyant/kerf/commit/270709d244798670614fbc04472bef2aa27b8684))
* verifiable IR for the mesh→G-code half of fabrication ([92efa76](https://github.com/Khushiyant/kerf/commit/92efa76a4380a29f89d23cbc35e6d8360016f9a5))
* **verify:** machine-checked Kani proofs and extended slicer coverage ([7d048b0](https://github.com/Khushiyant/kerf/commit/7d048b07821901cf54f3da640310690b1a33dd6b))
* **worker:** add kerf-worker — leases jobs, runs the engine, stores results ([787ad8a](https://github.com/Khushiyant/kerf/commit/787ad8aaf207cfa00ff027111a9ab256d2197ae6))


### Bug Fixes

* **api:** emit request span at INFO so OTLP traces are exported ([636e44f](https://github.com/Khushiyant/kerf/commit/636e44f4c90621be7f5bf0987e3caf2088584ad2))
* **parser:** segment layers on real PrusaSlicer output ([2bc7f41](https://github.com/Khushiyant/kerf/commit/2bc7f416d7c7a9b0c289df43f5f302c01ba8c4ae))


### Performance Improvements

* **denote:** ~5x faster verification with an identical marked set ([cca19ba](https://github.com/Khushiyant/kerf/commit/cca19ba8719014e95e821ac445d1ee81cf68a7ab))


### Continuous Integration

* use generic updater for nested version fields ([448b557](https://github.com/Khushiyant/kerf/commit/448b5577c54b4a211343416e58bd552114d6ee7f))

## Changelog

All notable changes to Kerf are documented here. This file is maintained automatically by
[release-please](https://github.com/googleapis/release-please) from the Conventional Commits on `main`.
