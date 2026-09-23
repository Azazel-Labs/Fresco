# Future proposals

This directory tracks **potential future work**, not the implemented language or
an agreed delivery roadmap. A proposal may be explored, changed, deferred, or
rejected. Example syntax is illustrative unless explicitly identified as existing
behavior. Inclusion here does not commit Fresco to implementing it.

[LANGUAGE.md](../../LANGUAGE.md) describes current semantics.
[TODO.md](../../TODO.md) and focused implementation plans track committed work.
A document describing an accepted or partly implemented feature should not be
moved here merely because some of its tasks remain unfinished.

## Proposal index

| Proposal | Status | Open decision / next investigation |
| --- | --- | --- |
| [Technique allocation, lifetime, and composition](technique-contract-generalization.md) | Exploring | Decide shared allocation descriptors, persistent version ownership, and cross-path composition |
| [Coordinate domains and coverage filtering](coordinate-domain-coverage-design.md) | Exploring | Define domain/footprint semantics and reference coverage before settling syntax |

## Status conventions

- **Exploring:** a potential direction; requirements, feasibility, or semantics are open.
- **Deferred:** retained for possible reconsideration; record why and what would reopen it.
- **Accepted:** a decision has been made; link the decision and implementation plan. Acceptance does not mean implemented.
- **Rejected / superseded:** record the reason or replacement so the same question is not rediscovered without context.

Keep the document status and index in agreement. Do not assign dates, owners,
priority, or implementation commitments that have not actually been agreed.

## Adding and advancing a proposal

Use one descriptively named Markdown file per proposal. Include:

1. Status and origin/revision date.
2. The problem and motivating examples.
3. Current behavior versus the proposed change, with explicit non-goals.
4. Candidate semantics/syntax, alternatives, tradeoffs, and unresolved questions.
5. Dependencies and an investigation/acceptance checklist where useful.
6. Decision history and links to related proposals or an implementation plan.

Add an index row with the next unresolved question. Investigation checkboxes
measure investigation, not feature delivery. When accepted, link a separate
implementation plan and preserve the decision here. Update `LANGUAGE.md` as
behavior ships; link that implemented account rather than maintaining another
current specification in the proposal. Git preserves earlier drafts.
