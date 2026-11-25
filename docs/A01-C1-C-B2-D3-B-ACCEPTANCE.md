# A01-C1-C-B2-D3-B — Managed supervisor fixture acceptance

## TECHNICALLY ACCEPTED — OWNER APPROVED — 2026-11-24

Git publication remains pending.

## 1. Mission chronology

D3-P established the parent architecture. The owner ratified B01–B09 in D3-B-P and technically accepted the I0 dependency preflight after independent review passed all seven gates. The bounded I1 execution assignment initially stopped during preflight because the ratified specification was missing. The owner supplied the exact specification, after which Codex completed implementation and automated verification. Independent ChatGPT source review passed all 14 gates with no blocking defect or required correction. The owner explicitly granted I1 technical acceptance on 2026-11-24. D3-B-F records that decision and prepares the publication candidate for independent documentation review. [Parent D3-P](plans/A01-C1-C-B2-D3-P-SPECIFICATION.md), [ratified D3-B-P](plans/A01-C1-C-B2-D3-B-P-SPECIFICATION.md)

## 2. Frozen source and dependency identities

| Evidence | Exact identity |
|---|---|
| Published a01 baseline | `63a71d37d5dc448f4f9632efd82942f43ab82c59` |
| Baseline Git tree | `0d8369ecb274617829551c87c789684bd9f4982b` |
| I0 dependency patch SHA-256 | `891f626afb35199d60d9f1f14569a4501f2f27847f717fea175cf5b46820642b` |
| I1 cumulative implementation patch SHA-256 | `14a571a8682de7101afdaa4a772e9dba7cb615a61b2f78405330c872ff6b9a53` |
| Ratified D3-B-P SHA-256 | `e7e8bfc8c011f42c753f77dfd7de767437e333901ddcbf0fc6aab7c7a911350a` |
| I0 archive SHA-256 | `a90bf7aa5daf30d0cd57096991511ffc0ebeaf52675ef98eeb285edb59b984fc` |
| I1 archive SHA-256 | `805ff68da10863bff30c7b87c09e1f3a62cab9d4fbe99a28e80ab56016575f5d` |

The I0 dependency candidate reproduced all 191 files. I1 reproduced all 194 candidate files from the 191-file baseline and reversed to all 191 baseline files. All 179 protected tracked identities matched. The exact accepted I0 adoption resolves desktop lockfile libc from 0.2.190 to 0.2.191 because the accepted supervisor pins exactly 0.2.191. Cargo unified compatible desktop requirements in the lockfile, while the effective default Linux build retains its rustix Linux-raw dependency path. The worker and frozen B1 lockfiles remain byte-identical. The managed desktop normal/build graph excludes the Vulkan runtime and headless acceptance harness.

The 15 accepted I1 paths are:

- `experiments/desktop-probe/Cargo.lock`
- `experiments/desktop-probe/Cargo.toml`
- `experiments/desktop-probe/src/body_presentation.rs`
- `experiments/desktop-probe/src/lib.rs`
- `experiments/desktop-probe/src/managed_supervisor.rs`
- `experiments/desktop-probe/src/presentation/gpu_handoff.rs`
- `experiments/desktop-probe/src/presentation/gpu_handoff/tests.rs`
- `experiments/desktop-probe/src/probe_tests/animated_presentation_tests.rs`
- `experiments/desktop-probe/src/probe_tests/asynchronous_presentation_tests.rs`
- `experiments/desktop-probe/src/probe_tests/body_presentation_tests.rs`
- `experiments/desktop-probe/src/probe_tests/managed_supervisor_tests.rs`
- `experiments/desktop-probe/src/x11/events.rs`
- `experiments/desktop-probe/src/x11/render.rs`
- `experiments/desktop-probe/tests/managed_supervisor_contract.rs`
- `experiments/gpu-worker/supervisor/src/bin/fake-worker.rs`

## 3. Accepted architecture

The existing desktop owner thread drives X11, lifecycle, supervisor progression and publication. ProbeRuntime owns presentation requests and the shared D3-A validation and commitment seam. FrameTiming owns animation time and request opportunities. D1 GpuHandoff owns job correlation, ready payloads and physical obligations. The managed adapter advances the accepted subprocess supervisor and translates correlated reports into D1 dispositions. The supervisor retains IPC, transmission, process observation and shutdown authority. The GPU-free worker fixture exercises protocol behavior over real subprocess IPC. Owned returned pixels are used to reconstruct canonical metadata before shared validation. The integration reuses the owner event loop, animation clock, authoritative D1 ledger and publication controller. The default CPU path retains its checked publication ordering.

## 4. N01 — Authoritative deadline retention

D1 retains the complete job identity, immutable original request and original freshness deadline through the one-shot ready transfer. Equality at the deadline rejects publication. Pending expiry revokes publication eligibility while physical obligations remain outstanding. Ready expiry discards the candidate before native mutation. Serviced expiries leave the active wait interests. Unpublished frame sequences may repeat while job identities remain distinct. N01 is implemented and accepted within the D3-B GPU-free verification scope.

## 5. N02 — Actual final admission

The owner observes admission before consuming D1-ready ownership and again after shared candidate validation, before native submission. The checks use supervisor drainage and observed faults, host eligibility, target authority, eligibility epoch and job correlation, X11 lookahead, host and control deadline precedence, and the original freshness deadline. A transient eligibility interruption within an event batch revokes the affected work even if eligibility returns by batch end. A consumed candidate expires with its owner turn.

The shared seam preserves Skipped before mutation, checked submission before SHAPE, and uncertain-publication handling when presentation state may already have changed. Checked host transport exercised this ordering. Native X11 execution was outside I1 verification. N02 is implemented and accepted within the D3-B GPU-free verification scope.

## 6. N03 — Asynchronous request opportunities

FrameTiming consumes opportunities once and removes consumed deadlines from active wait interests. Pending and ready ownership suppress duplicate admission while eligible animation time advances through worker latency. Unsuccessful dispositions rearm a fresh full period. Opportunities never accumulate for catch-up. Eligibility interruption suspends opportunities and backend failure permanently closes admission. N03 is implemented and accepted within the D3-B GPU-free verification scope. The evidence covers fixture scheduling correctness. Production pacing remains unverified.

## 7. Readiness, fairness and IPC drainage

One owner wait set covers the X11 socket, worker readability, conditional worker writability, worker pidfd and earliest applicable deadline. Mandatory bounded supervisor service runs on busy and ineligible turns as well as idle turns. Host deadlines take precedence in scheduling publication. Bounded receive service preserves the difference between a partial decoder frame, actual socket drainage and a budget-limited immediate continuation. Synchronous native operations bound what this owner-loop fairness can guarantee.

## 8. Cancellation and physical settlement

Ordinary cancellation revokes eligibility while preserving any physical obligation. Pre-dispatch rejection and proven zero-byte withdrawal establish that the work did not reach the worker. Partially or fully transmitted work retains its physical obligation until truthful correlated settlement. Unresolved work stays recorded as debt. Native publication commitment is a separate host outcome. Exit and reaping supply process evidence only. Where the contracts permit it, a later truthful correlated terminal settles outstanding debt while the failed session remains closed.

## 9. Shutdown and reaping

The owner closes managed admission and invokes accepted supervisor escalation and shutdown deadlines. Ordered host cleanup preserves original-error priority and closes X11 resources before prolonged process observation. Bounded quiescent progression advances reaping. Independent process confirmation and physical settlement remain separately accounted, and later reaping preserves earlier unconfirmed classifications. The eventual production policy for indefinitely unkillable children is deferred.

## 10. I1 safeguards

- I1-R01 PASS. Exact accepted I0 dependency patch and resulting identities.
- I1-R02 PASS. Concrete crate-type witness exercised the actual protocol and supervisor types over IPC.
- I1-R03 PASS. Experimental fixture verification requires explicit executable configuration through `DESKTOPROOMIE_MANAGED_FIXTURE`. The managed fixture owner entry requires an explicit absolute executable path.
- I1-R04 PASS. Accepted supervisor internals were protected, and both existing supervisor D1 adapter tests were preserved.

## 11. Recorded automated verification

I0 recorded 29/29 baseline and 33/33 candidate commands passed, all 1,377 frozen identities and all 887 accepted desktop identities under the experimental feature preserved, and a 191/191 exact scratch candidate.

| Configuration | Accepted baseline | D3-B candidate | Lost |
|---|---:|---:|---:|
| Desktop default | 887 | 888 | 0 |
| Worker default | 228 | 228 | 0 |
| Worker native-vulkan | 230 | 230 | 0 |
| Frozen B1 default | 16 | 16 | 0 |
| Frozen B1 native-vulkan | 16 | 16 | 0 |
| Frozen matrix total | 1,377 | 1,378 | 0 |
| Desktop managed feature | 887 | 907 | 0 |

All 1,377 frozen identities survived. The managed configuration adds 20 identities relative to the D3-A desktop baseline, including the new default D1 test. It overlaps the frozen matrix. Working and scratch I1 candidates each met 83/83 expectations, comprising 82 successes and one expected missing-fixture failure each. The supplemental invalid-executable check failed as expected. All 32 mandatory behavioral categories were mapped. Actual subprocess tests cover IPC and process progression, deterministic injection covers timing and race scenarios, and checked host transport covers publication ordering. D3-B-F repeats no automated implementation execution.

## 12. Independent review and owner acceptance

ChatGPT independently verified all 396 archive content checksums, reconstructed the baseline tree, inspected the source and all 15 authorized paths, replayed the complete patch strictly, reproduced 194/194 candidate files, verified all 179 protected tracked identities, restored 191/191 baseline files, and reviewed the frozen test identities and recorded Codex execution evidence. All 14 independent review gates passed. Cargo execution belongs to Codex's recorded I1 matrix. Owner-native execution was outside the review. The owner explicitly accepted I1 technically on 2026-11-24. Documentation review and Git publication remain pending.

## 13. Preserved D2-C native evidence

| Measurement | Accepted result |
|---|---:|
| Successful workloads | 9 |
| Successfully initialized Vulkan sessions | 8 |
| Expected initialization rejection | 1 |
| Dedicated exact-parity frames | 250/250 |
| Confirmed Vulkan completions | 1,639 |
| Persistent staging allocations | 8 |
| Staging reuses | 1,633 |
| Staging retirements | 0 |
| Unresolved jobs | 0 |
| Confirmed shutdown and reaping outcomes | 9 |
| Software Vulkan stress rate | 57.7 Hz |
| RTX 3050 stress rate | 57.7 Hz |

The unsuccessful original R1 N1-A observation remains unsuccessful, including its 127.485213 ms submission wait. These are headless Vulkan measurements. The I1 review represented prior private native archives through recorded hash-comparison evidence. [D2-C acceptance record](A01-C1-C-B2-D2-C-ACCEPTANCE.md)

## 14. Remaining limitations

| Gate | Status |
|---|---|
| Owner-native managed X11 behavior | UNVERIFIED |
| Managed Vulkan publication | NOT IMPLEMENTED |
| Automatic CPU fallback | NOT IMPLEMENTED |
| Automatic backend restart | NOT IMPLEMENTED |
| Production transport | UNSELECTED |
| Production empty-SHAPE acceptance | PENDING |
| Strict 60 Hz readiness | NOT ACHIEVED |
| Rust 1.88 compatibility | UNVERIFIED |
| Unrestricted all-platform dependency resolution | UNVERIFIED |
| Repository maintenance | DEFERRED |
| B1 progress-site publication confirmation | DEFERRED |
| Production policy for indefinitely unkillable children | DEFERRED |

X11 pixels, SHAPE and compositor presentation are non-atomic. Checked native acknowledgments establish protocol outcomes only. Physical scanout remains outside that evidence. Independent review inspected the actual managed owner entry at source level. GPU-free tests exercised shared scheduling and publication functions without launching a complete native X11 owner session.

## 15. Forward checkpoint

D3-B-F awaits independent documentation review and explicit owner publication authorization. D3-C is planned after successful D3-B publication and requires separate architecture preparation and authorization. Production and transport decisions remain open.
