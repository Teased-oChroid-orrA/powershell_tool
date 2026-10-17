# Issue 46 — Phase 46: bounded whole-body p-curve locus diagnostics

PR #73 merged at `f637443d27dc96fd20e21c3a988850f027ae762a`, old branch deleted. Continuation: `feature/native-cad-body-pcurve-audit`.

`step_curve_audit::audit` discovers distinct face/edge/p-curve associations on source planes/cylinders and dispatches through the existing shared line, circle and closed quadratic circle checks. Bounded wrapper traversal and exact document identities remain authoritative. Missing face p-curves or unsupported geometry refuse rather than creating a successful partial report. Repeated oriented edge uses count against their own budget, even when association keys deduplicate.

Hard limits cover discovery edge uses, distinct associations, cumulative admitted spline controls and cumulative verified spline spans. Lightweight source headers determine association before spline decoding; the remaining aggregate control budget limits allocation. Existing per-declaration caps continue to apply. The remaining span budget is passed into each spline verification. Progress reports discovery, completed/total associations and cumulative spline work; cancellation through the last association returns no report. These are diagnostic reports, not solver/backend admission tokens.

The feature-off developer CLI accepts `--pcurve-audit`, printing progress with an unknown total during discovery and a stable total during verification. Run:

```sh
cargo run -p solid-kernel --example step-inventory --no-default-features --locked --offline -j1 -- nema17-motor-bracket.step --pcurve-audit
```

The actual bracket passes 228 scoped checks: 176 finite lines, 37 primitive whole-circle carriers and 15 closed quadratic whole-circle carriers; 105 spline controls and 45 spans are accounted. This is **not** finite arc/master/trim/body validity, a single-winding guarantee or general full spline import. The CLI states that scope explicitly. Tests verify totals, monotonic progress, every aggregate budget, discovery/span/final cancellation, stale context and missing associations.

A read-only probe of the qualified reports found all 228 natural orientation signs positive relative to their 3D carriers; 114 checked p-curves are their edge's declared master. This is useful evidence for the next master/finite-trim investigation, not proof of complete parameter/master semantics.

[Verification](issue-46-phase-46-verification.json): 166 CAD-enabled / 149 feature-off tests pass, zero failures/ignored; app CAD lib/tests compilation and the actual feature-off CLI run pass. One unoptimized CLI process took 0.00944 seconds wall and 9344 KiB peak child RSS on Linux; cache state was uncontrolled, so this is a single-fixture baseline, not representative corpus throughput. GNU time was absent; measurement used an isolated Python parent and standard child resource accounting. No source development test failures, new dependency or hosted CI. OCCT remains production. Continue finite/master/trim/body and general spline/PLC/quality/P2/P3/final gates.
