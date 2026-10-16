# Workbench breadth roadmap — 2026-10-15

Follow-on proposals after the reliability/STEP/units implementation on `codex/workbench-reliability`.
These are proposals, not implemented capabilities or permission to merge. See
[the implementation and handoff](workbench-reliability-2026-10-15.md) for validation and remaining limits.

The existing application already has a native sketch/history editor, STEP solids, hybrid meshes, adaptive analysis,
mechanical/thermal studies, modal/buckling/load-release animation, aircraft frame/truss members, domain Bushing/Eccentric
workbenches and gated local action ranking. This roadmap extends those foundations rather than proposing them again.

| Priority | Area | First reviewable increment | Evidence required before release |
|---|---|---|---|
| 1 | CAD import health | An import report listing body count, declared source units, bounding dimensions, tessellation closure, default material and surfaces smaller than the requested mesh. Keep original source and report together. | The bracket plus thin-wall, multi-body and mixed-unit STEP fixtures; no automatic geometry changes without an explicit report. |
| 1 | Camera and navigation | Fit the rotated model to the actual viewport with padding; preserve the user's camera across ordinary edits. Add a visible Fit command and named views. | Tall/wide/rotated parts fit in narrow and wide windows; real desktop interaction and HiDPI checks. |
| 1 | Verification dashboard | Show equilibrium, residual, mesh convergence and formulation limits separately, with links to the governing benchmark and a clear distinction between numerical acceptance and engineering validity. | Closed-form bar, beam, cylinder, thermal expansion and eigenvalue references; intentionally invalid/coarse models remain flagged. |
| 1 | Material provenance | Mark default versus user-assigned versus file-defined section materials. Show property source, temperature, allowable basis and consistent density/thermal units. | Imported isotropic/orthotropic sections, handbook selections and custom materials retain their meaning across save, export and conversion. |
| 1 | Reproducible engineering packages | Export the problem, source geometry hash, units, solver/settings versions, checks, warnings and result fields as one review package. | A clean checkout can reproduce a package; missing external sources and stale results are explicit failures. |
| 2 | Native Lug, Preload and Pressure Vessel workbenches | Port one established terminal workflow at a time into native cards and diagrams, calling the existing domain solver unchanged. | Domain inputs/verdicts match the existing terminal workflow; diagrams do not become an independent numerical implementation. |
| 2 | Assemblies | Stable body/instance identities, local transforms and per-body material assignment before introducing a full assembly constraint system. | Save/load and transform invariance, named supports/loads and body-local picking; no accidental fusion of separate parts. |
| 2 | General contact workflow | Expose the kernel's supported contact capabilities with explicit contact-pair definitions, normals, initial gaps and convergence diagnostics. Start with one verified class. | Hertz/contact patch and friction benchmarks; force balance, gap/penetration and interruption behavior. Unsupported combinations remain unavailable. |
| 2 | Shell and mixed-member modelling | Define a dedicated shell authoring contract: midsurface, thickness, normals, offsets and coupling to frame/solid models. Audit the kernel's actual supported formulations before exposing choices. | Membrane/bending patch tests, plate/shell references and interface load transfer. Do not present a flat formulation as a general curved shell. |
| 2 | Persistent topology references | Record operation history and explicit face provenance so supports/loads can survive edits without relying only on geometric nearest matching. | Hole/boss/fillet, suppression and STEP round trips; ambiguous or removed references require review instead of silent reassignment. |
| 2 | Dynamics beyond load release | Add forced transient histories and frequency-response workflows in separate increments; make sampling, damping and excitation units explicit. | SDOF/beam references, temporal convergence and energy accounting; bounded frame storage and live progress. |
| 2 | Study comparison and design exploration | Compare load cases and mesh studies on common physical units; then expose the existing derivative engine only for supported smooth responses. | Finite-difference/Richardson checks, nonsmooth-response flags, reproducible study provenance and numerical acceptance on every candidate. |
| 2 | Aerospace composite layups | Add an explicit laminate/sandwich section model and ply orientation/allowable inputs before exposing failure criteria. The current flat shell law does not support laminate laws. | Classical laminate references, symmetric/unsymmetric layup coupling and ply-level thermal/strength checks with declared assumptions. |
| 2 | Conservative load transfer | Import spatial pressure/traction maps onto a selected body, with coordinate frames and unit metadata. | Interpolated resultant force and moment match the source map; out-of-domain points and unmapped surfaces are reported. |
| 3 | Durability studies | Begin with a separately scoped elastic stress-range workflow and sourced S-N data; add cycle counting and mean-stress treatment only with a defined engineering contract. | Analytic cycle-counting examples, endurance limits, surface/notch assumptions and explicitly stated applicability. |
| 3 | Mass and balance | Report body/assembly mass, centre of gravity and inertia from assigned density and transforms. | Primitive analytic references, parallel-axis checks and invariance across unit systems and rigid transforms. |
| 3 | Contextual engineering assistant | Extend the current rule-constrained local ranking with explanations, confidence and provenance. Suggest the next relevant operation while keeping solver inputs and acceptance under explicit user control. | Held-out real-session evidence, deterministic fallback, opt-out/deletion, and tests proving it cannot invent unsupported operations or bypass checks. |
| 3 | Engineering batch interface | Add a dedicated engineering CLI for validate/mesh/solve/export and parameter studies; keep it separate from the existing search CLI. | The same saved problem gives the same accepted result in batch and native modes; structured errors and exit codes support automation. |
| 3 | Platform delivery and accessibility | Signed/notarized macOS bundles, Windows packaging, explicit CAD licensing assets, keyboard navigation, readable focus/contrast and display scaling. | Clean-machine packaged startup on each platform, keyboard-only critical workflows and real macOS/Windows hardware graphics tests. |

Suggested sequence: import/verification/material provenance and review packages first; then one native domain port and
assembly identities; then contact/shell/dynamics extensions behind capability gates. Assistant and optimization features
should consume verified model capabilities and results, not substitute for them.

Product choices still needed for those future increments include the first general contact class, initial shell use case,
which native domain port comes first and the supported packaged-platform baseline. Aircraft frame/truss members remain the
user-selected aerospace use case. No new cloud ML service, telemetry, proprietary CAD dependency or merge is implied.
