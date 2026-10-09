# Documentation index

DesktopRoomie is the reusable software; Project Vanilla is its private character implementation. This map separates implemented facts and evidence from future possibilities. The [current checkpoint](STATUS.md) carries changing status; this index is not a roadmap or implementation authorization.

## Authority

| Material | Role and boundary |
| --- | --- |
| Active approved mission contract and explicit approved decisions | Assigned scope and acceptance requirements; approved corrections take precedence over older proposals. |
| [AGENTS](../AGENTS.md) / [WORKFLOW](WORKFLOW.md) | Mandatory execution restrictions and supervised collaboration/publication process. |
| [INVARIANTS](INVARIANTS.md) | Accepted non-regression review obligations; do not infer authority to override a specific approved change. |
| [STATUS](STATUS.md) | Current checkpoint and evidence pointers; no permission to begin work. |
| [ARCHITECTURE](ARCHITECTURE.md) / [source](../experiments/desktop-probe/src/) | Descriptive ownership map / implemented structure. Correct documentation when source and evidence disagree. |
| Acceptance records | Observations and review for identified revisions, with host, automated, injected, and pending evidence distinguished. |
| Implementation notes | Historical rationale and checkpoint state, not a current acceptance ledger. |
| Roadmaps / research / inventory | Proposals / evidence and recommendations / possibilities; none automatically authorizes coding. |

## Reading profiles

- **Implementation:** [AGENTS](../AGENTS.md) → [STATUS](STATUS.md) → [ARCHITECTURE](ARCHITECTURE.md) → [INVARIANTS](INVARIANTS.md) → assigned approved mission. Use this index only for relevant deeper evidence.
- **Research/design:** STATUS → relevant roadmap and completed study → bounded next mission; follow WORKFLOW for research ownership and approval.
- **Independent code review:** assigned contract and approved corrections → INVARIANTS → ARCHITECTURE → exact executable-SHA diff and matching CI → applicable acceptance and implementation notes. A docs-only HEAD is not a new tested executable.
- **Evidence/acceptance:** STATUS → relevant acceptance record and contract checklist → environment record, with revision and date boundaries → WORKFLOW. Preserve previous observations; add only actual new results.

## Context and mission contracts

- [D00-M01 documentation/context hygiene](DesktopRoomie-D00-M01-documentation-context-hygiene.md): **accepted** docs-only context layer, with independent review and owner approval recorded in the mission document.
- [A00-M01 desktop probe](DesktopRoomie-A00-M01-desktop-probe.md): accepted foundation's original contract.
- [A00-M02 dragging and release](DesktopRoomie-A00-M02-dragging-and-release.md): accepted foundation's original detailed contract.
- [A00-M03 placement, recovery and host selection](DesktopRoomie-A00-M03-placement-recovery-host-selection.md): accepted M03.1 contract plus the client-approved 2026-10-09 split (M03.2 Bring Top; M03.3 Bring Here and composed Recover); later scope remains unimplemented. Read STATUS before interpreting historical next-action language.
- [Openbox host integration](host/OPENBOX.md): optional user-managed shortcut/launcher/autostart guidance, distinct from correctness-critical executable behavior. Future CLI samples are not yet working commands.

## Acceptance and historical explanation

| Checkpoint | Evidence | Explanation |
| --- | --- | --- |
| M01 | [ACCEPTANCE](../experiments/desktop-probe/ACCEPTANCE.md) | [ENVIRONMENT](../experiments/desktop-probe/ENVIRONMENT.md): dated M01 host facts with M02 reconciliation, not a live host query. |
| Integrated M02 | [ACCEPTANCE-M02](../experiments/desktop-probe/ACCEPTANCE-M02.md) | [M02.5](../experiments/desktop-probe/M02.5-IMPLEMENTATION.md): movement/lifecycle safety; [M02.6](../experiments/desktop-probe/M02.6-IMPLEMENTATION.md): popup/capture. Later M03.1 extends the menu. |
| M03.1 | [ACCEPTANCE-M03.1](../experiments/desktop-probe/ACCEPTANCE-M03.1.md) | [M03.1 implementation](../experiments/desktop-probe/M03.1-IMPLEMENTATION.md): layer ownership, protocol, deadline decisions and corrected publication trail. |

Earlier pending/unimplemented labels describe their original checkpoints. Lifecycle notes point to later acceptance without rewriting historical contracts or observations.

## Plans, research and possibilities

- [Product roadmap](DesktopRoomie-roadmap-v0.1.md): proposed long-term adventures and private-content directions, not implemented services.
- [A00/A01 execution plan](DesktopRoomie-A00-A01-plan-v0.1.md): proposed desktop goals, defaults, boundaries and future scenes.
- [R00 research roadmap](research/ProjectVanilla-R00-external-projects-research-roadmap.md): just-in-time investigations and timing; not a list of required implementations.
- [Completed R00-M01 host study](research/studies/R00-M01-nekoai-linux-host-a00-g3.md): target-host findings and recommendations informing A00-G3; not authority to adopt third-party code or a new host.
- [Idea inventory](research/project_vanilla_all_ideas_design_inventory_v0.1.txt): historical design space, alternatives and experiments, not commitments.
- [DesktopRoomie verification skill](../.agents/skills/desktoproomie-verify/SKILL.md): Rust-change verification procedure; docs-only missions use content/diff review.
