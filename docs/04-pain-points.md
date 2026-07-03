# 04 — What users actually complain about (grounded, not marketing)

**Bottom line:** There is **no mass revolt about slicer quality.** OrcaSlicer is the de-facto "actually
good" free default and has absorbed the best ideas from PrusaSlicer/SuperSlicer/Bambu Studio. The loudest
dissatisfaction in the entire space is **not slicing quality — it's Bambu's ecosystem lock-in.** A "better
slicer" is defensible only if it attacks the **fundamental/algorithmic** layer, not the UI.

**Framing fact that shapes everything:** every mainstream FDM slicer *except Cura* is the **same engine**
(Slic3r 2011 → PrusaSlicer 2016 → Bambu Studio 2022 → OrcaSlicer 2022; SuperSlicer is a parallel Prusa
branch). Cura is the only independent engine, and it's losing development pace. So the shared algorithmic
weaknesses persist across the whole family at once — which is *why* an open IR that any of them could adopt
is more valuable than another competing engine.

## Top real pain points (ranked, with evidence class)

1. **Support generation** — wasteful, hard to remove, placed where it shouldn't be. Densest cluster of
   independent bug reports (OrcaSlicer supports inside holes; PrusaSlicer organic/tree supports starting
   mid-air and collapsing). **Fundamental (geometric algorithm).**
2. **Seam placement** — unreliable; the "fixes" create new artifacts (Cura "Sharpest Corner" regressed;
   "Random" isn't random; OrcaSlicer scarf-seam can *create* a seam or silently vanish from G-code).
   **Fundamental (toolpath/geometry).**
3. **Settings are a "black art"** — 400+ Cura settings, conservative defaults, messy interactions, heavy
   per-filament calibration burden ("100+ filament profiles… off-the-shelf profiles are simply not a good
   way to dial in perfect parts"). Bambu's answer is to move calibration into hardware (LiDAR/eddy-current).
   **Part fundamental, part UX.**
4. **Performance / memory** — Bambu Studio has a documented progressive memory leak; PrusaSlicer config-UI
   lag is present "on PrusaSlicer and its derivatives" (shared-codebase defect); Cura hangs on large STLs,
   especially with Tree Supports. **Fundamental/architectural.**
5. **Retraction/travel bugs** — "Avoid crossing walls" suppresses retraction-wipe G-code → stringing;
   version-bisected to `AvoidCrossingPerimeters.cpp`, confirmed upstream in PrusaSlicer and inherited by
   OrcaSlicer. **Fundamental; illustrates one engine's bug propagating to the whole family.**
6. **Long-requested features nobody ships** (trackers act as multi-year wishlists, all still open as of
   mid-2026): brick/staggered perimeters (Prusa #1823, most-upvoted open issue, since 2019); non-planar
   (Prusa #2794, Cura #5980); arc overhangs (Cura #14036); per-region/per-feature control; cross-machine
   profile sync. High demand → community post-processing script/fork first → sometimes native later.
   **Fundamental.**
7. **Profile/preset management** — inheritance doesn't propagate, cloud sync loses custom profiles. **UX.**
8. **Headless/CLI and plugin gaps** — CLI under-documented; OrcaSlicer has no plugin architecture; Cura's
   plugin API is criticized and breaks across versions. **UX/architectural.**

**Deliberately down-ranked (over-attributed to slicers):** overhang quality (often print
vibration/mechanics), first-layer adhesion (hardware/build-surface). Don't build a thesis on these.

## Do users want a new slicer?

Mostly satisfied at the "it works" level; hungry only at the frontier. Every 2026 comparison converges on
OrcaSlicer as the default recommendation. Real dissatisfaction is **targeted**:

- **Loudest signal by far: Bambu's "Authorization Control"** (Jan 2025) gated third-party software behind a
  "Bambu Connect" bridge. OrcaSlicer's dev publicly refused to adopt it; Josef Prusa called it "scary."
  A dev who restored blocked features got a cease-and-desist. Corroborated across 3D Printing Industry,
  Hackaday, All3DP, and Bambu's own blog.
- **Frontier appetite exists but is niche** (power users wanting intra-layer extrusion-width mixing, etc.).

**Implication:** the market isn't crying for "another slicer." It might reward (a) a slicer that solves a
fundamental algorithmic problem the shared engine can't, or (b) something that exploits the openness/
lock-in anxiety with a genuinely open, trustworthy alternative. Kerf's open-IR + verifiability angle hits
both — *without* trying to beat OrcaSlicer on UX.

## Fundamental vs. superficial (for a builder)

- **Fundamental (a moat):** supports, seams, non-planar, brick/arc perimeters, memory/perf architecture,
  travel planning, predictability / ML-assisted tuning (nobody ships real ML tuning; vendor "AI slicing" is
  overstated marketing), **and correctness/verification (empty field).**
- **Superficial (necessary, not a moat):** settings UI tiering, profile management, CLI docs, plugin
  ergonomics, preview polish.

## Evidence caveats

- GitHub upvote counts/states/dates confirmed via API. Bambu drama confirmed across 4+ outlets + Bambu blog.
- **Correction:** OrcaSlicer native brick-layer implementation (#8181) is **still open, not shipped** as of
  mid-2026 (community script exists).
- **Reddit gap:** Reddit was firewalled at every route; r/3Dprinting-specific sentiment is inferred from
  equivalent forum/news sources, not directly sampled.
- No hard market-share/migration poll exists; consumer "AI slicing" is largely marketing today; the Bambu
  AGPL-violation allegation and a disputed Prusa-layoffs figure are unadjudicated.
