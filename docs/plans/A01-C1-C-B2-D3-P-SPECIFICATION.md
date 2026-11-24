# A01-C1-C-B2-D3-P
## Managed X11 GPU Integration — Ratified Architecture Specification

| Field | Value |
|---|---|
| Project | DesktopRoomie / ProjectVanilla |
| Milestone | A01-C1-C-B2-D3-P |
| Revision | 1.0 — Ratified |
| Approval date | 2026-11-24 |
| Branch | `a01` |
| Baseline commit | `1313a9251427875d8c7814b2c64131da75d5a37b` |
| Architecture | OWNER APPROVED |
| Implementation | NOT AUTHORIZED |
| Production transport | UNSELECTED |
| Target environment | Arch Linux, X11, Openbox, Picom, Kitty |

## 1. Purpose

Integrate the accepted isolated Vulkan rendering subsystem with DesktopRoomie's managed X11 presentation architecture, preserving all previously accepted desktop behavior, physical settlement guarantees and failure-containment boundaries.

The integration shall be incremental, experimentally accessible and independently reviewable.

D3-P authorizes the architecture only. It authorizes no source modification, dependency modification, native execution, repository publication or production transport selection.

## 2. Frozen baseline

The independently verified published baseline is commit `1313a9251427875d8c7814b2c64131da75d5a37b` on `a01`.

All previously accepted test identities shall be preserved.

| Suite | Accepted passing identities |
|---|---:|
| Desktop-probe | 867 |
| Worker default | 228 |
| Worker native-vulkan | 230 |
| Frozen B1 default | 16 |
| Frozen B1 native-vulkan | 16 |

The following native acceptance evidence remains frozen:

- Nine successful native acceptance workloads.
- Eight successfully initialized Vulkan sessions and one expected initialization-rejection workload.
- 250/250 dedicated exact-parity frames.
- 1,639 confirmed Vulkan completions.
- Eight persistent staging allocations.
- 1,633 staging reuses.
- Zero staging retirements.
- Zero unresolved jobs.
- Nine confirmed shutdown and reaping outcomes.

Both software Vulkan and RTX 3050 stress workloads achieved 57.7 Hz over their fixed ten-second observation windows, with zero busy opportunities.

Strict 60 Hz readiness remains NOT ACHIEVED.

The original unsuccessful R1 N1-A observation, including its 127.485213 ms submission wait, remains historical failure evidence. It must not be erased or reclassified.

## 3. Normative architectural invariants

The following requirements apply throughout D3:

1. The desktop owner remains the sole authority over native X11 resources and operations.
2. The Vulkan device remains isolated in the worker subprocess.
3. GPU physical settlement and X11 publication commitment remain independent facts.
4. Cancellation, timeout, disconnection, process death and reaping never fabricate physical settlement.
5. Pending or obsolete work retains its exact identity until genuinely settled or truthfully classified as unresolved.
6. The GPU backend cannot directly mutate window geometry, SHAPE, pointer state or desktop lifecycle.
7. The currently committed frame remains authoritative until replaced by a successfully committed successor or explicitly invalidated.
8. No automatic retry may conceal uncertain native publication.
9. The normal CPU launch path remains operational and behaviorally unchanged.
10. All previously accepted A00 lifecycle, control, interaction, focus and geometry contracts remain binding.

## 4. Ownership architecture

### 4.1 Desktop event loop

The existing owner event loop remains responsible for:

- X11 event ordering and bounded servicing.
- Host deadlines and control operations.
- Lifecycle interruption and shutdown.
- Animation request opportunities.
- Managed GPU supervision orchestration.
- Final publication authorization.

No independent GPU desktop event loop or competing X11 connection owner shall be introduced.

### 4.2 ProbeRuntime

`ProbeRuntime` remains authoritative for:

- Body availability and input readiness.
- Gesture and popup state.
- Pointer capture and release obligations.
- Movement and placement confirmation.
- Input fencing and freshness.
- Animation eligibility and active elapsed time.

### 4.3 Managed GPU adapter

A minimal integration adapter shall translate between the desktop presentation layer, D1 and the D2 supervisor.

Its responsibilities are:

- Retaining authoritative frame requests.
- Translating and correlating complete job identities.
- Coordinating eligibility invalidation.
- Supplying consistent owner-observed monotonic time.
- Converting worker outcomes into truthful D1 outcomes.
- Delivering validated candidates at the authorized owner boundary.

It shall not introduce another authoritative job ledger, lifecycle machine, animation clock or publication controller.

The accepted headless adapter serves as a reference, not a production dependency. Its blocking convenience waits, acceptance oracle and workload-specific accounting shall not be imported wholesale.

### 4.4 D1 coordinator

`GpuHandoff` retains authority over:

- Session, epoch and job serial correlation.
- Admission and bounded outstanding work.
- Eligibility interruption.
- Freshness expiry.
- Physical settlement accounting.
- One-shot ready-frame transfer.
- Permanent closure following backend failure.

### 4.5 D2-A supervisor

The supervisor retains authority over:

- Child process and private IPC ownership.
- Partial and complete request transmission.
- Protocol decoding and correlation.
- Backend fault observation.
- Initialization, escalation and shutdown deadlines.
- Process termination and reaping.
- Truthful unresolved-job classification.

### 4.6 D2-B Vulkan worker

The worker retains authority over:

- Vulkan device, queue and pipeline.
- Native operation submission.
- Submission-specific completion.
- Mapping, callback and unmap verification.
- Persistent staging ownership and reuse.
- Terminal-report integrity.
- Native resource release.

The worker shall remain independent of X11, Openbox, Picom and the desktop interaction model.

## 5. P01 — Observation-bounded publication

**OWNER APPROVED**

An ordinary late backend fault may occur after genuine GPU settlement but during partial transmission of a truthful frame.

The accepted D2-B-R2 worker may complete that terminal transmission and subsequently attempt to report the backend fault.

D3 shall preserve this wire contract.

### 5.1 Publication rule

A physically settled frame is eligible for publication only when the owner has current publication authority and has not observed a disqualifying backend or transport fault before admitting the native publication transaction.

Before consuming a ready frame, the owner shall:

1. Service the supervisor's bounded receive progression.
2. Observe its failure and lifecycle outcome.
3. Require the existing supervisor drainage boundary.
4. Apply fault precedence and current eligibility.
5. Recheck freshness and native publication authority.
6. Consume and publish the candidate in the authorized owner turn.

Read-budget exhaustion is not a drainage boundary. A partially buffered subsequent protocol message does not authorize ready consumption.

The owner shall not retain a consumed D1-ready candidate across unrelated event turns.

### 5.2 Fault outcome matrix

| Observation | Publication | Settlement |
|---|---|---|
| Partial terminal | Forbidden | Pending |
| Invalidated or truncated terminal | Forbidden | Unresolved where settlement remains unproven |
| Complete truthful frame with fault observed before publication admission | Forbidden | Preserve genuine settlement |
| Complete valid frame with no observed disqualifying fault | Permitted if all other predicates hold | Settled |
| Fault first observed after successful X11 commitment | Stop future GPU admission | Preserve historical settlement and commitment |
| Uncertain X11 publication | Fence input and enter host failure path | Preserve independent GPU settlement facts |

The observation boundary is not a guarantee that no fault exists elsewhere in the system or will arrive later.

No fixed grace period, heartbeat or additional acknowledgment shall be treated as proof of future backend health.

Once native publication has begun, the existing checked transaction shall finish or enter its established failure path. Unrelated worker processing shall not interleave with that transaction.

## 6. P02 — Independent identities and invalidation

**OWNER APPROVED**

The following identity dimensions remain distinct:

| Identity | Authority |
|---|---|
| Worker session | Backend lifetime |
| Eligibility epoch | D1 interruption boundary |
| Job serial | Individual GPU operation |
| Frame generation | X11 publication target |
| Frame sequence | Candidate publication ordering |

An unpublished frame sequence may be proposed again if the existing `FrameGeometry` contract permits it. That does not permit reuse of a GPU job identity.

### 6.1 Publication eligibility

Every candidate must satisfy:

- Exact job and frame identity correlation.
- Retention of the original authoritative request.
- Current eligibility epoch.
- Current body and publication generation.
- Compatible fixed client extent and intent.
- Freshness strictly before expiry at publication admission.
- No observed disqualifying failure.
- Valid pixels, metadata and native request limits.
- Current host interaction and lifecycle eligibility.
- No higher-priority buffered event or due host deadline.

Validation shall reconstruct or independently verify visible bounds and canonical hit coverage from the returned pixels.

### 6.2 Invalidation classes

Ordinary interruption invalidates prospective GPU publication, not necessarily existing committed pixels.

A gesture, popup, visibility transition or placement operation shall cancel the relevant GPU eligibility without manufacturing physical settlement.

Backend failure shall permanently close the affected GPU session while preserving trustworthy host commitment.

Body destruction and uncertain native publication shall invalidate the relevant X11 publication authority.

## 7. P03 — Experimental empty-SHAPE cutover

**OWNER APPROVED**

The accepted CPU experimental cutover remains the reference mechanism for managed GPU experiments.

### 7.1 Equal coverage

When canonical hit coverage is unchanged:

- Validate the candidate.
- Perform checked pixel upload and blit.
- Flush and commit according to the existing renderer contract.
- Avoid unnecessary SHAPE mutation.

### 7.2 Changed coverage

Preserve the accepted transaction ordering:

1. Validate pixels, metadata and all native request limits.
2. Admit and mark the publication attempt.
3. Logically block body input.
4. Upload to the backing pixmap.
5. Install checked empty input SHAPE.
6. Settle pointer release obligations.
7. Perform checked blit and flush.
8. Install the exact candidate SHAPE.
9. Commit candidate frame geometry.
10. Complete timing bookkeeping.
11. Restore runtime input readiness with the full enabling sequence.

Any uncertainty following native submission shall enter the existing fail-closed publication path.

A timing failure following commitment shall explicitly revoke usable publication authority and fence input.

### 7.3 Atomicity limitation

X11 pixel updates, SHAPE changes and compositor presentation are not atomic.

The intermediate empty input region may allow clicks to pass through. Picom may display pixels from a different point in the update sequence.

Checked native acknowledgments do not establish physical scanout or simultaneous visible/input coverage.

The existing experimental empty-SHAPE acceptance shall not be promoted to production acceptance by D3-P.

Server grabs and old/new region union are excluded from the reference design. Alternative cutover policies require separate review.

## 8. P04 — Desktop timing and gesture authority

**OWNER APPROVED**

`ProbeRuntime` and `FrameTiming` remain authoritative.

GPU preparation must follow the accepted desktop eligibility predicates, including:

- No active owner control operation.
- No pending layer transition.
- Body input readiness.
- Settled interaction and pointer obligations.
- No in-flight movement.

Animation shall pause during grabs, drags, popup interaction and disqualifying lifecycle operations.

Pending GPU work shall become obsolete when an interruption revokes its eligibility.

The displayed frame shall remain frozen and usable where the existing host contract permits.

GPU latency alone shall not pause or distort active animation time.

Animation shall resume only after the existing desktop readiness conditions are restored.

An observed or buffered interaction event shall take precedence over consuming a prospective frame. Events that arise after native publication admission remain subject to existing freshness and input-fencing guarantees.

The worker shall never acquire authority over desktop interaction or animation timing.

## 9. P05 — Qualified CPU fallback and containment

**OWNER APPROVED**

The CPU rendering path shall remain independent of Vulkan availability.

Managed GPU operation shall initially be an explicit experimental mode. The default CPU behavior shall remain unchanged.

### 9.1 Backend failure

GPU preparation failure shall not invalidate an otherwise trustworthy X11 commit.

An initialization failure, worker failure, IPC failure or device fault shall disable the affected GPU session while preserving truthful diagnostic, process and settlement state.

An individual freshness expiry shall not automatically be classified as device failure.

### 9.2 Fallback restrictions

The first managed integration shall demonstrate last-committed-frame containment before automatic CPU fallback is introduced.

A later fallback implementation may prepare an equivalent CPU synthetic frame using the authoritative extent, intent and animation time.

Fallback shall:

- Use the existing checked X11 publication mechanism.
- Preserve the authoritative frame sequence and generation.
- Preserve unresolved GPU work and backend diagnostics.
- Avoid interference from the retired GPU session.
- Require trustworthy host publication state.

Fallback shall not:

- Recover blindly from uncertain X11 submission.
- Reuse invalidated native publication authority.
- Conceal worker failure.
- Automatically restart Vulkan.
- Silently substitute a software adapter for an explicitly required hardware adapter.

Process isolation reduces userspace failure coupling but does not guarantee recovery from kernel-wide or graphics-driver hangs.

## 10. P06 — Transport candidates

**OWNER APPROVED**

Production transport remains UNSELECTED.

The worker-to-owner and owner-to-X-server transport boundaries shall be evaluated separately.

| Candidate | Mechanism | Status |
|---|---|---|
| T1 | Owned BGRA8 IPC bytes and core X11 PutImage | Reference candidate |
| T2 | Owned IPC bytes and MIT-SHM X11 upload | Alternative |
| T3 | Cross-process shared frame memory | Alternative |
| T4 | Vulkan external memory, DMA-BUF, DRI3 and Present | Research candidate |

### 10.1 T1 reference candidate

T1 has the smallest initial integration surface because it preserves the accepted worker payload contract and existing X11 publication mechanism.

It is the preferred candidate for a separately authorized integration experiment, not the selected production transport.

### 10.2 Alternative transport requirements

MIT-SHM requires explicit server-consumption and shared-memory reuse evidence.

Cross-process shared memory requires bounded ownership, synchronization, immutable candidate access and crash-consistent lease handling.

DMA-BUF/DRI3/Present requires verified device capabilities, compatible formats and modifiers, external synchronization, pixmap ownership and buffer-reuse guarantees.

No alternative may weaken physical settlement, publication eligibility or hit-region integrity.

A transport that eliminates CPU pixel readback must still supply trustworthy coverage information correlated to the presented frame.

### 10.3 Current transfer reference

The current fixture is 160 × 160 tightly packed, top-left premultiplied BGRA8.

Each frame contains 102,400 bytes.

At 60 Hz, one uncompressed payload boundary represents 6.144 MB/s. Two byte-oriented boundaries represent approximately 12.288 MB/s of aggregate payload traffic, excluding protocol overhead and additional copies.

These figures are analytical estimates, not measured transport performance.

## 11. P07 — Readiness, pacing and backpressure

**OWNER APPROVED**

The existing desktop event loop shall integrate supervisor readiness rather than wait synchronously for worker operations.

### 11.1 Wait interests

The managed wait architecture shall accommodate:

- X11 socket readability.
- Worker socket readability.
- Worker socket writability when required.
- Process descriptor readiness when available.
- Earliest active monotonic deadline.

Where no process descriptor is available, the accepted supervisor's process-observation requirements must remain satisfied without introducing unbounded blocking.

Host socket failure and worker socket failure remain separate classifications.

### 11.2 Scheduling order

The owner shall preserve existing lifecycle and control scheduling, including deadline precedence.

Worker supervision shall receive bounded progress during sustained X11 event traffic.

GPU candidate publication shall occur only at the safe owner presentation boundary.

Worker receive-budget exhaustion shall request further bounded progress without starving desktop servicing.

Buffered X11 events and immediately available supervisor work shall be accounted for before sleeping.

### 11.3 Bounded work

The integration shall preserve:

- One outstanding GPU job or one ready candidate.
- No unbounded request queue.
- No catch-up rendering burst.
- No repeated submission while physical work remains outstanding.
- No busy loop caused by an already-consumed animation opportunity.
- No permanent animation polling during ineligible periods.

Supervision must remain active when animation is paused.

### 11.4 Pacing investigation

Absolute monotonic request deadlines are the preferred candidate for a later pacing investigation.

The existing CPU timing behavior shall remain unchanged.

An asynchronous implementation must distinguish request-opportunity consumption from successful publication. A pending job shall not leave a continuously due animation deadline that causes spinning.

The headless 100 ms stress freshness allowance is an experimental reference, not an approved production latency budget.

The exact-only 1,000 ms allowance remains confined to its accepted headless correctness workload.

No new production freshness threshold or scheduler mechanism is selected by D3-P.

### 11.5 Performance evidence

The accepted stress observations are:

| Metric | Software Vulkan | RTX 3050 |
|---|---:|---:|
| Median cadence | 17.335 ms | 17.358 ms |
| Median end-to-end latency | 1.524 ms | 1.629 ms |
| p95 end-to-end latency | 1.856 ms | 1.942 ms |
| p99 end-to-end latency | 2.041 ms | 2.168 ms |
| Valid frames in observation window | 577 | 577 |
| Busy opportunities | 0 | 0 |

These results motivate scheduler investigation but do not establish universal GPU performance.

Managed measurements shall distinguish request opportunities, eligible candidates, native commitments and independently observed compositor presentation.

## 12. P08 — Minimal dependency integration

**OWNER APPROVED**

Preserve the existing crate and workspace boundaries wherever feasible.

The supervisor currently has a development dependency on desktop-probe. Introducing a reverse production dependency requires an explicit Cargo graph and test-identity review.

The integration shall not introduce:

- A Vulkan runtime dependency into the desktop owner.
- The headless acceptance harness as a production dependency.
- Unrelated repository restructuring.
- Wholesale presentation-module extraction.
- Unnecessary workspace consolidation.
- Unreviewed dependency upgrades.

Any required manifest or lockfile changes shall be explicitly scoped and independently reviewed.

Rust 1.88 compatibility remains UNVERIFIED.

## 13. P09 — Incremental implementation roadmap

**OWNER APPROVED — EXECUTION NOT AUTHORIZED**

### D3-A — Asynchronous presentation seam

Establish separation between request construction, asynchronous preparation and checked publication.

Use deterministic fake completions.

Preserve synchronous CPU behavior, existing timing semantics and all accepted publication guarantees.

Do not integrate a native worker or select a transport.

Exit gate: independent source review and deterministic preservation evidence.

### D3-B — Managed supervisor integration

Integrate the accepted supervisor, job correlation, readiness, deadlines and late-fault policy.

Use GPU-free protocol fixtures first.

Exercise cancellation, truncation, partial transmission, closure, unresolved work and reaping.

Exit gate: independent review of integrated supervision and race tests.

### D3-C — Experimental managed Vulkan publication

Connect the accepted Vulkan worker to the managed X11 host using a separately authorized reference transport.

Exercise exact pixels, all five fixture phases, SHAPE, Expose, gesture coordination and desktop lifecycle.

Exit gate: independent source review followed by owner-native correctness acceptance.

### D3-D — Failure containment and CPU fallback

Establish safe backend disablement and qualified CPU fallback.

Exercise initialization rejection, worker failure, timeout, shutdown uncertainty and backend transition.

Preserve host fatal handling for uncertain publication.

Exit gate: deterministic containment evidence and separately authorized native observations.

### D3-E — Managed pacing and transport evaluation

Investigate scheduling precision, latency, backpressure and publication throughput.

Benchmark justified transport alternatives.

Reevaluate strict 60 Hz readiness using predefined observation windows and truthful measurement classifications.

Exit gate: reviewed performance evidence without fabricated compositor or scanout claims.

### D3-F — Integrated acceptance and closeout

Consolidate independently reviewed source, automated and native evidence.

Document remaining limitations and obtain explicit owner approval.

Production transport and production empty-SHAPE readiness require separate decisions if still outstanding.

No stage automatically authorizes the next stage.

## 14. Acceptance gate register

| Gate | Requirement |
|---|---|
| G01 | Preserve every accepted automated test identity |
| G02 | Preserve CPU presentation and A00 behavior |
| G03 | Exact session, epoch, serial and frame correlation |
| G04 | Observation-bounded late-fault publication |
| G05 | Truthful physical settlement and unresolved debt |
| G06 | Managed Vulkan exact pixel and metadata parity |
| G07 | SHAPE coverage, cutover safety and click-through |
| G08 | Gesture coordination and animation interruption |
| G09 | Visibility, placement, focus and lifecycle preservation |
| G10 | Worker failure isolation |
| G11 | Bounded work and backpressure |
| G12 | Truthful managed performance measurements |
| G13 | Resource release, shutdown and reaping |

### 14.1 Mandatory deterministic race categories

The staged assignments shall include, where applicable:

- Complete frame and backend fault observed in the same receive progression.
- Complete frame followed by a partially buffered fault message.
- Receive-budget exhaustion before drainage.
- Frame candidate invalidated by gesture or lifecycle interruption.
- Freshness expiry before publication admission.
- Backend fault first observed after successful X11 commitment.
- Ordinary late fault during transmission of a genuinely settled frame.
- Invalidated partial terminal and transport abandonment.
- Stale and duplicate identities.
- Zero-byte withdrawal versus partially transmitted work.
- Process death and reaping with unresolved work.
- X11 upload, blit, SHAPE or timing failure.
- Deadline and readiness contention.
- Bounded fairness under sustained event traffic.

### 14.2 Owner-native acceptance scope

Native testing shall be separately authorized and shall include the accepted software and hardware Vulkan environments.

Required observations shall cover:

- All five synthetic fixture phases.
- Exact pixel and metadata parity.
- Equal and changing coverage.
- Fully transparent frame behavior.
- Transparent-region click-through.
- Gesture and popup coordination.
- Expose repaint.
- Hide, Show, Bring Here, Bring Top and Recover.
- Genuine minimization and cross-workspace placement.
- Effective layer and focus preservation.
- Unavailable Vulkan ICD behavior.
- Orderly shutdown and owner reacquisition.

Dangerous system-wide driver hangs and deliberate physical GPU-loss experiments are excluded. Such cases remain qualified deterministic fault-injection evidence.

### 14.3 Performance classification

Functional correctness acceptance does not establish strict 60 Hz readiness.

Request rate, GPU completion rate, X11 commitment rate and actual compositor-visible rate shall remain separately reported.

The eventual performance assignment must define its thresholds, windows, cohorts and measurement mechanisms before execution.

## 15. Preserved limitations and exclusions

The following remain outside the authorization granted by D3-P:

- Production transport selection.
- Production empty-SHAPE cutover acceptance.
- Claims of atomic visible and input presentation.
- Strict 60 Hz readiness.
- Complete cold-start characterization.
- Rust 1.88 compatibility confirmation.
- General repository maintenance.
- B1 progress-site publication confirmation.
- Private Vanilla or Spine assets.
- Wayland or other window-manager integration.
- Automatic Vulkan restart.
- Unrelated A00 changes.

## 16. Workflow and review authority

ChatGPT retains responsibility for research, architecture, task management, specification and independent review.

Codex implements and performs authorized automated verification.

The owner controls native execution, technical acceptance and Git publication.

All work remains on the long-lived `a01` branch unless the owner explicitly authorizes otherwise.

Every implementation assignment shall establish a pinned baseline, allowed modification scope, preserved invariants, test requirements, review artifacts and an explicit stopping point.

No implementation, staging, commit, push, merge or progress-site mutation is authorized by this specification.

## 17. Source references

Repository references are pinned to the verified baseline commit.

- [Published baseline commit](https://github.com/zNe4/DesktopRoomie/commit/1313a9251427875d8c7814b2c64131da75d5a37b)
- [Accepted architecture](https://github.com/zNe4/DesktopRoomie/blob/1313a9251427875d8c7814b2c64131da75d5a37b/docs/ARCHITECTURE.md)
- [D2-C acceptance record](https://github.com/zNe4/DesktopRoomie/blob/1313a9251427875d8c7814b2c64131da75d5a37b/docs/A01-C1-C-B2-D2-C-ACCEPTANCE.md)
- [D2-B-R2 native settlement contract](https://github.com/zNe4/DesktopRoomie/blob/1313a9251427875d8c7814b2c64131da75d5a37b/experiments/gpu-worker/vulkan/NATIVE-CONTRACT.md)
- [X.Org client-server communication](https://www.x.org/guide/communication/)
- [X.Org Present protocol](https://cgit.freedesktop.org/xorg/proto/presentproto/tree/presentproto.txt)
- [Khronos Vulkan specification](https://registry.khronos.org/vulkan/specs/latest/html/vkspec.html)
- [Linux DMA-BUF synchronization](https://dri.freedesktop.org/docs/drm/driver-api/dma-buf.html)
- [Linux monotonic timerfd interface](https://man7.org/linux/man-pages/man2/timerfd_create.2.html)

## 18. Final ratification

All decisions P01–P09 were explicitly approved by the owner on 2026-11-24.

Owner observations: None.

The architecture is ratified. The outstanding late backend fault publication-policy question is resolved by P01.

The production transport remains unselected.

The next permissible activity is preparation of the bounded D3-A implementation assignment. Its execution requires separate explicit authorization.

**D3-P — ARCHITECTURE RATIFIED — OWNER APPROVED — IMPLEMENTATION PENDING AUTHORIZATION.**
