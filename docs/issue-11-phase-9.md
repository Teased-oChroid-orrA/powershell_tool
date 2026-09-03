# Issue #11 Phase 9: layout fix, OD/wall-thickness input, buckling

Post-v1 refinement round, in response to real feedback on the shipped
Phase 6 UI: a real horizontal-overflow bug, a real domain-modeling
correction (vessels are specified by outer diameter + wall thickness, not
inner/outer radius), and buckling pulled in from the explicit backlog by
direct request, including a mixed end-condition question that needed real
physics verification before any code was written.

## Overflow root cause (found, not guessed)

`.bushing-card .field-row .field { flex: none }` - a rule tuned for the
Bushing Workbench's own field pairs - forces each number field to its
natural width with no shrinking. This page's longer labels ("Required
minimum MS", "Internal pressure (psi)") could exceed the card at less
than a wide window. Confirmed via a mockup (3 options, HTML artifact,
reusing the app's real dark-theme tokens) rendered at a deliberately
narrow 600px stage width - the user picked Option C: a fixed-width
220px input rail (one field per row, so nothing to overflow) plus a
flexible results column. New CSS: `.input-rail-layout`/`.input-rail`/
`.results-column` (`main.rs`). The shared `.field-row` rule itself was
left untouched, not touched at all, to avoid any risk to the Bushing
Workbench's own already-verified layout.

## Geometry input: outer diameter + wall thickness

Matches how a real drawing/spec sheet calls out a vessel. Inner
diameter is derived (`outer_radius - wall_thickness`) and shown as a
read-only caption, not a separate input - the same "input vs. derived"
visual distinction already used elsewhere in this app.

## Mixed end condition - verified before writing any code

The user asked for an end-condition option covering "closed on one end,
open on the other, since it's a long member." Checked the physics before
building anything: axial stress from pressure thrust is a global force-
equilibrium quantity, not a local effect - Saint-Venant's principle says
*local* stress concentrations near a cap decay with distance, but the net
axial thrust itself does not fade just because a member is long; it has
to be reacted somewhere, and it's present in the wall the whole way there.
Asked the user to confirm which real scenario they meant
([`AskUserQuestion`]); they confirmed both offered readings combined: a
cap on one end, with an expansion joint somewhere along the length that
isolates the thrust before it reaches the far end.

Resolution: this maps onto the **existing** two-state model with no new
"mixed" option needed. The segment between the cap and the joint carries
full closed-end thrust (still `Closed` for that segment); a segment
analyzed past the joint carries none (`Open`). There is no third physical
state for a single uniform segment's axial stress - what was missing was
explanatory copy, not a new formula. Added a note under the End condition
chips stating exactly this, including the "closed-on-one-end doesn't
become open just because the member is long" correction plainly.

## Buckling: pulled in from the backlog, real research first

Two real formulas, one **fully derived** and one **cited** - stated
explicitly which is which, not blurred together (see `buckling.rs`'s own
module doc for the complete reasoning):

- **Fully derived**: the long-tube (`n=2` ring) buckling limit, from the
  classical Bryan (1888)/Timoshenko ring-instability energy method -
  pre-buckled state -> perturbed trial shape -> bending-moment relation
  -> eigenvalue condition `p_cr(n) = D*(n^2-1)/r^3` -> minimized over
  physically real modes (`n>=2`) at `n=2`. Every step is in the code's own
  doc comment. Cross-checked against an independently-stated diameter-
  based industry form (agrees within 2% in the thin-wall limit) and its
  own claim that `n=2` truly minimizes is checked against `n=3..19` in a
  real test, not asserted.
- **Cited, not re-derived**: Windenburg & Trilling (1934) for the finite
  unsupported-length correction. A primary source for the full Donnell/
  Von Mises finite-length eigenvalue problem was sought (a NASA technical
  report) but turned out to be a scanned image with no extractable text -
  and a 2026 literature search found Donnell's own simplified theory is
  *documented* to diverge from the correct answer specifically in the
  long-cylinder limit (Brush & Almroth, 1975), meaning a hand-
  reconstructed Donnell derivation would have been *less* trustworthy
  than the ring result above in exactly the regime this module needs to
  get right at the boundary. Windenburg-Trilling is used as the
  published, ASME-adjacent closed form directly, explicitly flagged in
  the module doc as the one cited-not-derived piece, per direct
  instruction to derive what can safely be derived and use shorthand only
  where it can't.
- **Combined**: `critical_external_pressure` takes the larger of the two,
  documented as this module's own reasoned synthesis (not itself a named
  formula from either source) - the Windenburg-Trilling formula's own
  denominator implies `P_cr -> 0` as length -> infinity, which isn't
  physical; a real long span floors at the fully-derived ring value.
- **Applicability**: only evaluated when external pressure is present
  (`NotApplicable` otherwise), an unsupported length was actually
  supplied (`InsufficientData` otherwise, never defaulted), and the shell
  is within the cited thin-shell validity range (outer diameter /
  thickness > 40, `OutsideValidityRange` otherwise) - matches the epic's
  own explicit applicability-state vocabulary rather than treating
  buckling as universally applicable.

New `CriticalLocation::UnsupportedSpan` (buckling is a global-instability
phenomenon, not a through-wall location - forcing it into
`InnerSurface`/`OuterSurface` would misrepresent what actually governs).
`margin_of_safety` promoted to `pub(crate)` so `buckling.rs` reuses the
exact same convention as the four stress-based modes, checked before
reusing it.

UI: a new "Support spacing (buckling)" input card (unsupported length;
`0.0` doubles as "not specified", matching `evaluate_buckling`'s own
gating with no need for an `Option`-aware widget); the Buckling result
appears in Checks when evaluated, or an explanatory not-applicable/
insufficient-data/outside-range note otherwise - never a silently missing
row.

## Verification

- `cargo test -p pressure-vessel-solver`: **37/37** (9 new in
  `buckling.rs`) - the textbook cross-check, the real `n=2`-minimizes-
  among-modes check, monotonic decrease with unsupported length, the
  governing-pressure combination picking WT for short spans and flooring
  at the ring value for very long spans, and all four applicability
  states.
- `cargo build --workspace` / `cargo test --workspace`: clean, all
  pre-existing suites unaffected.
- Full diff reviewed before committing (per standing instruction) - the
  shared `.field-row` rule and every pre-existing file outside this
  tool's own three files were confirmed untouched.
- UI rendering still not independently verified - no local GUI capability
  in this environment, same standing limitation as every UI phase this
  session.
