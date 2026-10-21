# A00-RF — DesktopRoomie Rust modularization and session-by-session refactor plan

**Design decision, 2026-10-21:** RF-P read-only architecture audit completed and independently reviewed. **Approved direction:** extract selected inline unit-test modules first; reassess production-only ownership afterward, then authorize small cohesive production extractions only where they improve dependency direction or maintainability. **Next candidate assignment: RF1a only.** This is a planning and documentation record, not implementation, source review, CI or native acceptance. No RF coding checkpoint is complete.

**Repository:** `zNe4/DesktopRoomie`, mission branch `m03`.
**RF-P audited documentation HEAD:** `5167a9aef2f55653776494e61f1a76ac539827f3`.
**Unchanged accepted executable:** `aa00f8e94aff1cf2ef1ff6a6cecf734f822d5603`; [Stage-E CI 39529051360](https://github.com/zNe4/DesktopRoomie/actions/runs/39529051360) (**737 tests**, fmt/build/Clippy passed); [Stage-E acceptance](../experiments/desktop-probe/M03.3-E-ACCEPTANCE.md) documents qualified Openbox/X11 owner N01–N19.
**Publication discipline:** a later docs-only HEAD must never be called the last tested executable. When a future RF implementation changes source, that exact code SHA will require its own source review, matching CI and applicable host checks.

## 1. Purpose, status and relationship to M03.3

The long-term goal is understandable, cohesive Rust modules with explicit dependencies and preserved ownership—not arbitrary file-size quotas. In particular, do **not** split structures, enums, constants, functions or tests merely because they are different item kinds. Cohesive small modules and self-contained unit tests can remain inline.

**M03.3 Stage E is accepted. Stage F planning is paused before implementation while this separate structural effort proceeds.** The original [M03.3 roadmap](DesktopRoomie-A00-M03.3-staged-implementation-roadmap.md) retains its F (lifecycle/concurrency/deadline hardening) and G (integrated M03.3 acceptance) gates and all accepted invariants. RF checkpoints neither complete F/G nor silently change their contract. The owner independently approves every new RF assignment and later continuation of F.

Owner/ChatGPT/Codex workflow: ChatGPT plans/independently reviews, Codex carries out one approved bounded mission, the owner supervises/approves native acceptance and publication. Follow [AGENTS](../AGENTS.md), [WORKFLOW](WORKFLOW.md), [STATUS](STATUS.md), [ARCHITECTURE](ARCHITECTURE.md) and [INVARIANTS](INVARIANTS.md).

## 2. RF-P source inventory and interpretation

Audit scope: **all 20 Rust files** under `experiments/desktop-probe/src/`, at `5167a9aef2f55653776494e61f1a76ac539827f3`. Physical lines include whitespace, documentation and comments; tests are counted by `#[cfg(test)]` modules including the existing external `destination/tests.rs`. This is a **read-only source count**, not a fresh Cargo test execution.

| File | Total lines | Approx. non-test lines | Test lines | `#[test]` |
| --- | ---: | ---: | ---: | ---: |
| `main.rs` | 19,727 | 6,033 | 13,694 | 339 |
| `control.rs` | 5,940 | 1,926 | 4,014 | 93 |
| `x11/control.rs` | 6,434 | 1,700 | 4,734 | 102 |
| `interaction.rs` | 1,629 | 646 | 983 | 37 |
| `x11/window.rs` | 1,485 | 723 | 762 | 17 |
| `x11/visibility.rs` | 1,342 | 564 | 778 | 17 |
| Remaining 14 files, combined | 5,865 | 2,773 | 3,092 | 132 |
| **Total (audit)** | **42,422** | **14,365** | **28,057** | **737** |

Approximately **66.1%** of physical lines are tests. Externalizing large tests improves navigation without changing production ownership. However, `main.rs` still contains roughly 6,000 non-test lines, so test moves are *not* the entire refactoring goal.

The existing accepted pattern is `src/x11/monitors/destination.rs` with `#[cfg(test)] mod tests;` and `src/x11/monitors/destination/tests.rs`. Rust unit-test child modules should retain their **existing module paths**, rather than becoming Cargo `tests/` integration crates requiring wider API visibility.

## 3. Production ownership and dependency map (verified RF-P observations)

| Source region at RF-P HEAD | Current ownership | Initial disposition |
| --- | --- | --- |
| `main.rs:30–155` | CLI parsing/help | Candidate separate `cli.rs` |
| `main.rs:157–518` | Entry/startup, owner publication and cleanup | Keep coordinated |
| `main.rs:520–1118` | `ProbeRuntime`, pointer/gesture/input fences | Keep together |
| `main.rs:1695–1992` | Native layer service around the sole pure controller | Candidate `layer_service.rs` after review |
| `main.rs:2096–3657` | Hide/Show/Bring Top, Recover Top readiness | Keep coupled initially |
| `main.rs:3659–4802`, `5415–5680` | Here/Recover progression and borrowed native adapters | Keep coupled initially |
| `main.rs:5682–6059` | Event classification, deadline/owner service, shared fatal policy | Keep ordered initially |
| `main.rs:6061–7127` | `run_probe` event loop, termination and wait helpers | Keep loop; perhaps extract only shared wait/drain helpers |
| `control.rs:9–480` | Pure protocol v2/wire/correlation vocabulary | Conditional `control/protocol.rs` candidate |
| `control.rs:482–1923` | One operation's progress, proof, admission, visibility and result validators | Keep together initially |
| `x11/control.rs:28–1119` | Selection/endpoint owner, active slot, peer transport and routing | Keep centralized |
| `x11/control.rs:1121–1313` | Caller outcome and diagnostic text | Candidate `x11/control/outcome.rs` |
| `x11/control.rs:1315–1699` | Independent caller invocation, discovery, correlation, cleanup | Conditional `x11/control/caller.rs` candidate |
| `geometry.rs`, `interaction.rs`, `layer.rs`, `x11/window.rs`, `x11/visibility.rs`, smaller `x11/*` modules | Cohesive geometry, gesture, single layer controller, managed body, native evidence/transport | No automatic production split |

The key direction remains root coordination → pure `control`/`geometry`/`interaction`/`layer` and borrowed `x11` adapters. `ControlWindow` owns the sole `Option<ActiveOperation>`; `ProbeRuntime` owns the single layer controller, interaction/pointer debt, popup and input fence. `NativeVisibility`/`NativeHere` are adapters, **not** independent owners or queues.

Known reverse support references: `x11/control.rs` calls root `wait_x11` and `drain_events_bounded`; `x11/window.rs` uses root `CONFIRMATION_TIMEOUT`. A narrowly scoped `x11/events.rs` could remove the wait/drain reverse dependency without changing scheduling; this is **not yet authorized**.

Do not split `ProbeRuntime` merely to expose all its fields. Do not separate `PlacementProgress` from the result/proof authority through cyclic, broadly visible modules. Do not break apart `run_probe` match arms or Here/Recover fatal policy until their event order is explicitly proven stable.

## 4. Test organization: approved direction, implementation separately gated

Use external **unit-test child files**, retaining the same Rust module names, paths, fixtures and assertions. Keep small inline tests if moving them would not materially aid maintenance.

| Proposed gate | Bounded scope | Evidence and special risks |
| --- | --- | --- |
| **RF1a — NEXT** | `control::tests` only: `control.rs` + new `control/tests.rs` | 26 tests, exact unchanged names; no production edit |
| RF1b | `control::here_progression_tests` and `control::placement_tests`, in separate or tightly bounded steps | All 93 `control` tests unchanged |
| RF1c | Selected smaller root tests (e.g., map/input, visibility fence, menu, CLI, owner) in bounded batches | Preserve `tests::test_window` and root sibling references |
| RF1d | `main.rs::show_tests` intact | ~5,256 lines, 149 cases, nested `h_c2`/`g3_hardening` and other-module fixtures |
| RF1e | `main.rs::here_service_tests` intact | ~4,236 lines, 99 cases, two source-text `include_str!` assertions |
| RF1f | `x11/control.rs::tests` and `transport_tests`, independently | 27 + 75 cases; preserve `pub(crate) transport_tests` and cross-module fixture wiring |
| RF1g (optional) | Other sufficiently large inline test suites | Prefer cohesive and useful extraction; do not force tiny modules apart |

Potential root layout: `src/probe_tests/<module>.rs` with explicit `#[path = "probe_tests/<module>.rs"]` on root test module declarations; alternative module path must preserve test identity and be approved at the individual checkpoint. For `control.rs`, use ordinary `#[cfg(test)] mod tests;` with `src/control/tests.rs`. For `x11/control.rs`, prefer `src/x11/control/{tests,transport_tests}.rs`. The parent-private unit-test access should remain valid, without making production internals `pub(crate)` merely for testing.

**Fixture/source-text hazards (required RF1 gates):**

- Root `tests::test_window` is shared by sibling tests.
- `show_tests::CallerTimeoutShow`, `h_c2` and `g3_hardening` supply lifecycle and transport fixtures.
- `here_service_tests::lifecycle_snapshots` and service fakes are used by native transport tests; `x11::control::transport_tests::HereOwner` is used by root Here/Recover tests.
- Two `here_service_tests` checks use `include_str!("main.rs")`. When moved to `src/probe_tests/here_service_tests.rs`, those macro paths must change to `include_str!("../main.rs")` while continuing to inspect the **same production source**.
- Source-text tests check the fatal loop's sites, owner routing, 64-event batch, deadline and end-of-batch ordering. Later production moves must preserve their intent and coverage; do not remove or weaken them merely to make extraction pass.
- Preserve existing `pub(crate)` visibility where required and avoid building a new test framework.

For each test move, compare the **complete set of test names** before/after (not merely the count of 737); preserve all reachable nested modules, fixtures and assertions. Capturing a baseline with `cargo test --manifest-path experiments/desktop-probe/Cargo.toml -- --list` is appropriate when authorized. Local `/tmp` diagnostic output is disposable and should not be committed.

## 5. Conditional production roadmap — not pre-approved implementation

**RF2-P is a mandatory second architectural review after selected RF1 test moves.** Re-inspect production-only source/coupling and approve each module boundary before extraction. The RF-P suggestions below are candidates, not standing instructions.

| Candidate | Proposed destination | Relative risk and review focus |
| --- | --- | --- |
| Caller diagnostic/outcome formatting | `src/x11/control/outcome.rs` | Low: preserve exact message bytes/exit codes and existing re-export |
| CLI/help | `src/cli.rs` | Low: preserve grammar, precedence, help and root test fixture lookup |
| Shared X11 `poll` + bounded drain | `src/x11/events.rs` | Low–medium: flush, EINTR, timeout ceil, buffered-events-first |
| Layer-service adapter | `src/layer_service.rs` | Medium: sole controller, removal/addition guards, focus/input and deadline ordering |
| Pure protocol codec | `src/control/protocol.rs` | Medium: wire values, validation order, private proof access; avoid widening beyond `pub(super)` where needed |
| Invoking client lifecycle | `src/x11/control/caller.rs` | Medium: capture before discovery, one frozen marker/send, bounded Unknown, exact owner/reply correlation and cleanup |
| Diagnostics only (optional) | `src/diagnostics.rs` | Low but small payoff |

**RF3** is a conditional review after RF2, not authorization to split the event loop. If earlier extractions do not improve ownership or would require cyclic dependencies or additional state owners, stop and leave the cohesive code together.

No new `types.rs`/`constants.rs`/`utils.rs` dumping grounds, library target, framework, dependency, protocol redesign, owner, layer controller, queue or retry system should be introduced for the sake of code distribution.

## 6. Invariants and stop conditions for all implementation gates

- Preserve all five commands and their complete current outcomes, exit codes, wire protocol v2, owner epoch/request IDs, `Busy` and no-owner behavior, and no implicit owner launch.
- Preserve fixed owner/caller budgets (ms): Hide **1000/4000**, Show **2500/4000**, Bring Top **3500/5000**, Bring Here **3500/5000**, Recover **7000/9000**.
- Recover retains Here ≤3500 ms, Top visibility ≤2500 ms from validated handoff, layer ≤1000 ms after admission, capped by unchanged owner total, and **one Recover-wide** checked correction needing a full **150 ms** interval.
- Retain one active operation slot and same-body/frozen destination/immutable correlation through handoff. Current success proof clears at handoff but genuinely confirmed relocation/restoration history and consumed attempts remain.
- Only confirmed historical relocation/restoration can justify Recover Partial after incomplete current proof. Native acknowledgement, existing state, no-op, possible dispatch or position correction cannot manufacture Success/Partial.
- Preserve one shared fatal owner finalizer, original-error priority, bounded read-only history, safe re-fencing, no automatic retries/resends, at most one correlated terminal response, and caller OutcomeUnknown when timely proof is absent.
- Preserve the single `LayerController`, body/colormap resource ownership, pointer release debt, SHAPE click-through, exact silhouette, checked geometry/root-versus-local spaces, retained-monitor identity, no activation/focus theft or user workspace switching.
- **Event order is behavior:** control event before ordinary movement/menu events; immediate service after consumed request at both routing sites; Here/Recover evidence before deferred input restoration; bounded batches (>64) cannot starve deadlines; end-of-batch service before socket wait; lookahead before coalesced movement flush; fatal placement finalization before global cleanup.
- Do not infer a hard upper bound for stalled synchronous X11 operations from logical operation budgets.
- Stop and request review for behavioral diffs, broader API visibility, source-text tests that cannot be preserved, unrelated dirty changes, missing/wrong tests, failed checks/CI, branch divergence or a required module/ownership redesign.

## 7. Checkpoint rules for fresh Codex sessions

Use **one new Codex session for each small RF checkpoint**, optionally a separate read-only reviewer session. Each session starts from repository files, not the previous agent's conversational state:

1. Read `AGENTS.md`, `docs/STATUS.md`, `docs/WORKFLOW.md`, `docs/INVARIANTS.md`, this plan, and only relevant implementation/acceptance evidence. No need to load the entire idea inventory.
2. Before edits verify `zNe4/DesktopRoomie`, `m03`, local HEAD, **live** `origin/m03` (not a stale local remote-tracking ref), clean tracked/untracked tree and frozen executable provenance. The initial RF-P docs baseline was `5167a9aef2f55653776494e61f1a76ac539827f3`; **later assignments must use the exact latest owner-specified baseline, including this plan-publication commit**. Do not use `5167a9aef2f55653776494e61f1a76ac539827f3` blindly once it is superseded.
3. The assignment states **Goal / Context / Constraints / Done when**, allowed paths, stop conditions and whether commit/push is authorized. This plan alone is **not** standing permission to edit or publish any checkpoint.
4. Before refactoring capture test-name inventory and inspect exact definitions/fixtures. Change only the assigned unit. If a necessary extra path or semantic repair appears, **stop** before widening scope.
5. For any Rust change run from repo root: `cargo fmt --manifest-path experiments/desktop-probe/Cargo.toml --check`; `cargo build --manifest-path experiments/desktop-probe/Cargo.toml`; `cargo clippy --manifest-path experiments/desktop-probe/Cargo.toml --all-targets -- -D warnings`; `cargo test --manifest-path experiments/desktop-probe/Cargo.toml`. Compare complete test names and test results; run `git diff --check`, review full diff and report final status.
6. Codex returns **exact starting/final SHA, modified paths, full check results, identical/changed test names, any behavior change, risks and remaining evidence**. Commit/push only with **specific** owner authorization for that checkpoint and an ordinary non-force update of `origin/m03`. Independently review the **exact** diff and matching CI. A passing local run or push is not acceptance.
7. Test-only extractions normally need no new owner desktop acceptance when production is byte-equivalent and full regressions pass. Production changes that affect layer, waiting, caller or owner paths need appropriate owner-native Openbox focus/typing, click-through/drag/popup, visible/minimized Here/Recover, Busy, no-owner or relaunch regressions. Keep injected/native evidence separate.
8. After owner acceptance, update STATUS/evidence only as explicitly authorized. A newly opened Codex session never automatically proceeds to RF1b, RF2, Stage F, merges or later stages.

**Model choice is per mission, not permanent:** RF1a is suitable for GPT-6 Medium; more coupled production changes may merit GPT-6 High. The owner/ChatGPT selects based on actual complexity and available Codex model settings.

## 8. RF1a — first bounded assignment contract (ready to copy into a new Codex session)

**Goal:** Externalize only `control::tests` (26 cases) using the existing Rust child-unit-test pattern. Improve readability **without changing production definitions or behavior**.

**Context:** exact paths `experiments/desktop-probe/src/control.rs` and **new** `experiments/desktop-probe/src/control/tests.rs`. In RF-P's inspected source, the inline module occupies roughly lines 1925–3387. Accepted current executable E4 and the RF-P inventory above are historical baselines; obtain a fresh expected starting SHA from the documentation publication/review and check it live.

**Constraints:**

- Replace the existing `#[cfg(test)] mod tests { ... }` with `#[cfg(test)] mod tests;` and move its **entire existing module body** into `src/control/tests.rs`.
- Retain all 26 `control::tests::*` names, fixture definitions, imports and assertions. No nested module restructuring.
- **Exactly two allowed source paths:** `src/control.rs` and new `src/control/tests.rs`. No production logic, type, protocol, test semantics, comments outside the module, dependency, configuration or docs changes.
- Do not format/reorganize unrelated code, add a test framework, widen visibility, alter other modules or run the GUI. Stop for any non-mechanical complication.

**Done when:** complete before/after test-name sets match, all **737** tests remain present and pass, all four Cargo checks pass, `git diff --check` passes, and a reviewer can verify the module extraction was the only change. State the exact changed paths and final Git status.

**Publication for the initial RF1a implementation assignment:** **NO commit, push, merge or automatic RF1b advancement.** Return the uncommitted diff and verification for independent review. Separate publication can be authorized later, and subsequent sessions must use its exact published SHA.

A compact per-session assignment may reference **Section 8** rather than copy this whole document, provided it explicitly supplies the actual fresh pinned branch/base SHA and preserves all stop/publication constraints.

## 9. Remaining decisions and documented stale comments

RF1a requires no further architecture choice. Later owner approvals must settle: which additional test modules warrant moving; exact RF2 subset and necessary `pub(super)` adjustments; native regression scope; whether stale comments are corrected in a separate small source-maintenance assignment; and whether any residual shared `run_probe` code should be redesigned (not presumed).

RF-P found stale text/comments at `control.rs:509` (Recover “pure/unwired”), `main.rs:3753` (guards through E3), `x11/monitors/destination.rs:3,68,217`, `x11/desktop.rs:3`, `x11/shape.rs:41`, and legacy diagnostic strings near `main.rs:190,393,7171`. These are **not** RF1a changes. Behavioral/diagnostic output text must not be silently altered under a mechanical refactor.

**RF-P disposition:** read-only inventory reviewed; strategy approved; RF1a is the next independently assignable implementation checkpoint. No refactoring code or fresh Cargo/native execution is represented by this planning document.
