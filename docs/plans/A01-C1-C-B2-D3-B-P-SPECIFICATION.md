# A01-C1-C-B2-D3-B-P
## Managed Supervisor Integration — Ratified Architecture Specification

| Field | Value |
|---|---|
| Project | DesktopRoomie / ProjectVanilla |
| Milestone | A01-C1-C-B2-D3-B-P |
| Revision | 1.0 — Ratified |
| Approval date | 2026-11-24 |
| Branch | `a01` |
| Baseline | `63a71d37d5dc448f4f9632efd82942f43ab82c59` |
| Parent specification | A01-C1-C-B2-D3-P |
| Architecture | OWNER APPROVED |
| Implementation | NOT AUTHORIZED |
| Production transport | UNSELECTED |
| Target environment | Arch Linux, X11, Openbox, Picom, Kitty |

## 1. Purpose

Integrate the accepted D1 asynchronous presentation coordinator and D2-A subprocess supervisor with the existing DesktopRoomie owner event-loop architecture.

The integration shall establish authoritative job correlation, monotonic deadline retention, bounded readiness progression, observation-bounded publication authorization, cancellation, physical settlement accounting, failure containment and subprocess shutdown.

D3-B shall initially use GPU-free protocol fixtures.

This specification authorizes the architecture only. It authorizes no source modification, dependency modification, Cargo execution, native execution, repository publication or progress-site mutation.

Managed Vulkan publication belongs to D3-C and requires separate authorization.

## 2. Frozen baseline

The independently verified remote `a01` HEAD is:

`63a71d37d5dc448f4f9632efd82942f43ab82c59`

Git tree:

`0d8369ecb274617829551c87c789684bd9f4982b`

D3-A is technically accepted, independently reviewed, owner-approved and published.

The following accepted automated test identities shall be preserved:

| Suite | Accepted passing identities |
|---|---:|
| Desktop-probe | 887 |
| Worker default | 228 |
| Worker native-vulkan | 230 |
| Frozen B1 default | 16 |
| Frozen B1 native-vulkan | 16 |
| Total across configurations | 1,377 |

Preservation requires comparison of individual target-qualified test identities, not aggregate counts alone.

The published D3-A documentation retains historical pre-publication wording. Independently verified Git publication supersedes that wording without retroactively changing the original evidence or authorization history.

All previously accepted D1, D2, D3-A and A00 contracts remain binding.

## 3. Normative invariants

1. The desktop owner remains the sole authority over native X11 resources, interaction and publication.
2. The supervisor remains the sole authority over child ownership, private IPC and transmission facts.
3. D1 remains the authoritative desktop-side job coordinator.
4. The worker remains independent of X11 resources and desktop interaction.
5. Physical settlement and native publication commitment remain independent facts.
6. Cancellation, timeout, transport loss, termination and reaping never fabricate physical settlement.
7. No second authoritative job ledger, animation clock, lifecycle controller or publication controller shall be introduced.
8. The last trustworthy committed frame remains authoritative until successfully replaced or explicitly invalidated.
9. No automatic retry shall conceal uncertain native publication.
10. The synchronous CPU path and accepted A00 behavior shall remain operational and unchanged.
11. No managed Vulkan publication or automatic CPU fallback shall be introduced by D3-B.
12. No stage automatically authorizes its successor.

## 4. Ownership architecture

### 4.1 Desktop owner

The existing owner thread shall retain responsibility for:

- X11 event ordering and servicing.
- Host and control deadlines.
- Lifecycle interruption.
- Animation request opportunities.
- Supervisor progression.
- Final publication authorization.
- Shutdown orchestration.

No second desktop event loop or competing X11 connection owner shall be introduced.

### 4.2 ProbeRuntime and FrameTiming

`ProbeRuntime` remains authoritative for body availability, interaction, pointer obligations, placement, input readiness and publication eligibility.

`FrameTiming` remains authoritative for eligible active animation time, frame-request timing and successful-publication bookkeeping.

Asynchronous request-opportunity consumption shall extend this existing timing model rather than introduce a parallel animation clock.

### 4.3 Managed supervisor adapter

A minimal adapter shall coordinate D1, the supervisor and the existing desktop owner.

Its responsibilities are:

- Translation between presentation requests and worker protocol requests.
- Exact job correlation.
- Consistent owner-observed monotonic time.
- Eligibility interruption and cancellation coordination.
- Translation of supervisor reports into D1 outcomes.
- Pixel-derived metadata reconstruction.
- Delivery of eligible candidates at the shared publication boundary.

Any retained correlation information shall be subordinate to the existing authoritative D1 and supervisor states.

The headless D2-C adapter is a reference, not a production dependency.

### 4.4 D1 and D2

`GpuHandoff` retains authority over session, epoch, job serial, outstanding work, freshness, settlement obligations and one-shot ready transfer.

The supervisor retains authority over transmission, protocol decoding, backend faults, process observation, initialization, escalation, shutdown and reaping.

The accepted Vulkan worker remains outside the D3-B integration target.

## 5. B01 — Minimal dependency integration

**OWNER APPROVED**

The preferred dependency arrangement is an opt-in desktop feature, provisionally named `managed-supervisor-fixture`, exposing the existing protocol and supervisor crates.

The default desktop launch behavior shall remain unchanged.

The existing supervisor development dependency on desktop-probe creates a potential reverse dependency relationship when desktop-probe begins depending on the supervisor.

This arrangement shall be investigated through a separately authorized dependency preflight before implementation.

The dependency preflight shall establish:

- Successful Cargo dependency resolution.
- Compatible crate and type identities.
- Independent protocol and supervisor builds.
- Preservation of existing D1 adapter tests.
- Preservation of all accepted worker configurations.
- Absence of Vulkan runtime dependencies in the desktop owner.
- Absence of the headless acceptance harness from the desktop production dependency graph.
- Explicit justification of every manifest and lockfile change.
- No unrelated dependency upgrades.

The existing workspace boundaries shall be preserved wherever feasible.

Wholesale presentation extraction, workspace consolidation and unrelated repository restructuring are excluded.

If the minimal dependency arrangement cannot satisfy these requirements, implementation shall stop pending architectural review.

Cargo dependency feasibility remains UNVERIFIED.

## 6. B02 — Authoritative deadline retention (N01)

**OWNER APPROVED**

### 6.1 Required correction

The accepted D1 implementation retains the freshness deadline while a job is pending but does not retain it in its existing ready representation.

D3-B shall preserve the original admitted-job deadline through completion, ready ownership, one-shot transfer and final publication admission.

Conceptually, the ready candidate shall carry:

- Complete `JobIdentity`.
- Original immutable `FrameRequest`.
- Original freshness deadline.
- Owned prepared frame.

D1 shall retain authoritative ownership of this information.

The existing `take_ready()` contract shall remain compatible with accepted callers and tests. A richer one-shot transfer interface may expose the complete admission information required by D3-B.

The exact API shall be determined during the bounded implementation assignment.

### 6.2 Monotonic time

The owner shall establish one monotonic origin shared by the adapter's desktop `Instant` observations and supervisor-relative `Duration` observations.

D1 and the supervisor shall receive consistent observations of the actual admission time.

The freshness deadline shall be calculated once from the actual admission observation and the applicable fixture freshness allowance.

Time conversion and arithmetic shall be checked.

No deadline shall be reconstructed from:

- Completion time.
- IPC arrival time.
- Ready consumption time.
- Current animation time.
- Resumed eligibility.
- Validation completion time.

Deadline equality constitutes expiry.

### 6.3 Pending and ready expiry

Expired pending work shall lose prospective publication eligibility without losing its physical settlement obligation.

An expired ready candidate shall be discarded without native mutation.

An eligible completion shall retain the original deadline in its ready state.

The original deadline shall remain available to the owner through final admission.

Expiry shall be serviced once. An already-serviced expiry shall not remain a continuously due wait interest.

**N01 architectural resolution: RATIFIED. Implementation and verification remain pending.**

## 7. B03 — Actual owner-observed final admission (N02)

**OWNER APPROVED**

### 7.1 Shared publication seam

The accepted D3-A publication architecture remains authoritative:

1. `BodyPresentation::frame_request` constructs the authoritative request.
2. Preparation produces an owned candidate.
3. `ProbeRuntime::publish_prepared_with` coordinates publication and successful timing.
4. `FrameGeometry::publish_with_admission` validates the candidate and invokes final admission.
5. The existing checked publication transaction performs native mutations.
6. Successful timing precedes restoration of runtime input readiness.

The final admission callback shall remain after candidate validation and before publication-state mutation or native submission.

The synchronous CPU path shall retain unconditional admission and its existing clock observations.

### 7.2 First observation checkpoint

Before consuming D1-ready, the owner shall require:

- Actual supervisor drainage.
- Absence of observed disqualifying backend or transport failure.
- Exact session, epoch, serial and frame correlation.
- Current eligibility epoch.
- Current body and target authority.
- Current host interaction and lifecycle eligibility.
- No higher-priority buffered X11 event.
- No due host or control deadline.
- Original freshness deadline strictly in the future.

A ready candidate may remain owned by D1 while higher-priority owner work is serviced.

Once consumed, the candidate shall not survive into an unrelated owner turn.

### 7.3 Final observation checkpoint

After validation, the final admission callback shall revalidate actual owner-observed predicates.

These shall include:

- Current supervisor progression and drainage.
- Observed fault precedence.
- Current host eligibility.
- Current target authority.
- Buffered X11 event ordering.
- Current host and control deadlines.
- A fresh monotonic observation compared against the original admitted-job deadline.

A bounded nonblocking observation may discover an event or fault that was not previously visible.

A newly observed X11 event shall be retained for normal ordered processing, not dispatched within the publication callback.

If supervisor progression exhausts its receive budget, leaves a partially buffered protocol message or observes a disqualifying fault, final admission shall decline.

If a consumed candidate is declined, it shall be discarded rather than retained for a later publication attempt.

Once native publication is admitted, unrelated supervisor progression and desktop event dispatch shall not interleave with the checked transaction.

### 7.4 Actual host eligibility

The integration shall reuse `ProbeRuntime::frame_eligible` and the existing runtime authority.

Eligibility requires:

- No active owner control operation.
- No pending layer transition.
- Body input readiness.
- Settled interaction and pointer obligations.
- No in-flight movement.

Eligibility invalidation shall observe actual interruption transitions, including interruptions that begin and end within one event batch.

Sampling only the final state of a batch is insufficient.

### 7.5 Admission outcomes

A declined admission shall preserve the existing `Skipped` semantics:

- No publication-state submission.
- No native pixel or SHAPE mutation.
- No successful-publication clock observation.
- No fabricated commitment.
- No loss of a trustworthy previous committed frame.

Native submission uncertainty and post-commit timing failure retain their existing D3-A failure paths.

**N02 architectural resolution: RATIFIED. Implementation and verification remain pending.**

## 8. B04 — Non-spinning request opportunities (N03)

**OWNER APPROVED**

### 8.1 Separate request and publication events

The asynchronous timing model shall distinguish:

- Request opportunity becoming due.
- Request opportunity being consumed.
- Job admission.
- Physical settlement.
- Candidate consumption.
- Successful native publication.

A request opportunity shall be consumed at most once.

Consumption shall remove that opportunity's expired deadline from the active wait interests.

It shall not require successful native publication.

### 8.2 Pending and ready work

While physical work remains outstanding or D1 owns a ready candidate:

- No duplicate admission.
- No accumulated request queue.
- No immediate retry loop.
- No catch-up rendering burst.
- No repeated zero-timeout polling caused by the consumed opportunity.

Supervision shall continue independently of animation eligibility.

### 8.3 Rearming

Successful publication shall retain the accepted successful-publication timing semantics.

After an unsuccessful terminal disposition, ordinary discard or transient admission refusal, a new opportunity may be armed from the current observation plus one fixture period, provided the owner is eligible and idle.

This rearming shall not advance successful-publication bookkeeping.

During ineligible periods, opportunities shall remain suspended.

After eligibility resumes, no accumulated opportunities shall be replayed.

Permanent backend failure shall disable future managed GPU request opportunities.

### 8.4 Active animation time

GPU latency and outstanding work shall not pause otherwise eligible animation time.

Suppressing request opportunities is not equivalent to interrupting animation eligibility.

The fixture may retain the existing synthetic 300 ms frame period. Deterministic tests shall also exercise shorter periods, overruns and extended latency.

This policy establishes bounded asynchronous correctness. It does not select a final pacing mechanism or production frame period.

**N03 architectural resolution: RATIFIED. Implementation and verification remain pending.**

## 9. B05 — Unified multi-descriptor waiting

**OWNER APPROVED**

The managed owner loop shall use one bounded readiness wait incorporating:

| Descriptor | Interest |
|---|---|
| X11 socket | Readability while available |
| Worker socket | Readability while open |
| Worker socket | Writability only when requested by the supervisor |
| Worker pidfd | Exit readiness until reaped |

The integration shall consume the supervisor's existing `WaitInterest` interface.

The active wait deadline shall be the earliest applicable deadline among:

- Host duration and lifecycle deadlines.
- Host interaction, placement and safety deadlines.
- Owner control deadlines.
- Unconsumed request opportunities.
- Active job or ready-candidate freshness deadlines.
- Supervisor initialization, escalation and shutdown deadlines.
- Necessary process-observation deadlines.

The wait shall preserve upward timeout rounding and interrupted-poll handling.

Interests shall be recomputed after progression.

Worker socket writability shall not be monitored when no outbound work requires it.

Already-consumed opportunities, already-serviced expiries and retired descriptors shall not cause permanent immediate wakeups.

X11 socket failure shall retain the existing host-fatal classification.

Worker socket failure shall retain its backend or transport classification.

Readable data shall be drained before final worker hangup classification.

Process readiness shall trigger accepted supervisor observation and reaping.

Failure to acquire a pidfd shall preserve the existing `ProcessObservation` failure policy. It shall not authorize ordinary managed operation.

Where further exit observation is necessary without a usable pidfd, finite periodic nonblocking process observation may be used. A provisional 50 ms fixture interval is permitted.

No asynchronous runtime, additional supervision thread or blocking convenience wait shall be introduced.

## 10. B06 — Bounded fairness

**OWNER APPROVED**

The existing owner event-loop ordering shall be preserved.

Managed supervisor progression shall receive bounded service opportunities independently of whether the desktop reaches its safe presentation boundary.

The integration shall preserve:

- Existing host deadline precedence.
- Existing bounded X11 event batching.
- Existing lifecycle and control-event ordering.
- Existing buffered-event lookahead.
- Existing supervisor receive and outbound-write bounds.

Sustained X11 traffic shall not indefinitely starve supervisor progression.

Sustained IPC traffic shall not indefinitely starve host obligations.

Existing early continuation paths shall not bypass supervision indefinitely.

Worker receive-budget exhaustion shall request another bounded owner iteration rather than an unbounded receive loop.

Publication may remain deferred during sustained higher-priority desktop work.

Fairness does not authorize publication of an ineligible frame.

The implementation shall distinguish actionable immediate work from incomplete work awaiting additional readiness or a deadline.

Existing checked X11 operations remain synchronous. Bounded service budgets do not establish hard real-time guarantees against blocking native operations.

## 11. B07 — IPC drainage and observation-bounded publication

**OWNER APPROVED**

The accepted supervisor remains authoritative for receive drainage.

The managed adapter shall use the actual `Progress` and report-consumption interfaces.

Neither socket readability nor elapsed time independently establishes drainage.

### 11.1 Drainage matrix

| Receive observation | Publication consequence |
|---|---|
| Complete frame, empty decoder and WouldBlock | Eligible for further admission checks |
| Complete frame followed by a partially buffered message | Forbidden until genuine drainage |
| Receive-budget exhaustion | Drainage not established |
| Truncated terminal | Forbidden; preserve unresolved work where applicable |
| Complete frame and observed backend fault | Forbidden; preserve genuine settlement |
| Fault first observed after successful commitment | Close future admission; preserve historical commitment |

### 11.2 Fault precedence

The accepted supervisor failure and correlation policy remains authoritative.

A disqualifying fault observed before publication admission shall prevent publication.

A fault observed after successful commitment shall not retroactively erase that commitment.

Physical settlement shall remain truthful in both cases.

The owner shall not introduce an additional acknowledgment, heartbeat or fixed grace interval as proof of future backend health.

The observation boundary is not a guarantee that no future fault exists.

## 12. B08 — Cancellation, correlation and physical settlement

**OWNER APPROVED**

### 12.1 Exact identity

Every admitted job shall preserve distinct identity dimensions:

- Worker session.
- Eligibility epoch.
- Job serial.
- Frame generation.
- Frame sequence.

The original immutable request shall remain authoritative.

Reuse of an unpublished frame sequence shall not permit GPU job identity reuse.

Wrong, stale or duplicate reports shall not discharge another job's physical obligation.

### 12.2 Report translation

| Supervisor report | D1 outcome |
|---|---|
| Valid correlated `Frame` | `Frame` |
| `Obsolete` | `SettledWithoutPayload` |
| `RejectedBeforeDispatch` | `RejectedBeforeSubmission` |
| `Rejected` | `RejectedBeforeSubmission` |
| `Failed` | `Failed` |
| Failure without settlement evidence | `Unresolved` |

A truthful correlated settlement report shall retain its settlement meaning even if its payload is unsuitable for publication.

Invalid payloads shall not become ready candidates.

Settlement evidence and publication validation shall not be conflated.

### 12.3 Metadata

The adapter shall validate returned BGRA8 layout and premultiplied alpha.

Canonical hit spans, visible bounds and anchor metadata shall be reconstructed from the actual returned pixels.

The existing shared host validation and native request-limit checks shall remain authoritative.

The headless acceptance oracle shall not become a production dependency.

Independent fixture comparisons may continue using accepted CPU reference behavior.

### 12.4 Cancellation

Ordinary gesture, popup, placement and lifecycle interruptions shall invalidate prospective publication through the existing D1 epoch and supervisor cancellation mechanisms.

An already admitted canceled render may finish transmission and settle as obsolete.

Ordinary cancellation shall not fabricate zero-byte withdrawal.

The accepted supervisor's proven zero-byte withdrawal path remains distinct from partially or completely transmitted work.

### 12.5 Physical obligations

The following shall not independently discharge a physical obligation:

- Cancellation.
- Freshness expiry.
- Transport abandonment.
- Backend fault.
- Process death.
- Forced termination.
- Reaping.

A matching truthful terminal report or accepted proof of pre-dispatch rejection is required.

Unresolved work shall retain its exact identity, including after reaping.

Automatic backend restart and replacement-session admission are excluded.

## 13. B09 — Shutdown and process reaping

**OWNER APPROVED**

The supervisor shall remain owned by the existing spawning owner thread, preserving its accepted thread-lifetime and parent-death assumptions.

### 13.1 Shutdown sequence

1. Permanently close D1 admission for the session.
2. Revoke prospective publication authority.
3. Request supervisor shutdown once.
4. Continue bounded IPC progression.
5. Preserve and consume truthful settlement reports.
6. Service accepted graceful-close and escalation deadlines.
7. Request termination through the accepted supervisor when required.
8. Observe child exit and reap through the accepted ownership mechanism.
9. Record shutdown confirmation and unresolved physical work independently.

This progression shall cover normal closure, duration expiry, body destruction, control endpoint loss and fatal host errors.

Existing host cleanup ordering and original-error priority shall remain binding.

A worker failure shall not overwrite an original host failure.

The supervisor's best-effort `Drop` behavior shall not establish confirmed shutdown.

### 13.2 Shutdown classification

Confirmed shutdown requires the accepted supervisor conditions, including Stopped, successful exit, reaping, no unresolved physical debt and no retained unconfirmed condition.

Forced termination does not constitute graceful confirmation.

A reaped child may still have unresolved physical work.

An exceeded exit-observation deadline shall remain classified as unconfirmed shutdown.

Later exit observation may establish reaping without rewriting previously recorded uncertainty.

### 13.3 Unconfirmed live-child retention

When shutdown remains unconfirmed and the child is still alive, retain the supervisor on its owner thread in a quiescent reaping phase.

Unsafe X11 publication authority shall not be retained merely to continue subprocess observation.

Observation shall avoid indefinite busy polling and unbounded blocking `wait()` calls.

Individual progression and waiting operations shall remain bounded.

Total process-exit latency cannot be guaranteed for an indefinitely unkillable child.

A production policy for forcibly abandoning observation or terminating the application under such a condition remains outside this specification.

## 14. GPU-free fixture architecture

D3-B shall establish evidence at three levels.

### 14.1 Deterministic policy tests

Exercise injected monotonic observations, readiness decisions, deadline precedence, cancellation, opportunity consumption and exact state transitions.

### 14.2 Actual GPU-free subprocess integration

Exercise the accepted supervisor with real Unix IPC, process ownership, protocol framing, partial transmission, transport closure, termination and reaping.

Reuse existing fake-worker modes wherever feasible.

Narrow fixture extensions may support deterministic synchronization of additional integration races.

Arbitrary sleeps alone shall not establish race ordering.

### 14.3 Integrated owner presentation tests

Exercise the managed supervisor adapter and actual owner orchestration through the existing D3-A publication seam.

Use the accepted checked test transport for pixel, SHAPE, commitment and failure-order assertions.

Tests shall exercise the same admission and scheduling logic intended for the managed integration, rather than substitute unrelated test-only predicates.

No real Vulkan device, managed Vulkan publication or owner-native X11 execution is required or authorized by D3-B-P.

## 15. Mandatory verification categories

The following 32 categories are required behaviors, not a prescribed number of test functions.

### N01 — Deadline retention

| ID | Required behavior |
|---|---|
| T01 | Immutable deadline survives Pending to Ready |
| T02 | Expiry before completion, consumption and final admission |
| T03 | Deadline equality rejects publication |
| T04 | Validation crossing the original deadline prevents submission |
| T05 | Reused frame sequence does not reuse job identity or deadline |

### N02 — Owner admission

| ID | Required behavior |
|---|---|
| T06 | Complete frame and fault observed in the same progression |
| T07 | Partially buffered following fault prevents admission |
| T08 | Receive-budget exhaustion does not establish drainage |
| T09 | Fault observed during final validation prevents submission |
| T10 | Fault first observed after commitment preserves historical commitment |
| T11 | Buffered X11 event takes precedence |
| T12 | Due host deadline takes precedence |
| T13 | Transient interruption within one event batch invalidates prior eligibility |
| T14 | Invalidated target generation cannot publish |

### N03 — Opportunity consumption and fairness

| ID | Required behavior |
|---|---|
| T15 | Pending work does not cause expired-deadline spinning |
| T16 | Ready work prevents duplicate admission |
| T17 | Expiry and refusal do not cause immediate retry loops |
| T18 | Extended GPU latency preserves eligible animation time |
| T19 | Pause and resume do not accumulate catch-up opportunities |
| T20 | Sustained X11 traffic permits supervisor progression |
| T21 | Sustained IPC traffic permits host progression |
| T22 | Partially buffered messages wait appropriately |

### IPC, settlement and lifecycle

| ID | Required behavior |
|---|---|
| T23 | Proven zero-byte withdrawal remains distinct from partial transmission |
| T24 | Canceled work can genuinely settle without publication |
| T25 | Truncation preserves unresolved job identity |
| T26 | Wrong and duplicate identities cannot discharge current debt |
| T27 | Worker failure preserves trustworthy host commitment |
| T28 | Process death does not fabricate physical settlement |
| T29 | Confirmed shutdown and reaping |
| T30 | Unconfirmed shutdown and later exit observation |
| T31 | Host and worker socket failures remain separate |
| T32 | Host publication failure preserves original error priority |

All previously accepted automated behaviors remain mandatory in addition to these categories.

## 16. Protected implementation areas

The eventual implementation shall preserve, byte-for-byte wherever modification is unnecessary:

- Vulkan worker implementation.
- Native settlement contract.
- Worker wire protocol.
- Headless acceptance harness, workloads and oracle.
- Previously accepted native evidence.
- Frozen B1.
- Default CPU behavior.
- A00 lifecycle, placement, control and interaction behavior.
- Existing checked publication and SHAPE ordering.

Any required changes to accepted D1, timing, renderer, owner-loop or supervisor components shall receive explicit bounded authorization.

No unrelated refactoring or maintenance shall be included.

## 17. Staged execution roadmap

### D3-B-P — Architecture ratification

Current checkpoint.

Exit requirement: explicit owner approval of B01–B09.

Status: OWNER APPROVED.

### D3-B-I0 — Dependency integration preflight

Investigate and verify the minimal Cargo dependency arrangement.

Establish compatible crate identities, feature boundaries, lockfile implications and the eventual authorized modification scope.

Do not begin managed supervisor implementation.

Exit requirement: independent review and explicit authorization of the next checkpoint.

Status: NOT AUTHORIZED.

### D3-B-I1 — Managed supervisor implementation

Implement the approved architecture within a separately defined modification scope.

Exercise GPU-free fixtures and deterministic race categories.

Preserve all accepted automated test identities.

Exit requirement: independent source, patch, dependency and verification review.

Status: NOT AUTHORIZED.

### D3-B-F — Acceptance and documentation closeout

Consolidate accepted source and automated evidence.

Record the exact accepted implementation identity, limitations and remaining gates.

Obtain explicit owner technical acceptance and owner-controlled publication.

Status: PENDING.

D3-C shall not begin automatically after D3-B.

## 18. Review package requirements

The eventual implementation review package shall contain:

- Pinned baseline identity.
- Authorized path manifest.
- Baseline and candidate source hashes.
- Incremental and cumulative patches.
- Dependency graph and lockfile comparisons.
- Raw Cargo verification logs.
- Target-qualified test identity inventories.
- Requirement-to-test mapping.
- GPU-free fixture and race evidence.
- Independent patch replay materials.
- Complete Git status.

Recorded Codex execution evidence shall remain distinct from independent ChatGPT source and patch verification.

No test count or review result shall be claimed before its corresponding evidence exists.

## 19. Preserved D2-C native evidence

The accepted D2-C native evidence remains unchanged:

| Measurement | Accepted result |
|---|---:|
| Successful acceptance workloads | 9 |
| Successfully initialized Vulkan sessions | 8 |
| Expected initialization-rejection workloads | 1 |
| Dedicated exact-parity frames | 250/250 |
| Confirmed Vulkan completions | 1,639 |
| Persistent staging allocations | 8 |
| Staging reuses | 1,633 |
| Staging retirements | 0 |
| Unresolved jobs | 0 |
| Confirmed shutdown and reaping outcomes | 9 |

The accepted software Vulkan and RTX 3050 stress windows each contain 577 valid frames over ten seconds, corresponding to 57.7 Hz.

The unsuccessful original R1 N1-A observation, including its 127.485213 ms submission wait, remains historical failure evidence.

D3-B shall not reclassify these headless results as managed X11 performance.

## 20. Preserved limitations and exclusions

| Classification | Status |
|---|---|
| Managed supervisor implementation | NOT STARTED |
| Managed Vulkan publication | NOT IMPLEMENTED |
| Automatic CPU fallback | NOT IMPLEMENTED |
| Production transport | UNSELECTED |
| Production empty-SHAPE acceptance | PENDING |
| Strict 60 Hz readiness | NOT ACHIEVED |
| Rust 1.88 compatibility | UNVERIFIED |
| Cargo dependency feasibility | UNVERIFIED |
| Repository maintenance | DEFERRED |
| B1 progress-site publication confirmation | DEFERRED |
| Private Vanilla/Spine assets | OUT OF SCOPE |

X11 pixels, input SHAPE and compositor presentation remain non-atomic.

Checked X11 acknowledgments do not establish compositor-visible presentation or physical scanout.

Process isolation does not guarantee recovery from kernel-wide or graphics-driver hangs.

No new production freshness threshold, pacing mechanism or transport is selected.

## 21. Workflow and authority

ChatGPT retains responsibility for research, architecture, specification, task management and independent review.

Codex performs separately authorized implementation and automated verification.

The owner retains control over native execution, technical acceptance, Git publication and milestone authorization.

All work shall remain on the long-lived `a01` branch unless explicitly authorized otherwise.

No implementation, staging, commit, push, merge, branch creation or progress-site mutation is authorized by this specification.

## 22. Pinned source references

All repository references use the independently verified D3-B baseline.

- [Published baseline](https://github.com/zNe4/DesktopRoomie/commit/63a71d37d5dc448f4f9632efd82942f43ab82c59)
- [Ratified D3-P specification](https://github.com/zNe4/DesktopRoomie/blob/63a71d37d5dc448f4f9632efd82942f43ab82c59/docs/plans/A01-C1-C-B2-D3-P-SPECIFICATION.md)
- [D3-A acceptance record](https://github.com/zNe4/DesktopRoomie/blob/63a71d37d5dc448f4f9632efd82942f43ab82c59/docs/A01-C1-C-B2-D3-A-ACCEPTANCE.md)
- [D2-C acceptance record](https://github.com/zNe4/DesktopRoomie/blob/63a71d37d5dc448f4f9632efd82942f43ab82c59/docs/A01-C1-C-B2-D2-C-ACCEPTANCE.md)
- [Implemented architecture](https://github.com/zNe4/DesktopRoomie/blob/63a71d37d5dc448f4f9632efd82942f43ab82c59/docs/ARCHITECTURE.md)
- [Desktop owner and ProbeRuntime](https://github.com/zNe4/DesktopRoomie/blob/63a71d37d5dc448f4f9632efd82942f43ab82c59/experiments/desktop-probe/src/lib.rs)
- [FrameTiming and body presentation](https://github.com/zNe4/DesktopRoomie/blob/63a71d37d5dc448f4f9632efd82942f43ab82c59/experiments/desktop-probe/src/body_presentation.rs)
- [D1 GpuHandoff](https://github.com/zNe4/DesktopRoomie/blob/63a71d37d5dc448f4f9632efd82942f43ab82c59/experiments/desktop-probe/src/presentation/gpu_handoff.rs)
- [Checked publication boundary](https://github.com/zNe4/DesktopRoomie/blob/63a71d37d5dc448f4f9632efd82942f43ab82c59/experiments/desktop-probe/src/x11/render.rs)
- [X11 readiness helpers](https://github.com/zNe4/DesktopRoomie/blob/63a71d37d5dc448f4f9632efd82942f43ab82c59/experiments/desktop-probe/src/x11/events.rs)
- [Supervisor process ownership](https://github.com/zNe4/DesktopRoomie/blob/63a71d37d5dc448f4f9632efd82942f43ab82c59/experiments/gpu-worker/supervisor/src/process.rs)
- [Supervisor policy machine](https://github.com/zNe4/DesktopRoomie/blob/63a71d37d5dc448f4f9632efd82942f43ab82c59/experiments/gpu-worker/supervisor/src/machine.rs)
- [Supervisor outbound transmission](https://github.com/zNe4/DesktopRoomie/blob/63a71d37d5dc448f4f9632efd82942f43ab82c59/experiments/gpu-worker/supervisor/src/outbound.rs)
- [GPU-free worker fixture](https://github.com/zNe4/DesktopRoomie/blob/63a71d37d5dc448f4f9632efd82942f43ab82c59/experiments/gpu-worker/supervisor/src/bin/fake-worker.rs)

## 23. Final ratification

The owner explicitly approved B01–B09 on November 24, 2026.

Owner observations: None.

The three forward D3-A integration obligations have ratified architectural resolutions:

- N01: authoritative admitted-job deadline retention.
- N02: actual owner-observed final admission predicates.
- N03: non-spinning request opportunity consumption.

Their implementation and verification remain pending.

The next permissible activity is preparation of the bounded D3-B-I0 dependency integration preflight assignment. Its execution requires separate explicit owner authorization.

**D3-B-P — ARCHITECTURE RATIFIED — OWNER APPROVED — IMPLEMENTATION PENDING AUTHORIZATION.**
