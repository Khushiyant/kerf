#!/usr/bin/env python3
"""Throwaway RL demo: learn a toolpath whose deposited material matches a target, using kerf as the
reward oracle. Pure-Python REINFORCE — only needs `pykerf` (no numpy / torch).

    uv run python examples/rl_toolpath.py

The agent decides, for each candidate infill line, how many times to lay it (0, 1, or 2). kerf scores
every candidate move plan:

  reward = occupancy IoU vs the target      (pykerf.diff_programs)   "deposit the right shape"
         - 0.5 * over-deposition fraction    (pykerf.deposit_stats)   "don't lay a line twice"
         - 0.05 * normalized travel          (pykerf.program_stats)   "be efficient"

and any candidate that doesn't survive emit -> re-parse (pykerf.verify_roundtrip) is rejected. The
optimal policy is therefore: lay each TARGET line exactly once, skip the rest. The agent must discover
which lines those are from the reward alone.
"""

import json
import math
import random

import pykerf

random.seed(0)

# ---- problem setup ---------------------------------------------------------
Z = 200  # single layer at 0.2 mm
WIDTH = 400  # 0.4 mm bead
SPAN_UM = 20_000  # 20 mm long lines
N_LINES = 10  # candidate infill lines...
SPACING = 2_000  # ...spaced 2 mm apart in Y
TARGET = {0, 2, 4, 6, 8}  # lines the target deposits (the agent must discover this)
COUNTS = [0, 1, 2]  # per-line action: lay it 0, 1, or 2 times


def build_program(counts):
    """counts[i] passes of candidate line i, with travels between active lines -> lo::Program dict."""
    toolpaths, prev_end = [], None
    for i, c in enumerate(counts):
        if c == 0:
            continue
        y = i * SPACING
        start, end = {"x": 0, "y": y}, {"x": SPAN_UM, "y": y}
        if prev_end is not None:
            toolpaths.append({"kind": "Travel", "path": {"points": [prev_end, start]}, "width_um": 0})
        for _ in range(c):
            toolpaths.append(
                {"kind": {"Extrude": "Infill"}, "path": {"points": [start, end]}, "width_um": WIDTH}
            )
        prev_end = end
    return {"layers": [{"z_um": Z, "toolpaths": toolpaths}]}


TARGET_JSON = json.dumps(build_program([1 if i in TARGET else 0 for i in range(N_LINES)]))


def reward(counts):
    """Score a candidate with kerf. Returns (reward, iou)."""
    prog_json = json.dumps(build_program(counts))
    rt = json.loads(pykerf.verify_roundtrip(prog_json))
    if not rt["has_geometry"]:
        return 0.0, 0.0  # empty plan: nothing deposited
    if not (rt["occupancy_preserved"] and rt["deposit_preserved"]):
        return -1.0, 0.0  # would not survive to a printer: reject
    iou = json.loads(pykerf.diff_programs(prog_json, TARGET_JSON))["iou"] or 0.0
    dep = json.loads(pykerf.deposit_stats(prog_json))
    over = dep["redeposited_cells"] / max(dep["total_cells"], 1)
    travel = json.loads(pykerf.program_stats(prog_json))["travel_distance_um"] / (N_LINES * SPACING)
    return iou - 0.5 * over - 0.05 * travel, iou


# ---- pure-Python REINFORCE -------------------------------------------------
def softmax(xs):
    m = max(xs)
    es = [math.exp(x - m) for x in xs]
    s = sum(es)
    return [e / s for e in es]


logits = [[0.0, 0.0, 0.0] for _ in range(N_LINES)]  # per-line categorical policy over COUNTS
LR = 0.4
baseline = 0.0
EPISODES = 400

print(f"target lines: {sorted(TARGET)}  (agent must discover these from reward)\n")
for ep in range(1, EPISODES + 1):
    probs = [softmax(row) for row in logits]
    action = [random.choices(range(3), weights=p)[0] for p in probs]  # sample per-line counts
    r, iou = reward([COUNTS[a] for a in action])
    baseline += 0.05 * (r - baseline)  # running-mean baseline
    adv = r - baseline
    for i in range(N_LINES):  # policy-gradient update
        for a in range(3):
            grad = (1.0 if a == action[i] else 0.0) - probs[i][a]
            logits[i][a] += LR * adv * grad
    if ep == 1 or ep % 40 == 0:
        print(f"ep {ep:4d}   reward {r:+.3f}   IoU {iou:.3f}   baseline {baseline:+.3f}")

# ---- result ----------------------------------------------------------------
learned = [COUNTS[max(range(3), key=lambda a: logits[i][a])] for i in range(N_LINES)]
target = [1 if i in TARGET else 0 for i in range(N_LINES)]
r, iou = reward(learned)
print("\nlearned counts:", learned)
print("target  counts:", target)
print(f"greedy policy -> IoU {iou:.3f}, reward {r:+.3f}")
print("EXACT MATCH — the agent recovered the target toolpath from kerf's reward alone."
      if learned == target else "close (compare IoU); tune LR/EPISODES to sharpen.")
