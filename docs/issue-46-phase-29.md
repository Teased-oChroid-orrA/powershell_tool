# Issue 46 — Phase 29: bounded global point-to-curve distance

Source `d355037` adds `nurbs_distance::distance`, a deterministic branch-and-bound query over every nonempty knot span. Interval boxes provide lower bounds on distance to each curve subdomain; point enclosures provide upper bounds for actual curve parameters. A priority queue refines the smallest remaining lower bound. Pruning uses the current upper bound; termination requires the outward-rounded global upper-minus-lower distance gap to meet the requested tolerance.

The result records the global distance bracket, a candidate parameter, its binary64 evaluated representative, the enclosure of the exact curve point at that parameter and work count. The mathematical candidate's distance is within the requested objective gap of the global minimum. This does **not** bound error in the nearest parameter/position, prove a unique minimizer, establish a regular projection, validate a trimmed edge, or enable full spline B-Rep import. Nonunique minima (e.g. circle centre) remain valid distance queries without a uniqueness claim.

Finite query/arithmetic-domain and positive tolerance validation, enclosure/queue budgets, precision exhaustion, deterministic tie ordering, monotonic progress counters and initial/adaptive/final cancellation prevent returning unconverged results. Geometry units remain explicit; distance tolerances use the control coordinate units and knot parameters are unchanged.

## Reproduced conditioning problem and correction

Initial tests passed three groups, but a constant rational curve with unequal positive weights exhausted the unchanged 100,000-enclosure budget at 1e-6 accuracy. Independent homogeneous numerator/denominator intervals discarded their exact correlation. The expected distance was known analytically; the failing development log is preserved separately.

The curve enclosure now also intersects its rational interval with the active **Cartesian** control-point hull. Positive rational basis functions form a partition of unity, so this is an exact geometric bound, including constant curves and unequal weights. It is not a tolerance increase or a budget workaround. The same constant-curve query now passes with the original budget/tolerance, and all earlier enclosure/sampling tests remain required. No acceptance limit or default resource budget changed.

Four regression groups verify rational straight-line interior/endpoint distances; a degenerate constant curve; analytic circle interior/endpoint/nonunique distances; searching a later C0 knot span; length/common-weight/parameter transformations; and work/queue/input/cancellation failures. The tests compare against independent analytic distances rather than accepting a local optimizer residual.

No new dependency, production backend switch, hosted CI, main modification or OCCT retirement. Enclosure conservatism may still exhaust budgets at tight tolerances, particularly for many near-minimizers; failure remains explicit. Not representative CAD performance or training evidence. Full checks and development failure are recorded in [verification](issue-46-phase-29-verification.json).

Next: tensor-surface patch enclosures and bounded distance/projection candidates; source-bound edge sampling and endpoint/locus verification; p-curves/trims and full spline-body admission. General CAD/assembly, constrained PLC recovery/refinement/cavities, sliver/conditioning and P2/P3/final gates remain open.

Final checks at `d355037d06abaf578410d7158ac9ac3b2ce408ce`: **118 CAD-enabled / 101 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero final failures/ignored checks. The development failure remains separately recorded.
