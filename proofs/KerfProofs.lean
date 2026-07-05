/-
Kerf — machine-checked soundness proofs (P1–P5) over the discrete denotational model.

`denote` maps a program to an occupancy: a characteristic function `Cell → Bool` marking the deposited
cells — exactly what `kerf-core`'s rasterizer computes, per layer, on the integer lattice. A layer is a
list of deposited segments; its occupancy is the union (pointwise `or`) of their coverage.

Everything reduces to three abstract properties of coverage: it sees a segment only through its
unordered endpoints (`Coverage.symm`), and it is equivariant under whole-cell translation
(`Coverage.trans`) and 90° lattice rotation (`Coverage.rot90`) — both isometries of the integer grid.
The `kerf-core` implementation discharges `symm` via `canon_seg`, which **Kani proves order-independent
for all i64 endpoints** (`canon_seg_is_order_independent`); `trans` and `rot90` hold because the capsule
reach test depends only on lattice distances, which those isometries preserve. So Lean proves,
unbounded, that these properties imply P1–P5. Together: the soundness argument, mechanically checked
end to end. No `sorry`, no Mathlib (checked by `#print axioms` at the bottom).
-/

namespace Kerf

/-- A cell (and a point) on the integer-micron lattice. -/
abbrev Cell := Int × Int

/-- A layer occupancy: which cells hold deposited material. -/
abbrev Occ := Cell → Bool

/-- A deposited segment: two endpoints and a width. -/
structure Seg where
  a : Cell
  b : Cell
  w : Int
deriving DecidableEq, Repr

/-- Reverse a segment — what path reversal does to each of its segments. -/
def Seg.swap (s : Seg) : Seg := ⟨s.b, s.a, s.w⟩

/-- Translate a segment by a vector `d`. -/
def Seg.shift (d : Cell) (s : Seg) : Seg :=
  ⟨(s.a.1 + d.1, s.a.2 + d.2), (s.b.1 + d.1, s.b.2 + d.2), s.w⟩

/-- Rotate a cell 90° CCW about the origin on the integer lattice: `(x, y) ↦ (-y-1, x)` — the exact
    index map `kerf-core`'s voxel `rot_z90` uses. -/
def rot90cell : Cell → Cell
  | (x, y) => (-y - 1, x)

/-- The inverse of `rot90cell`: `(x, y) ↦ (y, -x-1)`. -/
def rot90cellInv : Cell → Cell
  | (x, y) => (y, -x - 1)

/-- Rotate a segment 90° CCW by rotating both endpoints. -/
def Seg.rot90 (s : Seg) : Seg := ⟨rot90cell s.a, rot90cell s.b, s.w⟩

/-- The abstract guarantees `kerf-core`'s rasterizer provides about coverage. `symm` is discharged in
    the implementation by `canon_seg` (Kani: `canon_seg_is_order_independent`); `trans` and `rot90` by
    the reach test seeing only lattice distances, which translations and 90° turns preserve (`docs/08`). -/
structure Coverage where
  cover : Seg → Cell → Bool
  /-- Coverage depends on a segment only through its unordered endpoints. -/
  symm  : ∀ s c, cover s.swap c = cover s c
  /-- Coverage is equivariant under a whole-cell translation. -/
  trans : ∀ d s c, cover (Seg.shift d s) (c.1 + d.1, c.2 + d.2) = cover s c
  /-- Coverage is equivariant under a 90° lattice rotation (an isometry of the integer grid): the
      rotated segment covers `c` exactly where the original covers the inverse-rotated cell. -/
  rot90 : ∀ s c, cover (Seg.rot90 s) c = cover s (rot90cellInv c)

variable (C : Coverage)

/-- Occupancy of a list of deposited segments: the union of their coverage. -/
def denote (ss : List Seg) : Occ := fun c => ss.any (fun s => C.cover s c)

/-! ### Generic `List.any` lemmas, proved from scratch (no Mathlib). -/

theorem any_append (p : α → Bool) :
    ∀ (l₁ l₂ : List α), (l₁ ++ l₂).any p = (l₁.any p || l₂.any p)
  | [], _ => by simp
  | a :: t, l₂ => by simp [List.any_cons, any_append p t l₂, Bool.or_assoc]

theorem any_reverse (p : α → Bool) : ∀ (l : List α), l.reverse.any p = l.any p
  | [] => by simp
  | a :: t => by
      simp [List.reverse_cons, any_append, any_reverse p t, List.any_cons, Bool.or_comm]

theorem any_map (f : α → β) (p : β → Bool) :
    ∀ (l : List α), (l.map f).any p = l.any (fun x => p (f x))
  | [] => by simp
  | a :: t => by simp [List.map_cons, List.any_cons, any_map f p t]

theorem any_congr {p q : α → Bool} (h : ∀ x, p x = q x) :
    ∀ (l : List α), l.any p = l.any q
  | [] => by simp
  | a :: t => by simp [List.any_cons, h a, any_congr h t]

/-! ### P1 — reversal invariance. -/

/-- **P1.** Reversing a path (reverse the segment list, each segment's endpoints swapped) deposits
    exactly the same material. This is the property the whole pass framework relies on. -/
theorem reversal_invariant (l : List Seg) :
    denote C ((l.map Seg.swap).reverse) = denote C l := by
  funext c
  unfold denote
  rw [any_reverse, any_map]
  exact any_congr (fun s => C.symm s c) l

/-! ### P2 — translation invariance. -/

/-- Shift an occupancy by a whole-cell vector. -/
def shiftOcc (d : Cell) (o : Occ) : Occ := fun c => o (c.1 - d.1, c.2 - d.2)

/-- **P2.** Translating every segment by `d` shifts the deposited material by `d` — coordinate handling
    is consistent under a whole-cell translation. -/
theorem translation_invariant (d : Cell) (l : List Seg) :
    denote C (l.map (Seg.shift d)) = shiftOcc d (denote C l) := by
  funext c
  unfold denote shiftOcc
  rw [any_map]
  apply any_congr
  intro s
  have h := C.trans d s (c.1 - d.1, c.2 - d.2)
  have e1 : (c.1 - d.1) + d.1 = c.1 := by omega
  have e2 : (c.2 - d.2) + d.2 = c.2 := by omega
  rw [e1, e2] at h
  exact h

/-! ### P5 — 90° rotation invariance. -/

/-- Rotate an occupancy 90° CCW: read the source at the inverse-rotated cell. -/
def rot90Occ (o : Occ) : Occ := fun c => o (rot90cellInv c)

/-- **P5.** Rotating every segment 90° about the origin rotates the deposited material the same way —
    the grid rotation is denotation-equivariant, extending P2 from whole-cell translations to 90°
    turns. This is the property `kerf-core`'s voxel `rot_z90` relies on. -/
theorem rotation90_invariant (l : List Seg) :
    denote C (l.map Seg.rot90) = rot90Occ (denote C l) := by
  funext c
  unfold denote rot90Occ
  rw [any_map]
  exact any_congr (fun s => C.rot90 s c) l

/-! ### P4 — pass soundness (reorder + per-segment reversal). -/

/-- Reordering the segments (a permutation) does not change the union. -/
theorem denote_perm {l₁ l₂ : List Seg} (h : l₁.Perm l₂) : denote C l₁ = denote C l₂ := by
  funext c
  unfold denote
  induction h with
  | nil => rfl
  | cons x _ ih => simp [List.any_cons, ih]
  | swap x y l => simp [List.any_cons]; rw [← Bool.or_assoc, ← Bool.or_assoc, Bool.or_comm (C.cover y c) (C.cover x c)]
  | trans _ _ ih₁ ih₂ => rw [ih₁, ih₂]

/-- Reversing an arbitrary subset of the segments does not change the union (each covers the same cells
    by `symm`). -/
theorem denote_optswap (f : Seg → Bool) (l : List Seg) :
    denote C (l.map (fun s => if f s then s.swap else s)) = denote C l := by
  funext c
  unfold denote
  rw [any_map]
  apply any_congr
  intro s
  by_cases hf : f s = true
  · simp [hf, C.symm]
  · simp [hf]

/-- **P4.** A pass that reorders the toolpaths and reverses any subset of them (exactly what
    `TravelOrder` does) preserves the deposited material. -/
theorem pass_sound (f : Seg → Bool) {l l' : List Seg}
    (h : l'.Perm (l.map (fun s => if f s then s.swap else s))) :
    denote C l' = denote C l := by
  rw [denote_perm C h, denote_optswap]

/-! ### P3 — lowering soundness. -/

/-- A move-plan instruction: an extruding segment, or a (material-free) travel. -/
inductive Move where
  | extrude (s : Seg)
  | travel (a b : Cell)

/-- Denotation of a move: an extrude deposits its segment's coverage; a travel deposits nothing. -/
def denoteMove (m : Move) (c : Cell) : Bool :=
  match m with
  | .extrude s => C.cover s c
  | .travel _ _ => false

/-- Occupancy of a move plan: the union over its moves (travels contribute nothing). -/
def denoteMoves (ms : List Move) : Occ := fun c => ms.any (fun m => denoteMove C m c)

/-- The extruding segments of a move plan, in order. -/
def extrudesOf : List Move → List Seg
  | [] => []
  | .extrude s :: t => s :: extrudesOf t
  | .travel _ _ :: t => extrudesOf t

/-- A move plan denotes exactly what its extruding segments denote. -/
theorem denoteMoves_eq_denote_extrudes (ms : List Move) :
    denoteMoves C ms = denote C (extrudesOf ms) := by
  funext c
  unfold denoteMoves denote
  induction ms with
  | nil => rfl
  | cons m t ih =>
    -- Expand the cons structure WITHOUT unfolding `denoteMove` (so the tail still matches `ih`),
    -- apply `ih`, and only then reduce the head via `denoteMove`.
    cases m with
    | extrude s =>
      simp only [extrudesOf, List.any_cons]
      rw [ih]
      simp only [denoteMove, Bool.false_or]
    | travel a b =>
      simp only [extrudesOf, List.any_cons]
      rw [ih]
      simp only [denoteMove, Bool.false_or]

theorem extrudesOf_append (xs ys : List Move) :
    extrudesOf (xs ++ ys) = extrudesOf xs ++ extrudesOf ys := by
  induction xs with
  | nil => simp [extrudesOf]
  | cons m t ih => cases m <;> simp [extrudesOf, ih]

theorem extrudesOf_map_extrude (fills : List Seg) :
    extrudesOf (fills.map Move.extrude) = fills := by
  induction fills with
  | nil => simp [extrudesOf]
  | cons s t ih => simp [List.map_cons, extrudesOf, ih]

theorem extrudesOf_map_travel (travels : List (Cell × Cell)) :
    extrudesOf (travels.map (fun p => Move.travel p.1 p.2)) = [] := by
  induction travels with
  | nil => simp [extrudesOf]
  | cons p t ih => simp [List.map_cons, extrudesOf, ih]

/-- **P3.** The `hi → lo` lowering — copy each fill into an extruding move and insert (arbitrary) travel
    moves — preserves the deposited material. Travels denote nothing, so the occupancy is exactly the
    fills'. -/
theorem lowering_sound (fills : List Seg) (travels : List (Cell × Cell)) :
    denoteMoves C (fills.map Move.extrude ++ travels.map (fun p => Move.travel p.1 p.2))
      = denote C fills := by
  rw [denoteMoves_eq_denote_extrudes, extrudesOf_append,
      extrudesOf_map_extrude, extrudesOf_map_travel, List.append_nil]

end Kerf

-- No `sorry` and no axioms beyond Lean's standard `propext`/`Classical.choice`/`Quot.sound`.
#print axioms Kerf.reversal_invariant
#print axioms Kerf.translation_invariant
#print axioms Kerf.rotation90_invariant
#print axioms Kerf.pass_sound
#print axioms Kerf.lowering_sound
