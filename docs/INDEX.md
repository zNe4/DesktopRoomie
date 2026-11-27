# Documentation index

Use [STATUS](STATUS.md) for the current checkpoint, [WORKFLOW](WORKFLOW.md#document-responsibilities) for authoritative document homes and [AGENTS](../AGENTS.md) for mandatory bootstrap guardrails. This is a directory, not a parallel acceptance or publication ledger.

## Reading profiles

All roles bootstrap: AGENTS → STATUS → explicit assignment and approved specification → relevant profile below. Before assigned actions, selectively read applicable [ARCHITECTURE](ARCHITECTURE.md), [INVARIANTS](INVARIANTS.md) and [WORKFLOW](WORKFLOW.md) obligations; record applicability or justified non-applicability. Load historical evidence only when a requirement, claim or finding depends on it. No previous conversations are required.

| Role/task | Additional context before action |
| --- | --- |
| Architect / research-design | Relevant approved decisions, roadmap and completed research; WORKFLOW role/scope limits. Research is evidence, not coding authority. |
| Implementer | Affected implemented owners/interfaces, technical contracts and assigned evidence; documentation workers load the owning document and authority sources. |
| Verifier | Mission check requirements, changed components, applicable invariants and [Rust verification procedure](../.agents/skills/desktoproomie-verify/SKILL.md) for Rust changes; documentation uses WORKFLOW checks. |
| Independent reviewer | Exact contract/candidate, previous review anchor and open findings, delta plus cumulative diff, affected owners/obligations and evidence actually needed. |
| Owner / evidence-acceptance | Exact review/evidence records, required native or cold-start gates, environment and revision boundaries, WORKFLOW publication/acceptance authority. |

Source inspection: [implemented modules](../experiments/desktop-probe/src/). Host qualification: [A00 supported envelope](DesktopRoomie-A00-M03.4-integrated-acceptance.md#7-supported-host-envelope-and-qualifications). Build/run entry point: [README](../README.md).

## Workflow contracts

- [D00-M01 accepted context layer](DesktopRoomie-D00-M01-documentation-context-hygiene.md): historical contract and acceptance.
- [DR-W01 workflow/context contract](plans/DR-W01-SPECIFICATION.md): responsibility map, risk/review design, acceptance gates and DR-W02 boundary; [final acceptance record](plans/DR-W01-SPECIFICATION.md#12-final-acceptance-and-dr-w01-f-closeout--2026-11-26) for exact candidate and review provenance.
- [WORKFLOW](WORKFLOW.md): assignments, risk, verification, independent review, evidence and publication.

## Context and mission contracts

- A01-C1-A: [C1-A acceptance](A01-C1-A-ACCEPTANCE.md).
- A01-C1-B-B1: [B1 acceptance record](A01-C1-B-B1-ACCEPTANCE.md).
- A01-C1-B-B2: [B2 acceptance record](A01-C1-B-B2-ACCEPTANCE.md).
- A01-C1-B-B3a: [B3a acceptance record](A01-C1-B-B3a-ACCEPTANCE.md).
- A01-C1-B-B3b: [B3b acceptance record](A01-C1-B-B3b-ACCEPTANCE.md).
- A01-C1-C-A1/A2: [A1/A2 acceptance record](A01-C1-C-A1-A2-ACCEPTANCE.md).
- A01-C1-C-B1: [B1 acceptance record](A01-C1-C-B1-ACCEPTANCE.md).
- A01-C1-C-B2-D1: [D1 acceptance record](A01-C1-C-B2-D1-ACCEPTANCE.md).
- A01-C1-C-B2-D2-C: [D2-C acceptance record](A01-C1-C-B2-D2-C-ACCEPTANCE.md).
- A01-C1-C-B2-D3-A: [D3-A acceptance record](A01-C1-C-B2-D3-A-ACCEPTANCE.md).
- [A01-C1-C-B2-D3-P ratified architecture specification](plans/A01-C1-C-B2-D3-P-SPECIFICATION.md).
- [Ratified D3-B-P specification](plans/A01-C1-C-B2-D3-B-P-SPECIFICATION.md).
- [D3-B acceptance record](A01-C1-C-B2-D3-B-ACCEPTANCE.md).
- A01-C1-B-B4: [B4 acceptance record](A01-C1-B-B4-ACCEPTANCE.md).
- [A00-M03.4 integrated A00 acceptance](DesktopRoomie-A00-M03.4-integrated-acceptance.md).
- [A00-M03.4 host architecture decision](DesktopRoomie-A00-M03.4-host-architecture-decision.md).
- [D00-M01 documentation/context hygiene](DesktopRoomie-D00-M01-documentation-context-hygiene.md).
- [A00-M01 desktop probe](DesktopRoomie-A00-M01-desktop-probe.md).
- [A00-M02 dragging and release](DesktopRoomie-A00-M02-dragging-and-release.md).
- [A00-M03 placement, recovery and host selection](DesktopRoomie-A00-M03-placement-recovery-host-selection.md).
- [A00-M03.2 approved implementation design](DesktopRoomie-A00-M03.2-approved-implementation-plan.md).
- [A00-M03.2-G approved implementation plan](DesktopRoomie-A00-M03.2-stage-G-approved-plan.md).
- [A00-M03.2 approved staged implementation roadmap](DesktopRoomie-A00-M03.2-staged-implementation-roadmap.md).
- [A00-RF modularization closeout](DesktopRoomie-A00-RF-modularization-plan.md#current-closeout-2026-10-21).
- [A00-M03.3 approved staged implementation roadmap](DesktopRoomie-A00-M03.3-staged-implementation-roadmap.md).
- [A00-M03.3 Stage F-P coverage and architecture audit](DesktopRoomie-A00-M03.3-stage-F-coverage-audit.md).
- [A00-M03.3 Stage-F deterministic closeout](DesktopRoomie-A00-M03.3-stage-F-closeout.md).
- [A00-M03.3 Stage-G integrated owner-native acceptance](DesktopRoomie-A00-M03.3-stage-G-acceptance.md).
- [A00-M03.3 H0 Openbox feasibility evidence](../experiments/desktop-probe/M03.3-H0-OPENBOX.md).
- [A00-M03.3 Stage-C implementation/acceptance](../experiments/desktop-probe/M03.3-C-IMPLEMENTATION.md).
- [A00-M03.3 Stage-D acceptance](../experiments/desktop-probe/M03.3-D-ACCEPTANCE.md).
- [A00-M03.3 Stage-E acceptance](../experiments/desktop-probe/M03.3-E-ACCEPTANCE.md); [CI 39529051360](https://github.com/zNe4/DesktopRoomie/actions/runs/39529051360).
- [A00-M03.3 Stage-D historical proposed plan](DesktopRoomie-A00-M03.3-stage-D-proposed-plan.md).
- [A00-M03.3 Stage-E historical approved E-P implementation plan](DesktopRoomie-A00-M03.3-stage-E-approved-plan.md).
- [Openbox host integration](host/OPENBOX.md); [corrected F acceptance](host/OPENBOX.md#stage-f-acceptance-recorded-2026-10-13).

## Acceptance and historical explanation

| Checkpoint | Evidence and explanation |
| --- | --- |
| M01 | [ACCEPTANCE](../experiments/desktop-probe/ACCEPTANCE.md); [ENVIRONMENT](../experiments/desktop-probe/ENVIRONMENT.md) |
| Integrated M02 | [ACCEPTANCE-M02](../experiments/desktop-probe/ACCEPTANCE-M02.md); [M02.5](../experiments/desktop-probe/M02.5-IMPLEMENTATION.md); [M02.6](../experiments/desktop-probe/M02.6-IMPLEMENTATION.md) |
| M03.1 | [ACCEPTANCE-M03.1](../experiments/desktop-probe/ACCEPTANCE-M03.1.md); [M03.1 implementation](../experiments/desktop-probe/M03.1-IMPLEMENTATION.md) |
| M03.2 Stage F | [Stage-F acceptance](host/OPENBOX.md#stage-f-acceptance-recorded-2026-10-13); [Historical Openbox amendment](host/OPENBOX.md#openbox-off-workspace-visibility-amendment-2026-10-10) |
| M03.2 Stage H | [ACCEPTANCE-M03.2](../experiments/desktop-probe/ACCEPTANCE-M03.2.md); [CI 39098551853](https://github.com/zNe4/DesktopRoomie/actions/runs/39098551853); [M03.2 implementation](../experiments/desktop-probe/M03.2-IMPLEMENTATION.md) |
| M03.2 Stage G | [Stage-G acceptance](host/OPENBOX.md#stage-g-acceptance-recorded-2026-10-13); [Approved G plan](DesktopRoomie-A00-M03.2-stage-G-approved-plan.md); [architecture](ARCHITECTURE.md); [current checkpoint](STATUS.md) |
| M03.3 Stage A | [Stage-A implementation](../experiments/desktop-probe/M03.3-A-IMPLEMENTATION.md); [CI 39152686625](https://github.com/zNe4/DesktopRoomie/actions/runs/39152686625) |
| M03.3 Stage B | [Stage-B implementation](../experiments/desktop-probe/M03.3-B-IMPLEMENTATION.md); [CI 39155316992](https://github.com/zNe4/DesktopRoomie/actions/runs/39155316992) |
| M03.3 Stage C | [Stage-C implementation](../experiments/desktop-probe/M03.3-C-IMPLEMENTATION.md); [CI 39159965288](https://github.com/zNe4/DesktopRoomie/actions/runs/39159965288) |
| M03.3 Stage D | [Stage-D acceptance](../experiments/desktop-probe/M03.3-D-ACCEPTANCE.md); [CI 39377294762](https://github.com/zNe4/DesktopRoomie/actions/runs/39377294762) |
| M03.3 Stage E | [Stage-E acceptance](../experiments/desktop-probe/M03.3-E-ACCEPTANCE.md); [CI 39529051360](https://github.com/zNe4/DesktopRoomie/actions/runs/39529051360) |
| M03.3 Stage F | [Deterministic closeout](DesktopRoomie-A00-M03.3-stage-F-closeout.md); [CI 39688537222](https://github.com/zNe4/DesktopRoomie/actions/runs/39688537222) |
| M03.3 Stage G | [Owner-approved native acceptance](DesktopRoomie-A00-M03.3-stage-G-acceptance.md) |
| M03.4 / overall A00 | [Integrated owner signoff](DesktopRoomie-A00-M03.4-integrated-acceptance.md#8-owner-signoff-and-documentation-closeout); [Ratified host ADR](DesktopRoomie-A00-M03.4-host-architecture-decision.md) |
| R00-M13 P2-C2-B2 independent offscreen renderer | [B2 acceptance](renderer/R00-M13-P2-C2-B2/B2-ACCEPTANCE.md) |
| R00-M13 P2-C2-C1 independent offscreen renderer | [C1 acceptance](renderer/R00-M13-P2-C2-C1/C1-ACCEPTANCE.md) |
| R00-M13 P2-C2-C2 independent offscreen renderer | [C2-C2 acceptance](renderer/R00-M13-P2-C2-C2/C2-C2-ACCEPTANCE.md) |
| R00-M13 P3 static Vanilla rendering | [P3 acceptance](renderer/R00-M13-P3/P3-ACCEPTANCE.md) |
| R00-M13 P4-C1 native Vanilla animation preview | [P4-C1 acceptance](renderer/R00-M13-P4-C1/P4-C1-ACCEPTANCE.md) |
| R00-M13 P4-D native animation and directional transitions | [P4-D acceptance](renderer/R00-M13-P4-D/P4-D-ACCEPTANCE.md) |

Historical labels apply to their original candidates. Use [STATUS](STATUS.md) for the maintained checkpoint and publication reconciliation; records retain distinct evidence scopes.

## Plans, research and possibilities

- [Spine backend separation decision](research/R00-M12-spine-backend-separation-decision.md).
- [Product roadmap](DesktopRoomie-roadmap-v0.1.md).
- [A00/A01 execution plan](DesktopRoomie-A00-A01-plan-v0.1.md).
- [R00 research roadmap](research/ProjectVanilla-R00-external-projects-research-roadmap.md).
- [Completed R00-M01 host study](research/studies/R00-M01-nekoai-linux-host-a00-g3.md).
- [Idea inventory](research/project_vanilla_all_ideas_design_inventory_v0.1.txt).
- [DesktopRoomie verification skill](../.agents/skills/desktoproomie-verify/SKILL.md).
