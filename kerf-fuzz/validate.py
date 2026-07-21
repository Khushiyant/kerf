"""Self-validation: run the FULL oracle (determinism, rotation, mirror, translation, containment, and
a self-differential) over the real corpus against the kerf REFERENCE slicer.

kerf's lower->G-code is exact and metamorphically sound by construction, so every invariant MUST hold
(drifts ~0, no violations, containment clean, kerf-ref vs kerf-ref agree). If this passes, the oracle +
transforms + comparison + tolerances are trustworthy — any violation later against a REAL slicer is a
finding, not a harness bug. Run:  python validate.py
"""

from __future__ import annotations

from kerffuzz import corpus, oracle
from kerffuzz.adapters import KerfReference


def main():
    adapter = KerfReference()
    instances = [(n, i) for n, i in corpus.boundary_corpus() if i.to_kerf_hi() is not None]
    total = viol = 0
    print(f"reference slicer: {adapter.name}   ({len(instances)} prism instances)\n")
    print(f"{'shape':22} {'invariant':16} {'class':6} {'mean(um)':>9} {'max(um)':>9}  verdict")
    for name, inst in instances:
        results = list(oracle.run_all(adapter, inst))
        results.append(oracle.differential(adapter, KerfReference(), inst))  # self-differential must agree
        for r in results:
            total += 1
            viol += int(r.violation)
            v = "VIOLATION" if r.violation else "ok"
            print(f"{name:22} {r.kind[:16]:16} {r.soundness_class:6} {r.mean_um:9.1f} {r.max_um:9.1f}  {v}")
    print(f"\n{total - viol}/{total} invariants held  ({viol} violations)")
    if viol:
        raise SystemExit("SELF-VALIDATION FAILED — fix before trusting real-slicer runs")
    print("SELF-VALIDATION PASSED — oracle is sound; ready to point at real slicers")


if __name__ == "__main__":
    main()
