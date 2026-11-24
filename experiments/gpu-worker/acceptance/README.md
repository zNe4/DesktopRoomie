# D2-C-A-R2 headless acceptance infrastructure

Implementation candidate for independent review. R2 native acceptance is **NOT
EXECUTED**. This package does not connect to X11 or publish desktop frames.

The owner-approved `A01-C1-C-B2-D2-C-P-SPECIFICATION.md` governs the package.
The private review handoff preserves its exact copy, provenance and hash.

## Execution boundary

The harness library depends on public presentation types, protocol and supervisor,
and never links the Vulkan renderer. `Adapter` retains the authoritative
`FrameRequest`, reserves D1's sole slot, submits its exact identity through the
actual supervisor, and consumes only drained supervisor reports. The supervisor
spawns a subprocess with its accepted environment/descriptor isolation and Unix
socket. There is no direct-renderer fast path in either acceptance workload.

`fixture/` is a separate, explicitly synthetic subprocess package. It links the
accepted worker state machine, transport, persistent transfer engine and CPU-only
operations. It supplies CPU fixture pixels and injected failures for plumbing
tests. This is not a Vulkan fallback. The harness executable requires an explicit
`--synthetic` option to launch it; native commands require `--adapter software` or
`--adapter hardware`. Hardware policy never falls back to a CPU adapter.

Run the established non-native matrix from the repository root. Build workspace
binaries before integration tests, which launch `target/debug/roomie-acceptance-fixture`:

```sh
cargo build --manifest-path experiments/gpu-worker/Cargo.toml --workspace --all-targets --locked
cargo fmt --manifest-path experiments/gpu-worker/Cargo.toml --all --check
cargo clippy --manifest-path experiments/gpu-worker/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo test --manifest-path experiments/gpu-worker/Cargo.toml --workspace --all-targets --locked
```

Repeat build, Clippy and tests with `--features native-vulkan` for feature-enabled
compilation and CPU-only tests. None of these tests request a Vulkan device. The
frozen B1 and desktop-probe checks remain separate obligations. Exact Rust 1.88
compatibility is **UNVERIFIED** unless that actual compiler is tested.

## Settlement and failure handling

Both D1 and supervisor receive the same owner-observed monotonic time and
cancellation decisions. D1 deadlines use that time relative to one `Instant`
origin; supervisor times use durations from the same origin. Full session,
epoch, serial, frame generation and frame sequence remain correlated. Admission
or a queued Render does not prove dispatch: the dispatched counter advances only
after the supervisor observes complete transmission of that exact Render.

The minimal D1 `SettledWithoutPayload` extension maps a supervisor `Obsolete(job)`
to genuine settlement without inventing pixels or a GPU failure. It always
returns Obsolete and never creates a ready frame, including after resume. This
contract extension passed independent D2-C-A source review and is unchanged in R1.

Backend-wide failure closes D1 eligibility before report consumption. A matching
terminal can still discharge physical debt. Protocol truncation, exit, cancellation,
timeouts and reaping cannot discharge it. The adapter retains the exact D1 job
after terminal transport abandonment and reaping. No respawn/retry is implemented.
Before-dispatch cancellation uses the supervisor's accepted zero-byte withdrawal
during shutdown; ordinary post-dispatch cancellation preserves the session and
debt until a real terminal report.

Shutdown uses the accepted supervisor's graceful/escalation/exit-observation
policy. `finish` adds only a **200 ms observation margin** to its finite owner
loop; this implementation parameter does not extend or renew any supervisor
deadline. Repeated shutdown cannot reopen admission. The loop returns an
unconfirmed outcome when reaping cannot be established in that bound. Resource
release, Stopped and successful reaping are distinct facts. A complete telemetry
report or Stopped cannot override an unsuccessful process exit.

Tests simulate uncertain native operations, native error callbacks, truncation,
abnormal exit, a parked worker and teardown failure. They do not observe a
physical GPU loss or a hung driver. Existing R2 transport-abandonment,
unwind-safe-release and interrupted-teardown tests remain unchanged.

## Oracle and exact frames

`oracle.rs` compiles the accepted CPU producer source verbatim using the same
presentation API, independently of worker pixels and shader output:

`experiments/desktop-probe/src/presentation/synthetic.rs`

SHA-256: `9bcca46707113b45f897e2ec688e4ff2467efb14d4416335a8d5729238ebb099`.

The five samples remain **0, 300, 600, 900 and 1200 ms**, repeating at 1500 ms.
Each exact run issues five warmup sweeps and twenty measured sweeps: 25 warmup
and 100 measured frames. All 125 frames undergo exact validation with zero
permitted mismatches. The protocol fixes 160 × 160, tightly packed top-left PMA
BGRA8 and 102,400 bytes. The adapter checks the exact retained request/job,
derives observed bounds, anchor and canonical alpha spans from returned pixels,
and compares bytes and metadata to the independent CPU producer. The wire does
not supply separate metadata; observed metadata is reconstructed, not presented
as worker-supplied metadata. Byte equality also establishes exact alpha-mask
equality. Corruption tests reject channel/row changes, alpha/PMA, length, identity
and canonical hit geometry.

Optional telemetry is required to assert the resource/counter gates: 125
submission attempts, 125 confirmed completions, 125 mapping requests, callbacks, successful maps,
unmaps and timed operations, one
staging allocation, 124 reuses, zero retirements and peak outstanding one. An exact
run without telemetry validates frames and lifecycle and explicitly leaves those
resource metrics unverified.

## Workload freshness and latency reference

The acceptance CLI selects a fixed policy from the explicit workload. Only
`exact` selects **1,000 ms** freshness. `stress`, `cancel`, `cancel-before`,
`unsupported` and `init-failure` retain the supervisor's accepted **100 ms**
default. Initialization (5 s), escalation (2,500 ms), graceful shutdown (500 ms)
and exit observation (500 ms) remain identical to `Policy::default()` for every
workload. There is no deadline option, adaptive allowance or automatic retry.
The single selected policy is passed to `Adapter::spawn`: the supervisor receives
it unchanged and D1 uses its same freshness field. The output prints the workload,
effective budget, both authorities, full policy and purpose before spawning.

The exact allowance is bounded **correctness verification eligibility**, not a
native real-time performance requirement. Every one of the 25 warmup and 100
measured frames must still pass the independent CPU oracle and current eligibility
checks. Equality with the selected deadline expires eligibility. Obsolete results
can settle physical work but fail exact acceptance. The first failed warmup or
measured frame stops the run; none is discarded or retried. No additional native
prewarming is performed.

Exact output separately reports warmup and measured IPC/end-to-end median/p95/p99.
Those distributions contain accepted, validated frames only. The original
**100 ms** reference remains visible in separate warmup, measured and total
end-to-end latency violation counts. A frame observation at or above 100 ms is a
violation, consistent with expiry at equality. These counts include correlated
obsolete frame settlements if an exact run expires; such a frame never enters
the accepted counts or latency distributions. Partial summaries are printed on
failure as well as success. A reference violation does not establish a pixel
mismatch and does not by itself fail the longer exact eligibility policy. Passing
exact parity does not assert real-time performance. Worker stage totals still
include all operations and remain separate from these owner-side cohorts.

The owner-authorized R1 N1-A observation at
`.private-reviews/A01-C1-C-B2-D2-C-N1/run.ibS7aAaL/` initialized Mesa llvmpipe
26.3.1, submitted and physically settled one frame, mapped/unmapped, released
resources and reaped the child successfully. The first frame was obsolete under
the then-default 100 ms budget: Vulkan wait alone was 127.485213 ms. The exact
workload correctly failed; no pixel mismatch was established. R2 preserves that
failed evidence unchanged. It does not retroactively accept that run.

## Optional telemetry

Telemetry is disabled by default. `--telemetry` creates a fresh private run
directory (mode 0700, 128-bit OS random suffix, at most 32 exclusive name attempts)
and passes only the literal `--acceptance-report PATH` argument. The supervisor's
environment policy and version 1 wire protocol are unchanged. The worker uses
an owned directory descriptor and exclusive `openat`, `O_NOFOLLOW` and mode 0600.
It rejects nonabsolute paths, parent traversal, noncanonical/symlink parents,
nonprivate/foreign-owned parents, other filenames and preexisting destinations.
The required basename is `worker.telemetry`; unrelated files are never replaced.

D2-C-P does not supply a numerical file cap or concrete serialization limits.
The following are explicit, reviewable **implementation parameters**, not new
native performance or settlement policies:

- One report file capped at **16,384 bytes**, with one initial header and one final
  bounded snapshot; no recurring per-frame file logging.
- UTF-8 identity values encoded as hexadecimal, at most 512 source bytes each.
- Independent parser: at most 40 fields, 64-byte keys and 1024-byte values; input
  reading is capped at 16,385 bytes to reject oversize files.
- At most 720 observations per harness timing series (one bounded run).

The initial `roomie_acceptance_report=1`, `complete=0` header survives abrupt exit.
The final snapshot ends in `complete=1` only after preceding writes and flush
succeed. Missing, truncated, oversized, malformed or I/O-failed reports are
explicit evidence errors; missing metrics are unknown, never zero. Completeness
describes a complete byte-level snapshot, not filesystem durability or GPU safety.
`telemetry_error=0` is required for a valid final snapshot. If reporting itself
fails, the worker keeps its original IPC/settlement/exit behavior; the harness
reports the missing/incomplete evidence error. Telemetry is never read by a
settlement predicate. Files remain available at the printed path for review.

Native identity includes adapter name, device class, backend and driver identity.
Initialization records device/setup, pipeline and total wall time. An
`initialization_complete=0` snapshot marks device/pipeline timings unavailable
after failed native setup; their default zeros are not measured timings. A
complete initialization is required for normal native adapter gates.

| Counter | Increment site / meaning |
| --- | --- |
| `submission_attempts` (`Telemetry.submissions`) | Immediately before the native/abstract submit call; uncertainty may follow. |
| `accepted_submissions` | Quiet successful wait for the exact opaque submission index. |
| `confirmed_completions` | Same wait's independent completion fact; an attempted submit alone cannot increment it. |
| `native_gpu_completions` | Confirmed completions only for a native report; explicitly zero in synthetic reports. |
| `map_requests` | Immediately before requesting the tagged map callback. |
| `map_callbacks` | Matching callback observed, success or failure. |
| `successful_maps` | Matching successful mapping status observed, independent of later frame acceptance. |
| `unmaps` | Only after the accepted checked unmap call succeeds. |
| staging allocation/reuse/retirement and peak | Existing engine ownership sites; no reuse/replacement policy changes. |
| `timed_operations` | Returned render operations, including returned failures. Panics leave incomplete reports. |

Encode, submit, wait, map and repack fields are cumulative **host wall-time sums**
over all timed operations, including warmup and returned failures. They are not
GPU timestamp measurements, per-phase percentiles or measured-only statistics.
IPC and request latency samples belong to the harness and are reported separately.
Synthetic reports identify the CPU operations fixture and have zero native GPU
completions. Exit/reaping, cancellations, obsolete results and unresolved identity
come from the owner-side report; the worker cannot certify its own reaping.

## Sustained request scheduling

The separate stress run has 120 warmup and 600 measured opportunities. Rational
deadlines are `floor(index * 1 second / 60)`, with measurement starting at exactly
2 seconds and ending at 12 seconds. Animation derives from owner-supplied active
elapsed time and retains the 300 ms fixture period.

On an overrun, the scheduler counts old opportunities as missed and emits at
most the latest due opportunity once. After a late attempt the next eligible wakeup
is the greater of its ideal deadline and the last actual attempt plus the next
rational 60 Hz interval. A wake just before the next boundary cannot produce two
adjacent catch-up attempts; missed slots remain missed. This enforces the approved
no-catch-up rule without changing the 120/600 opportunity register. It never
replays expired deadlines. Waiting
uses the supervisor's socket/process readiness and the next deadline; it does
not busy-poll. Measurement cutoff invalidates both authorities **before** consuming
a result observed at or after cutoff. Late settlement during shutdown is obsolete.

Warmup and measured segments separately report:

- `scheduled`: 120/600 logical opportunities.
- `attempts`: actual D1 request attempts; partitioned into admitted, busy and
  admission failures.
- `dispatched`: exact jobs completely written, distinct from admission.
- `missed`: historical deadlines superseded before a request attempt.
- `skipped`: opportunities canceled because the run could not continue.
- `valid`, genuinely settled `obsolete`, proven zero-byte local `withdrawn`,
  explicit `rejected`, `settled_failures`, and exact outstanding `unresolved` jobs.
  `failures` is the sum of explicit rejections and settled failures.
- `late_completed`: obsolete frame settlements observed at/after 12 seconds;
  local withdrawals do not count as completed work.
- `late`: attempted opportunities observed strictly after their ideal deadline;
  normal OS wakeup lateness can make this nonzero on every opportunity.

The per-cohort partitions are:

```text
scheduled = attempts + missed + skipped
attempts = admitted + busy + admission_failures
admitted = valid + obsolete + withdrawn + rejected + settled_failures + unresolved
```

A fixed 720-slot ledger correlates every admitted identity, excludes overlapping
admissions and rejects duplicate/unrelated terminal outcomes. Dispatch remains
separate, reconciled against the adapter's exact transmission counter. An explicit
supported-request rejection or failure fails correctness even if shutdown is clean.
Busy/missed opportunities and healthy slow execution do not themselves fail
correctness. Partial reports retain their error and unresolved identity.

Cohort totals follow the request's opportunity index, including terminal outcomes
after the pacing window. Observation throughput separately counts valid frames
observed in the fixed half-open **[2s,12s)** window, including a warmup request
completed in that interval. `window_valid_hz` uses valid frames; the separate
`window_settled_frame_hz` includes genuinely obsolete frame settlements in that
same window. Late teardown settlements and local withdrawals are excluded from
both rates. An aborted run also excludes settlements observed after its actual
pacing end from window throughput, while retaining their request-cohort outcomes. The denominator is the observed intersection with that window (10 s
for a complete run), printed as `window_denominator`. `pacing_elapsed` retains the
actual elapsed time including final wakeup overrun; `actual_measurement_elapsed`
retains actual elapsed time after the two-second warmup without clamping overrun.
Initialization and controlled-shutdown durations are printed independently and do
not enter either throughput denominator. Initialization failure has zero pacing
duration.

Cadence is the interval between actual measured-cohort request attempts; lateness
compares each attempt to its ideal deadline. IPC latency begins when the owner
observes Render completely written; end-to-end latency begins at admission.
Both end at owner observation, before oracle comparison or presentation.
Measured-cohort latency samples are distinct from observation-window throughput.
Median/p95/p99 use nearest rank. Worker encode/submit/wait/map/repack measurements
are whole-process host wall-time totals, including warmup and returned operations
during late settlement; their explicit denominator is `timed_operations`, not
600 opportunities or the measured request cohort. They are not native GPU
service-time measurements.

With telemetry enabled, stress requires a matching synthetic/native domain,
successful initialization, complete metrics and controlled worker release plus
supervisor-confirmed shutdown/reaping. Submission, map, callback, unmap and timed
operation totals must equal actual dispatched and genuinely settled frames
(`valid + obsolete` across both cohorts). A local withdrawal performs zero backend
operations. Staging allocation must persist at one with `operations - 1` reuses
(saturating at zero), zero retirements and peak outstanding zero/one according to
actual operations. Missing metrics remain unknown; disabled telemetry leaves
resource gates unverified. No report can settle a job.

The `init-failure` workload requires `--telemetry`; native N3 also requires an
explicit `--driver` manifest. Proof requires an initialization-correlated
`BackendFault`, the accepted native unavailable-adapter diagnostic, no admitted
job, reaping, a matching native report with `initialization_complete=0` and all
operation/allocation counters zero. Device/pipeline default zero timings remain
unavailable, rather than measured zero setup time. Complete release evidence must
agree with actual Stopped, exit and shutdown classification. An expected backend
initialization rejection may have supervisor-confirmed *shutdown*; its BackendFault
is retained and it never becomes successful initialization. Generic process,
protocol, transport or unrelated backend failures cannot prove N3. Uncertain
release remains failed evidence. Synthetic init rejection is explicitly simulated
and cannot prove native N3. Adversarial protocol peers in fixture tests are labeled
simulations and do not claim to implement native worker behavior.

The approved 60 Hz figure is a readiness target. No new native pass/fail tolerance,
lateness threshold or comparison to historical B1 timings is introduced. A
synthetic run completing this schedule is not native Vulkan performance acceptance.

## Proposed owner-native commands — NOT EXECUTED

Execute only after independent harness review and separate owner authorization.
From `/home/zzzne4/Desktop/Utils/ProjectVanilla`:

```sh
cargo build --manifest-path experiments/gpu-worker/Cargo.toml --release --locked --workspace --features native-vulkan
```

N1 requires an installed software Vulkan ICD. The earlier R1 N1-A observation
used Mesa llvmpipe 26.3.1; R2 has not executed Vulkan. These commands select CPU
Vulkan explicitly and fail if unavailable. Retain stdout, stderr, exit status
and the printed report:

```sh
experiments/gpu-worker/target/release/roomie-worker-acceptance --run --worker /home/zzzne4/Desktop/Utils/ProjectVanilla/experiments/gpu-worker/target/release/roomie-worker-vulkan --adapter software --workload exact --telemetry
experiments/gpu-worker/target/release/roomie-worker-acceptance --run --worker /home/zzzne4/Desktop/Utils/ProjectVanilla/experiments/gpu-worker/target/release/roomie-worker-vulkan --adapter software --workload cancel --telemetry
experiments/gpu-worker/target/release/roomie-worker-acceptance --run --worker /home/zzzne4/Desktop/Utils/ProjectVanilla/experiments/gpu-worker/target/release/roomie-worker-vulkan --adapter software --workload cancel-before --telemetry
experiments/gpu-worker/target/release/roomie-worker-acceptance --run --worker /home/zzzne4/Desktop/Utils/ProjectVanilla/experiments/gpu-worker/target/release/roomie-worker-vulkan --adapter software --workload stress --telemetry
```

N2 selects hardware explicitly. The recorded manifest path exists locally; no
device was accessed to verify it in D2-C-A:

```sh
experiments/gpu-worker/target/release/roomie-worker-acceptance --run --worker /home/zzzne4/Desktop/Utils/ProjectVanilla/experiments/gpu-worker/target/release/roomie-worker-vulkan --adapter hardware --driver /usr/share/vulkan/icd.d/nvidia_icd.json --workload exact --telemetry
experiments/gpu-worker/target/release/roomie-worker-acceptance --run --worker /home/zzzne4/Desktop/Utils/ProjectVanilla/experiments/gpu-worker/target/release/roomie-worker-vulkan --adapter hardware --driver /usr/share/vulkan/icd.d/nvidia_icd.json --workload cancel --telemetry
experiments/gpu-worker/target/release/roomie-worker-acceptance --run --worker /home/zzzne4/Desktop/Utils/ProjectVanilla/experiments/gpu-worker/target/release/roomie-worker-vulkan --adapter hardware --driver /usr/share/vulkan/icd.d/nvidia_icd.json --workload unsupported --telemetry
experiments/gpu-worker/target/release/roomie-worker-acceptance --run --worker /home/zzzne4/Desktop/Utils/ProjectVanilla/experiments/gpu-worker/target/release/roomie-worker-vulkan --adapter hardware --driver /usr/share/vulkan/icd.d/nvidia_icd.json --workload stress --telemetry
```

Each stress invocation must retain cohort partitions, fixed-window counts and
both actual elapsed and window-denominator durations, independent initialization/
shutdown durations, native domain and complete operation/resource reconciliation.
A 60 Hz shortfall alone is a performance observation; supported request rejection,
failed settlement or contradictory telemetry is correctness failure. No native
performance threshold is added. N3 must retain the correlated diagnostic and
unsuccessful initialization report; a generic startup error is insufficient.

Each exact invocation must print `budget_ms=1000`, a full otherwise-default policy,
25 warmup and 100 measured accepted frames, all five phase counts at 25, distinct
warmup/measured latency quantiles, and the original `latency_reference_ms=100`
with warmup, measured and total violation counts. Retain nonzero reference
violations as performance evidence even when parity passes. Cancellation and
stress invocations must print `budget_ms=100`. A new exact run is a separate
owner-authorized observation; preserve the failed R1 N1-A archive and do not
insert preliminary unmeasured native render work.

Repeat exact runs as separately authorized to retain independent controlled
shutdown/reaping evidence. These are explicit owner invocations, not harness
recovery. For N3, create a private, valid JSON manifest pointing at an absent
library (an absent manifest file would test launch validation, not native setup):

```sh
umask 077
roomie_bad_icd_dir=$(mktemp -d /tmp/roomie-unavailable-icd.XXXXXXXX)
printf '%s\n' '{"file_format_version":"1.0.0","ICD":{"library_path":"/nonexistent/desktoproomie-unavailable-vulkan.so","api_version":"1.3.0"}}' > "$roomie_bad_icd_dir/unavailable.json"
experiments/gpu-worker/target/release/roomie-worker-acceptance --run --worker /home/zzzne4/Desktop/Utils/ProjectVanilla/experiments/gpu-worker/target/release/roomie-worker-vulkan --adapter hardware --driver "$roomie_bad_icd_dir/unavailable.json" --workload init-failure --telemetry
```

All commands in this section are **NOT EXECUTED**. Source review, native evidence
review and owner acceptance remain separate gates. Production transport, X11
integration, compositor scanout and private character integration remain open.
