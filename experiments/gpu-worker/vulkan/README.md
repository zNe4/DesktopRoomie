# D2-B-R2 persistent headless Vulkan worker

Implementation corrected through D2-B-R2 for independent ChatGPT source review; **not technical or native acceptance**.
[Native source contract and revised settlement proof](NATIVE-CONTRACT.md) describes the reviewed error semantics and F01–F10 corrections.
Pinned baseline: `4bfe899ed0f3dd18e721b6e86b5d53dd85c24b43`, branch `a01`.
No X11 owner, SHAPE, private content, restart, retry or production transport
selection is introduced. The accepted protocol and supervisor sources are unchanged.

## Packages and launch

```text
roomie-worker-vulkan ──► roomie-worker-protocol (std only)
                    ├─► libc =0.2.191 (worker-local socket poll)
                    └─► [native-vulkan only] wgpu =29.0.1 (std,vulkan,wgsl)
roomie-worker-supervisor ──► roomie-worker-protocol + libc =0.2.191
vulkan tests only ──► roomie-worker-supervisor
supervisor tests only ──► desktop-probe
```

Neither protocol nor supervisor enables or depends on Vulkan. `desktop-probe`
and frozen `gpu-readback` remain separate unchanged packages. Default worker
execution reports BackendFault on Initialize because native execution is disabled;
it never substitutes the test backend. `vulkan-worker-fixture` is a separate,
explicit GPU-free verification executable using the same engine/writer/transport.

Build the real worker with `--features native-vulkan`. Launch it only using
the accepted supervisor's private inherited bidirectional socket at fd 0. Use
literal arguments `--adapter default|software|hardware` and optionally
`--driver /absolute/path/to/driver.json`. Default uses B1's high-performance
Vulkan adapter request. Software requires a CPU device; hardware requires an
integrated or discrete GPU. Required policies enumerate Vulkan adapters and
filter before device creation; they do not fall back to another device type.

`--driver` accepts one readable regular JSON manifest, canonicalizes its absolute
UTF-8 path, and rejects lists, embedded NUL, non-JSON paths, unavailable files
and paths over 4096 bytes. The loader validates manifest content and driver
compatibility during initialization. The single-threaded entry point sets only
`VK_DRIVER_FILES`, before creating wgpu. It removes that variable if no manifest
was requested. The existing supervisor passes literal argv and clears the
environment; **no supervisor extension or inherited environment allowlist was
added**. There is no named listener or diagnostic output channel. Driver manifests
and shared libraries must remain available through initialization; a filesystem
race is an initialization failure, never a permission to submit elsewhere.

Rust minimum remains **1.88**, as in B1. The workspace lock retains all existing
D2-A registry records and every used B1 rendering dependency record exactly,
including `wgpu-core`/`wgpu-hal`/`wgpu-types`/`naga` 29.0.4 and `ordered-float`
5.4.0. B1's SHA-256/CPU-oracle-only dependencies are not needed by this package.
No rendering dependency was upgraded. The frozen B1 manifest and lock are unchanged.

## Controlled B1 provenance

All paths below are relative to `experiments/gpu-readback/` at the pinned baseline.
Original files are unchanged. No third-party source or assets were copied.

| B1 reference | Original SHA-256 | Reuse and deliberate changes |
| --- | --- | --- |
| `src/fixture.wgsl` | `bd194b7f6049e3ba56263bcae7466888192fd651cf75034e368628a92e5013f8` | Copied verbatim to `src/fixture.wgsl`. |
| `src/layout.rs` | `adb54f05ceb35e5e4e67b8f9040650ddfd2ba51d6675caabb12856fb37aa81f8` | Copied verbatim including its three tests to `src/layout.rs`. |
| `src/gpu.rs` | `f252595349af7b73726da7ed37e3376d4ff1b370ca85ac4c8610f746d172ecf7` | Setup, pipeline and command encoding adapted in `src/native.rs`; no console output, explicit adapter policy and internal telemetry. Transfer moved into the new worker's generic `engine.rs`, not extracted from B1. |
| `src/readback.rs` | `150ebad9a696f376ef203488c460cd89a6fe0062f530428dac719a6ff45e8e78` | Reference for first-fault latch, tagged callback, deadline, view/unmap ordering and poll containment. Reimplemented with full session/job identity, a one-element nonblocking callback channel and independent settlement facts. B1's staging helper and `unsettled` flag are not copied. |
| `src/run.rs` | `b0b3a238707cce6c76199318d0c01d5149f52c5faae6fee86cf168f0d06aed1c` | Reference for process-exit containment; no benchmark loop or stdout telemetry reused. |
| `src/main.rs` | `89a180b8df5a31ba783dc4ed671aed9f53c294192bc71ccc1ec2fc81489ce141` | Inspected feature boundary; new entry point implements private worker IPC. |
| `src/oracle.rs` | `d9aceb39269634314e96e7a3d71bfa8efd992a542e60e2d40ea14e085a1d24a2` | Inspected CPU parity oracle and phase edges; not copied or used as a fallback. |
| `Cargo.toml` | `115eaea2ef94eae41648151f96c98a11ea0828eaf763f619845daa730c2ce660` | Same pinned rendering feature/dependency contract and Rust compatibility floor. |
| `Cargo.lock` | `538832bfb16548f97b0a3385551703bc98752c4b54a6252ab3127754ddac5278` | Used to seed exact rendering dependency records into the worker workspace lock. |

The inspected accepted CPU fixture at
`experiments/desktop-probe/src/presentation/synthetic.rs` has SHA-256
`9bcca46707113b45f897e2ec688e4ff2467efb14d4416335a8d5729238ebb099`.
The B1 acceptance record is `docs/A01-C1-C-B1-ACCEPTANCE.md`.

## Worker lifecycle

```text
AwaitInitialize -- validated Initialize --> initialize once
    initialization + fault barrier succeeds --> Ready response --> Ready
    initialization failure                 --> BackendFault --> Faulted
Ready -- checked increasing Render serial --> one synchronous engine operation
    unsupported valid intent --> RejectedBeforeSubmission --> Ready
    settled success          --> Frame + send fault barrier --> Ready
    native failure           --> BackendFault --> JobFailed --> Faulted
    uncertainty              --> BackendFault --> Unresolved --> process exit(2)
Ready/Faulted -- Shutdown --> close admission --> prove settlement --> release
    release succeeds + fault barrier --> Stopped --> exit(0)
    release uncertain                --> no Stopped --> exit(2)
transport/protocol failure --> close + safe release if provable --> exit(1/2)
unexpected non-poll panic --> process failure, never Stopped
```

Only Initialize may establish the session. Every later request must match it;
serials are nonzero, strictly increasing with checked arithmetic and never wrap.
Epoch/generation/sequence are owner-issued values, preserved without invention.
A successful backend result carries bytes and explicit settlement evidence; the writer checks exact operation identity, length, frame-ready facts and equality with the backend's retained evidence. Failure and pre-submission rejection also require correlated physical evidence.
The original version 1 codec validates message lengths, extent, format, enum and
time encodings. The synthetic backend accepts only Idle/Neutral. Phase is
`(elapsed.as_millis() / 300) % 5` using the immutable request duration and u128
arithmetic; delta and worker wall time never advance animation.

There is one decoder, one transmitter, and at most two pending typed responses
(fault plus associated report). No further command is read while any response
remains incomplete. Short writes and WouldBlock retain the prefix. Fault inspection
before a zero-offset terminal can suppress or replace it. Frame, JobFailed and
RejectedBeforeSubmission require current correlated physical settlement at every
write barrier. If that proof becomes invalid while entirely unsent, BackendFault
and Unresolved replace the terminal (an already reported fault is not duplicated).
If a prefix exists, the worker permanently abandons the transport, writes no more
bytes, closes the private stream and exits uncertain. It neither completes the
invalid terminal nor splices a fault into its prefix. Stopped follows the same
unsent suppression / partial abandonment policy when release proof is invalidated.
Loss observed after a complete Stopped still makes the session exit uncertain;
controlled shutdown requires successful child exit as well as Stopped.

An ordinary late backend fault can leave genuine prior physical settlement valid.
An unsent Frame becomes a settled JobFailed; a valid rejection remains a rejection.
A partially sent truthful terminal finishes before any new BackendFault. Fault
notification can therefore be delayed by backpressure or lost on disconnection;
an already reported first fault is not sent again for each secondary error.
Complete messages already transmitted cannot be retracted, and no response rules
out a future asynchronous fault. Callback threads never write protocol bytes.
After a settled fault, a Render already in transit is correlated and rejected
before submission; GPU admission remains closed. Unresolved faults exit instead.
Idle/backpressure observation uses one worker-local 100 ms finite socket poll and
nonblocking device progress, with no idle busy loop or supervisor polling change.

## Submission and staging evidence

The session owns one device/queue, render pipeline, 160×160 linear `Bgra8Unorm`
single-sample target/view and 122,880-byte staging buffer. Blending is disabled.
Each operation creates commands, an operation-specific callback channel, a mapped
view when authorized and owned packed bytes. It never replaces staging or devices.

`Evidence` retains the complete `Operation { session, job, phase }` separately
from the opaque native submission index. Its facts progress independently:

| Fact | Progression / required proof |
| --- | --- |
| Submission | NotAttempted → Attempted before submit → IndexReturned (unaccepted candidate) → Accepted → Complete. Only a quiet successful submission-specific wait proves the last two facts. A normal API return or opaque index alone proves neither. |
| Mapping | NotRequested → Requested before map call → Confirmed or Failed only on a matching callback. Missing/stale callbacks remain Requested even after GPU completion. |
| CPU view | NeverOpened → Live only after matching successful map → Released only after the actual view scope ends. |
| Explicit unmap | false → true only after releasing any live view, a checked native unmap, a quiet post-call queue-empty poll, and unchanged operational fault revision. A normal return alone is insufficient; failed cleanup stays unresolved. |
| Slot | Available → InFlight → Available only on success; any real failure permanently retires it. No reacquisition/reallocation after retirement. |
| Retirement | Retired records admission/ownership policy only. `safely_retired()` additionally requires submission **and** staging settlement. |
| Uncertain native state | Error-handler observations, unexpected device loss, interrupted operations and unproven polls/unmap block reuse and ordinary release. One release-eligibility predicate governs explicit release and defensive Drop. |

Submission and mapping waits share one two-second budget, beginning before the
submit call. Both native waits name the same exact wgpu submission. A mapped view
is repacked by copying only each 640-byte row prefix from the 768-byte pitch into
102,400 owned bytes, then released before explicit unmap. Even layout failure
releases the view and explicitly unmaps before it may become a settled JobFailed.
A failed matching map callback also requires affirmative cleanup evidence. In the native core, an unsuccessful mapping can already be Idle, so a subsequent unmap can report NotMapped; that is unresolved cleanup, not settled JobFailed. A missing, stale or
unproven callback never permits access, unmap-based settlement inference or reuse.

## Failure classification and native containment

| Event | Assertion / session consequence |
| --- | --- |
| Unsupported valid intent | RejectedBeforeSubmission, zero submissions; session remains usable. |
| Settled owned readback with send barrier clear | Frame. |
| Initialization failure | BackendFault, no Ready. Safe empty-backend shutdown may later send Stopped. |
| Settled GPU/readback failure | BackendFault first, then JobFailed for original owner key; admission permanently closed. |
| Submission/staging uncertainty, including stale callbacks | BackendFault first, then Unresolved; no Stopped or ordinary native cleanup. |
| Idle backend fault | BackendFault independent of job freshness; admission permanently closed. |
| Unexpected panic/crash | Process failure; no fabricated settlement. |
| Proven controlled release | Stopped only after release, then successful process exit. |
| Uncertain teardown | No Stopped. |

The first diagnostic retains at most 512 UTF-8 bytes. A separate saturating
revision records every native error observation, including secondary errors;
unexpected device loss is independently sticky. Native error callbacks do not
write the socket. `Destroyed` is expected only inside a checked explicit
Device::destroy context after a quiet release preflight; it is a fault everywhere
else. Unknown loss is always retained, including during destruction. The writer checks
the latch before Ready, work, Frame and Stopped, including late faults at the send
barrier. Faults never lose authority because their associated job is stale.

B1's `Device::poll` panic containment is retained: a narrow catch around only
that call leaves settlement unproven. The native executable's panic hook exits
immediately for **every other panic**, including submit, map, view, unmap, setup
or destruction panics. It does not catch arbitrary panics and resume cleanup.
Unresolved transfer paths transmit the bounded fault/Unresolved pair if possible,
then call `process::exit(2)` while native owners remain alive. Transport failure
in uncertain state follows the same exit boundary. A defensive engine Drop leaks
its native owner when evidence is uncertain or transfer is still in flight,
protecting early-return/library paths as well. Release preparation latches a
release-in-progress state before touching native teardown. An interrupted or
uncertain attempt cannot be retried by explicit release or defensive Drop. The
intentional Destroyed context is scoped by an unwind guard; Unknown loss is never
suppressed. Production panic containment is unchanged. This intentionally bypasses
Queue::drop's implicit wait, resource destruction and ordinary unwinding; it
does **not** establish physical GPU settlement. Process termination, job settlement
and future X11 commitment remain distinct.

## Telemetry and verification boundary

`Persistent::telemetry()` and `NativeBackend::telemetry()` expose bounded internal
adapter name/type, total initialization time, submission-attempt/map/unmap counters,
staging allocation/reuse/retirement counters, peak outstanding work, the most
recent frame's at-most-ten preparation stages/time, and controlled/uncertain
shutdown. Fixed-size last-operation durations cover encode, submission, submission wait, mapping and repacking. Invariant-bearing engine and worker state is private with read-only accessors. No telemetry changes the wire or opens a logging channel. Synthetic
test observers inspect these values and actual view/resource Drop ordering.

From repository root, run workspace formatting and, separately for default and
`--features native-vulkan`, `cargo build`, `cargo clippy -- -D warnings` and
`cargo test`, using `--manifest-path experiments/gpu-worker/Cargo.toml --workspace
--all-targets --locked`. Tests run without a GPU in both configurations. Native
poll-panic unit tests invoke a synthetic closure; they create no native objects.
Subprocess tests execute the production protocol loop with injected operations,
including an end-to-end run through the unchanged accepted supervisor. The real
native binary is exercised only with an invalid launch configuration, before
any Vulkan initialization. `tests/support.rs` is fixture code only.

D2-C must still establish real software/hardware adapter selection and explicit
manifest behavior, all five phases' exact CPU/GPU parity (including repeated
owner times), persistent-resource telemetry, native fault callback delivery,
submission/map timeout and device-loss behavior where safely exercisable,
controlled native teardown and process-exit containment. Driver calls and
destruction have no hard wall-clock guarantee. Compilation, injected evidence
and historical B1 parity are **not** successful native execution of this worker.
No desktop acceptance, production transport suitability, compositor/scanout or
SHAPE behavior is asserted. Publication and all native acceptance remain with
the owner; stop at independent source review.
