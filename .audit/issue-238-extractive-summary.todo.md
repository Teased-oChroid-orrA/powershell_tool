# Issue #238 source-cited extractive summary

Goal: Complete source-cited summary support with the bundled neural encoder while preserving source lines, exact results, bounded work, and offline execution.

- [x] Ground existing encoder, semantic evidence, selected preview, and Index UI ownership.
- [x] Compare two bounded designs and select one against explicit criteria.
- [ ] Implement neural extractive summary in the owning search layer and async Search UI.
- [ ] Test ranking, citations, bounds, empty input, cancellation/stale completion, exact-result identity.
- [ ] Verify neural-feature `search-core`, Search UI tests, full affected suites, and rendered behavior.

Done predicate: a user can request a concise source-line-cited extractive summary for the selected document; no model-generated claims or exact-search changes; inputs and work are capped.
