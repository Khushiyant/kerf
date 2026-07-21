"""Browsable violation reports for kerf-fuzz runs.

kerf-fuzz metamorphically/differentially fuzzes 3D-printing slicers, using kerf
as a semantic oracle. When a run finds violations of the sound invariants, this
module renders a self-contained HTML report plus minimal reproducer bundles a
maintainer can hand back to a slicer's authors.

A violation is a plain dict (intentionally decoupled from the rest of kerffuzz):

    {
      "slicer": str,                    # adapter name, e.g. "prusaslicer"
      "instance": str,                  # instance label
      "invariant": str,                 # e.g. "rotate_90deg", "determinism"
      "soundness_class": str,           # EXACT | GRADED | DIRECTIONAL | GATE
      "mean_um": float, "max_um": float,
      "detail": str,
      "stl_b64": str | None,            # base64 of the (shrunken) instance STL
      "gcode_base": str | None,         # baseline slicer G-code
      "gcode_transformed": str | None,  # transformed-input slicer G-code
      "shrink": dict | None,            # {"before": {...}, "after": {...}}
    }

Standard library only.
"""

from __future__ import annotations

import base64
import binascii
import html
import json
from datetime import datetime, timezone
from pathlib import Path

# Severity ordering + palette. Most-severe first.
CLASS_ORDER = ["EXACT", "GRADED", "DIRECTIONAL", "GATE"]
_CLASS_RANK = {name: i for i, name in enumerate(CLASS_ORDER)}

# (background, border, text) per soundness class, used for row color-coding.
CLASS_COLORS = {
    "EXACT":       ("#fdecec", "#e5484d", "#8f1d1f"),  # red
    "GRADED":      ("#fdf3e3", "#e6a23c", "#8a5a12"),  # amber
    "DIRECTIONAL": ("#eef2f8", "#5b7fa6", "#2f4a6b"),  # slate/blue
    "GATE":        ("#f1f1f2", "#9aa0a6", "#4a4d52"),  # grey
}
_UNKNOWN_COLOR = ("#f6f6f7", "#c8ccd1", "#5a5d63")


def _rank(violation: dict) -> tuple:
    """Sort key: severity class first, then max_um descending."""
    cls = str(violation.get("soundness_class", "")).upper()
    class_rank = _CLASS_RANK.get(cls, len(CLASS_ORDER))
    try:
        max_um = float(violation.get("max_um", 0.0))
    except (TypeError, ValueError):
        max_um = 0.0
    return (class_rank, -max_um)


def summarize(violations: list[dict]) -> dict:
    """Counts by soundness class + by slicer + total."""
    by_class: dict[str, int] = {}
    by_slicer: dict[str, int] = {}
    for v in violations:
        cls = str(v.get("soundness_class", "UNKNOWN")).upper()
        slicer = str(v.get("slicer", "unknown"))
        by_class[cls] = by_class.get(cls, 0) + 1
        by_slicer[slicer] = by_slicer.get(slicer, 0) + 1
    return {
        "total": len(violations),
        "by_class": by_class,
        "by_slicer": by_slicer,
    }


def _fmt_um(value) -> str:
    try:
        return f"{float(value):.3f}"
    except (TypeError, ValueError):
        return html.escape(str(value))


def _decode_b64(data: str) -> bytes | None:
    try:
        return base64.b64decode(data, validate=True)
    except (binascii.Error, ValueError):
        return None


def _write_repro(violation: dict, repro_dir: Path) -> None:
    """Materialize one reproducer bundle: model.stl, base/transformed gcode, finding.json."""
    repro_dir.mkdir(parents=True, exist_ok=True)

    stl_b64 = violation.get("stl_b64")
    if stl_b64:
        raw = _decode_b64(stl_b64)
        if raw is not None:
            (repro_dir / "model.stl").write_bytes(raw)

    base = violation.get("gcode_base")
    if base is not None:
        (repro_dir / "base.gcode").write_text(str(base), encoding="utf-8")

    transformed = violation.get("gcode_transformed")
    if transformed is not None:
        (repro_dir / "transformed.gcode").write_text(str(transformed), encoding="utf-8")

    (repro_dir / "finding.json").write_text(
        json.dumps(violation, indent=2, sort_keys=True, default=str),
        encoding="utf-8",
    )


# ---------------------------------------------------------------------------
# HTML rendering
# ---------------------------------------------------------------------------

_CSS = """
:root { color-scheme: light; }
* { box-sizing: border-box; }
body {
  margin: 0; padding: 0 24px 64px;
  font: 15px/1.5 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
  color: #1b1d21; background: #fafafb;
}
.wrap { max-width: 1180px; margin: 0 auto; }
header { padding: 28px 0 8px; }
h1 { font-size: 24px; margin: 0 0 4px; letter-spacing: -0.01em; }
h1 .accent { color: #e5484d; }
.subtitle { color: #6a6e76; font-size: 13px; margin: 0 0 20px; }
.meta-grid {
  display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 10px 20px; margin: 0 0 24px;
  padding: 14px 18px; background: #fff; border: 1px solid #e6e8eb; border-radius: 10px;
}
.meta-grid dt { font-size: 11px; text-transform: uppercase; letter-spacing: 0.04em; color: #8b8f97; margin: 0; }
.meta-grid dd { margin: 1px 0 0; font-weight: 600; word-break: break-word; }
.pills { display: flex; flex-wrap: wrap; gap: 10px; margin: 0 0 28px; }
.pill {
  display: inline-flex; align-items: baseline; gap: 8px;
  padding: 8px 14px; border-radius: 999px; border: 1px solid; font-size: 13px; font-weight: 600;
}
.pill .n { font-size: 18px; font-weight: 700; }
.pill.total { background: #1b1d21; border-color: #1b1d21; color: #fff; }
.section-label { font-size: 12px; text-transform: uppercase; letter-spacing: 0.05em; color: #8b8f97; margin: 22px 0 8px; }
table { width: 100%; border-collapse: separate; border-spacing: 0; background: #fff;
  border: 1px solid #e6e8eb; border-radius: 10px; overflow: hidden; }
thead th {
  text-align: left; font-size: 11px; text-transform: uppercase; letter-spacing: 0.04em;
  color: #6a6e76; padding: 11px 14px; background: #f4f5f6; border-bottom: 1px solid #e6e8eb; white-space: nowrap;
}
tbody td { padding: 12px 14px; border-bottom: 1px solid #eef0f2; vertical-align: top; }
tbody tr:last-child td { border-bottom: none; }
tbody tr { border-left: 4px solid transparent; }
.badge {
  display: inline-block; padding: 2px 9px; border-radius: 6px; font-size: 11px;
  font-weight: 700; letter-spacing: 0.03em; border: 1px solid;
}
td.num { font-variant-numeric: tabular-nums; text-align: right; white-space: nowrap; }
td.num.hot { font-weight: 700; }
.mono { font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace; font-size: 13px; }
.detail { color: #3a3d43; max-width: 420px; }
.repro a { color: #2f6feb; text-decoration: none; font-weight: 600; }
.repro a:hover { text-decoration: underline; }
.empty {
  margin: 40px 0; padding: 48px 24px; text-align: center; background: #eafaf0;
  border: 1px solid #b7e4c7; border-radius: 12px;
}
.empty .big { font-size: 22px; font-weight: 700; color: #1a7f42; margin: 0 0 6px; }
.empty .sub { color: #3a6b4d; margin: 0; }
footer { margin-top: 32px; color: #9aa0a6; font-size: 12px; }
"""


def _pill(label: str, count: int, colors: tuple[str, str, str]) -> str:
    bg, border, fg = colors
    return (
        f'<span class="pill" style="background:{bg};border-color:{border};color:{fg}">'
        f'{html.escape(label)} <span class="n">{count}</span></span>'
    )


def _render_meta(meta: dict, generated_at: str) -> str:
    rows = [("Generated", generated_at)]
    for key, val in meta.items():
        rows.append((str(key), val))
    items = "".join(
        f"<dt>{html.escape(str(k))}</dt><dd>{html.escape(str(v))}</dd>" for k, v in rows
    )
    return f'<dl class="meta-grid">{items}</dl>'


def _render_summary(summary: dict) -> str:
    pills = [_pill("Total", summary["total"], ("#1b1d21", "#1b1d21", "#fff"))]
    pills[0] = f'<span class="pill total">Total <span class="n">{summary["total"]}</span></span>'
    for cls in CLASS_ORDER:
        n = summary["by_class"].get(cls, 0)
        if n:
            pills.append(_pill(cls, n, CLASS_COLORS[cls]))
    # Any unexpected classes.
    for cls, n in summary["by_class"].items():
        if cls not in CLASS_ORDER and n:
            pills.append(_pill(cls, n, _UNKNOWN_COLOR))

    by_slicer = "".join(
        f'<span class="pill" style="background:#fff;border-color:#d7dade;color:#3a3d43">'
        f'{html.escape(sl)} <span class="n">{n}</span></span>'
        for sl, n in sorted(summary["by_slicer"].items(), key=lambda kv: (-kv[1], kv[0]))
    )
    out = f'<div class="pills">{"".join(pills)}</div>'
    if by_slicer:
        out += '<div class="section-label">By slicer</div>'
        out += f'<div class="pills">{by_slicer}</div>'
    return out


def _render_row(idx: int, v: dict) -> str:
    cls = str(v.get("soundness_class", "")).upper()
    bg, border, fg = CLASS_COLORS.get(cls, _UNKNOWN_COLOR)
    repro_rel = f"repro/{idx:03d}/"
    badge = (
        f'<span class="badge" style="background:{bg};border-color:{border};color:{fg}">'
        f'{html.escape(cls or "?")}</span>'
    )
    return (
        f'<tr style="border-left-color:{border}">'
        f'<td class="mono">{html.escape(str(v.get("slicer", "")))}</td>'
        f'<td class="mono">{html.escape(str(v.get("instance", "")))}</td>'
        f'<td class="mono">{html.escape(str(v.get("invariant", "")))}</td>'
        f"<td>{badge}</td>"
        f'<td class="num">{_fmt_um(v.get("mean_um"))}</td>'
        f'<td class="num hot">{_fmt_um(v.get("max_um"))}</td>'
        f'<td class="detail">{html.escape(str(v.get("detail", "")))}</td>'
        f'<td class="repro"><a href="{html.escape(repro_rel)}finding.json">{html.escape(repro_rel)}</a></td>'
        f"</tr>"
    )


def _render_html(ordered: list[tuple[int, dict]], summary: dict, meta: dict, generated_at: str) -> str:
    head = (
        '<div class="wrap"><header>'
        '<h1><span class="accent">kerf</span>-fuzz violation report</h1>'
        '<p class="subtitle">Sound-invariant violations found while metamorphically / '
        "differentially fuzzing slicers, with kerf as the semantic oracle.</p>"
        f"{_render_meta(meta, generated_at)}"
        "</header>"
    )

    if not ordered:
        body = (
            '<div class="empty">'
            '<p class="big">No violations — all invariants held.</p>'
            '<p class="sub">Every sliced instance satisfied its sound invariants across the run.</p>'
            "</div>"
        )
    else:
        rows = "".join(_render_row(idx, v) for idx, v in ordered)
        body = (
            f"{_render_summary(summary)}"
            '<div class="section-label">Violations (most severe first)</div>'
            "<table><thead><tr>"
            "<th>Slicer</th><th>Instance</th><th>Invariant</th><th>Class</th>"
            "<th>mean &micro;m</th><th>max &micro;m</th><th>Detail</th><th>Repro</th>"
            "</tr></thead><tbody>"
            f"{rows}"
            "</tbody></table>"
        )

    foot = (
        f'<footer>{len(ordered)} finding(s) — generated by kerffuzz.report at '
        f"{html.escape(generated_at)}. Self-contained report; open repro/&lt;NNN&gt;/ for bundles.</footer>"
        "</div>"
    )
    return f"<style>{_CSS}</style>{head}{body}{foot}"


# ---------------------------------------------------------------------------
# Public entry point
# ---------------------------------------------------------------------------

def write_report(violations: list[dict], outdir: str, meta: dict | None = None) -> str:
    """Write report.json, report.html, and repro/<NNN>/ bundles into outdir.

    Returns the path to report.html.
    """
    meta = dict(meta or {})
    out = Path(outdir)
    out.mkdir(parents=True, exist_ok=True)

    ordered = sorted(enumerate(violations), key=lambda item: _rank(item[1]))
    # Re-index so repro dir numbers follow the severity-sorted display order.
    ordered = [(i, v) for i, (_, v) in enumerate(ordered)]

    summary = summarize(violations)
    generated_at = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")

    report_obj = {
        "generated_at": generated_at,
        "meta": meta,
        "summary": summary,
        "violations": [v for _, v in ordered],
    }
    (out / "report.json").write_text(
        json.dumps(report_obj, indent=2, sort_keys=False, default=str),
        encoding="utf-8",
    )

    for idx, v in ordered:
        _write_repro(v, out / "repro" / f"{idx:03d}")

    html_path = out / "report.html"
    html_path.write_text(_render_html(ordered, summary, meta, generated_at), encoding="utf-8")
    return str(html_path)


# ---------------------------------------------------------------------------
# Self-test
# ---------------------------------------------------------------------------

if __name__ == "__main__":
    import tempfile

    stl_b64 = base64.b64encode(b"solid\n").decode("ascii")

    violations = [
        {
            "slicer": "prusaslicer",
            "instance": "gear-bracket-42",
            "invariant": "rotate_90deg",
            "soundness_class": "EXACT",
            "mean_um": 3.14159,
            "max_um": 128.5,
            "detail": "Rotating input 90deg changed extruded volume beyond exact tolerance "
                      "(<>&\"' escaping check).",
            "stl_b64": stl_b64,
            "gcode_base": "G1 X0 Y0 E0\nG1 X10 Y0 E0.5\n",
            "gcode_transformed": "G1 X0 Y0 E0\nG1 X0 Y10 E0.7\n",
            "shrink": {"before": {"triangles": 1200}, "after": {"triangles": 6}},
        },
        {
            "slicer": "cura",
            "instance": "overhang-tab",
            "invariant": "scale_uniform",
            "soundness_class": "GRADED",
            "mean_um": 12.0,
            "max_um": 61.2,
            "detail": "Uniform scaling drifted deposited material past the graded band.",
            "stl_b64": None,
            "gcode_base": "G1 X1 Y1 E1\n",
            "gcode_transformed": None,
            "shrink": None,
        },
        {
            "slicer": "prusaslicer",
            "instance": "thin-wall-probe",
            "invariant": "translate_xy",
            "soundness_class": "DIRECTIONAL",
            "mean_um": 0.5,
            "max_um": 4.0,
            "detail": "Translation produced a directional bias in wall placement.",
            "stl_b64": None,
            "gcode_base": None,
            "gcode_transformed": None,
            "shrink": None,
        },
        {
            "slicer": "orcaslicer",
            "instance": "seam-gate",
            "invariant": "determinism",
            "soundness_class": "GATE",
            "mean_um": 0.0,
            "max_um": 0.9,
            "detail": "Re-slicing identical input crossed the determinism gate.",
            "stl_b64": None,
            "gcode_base": None,
            "gcode_transformed": None,
            "shrink": None,
        },
    ]

    tmp = tempfile.mkdtemp(prefix="kerffuzz-report-")
    html_path = write_report(
        violations,
        tmp,
        meta={"run_id": "demo-0001", "slicers": "prusaslicer, cura, orcaslicer", "seed": 42},
    )

    out = Path(tmp)
    assert (out / "report.html").exists(), "report.html missing"
    assert (out / "report.json").exists(), "report.json missing"
    assert (out / "repro" / "000" / "finding.json").exists(), "repro finding.json missing"
    assert (out / "repro" / "000" / "model.stl").exists(), "EXACT repro should have model.stl"

    html_text = (out / "report.html").read_text(encoding="utf-8")
    for name in ("gear-bracket-42", "overhang-tab", "thin-wall-probe", "seam-gate"):
        assert name in html_text, f"instance {name} missing from HTML"
    assert "&lt;&gt;&amp;" in html_text, "detail text was not HTML-escaped"

    loaded = json.loads((out / "report.json").read_text(encoding="utf-8"))
    assert loaded["summary"]["total"] == 4
    assert loaded["summary"]["by_class"]["EXACT"] == 1
    assert loaded["violations"][0]["soundness_class"] == "EXACT", "most-severe should sort first"

    # Zero-violation path.
    empty_dir = tempfile.mkdtemp(prefix="kerffuzz-report-empty-")
    empty_html = write_report([], empty_dir)
    assert "no violations" in Path(empty_html).read_text(encoding="utf-8").lower()

    print(html_path)
    print("report OK")
