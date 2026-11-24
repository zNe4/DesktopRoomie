# D2-B-R2 native settlement contract

Uncommitted implementation correction for independent ChatGPT source review.
D2-B is **not technically accepted**. No D2-C Vulkan execution is claimed.
Baseline `4bfe899ed0f3dd18e721b6e86b5d53dd85c24b43`; reviewed D2-B cumulative
patch SHA-256 `4f5a2db9d99bf24cc8ae7060eac80a786ab61584e75c0e66797ad6899d960af7`.

## Exact source inspected

This is bounded verification of the dependencies actually compiled, not a new
architecture or comparative study. The handoff's `upstream-source-sha256.json`
authenticates each inspected registry source against its cached `.crate` archive
and each archive against the unchanged Cargo.lock checksum. All comparisons
match. `upstream-contract-excerpts.txt` retains exact numbered source excerpts.
Line ranges below are **one-based lines in those authenticated crate sources**.
The versioned source links are navigation references; browser retrieval of those
versioned docs.rs pages was unavailable. Web navigation responses were not used to establish byte identity or these
line ranges.

| Dependency and source | Precise contract inspected |
| --- | --- |
| [wgpu 29.0.1, `src/api/queue.rs`](https://docs.rs/crate/wgpu/29.0.1/source/src/api/queue.rs), 257–279 | Public submit wraps the returned native index without a Result. |
| [wgpu 29.0.1, `src/backend/wgpu_core.rs`](https://docs.rs/crate/wgpu/29.0.1/source/src/backend/wgpu_core.rs), 2086–2113 | Native submit handles `Err((index,error))` through the error sink and still returns the index. |
| Same backend, 265–309 and 641–677 | With this worker's registered uncaptured handler and no error scopes, validation/internal/OOM errors invoke that handler synchronously before the native call returns. DeviceLost is deliberately omitted from this error route. |
| Same backend, 1903–1916; [core 29.0.4 `src/device/life.rs`](https://docs.rs/crate/wgpu-core/29.0.4/source/src/device/life.rs), 128–151 | Poll exposes timeout/wrong-index as errors; other core wait errors take the fatal panic path. Only the poll call has narrow panic containment. |
| [core 29.0.4 `src/device/queue.rs`](https://docs.rs/crate/wgpu-core/29.0.4/source/src/device/queue.rs), 1189–1215 and 1439–1507 | An index is allocated before validation; successful-index advancement follows HAL submit. A later maintenance error can occur after submission. An error index never proves non-submission or completion. |
| [core 29.0.4 `src/device/resource.rs`](https://docs.rs/crate/wgpu-core/29.0.4/source/src/device/resource.rs), 827–917 and 954–999; [core `src/device/global.rs`](https://docs.rs/crate/wgpu-core/29.0.4/source/src/device/global.rs), 1860–1875 | Specific wait validates against the last successful index and checks the finished fence. QueueEmpty/WaitSucceeded prove that target's completion under core's contract. Callbacks are fired before device_poll returns. |
| Backend, 2205–2237; core global, 2066–2096; [core `src/resource.rs`](https://docs.rs/crate/wgpu-core/29.0.4/source/src/resource.rs), 780–836 | A mapping request's normal return is not completion. Immediate errors can call the mapping callback and the error sink; later map failures can leave Idle. Successful mapping establishes Active before its callback. |
| [wgpu `src/api/buffer.rs`](https://docs.rs/crate/wgpu/29.0.1/source/src/api/buffer.rs), 322–332; backend, 2257–2266 | Public unmap resets the frontend map context and delegates to a unit-return native call. That native call can report a core error through the sink while returning normally. |
| Core global, 2110–2126; core resource, 837–857 and 906–947 | Unmap checks buffer/device validity. Idle produces NotMapped; Waiting cancellation, Active read unmap and errors have distinct behavior. A failed map followed by normally returning unmap is not affirmative cleanup evidence. |
| Core device resource, 701–713 and 5209–5229; core global, 1995–2031; core device resource, 954–999 | HAL errors call lose, invalidate the device and invoke an available Unknown callback. Explicit destroy invalidates and relies on maintenance to deliver Destroyed when empty. The callback can be absent/already consumed; reason alone is not a universal proof of intent. |
| Backend, 1929–1934 and 2142–2147; core queue, 157–251; core device resource, 307–342 | Wrapper Drop reaches native owners. Queue Drop performs its own waits and can panic; Device Drop destroys native resources. Neither is a substitute for prior settlement proof. |

These are source-contract conclusions for an exclusively owned device/queue:
the worker has one native caller, no external device/queue clones, no error
scopes and no concurrent submission authority. They do not prove driver or
hardware correctness or absence of a future asynchronous fault.

## Revised submission and staging state machine

```text
NotAttempted
  -> Attempted (before native submit)
  -> IndexReturned (only a candidate; acceptance unproven)
  -> Accepted -> Complete
       both justified by the same quiet successful specific-index wait
```

Submit's error-handler failure leaves Attempted. An index followed by a failed,
wrong-index, faulted or panicking wait leaves IndexReturned. Neither state is
settled. A native error after HAL submit may mean real work exists, so these
paths report BackendFault then Unresolved and preserve the process boundary.
Accepted and Complete are separate facts/stages even when the successful wait
establishes them together. Both waits still target that exact opaque index;
the full owner session/job/phase remains a separate operation identity. The
submission and mapping waits share the original two-second controlled budget.

```text
NotRequested -> Requested -> Confirmed or Failed (matching tagged callback only)
Confirmed -> Live view -> actual view released -> checked unmap
Failed    -> cleanup attempt only when native state allows a checked call
checked unmap -> quiet post-call QueueEmpty observation -> explicitly unmapped
any missing/stale/unproven callback, native error/loss or failed cleanup -> uncertain
```

`checked_call` samples the operational revision and rejects any prior native
error conservatively. It checks normal returns against subsequent native error
observations and sticky unexpected device loss. `checked_unmap` checks immediately
after the native call and again after a nonblocking queue-empty poll. The poll
also drains deferred invalid-device callbacks; quiet error-handler state alone
cannot cover core's DeviceLost suppression. It is additional cleanup evidence,
not a replacement for the earlier submission-specific waits or matching map.

The first diagnostic has independent bounded retention. Mapping/readback errors
retain their original cause before cleanup, while cleanup's native errors still
advance the revision. Saturation never wraps or restores eligibility. A prior
native fault prevents a new cleanup call from asserting success. An abstract
GPU-free backend may affirm successful cleanup after a failed map; the native
adapter specifically cannot do so when core reports NotMapped or device failure.

Slot retirement closes admission; it does not settle staging. No replacement,
retry or reuse is introduced. Readback still copies meaningful row prefixes into
owned bytes, releases the actual view and only then attempts explicit unmap.

## Backend-to-writer and release obligations

A successful backend returns `Prepared { bgra, evidence }`. Frame requires the
complete expected operation identity, exact 102400-byte length, Complete submit,
Confirmed map, Released view, affirmative unmap, no uncertainty/retirement, and
equality with the backend's retained evidence plus its current settlement check.
JobFailed also requires correlated retained settlement evidence; invalid bytes
or an Ok return cannot manufacture it. RejectedBeforeSubmission requires an
explicit correlated never-submitted/never-mapped proof from a settled backend.
The writer rechecks terminal evidence at the send barrier. Contract violations
produce BackendFault first and Unresolved for the original owner key.

`Persistent::release_eligible()` is the common authority for explicit release,
rejection eligibility, backend settlement and defensive Drop. InFlight alone
blocks release, even if a prior operation's stored evidence was settled or the
interrupted operation never recorded a result. Unproven mapping/unmap, unexpected
loss or native uncertainty also blocks it. Tests catch an injected operation
panic outside the engine and call explicit release before Drop; protection does
not depend on the executable panic hook.

An eligible native release first polls outside any intentional-destruction
context. A new error, nonempty queue or unexpected loss aborts preparation and
keeps the native owner alive. Only then does `checked_release` open the explicit
Device::destroy context, call destroy and drain its callback with another
queue-empty poll. Destroyed is expected only there. Unknown is always a fault.
Wrappers are dropped only after preparation succeeds and eligibility still
holds; new errors during actual Drop prevent Stopped. Defensive Drop uses the
same release path and forgets any remaining uncertain native owner.

Faulted native initialization after constructing resources also retains/leaks
that uncertain owner and prohibits a controlled empty-backend acknowledgment.
Early failure before native resources exist can release the empty backend.
Unexpected non-poll panics still exit immediately through the native hook.
Unresolved transfers exit after attempting the correlated fault/report pair;
uncertain resource owners never undergo ordinary destruction merely to clean up.
Process termination does not establish physical settlement.

## Finding-by-finding resolution (F01–F10)

IDs below correspond to the ten required R1 regressions in the assignment.

| Finding | Resolution and deterministic evidence |
| --- | --- |
| F01 — Index returned with native error | Shared checked native call catches the handler observation; no acceptance/completion or map. `native_submission_index_with_error_handler_failure_is_not_acceptance`. |
| F02 — Normal unmap return with native error | Shared native unmap contract leaves unmap unproven; no safe retirement, Frame or JobFailed. `normal_unmap_return_with_native_error_never_proves_unmap_or_safe_retirement`. |
| F03 — Secondary fault after first diagnostic | Separate retain/revision/loss facts; later faults retain operational authority without replacing the cause. `retained_diagnostic_and_secondary_native_errors_have_independent_authority`. |
| F04 — Failed map cleanup | Retain the map cause before checked cleanup; native NotMapped/error or device loss remains unresolved. `failed_map_cleanup_error_retains_first_diagnostic_and_does_not_settle`, `device_loss_before_unmap_and_deferred_destroyed_block_settlement`. |
| F05 — Apparent success lacks settlement | Prepared bytes need explicit retained physical evidence. `apparently_successful_backend_must_supply_current_physical_settlement`. |
| F06 — Successful evidence for another operation | Compare all session/job/phase fields and retained evidence before any terminal assertion. `success_or_failure_evidence_for_another_operation_cannot_settle_owner_job`. |
| F07 — Invalid payload plus unresolved backend | Length errors only become JobFailed when independently settled; otherwise Unresolved. `invalid_payload_with_unsettled_backend_cannot_become_job_failed`. |
| F08 — Explicit release bypasses in-flight protection | One predicate and one release path for explicit release and Drop, including interrupted first/subsequent submissions. `explicit_release_and_drop_share_interrupted_operation_protection`. |
| F09 — Intentional versus unexpected destruction | Explicit destroy context follows a quiet preflight; unexpected Destroyed and every Unknown remain faults. Unit loss/preflight tests and `intentional_drop_is_distinct_from_unexpected_loss_during_teardown`. |
| F10 — Partial control messages and real IPC precedence | Ready/Frame/Stopped are tested both before first byte and after a prefix. R1 finished existing framing before later faults. R2 below restricts that behavior to still-valid settlement; no observed fault is cleared. Private IPC executes normal-return native-error injections and preserves BackendFault→Unresolved, plus backend fault through settled failure, Stopped and successful exit. |

The injected call closures really return an index/unit and notify the same
revision-aware contract helpers used by the Vulkan adapter. These are not tests
that merely change an assumed infallible native API into an explicit Err.

## Telemetry, compatibility and remaining native claims

Fixed-size per-stage durations now cover encode, submission, submission wait,
mapping (request/wait/callback) and repacking, including failed returned calls.
Existing counters and bounded last-frame stage trace remain. Engine/worker
invariant-bearing mutable fields are private; inspection is read-only. No wire
bytes, production dependencies, renderer pins, shader/layout, supervisor or
protocol were changed by R1. Rust 1.88 is still the declared floor; only Rust
1.99 is installed here, so exact 1.88 compilation remains **unverified**.

D2-C must still verify this worker's real initialization/adapter/manifest path,
five-phase CPU/GPU parity, persistent resources, timing/counters, successful
specific waits and quiet unmap, native callback observation and explicit native
destroy/release. Safely exercisable fault/timeout containment remains native
verification work. No physical device loss or unsafe hang was deliberately
induced. Source-contract and GPU-free evidence cannot replace those runs.
Synchronous driver calls/destruction have no hard wall-clock guarantee, and a
fault may arrive after a previously valid response. Already transmitted bytes
cannot be retracted. R2 abandons an invalid partial terminal; a later fault remains
authoritative, including after a complete Stopped (which cannot be retracted).
No X11 commitment, production transport or display acceptance is asserted.


## R2 terminal integrity and interrupted release

R1 cumulative source is pinned by patch SHA-256
`de7fdd4d0aa657249214315a57bc2a591438ba38b8d1c024bb01b613265d44da`.
This correction preserves the native submit/unmap proof above and the production
panic hook. It adds no telemetry transport or native execution evidence.

| Observation | Entirely unsent terminal | Partial terminal | Complete terminal |
| --- | --- | --- | --- |
| Ordinary backend fault; physical settlement still valid | BackendFault then settled JobFailed for Frame; keep valid JobFailed/rejection | Finish truthful terminal, then attempt BackendFault if not already reported | Cannot retract; attempt independent fault notification |
| Unexpected loss, native observation uncertainty, missing/unsettled/mismatched evidence | BackendFault then exact-key Unresolved; uncertain exit | Sticky transport abandonment; no further writes or message splicing; private stream shutdown; uncertain exit | Cannot retract or invent a new unresolved job report; later backend fault/unsafe teardown remains authoritative |
| Invalidated release / Stopped proof | Suppress Stopped; fault; uncertain exit | Same sticky abandonment | Cannot retract Stopped; session exit becomes uncertain if observed before return |

A fault behind a truthful partial terminal is best-effort: backpressure can delay
it indefinitely until owner supervision closes the session, and disconnection can
prevent delivery. First-cause diagnostics remain bounded and independent native
error/loss tracking remains sticky. No complete terminal can promise immunity to
future asynchronous faults. At the supervisor, truncated bytes/EOF never settle
the outstanding job, and Stopped alone never confirms controlled shutdown without
successful child exit.

The R1 test identity `loss_before_or_during_stopped_never_becomes_successful_shutdown`
is preserved with an intentionally strengthened partial expectation: Truncated,
no decoded Stopped, and Uncertain, replacing completed Stopped plus BackendFault.
The existing ordinary-fault prefix expectations remain valid because their
physical settlement stays intact. `tests/r2.rs` adds the 36 terminal/boundary/fault
cases, missing and all mismatched operation fields, both unexpected loss kinds,
injected native observation uncertainty, stopped boundaries and teardown unwind.
The actual private-IPC regression uses the accepted supervisor unchanged, fully
decodes Render before one controlled short terminal write, then injects loss and
runs `serve` to abandon/close. It checks a protocol truncation failure, no consumed
frame, exact completely dispatched key unresolved after child exit/reaping.

`intentional_release` restores its previous context with a guard even if the
closure unwinds; a subsequent unexpected Destroyed is observed, and Unknown is
never hidden even inside the guard. `Persistent::release` latches release started
and uncertain-shutdown telemetry before native preparation. A preparation unwind
caught by an external test caller leaves release ineligible; explicit retry and
Drop cannot invoke preparation/destruction again or assert controlled shutdown.
Only a completely checked release clears that latch and records controlled
shutdown. Normal production non-poll panics still exit immediately; tests catching
injected panics do not authorize production cleanup recovery.
