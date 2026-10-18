# Documentation index

**Current M03.3 status (2026-10-18): Stage B ACCEPTED.** M03.2 remains formally accepted at its frozen executable. Stage A protocol-v2/pure foundations are accepted at `d7727b3…` / CI 39152686625 (488 tests); Stage B read-only destination/layout evidence is accepted at `4e2bd4d…` / CI 39155316992 (**537 tests**). **The next separately authorized gate is H0 owner-native Openbox feasibility; no native Bring Here/Recover relocation implementation exists yet.**

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
- [A00-M03 placement, recovery and host selection](DesktopRoomie-A00-M03-placement-recovery-host-selection.md): accepted M03.1 contract and recovery separation; M03.2 is formally accepted at frozen executable `a46e432…`. Its M03.3 section now points to the owner-approved P0 architecture and staged roadmap for Bring Here and composed Recover.
- [A00-M03.2 approved implementation design](DesktopRoomie-A00-M03.2-approved-implementation-plan.md): authoritative independently reviewed visibility/IPC/input protocols, tests, host gates, and stop conditions. **Historical approved technical baseline with dated Show-only 2500 ms and Openbox off-workspace amendments; M03.2 is now formally accepted after H-A sign-off. The Stage-G plan below supersedes older Bring Top timing assumptions.**
- [A00-M03.2-G approved implementation plan](DesktopRoomie-A00-M03.2-stage-G-approved-plan.md): owner-approved visibility→Above composition, structured Partial semantics, narrow off-workspace layer eligibility, event-loop progression, failure/test matrices, and D1/D2 timing (**3500 ms owner / 2500 ms visibility / ≤1000 ms layer; 5000 ms Bring Top caller**). **Historical approved plan, now implemented and owner-accepted at G3 `80d7df4…`; see Stage-G host evidence below.**
- [A00-M03.2 approved staged implementation roadmap](DesktopRoomie-A00-M03.2-staged-implementation-roadmap.md): historical owner-approved A–H checkpoint boundaries, dependencies, test/host gates, publication rules, and Codex effort guidance. **M03.2 is now formally accepted**; later M03.3 work does not alter that evidence identity.
- [A00-M03.3 approved staged implementation roadmap](DesktopRoomie-A00-M03.3-staged-implementation-roadmap.md): owner-approved P0 contracts, A–G implementation checkpoints plus the H0 Openbox feasibility gate, exact Bring Here/Recover boundaries, Partial semantics, deadlines, native evidence requirements and stop conditions. **Stages A and B are accepted (`d7727b3…`, `4e2bd4d…`); next gate is separately authorized H0 native Openbox feasibility. Native relocation implementation remains disabled.**
- [Openbox host integration](host/OPENBOX.md): optional user-managed shortcut/launcher/autostart guidance, distinct from correctness-critical executable behavior. Records historical failed F evidence, manual ADD/REMOVE-HIDDEN experiments, implemented request matrix and [corrected F acceptance](host/OPENBOX.md#stage-f-acceptance-recorded-2026-10-13). Hide, local/off-workspace Show and original-workspace Bring Top are accepted on Openbox. Optional Openbox shortcuts remain user-managed.

## Acceptance and historical explanation

| Checkpoint | Evidence | Explanation |
| --- | --- | --- |
| M01 | [ACCEPTANCE](../experiments/desktop-probe/ACCEPTANCE.md) | [ENVIRONMENT](../experiments/desktop-probe/ENVIRONMENT.md): dated M01 host facts with M02 reconciliation, not a live host query. |
| Integrated M02 | [ACCEPTANCE-M02](../experiments/desktop-probe/ACCEPTANCE-M02.md) | [M02.5](../experiments/desktop-probe/M02.5-IMPLEMENTATION.md): movement/lifecycle safety; [M02.6](../experiments/desktop-probe/M02.6-IMPLEMENTATION.md): popup/capture. Later M03.1 extends the menu. |
| M03.1 | [ACCEPTANCE-M03.1](../experiments/desktop-probe/ACCEPTANCE-M03.1.md) | [M03.1 implementation](../experiments/desktop-probe/M03.1-IMPLEMENTATION.md): layer ownership, protocol, deadline decisions and corrected publication trail. |
| M03.2 Stage F | [Stage-F acceptance](host/OPENBOX.md#stage-f-acceptance-recorded-2026-10-13): executable `ccd01915bef13850067391f70f1524362a6440dc`, CI 38349035176, 347 tests, owner native gates | [Historical Openbox amendment](host/OPENBOX.md#openbox-off-workspace-visibility-amendment-2026-10-10), including failed original F candidate preserved separately. |
| M03.2 Stage H | [ACCEPTANCE-M03.2](../experiments/desktop-probe/ACCEPTANCE-M03.2.md): frozen `a46e432…`, [CI 39098551853](https://github.com/zNe4/DesktopRoomie/actions/runs/39098551853), 453 tests and H-N1/N2/N3 owner-native observations. | [M03.2 implementation](../experiments/desktop-probe/M03.2-IMPLEMENTATION.md). H23 conditional/not exercised; H14 unobserved; H28 optional. Final H-A owner sign-off completed 2026-10-18. |
| M03.2 Stage G | [Stage-G acceptance](host/OPENBOX.md#stage-g-acceptance-recorded-2026-10-13): executable `80d7df4aa1524122bd7c232f5aa09a4e39c8dd9f`, CI 38468930784, 447 tests, G1/G2/G3 review plus owner native gates | [Approved G plan](DesktopRoomie-A00-M03.2-stage-G-approved-plan.md), [architecture](ARCHITECTURE.md), [current checkpoint](STATUS.md). H native verification and final H-A acceptance are separately recorded at the later H SHA. |
| M03.3 Stage A | [Stage-A implementation](../experiments/desktop-probe/M03.3-A-IMPLEMENTATION.md): corrected executable `d7727b3…`, [CI 39152686625](https://github.com/zNe4/DesktopRoomie/actions/runs/39152686625), 488 tests. | Protocol v2/pure placement and Recover proof foundations accepted after correction; native Bring Here/Recover execution remains Unsupported. |
| M03.3 Stage B | [Stage-B implementation](../experiments/desktop-probe/M03.3-B-IMPLEMENTATION.md): executable `4e2bd4d…`, [CI 39155316992](https://github.com/zNe4/DesktopRoomie/actions/runs/39155316992), 537 tests. | Strict read-only destination count/current/workarea/viewport/retained-monitor/fit evidence and two-read agreement; owner accepted after independent source review. H0 is next and remains separate native evidence. |

Earlier pending/unimplemented labels describe their original checkpoints. Lifecycle notes point to later acceptance without rewriting historical contracts or observations.

## Plans, research and possibilities

- [Product roadmap](DesktopRoomie-roadmap-v0.1.md): proposed long-term adventures and private-content directions, not implemented services.
- [A00/A01 execution plan](DesktopRoomie-A00-A01-plan-v0.1.md): proposed desktop goals, defaults, boundaries and future scenes.
- [R00 research roadmap](research/ProjectVanilla-R00-external-projects-research-roadmap.md): just-in-time investigations and timing; not a list of required implementations.
- [Completed R00-M01 host study](research/studies/R00-M01-nekoai-linux-host-a00-g3.md): target-host findings and recommendations informing A00-G3; not authority to adopt third-party code or a new host.
- [Idea inventory](research/project_vanilla_all_ideas_design_inventory_v0.1.txt): historical design space, alternatives and experiments, not commitments.
- [DesktopRoomie verification skill](../.agents/skills/desktoproomie-verify/SKILL.md): Rust-change verification procedure; docs-only missions use content/diff review.
