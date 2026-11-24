# A01-C1-C-B2-D2-A — experimental worker supervision

Uncommitted D2-A-R1 correction for independent ChatGPT source review. R1 adds dispatch lifecycle and response-order checks without changing version 1 wire bytes or package dependencies. There is no GPU execution, desktop event-loop integration, automatic restart or CPU fallback. `desktop-probe` and the frozen `gpu-readback` experiment are unchanged.

## Package boundary

This directory is an independent Cargo workspace with two packages:

```text
future desktop owner ──► roomie-worker-supervisor ──► roomie-worker-protocol
                                    └─────────────► libc =0.2.191
future GPU worker ─────────────────────────────────► roomie-worker-protocol

supervisor tests only ──► desktop-probe (existing x11rb dependency)
```

`protocol/` depends only on std and defines portable owned values, checked codecs and bounded I/O slots. `supervisor/` is Linux-only and owns pure policy plus the native child/socket/pidfd driver. Its `fake-worker` and `parent-fixture` binaries are synthetic verification fixtures. The desktop dependency is a **dev-dependency only**, used by `tests/d1_adapter.rs` to demonstrate explicit identity/request conversion and retention of the original request in D1. There is no production adapter, source extraction, dependency cycle or existing-package manifest change. Future owner and worker packages can independently depend on these crates.

The byte contract, message limits and settlement table are in [WIRE.md](WIRE.md). `Cargo.lock` pins the workspace, including test-only dependencies. There is no wgpu dependency or feature.

## State ownership

`Machine` owns one session, one admitted obligation or one completed report, dispatch/settlement facts for the current or most recently retired job, immutable admission-relative deadlines, a first-cause failure latch, and bounded diagnostic retention. `Supervisor` exclusively owns that machine, one `Child`, one parent socket endpoint, one optional pidfd, one decoder and one `Outbound` containing the sole transmitter. `outbound.rs` reconciles the queued command with positive-write progress and phase before any native write; its generic writer seam permits deterministic short-write/backpressure tests of that same path. It owns no child or independent session. No replacement process or reaper thread is spawned.

| Phase / independent fact | Meaning and progression |
| --- | --- |
| Starting | Initialize queued; only timely correlated Ready **after complete Initialize transmission** opens admission |
| Ready | No physical work; an unconsumed report still prevents another admission |
| Outstanding | Exactly one admitted Render; separate transmission facts distinguish unsent, partial and complete dispatch; cancellation alone cannot retire it |
| Closing | Admission closed and ready bytes revoked; explicitly reject an entirely unsent Render, or finish an existing Render prefix before Shutdown; no new Render begins transmission |
| TerminationRequested | **All protocol writes prohibited**, including queued control messages; one SIGKILL attempt requested; fixed exit-observation deadline active; errno exposed separately |
| ShutdownUnconfirmed | **All protocol writes prohibited**; retain child/pidfd and debt for later evidence/exit; no recurring observation timer |
| Exited | Exit status observed with `try_wait` and child reaped; this fact does not settle GPU work |
| `failure: Some(...)` | Independently terminal session failure, including after exit; no admission can resume |
| `unresolved: Some(JobKey)` | Unretired obligation key, including an unsent admission until explicit rejection; `job.transmission` and `job.state` distinguish dispatch from physical uncertainty |
| `stopped`, `reaped`, `termination_requested`, `shutdown` | Separate acknowledgment/process/outcome facts; no fabricated controlled shutdown |

A matching Frame, RejectedBeforeSubmission or JobFailed discharges the physical slot according to the worker contract **only after complete Render transmission**. Premature matching reports, including Unresolved, are protocol failures and do not discharge the admitted obligation. BackendFault retains independent same-session authority before Initialize or Render completes.

| `job.state` | Required fact / transition |
| --- | --- |
| AdmittedUnsent | Complete Render queued, with zero bytes written |
| PartiallyTransmitted | Positive write of a proper Render prefix; entire command not yet written |
| CompletelyTransmitted | Final Render byte written; worker settlement still owed |
| PhysicallySettled | Matching terminal worker assertion accepted after complete dispatch |
| RejectedBeforeDispatch | Local `Report::RejectedBeforeDispatch`: sole transmitter proves offset zero and discards the entire queued Render; no GPU submission is inferred or claimed |
| Unresolved | A dispatched job lacks settlement proof after a fault, explicit Unresolved or termination; retained `job.transmission` distinguishes Partial from Complete |

Entirely unsent work may be retired during Closing, termination or exit only by that explicit local rejection. The proof is its actual zero write offset, **never** a deadline, EOF, kill request or reaping. It cannot retire partially or completely transmitted work. The report is local and adds no wire tag. For admitted work that has reached escalation before rejection, termination still proceeds; the local rejection removes only the never-dispatched job obligation.

On graceful Closing, `shutdown()` reconciles an unsent Render immediately and queues Shutdown so write readiness is visible without waiting for a timer. A partial Render may finish its existing prefix; Shutdown is queued only behind its final byte. Partial writes and backpressure never reset the original close/escalation deadlines. If either cutoff requests termination first, the remaining prefix and all later protocol output are left unwritten; no Shutdown is spliced into an incomplete Render. Initialize/control framing may finish during Closing, but all command kinds stop when termination begins.

Even an obsolete Frame settles that exact job, while its pixels are discarded. An Unresolved report makes the session permanently non-reusable. A later terminal job report can discharge its obligation without restoring admission. Stopped cannot retire an outstanding job. Failure and settlement facts survive child reaping in the final outcome, whose session field scopes every unresolved job key.

The first causal failure is retained. A correlated BackendFault overrides ordinary failures and always revokes affected ready bytes; invalid owner time has highest priority. Wrong-session faults are correlation failures, never device faults attributed to the current backend. Diagnostics retain the first bounded worker string, independently of failure classification.

## Owner progression and precedence

The owner supplies monotonic `Duration` values from one origin. Decreasing time fails closed; deadline arithmetic saturates instead of wrapping. The native driver does not read a clock, sleep, wait synchronously for exit or join a thread.

1. Service a bounded nonblocking read batch before consuming output or writing. Readable data is drained even with hangup/error readiness. Reads stop at protocol boundaries, so one write need not equal one read.
2. Ready requires a completely transmitted Initialize; matching job reports require a completely transmitted Render. Queuing or writing only a prefix is insufficient. A valid post-dispatch terminal report settles its job, including during termination when writes are forbidden. Frame eligibility uses **owner observation time < original freshness deadline**; equality is obsolete, even if a timer has not fired. Faults revoke a ready frame in either wire order. Invalid/duplicate/mismatched messages close the session.
3. Only after the batch is drained does `try_wait` observe exit. An exit racing the first WouldBlock triggers another bounded drain before exit classification, preserving pending terminal reports. Each drain has a 64-syscall budget; at most two drains occur per progression. If exhausted, `continue_immediately` requests another active turn and `take_report` is withheld. Partial trailing messages also withhold reports. No packet can prove absence of a future asynchronous fault; consumption belongs immediately after a drained progression in the same owner turn.
4. Process exit and channel failure revoke ready bytes. Complete matching terminal job assertions already observed remain settlement evidence. Malformed/truncated messages never supply it.
5. Apply deadlines after the observed message/exit evidence. Thus a matching terminal report observed at the escalation boundary can settle before escalation; its stale pixels cannot become fresh. Ready at or after the initialization cutoff does not open admission. Existing causal failures remain primary over later EOF/reaping consequences; a same-session BackendFault has the override described above.
6. After applying deadlines, explicitly reject any entirely unsent Render if admission has closed. In graceful Closing, finish an existing prefix and queue Shutdown once behind it; never start a new Render. In TerminationRequested, ShutdownUnconfirmed or Exited, suppress **all protocol writes and writable interest**, regardless of queued bytes or supplied writable readiness. Continue bounded reads for valid post-dispatch settlement and exit evidence. Perform at most one allowed nonblocking write per progression and one explicit kill attempt. No activity or repeated close renews a timer.

The owner must use `interest()` to wait for socket readability, conditional writability, pidfd exit readiness and the earliest active deadline; honor `continue_immediately` and call progression before consuming a report. Idle Ready has no write interest and **no recurring deadline**. The finite `poll_ready(max_wait, now)` convenience method supports headless harnesses; its maximum wait is supplied by the caller, not an internal idle polling policy. The future X11 owner must integrate these interests separately; this mission does not do so.

## Experimental deadlines

All defaults are configurable through `Policy`; they are supervisor policy, not kernel/process-creation/driver execution guarantees.

| Policy | Default | Fixed origin and effect |
| --- | --- | --- |
| Initialization | 5 seconds | Supplied spawn-admission time, before synchronous spawn; expiry closes and starts graceful shutdown |
| Frame freshness | 100 ms | Job admission; expiry cancels result eligibility, leaving settlement debt and escalation active |
| Outstanding-job escalation | 2.5 seconds | Original job admission; if still unretired (unsent, partial or complete), session fails and termination is requested; only explicit zero-byte rejection can then retire unsent work |
| Graceful shutdown | 500 ms | First close, including an explicit Unresolved report or terminal fault; expiry requests termination |
| Post-termination exit observation | 500 ms | First termination request; expiry records ShutdownUnconfirmed and stops recurring deadline wakeups |

While Closing with outstanding work, the earlier of its original escalation and the graceful-close cutoff wins. An explicit Unresolved report closes immediately and normally reaches the graceful cutoff before its original escalation cutoff. Silent overdue work remains outstanding after freshness expiry until original escalation. Controlled Stopped without exit still has the original graceful cutoff. Repeated shutdown calls do not renew either cutoff. The future B1-derived worker retains its separate two-second controlled GPU wait budget; no such GPU wait runs here.

## Linux process policy and cleanup

The executable path is canonicalized and passed directly to `Command`, with literal argument strings and a cleared environment. Only the private child socket becomes fd 0; fd 1/2 are `/dev/null`. The socket pair is initially CLOEXEC. In an async-signal-safe `pre_exec` hook, `PR_SET_PDEATHSIG(SIGKILL)` is installed, `getppid` is compared to the captured parent PID to close the process-parent fork/prctl initialization race, and Linux `close_range(..., CLOSE_RANGE_CLOEXEC)` marks every fd ≥3 for closing on exec, including unrelated descriptors originally lacking CLOEXEC. The exec-error pipe retains its proper close-on-exec behavior. There is no inherited owner X11 connection after exec.

Linux parent-death signaling follows the **spawning parent thread's** lifetime. `Supervisor` is deliberately !Send/!Sync, and the owner must keep that thread alive through supervision and exit. The PPID check identifies the parent process; it does not replace that thread-lifetime requirement or cover arbitrary thread cancellation during spawn. An ordinary non-setuid Rust worker preserves this policy through exec. Privileged exec/credential changes, driver/kernel uninterruptible states, machine failure, deliberate worker policy changes, descendant process trees and universal orphan prevention are outside the claim. The ordinary fake worker exits on control EOF; the parent-death fixture deliberately waits on EOF so the tests specifically prove SIGKILL delivery instead of accidentally passing through cooperative EOF exit.

The driver uses Linux pidfds for event-driven exit observation and `Child::try_wait` for reaping. `close_range` support is required at spawn. If `pidfd_open` fails after spawning, the returned supervisor retains ownership, latches ProcessObservation failure and closes/terminates under the same finite policy instead of abandoning the child. In that exceptional unsupported state, a later unconfirmed exit has no pidfd readiness and needs explicit owner progression; no recurring polling fallback is installed.

A kill request's success is not exit, settlement or cleanup proof. Keep an unconfirmed supervisor alive and service its pidfd to reap a later exit. Drop makes only a best-effort kill if not already attempted and a single nonblocking `try_wait`; it cannot certify shutdown or guarantee reaping a process that has not exited. The explicit lifecycle is required. The final `exit_status()` and termination errno are inspectable separately from settlement debt.

## Verification and review boundary

Run from the repository root:

```sh
cargo fmt --manifest-path experiments/gpu-worker/Cargo.toml --all -- --check
cargo build --manifest-path experiments/gpu-worker/Cargo.toml --workspace --all-targets --locked
cargo clippy --manifest-path experiments/gpu-worker/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo test --manifest-path experiments/gpu-worker/Cargo.toml --workspace --all-targets --locked
```

Protocol, policy, outgoing-stream short-write/backpressure and D1 adapter tests are deterministic. The existing D1 adapter/pure tests now explicitly supply full-dispatch facts; they do not infer transmission from admission. The preserved native error-readiness test fully dispatches a silent job before injecting its transport failure, so it continues to verify actual settlement debt rather than the newly authorized zero-byte rejection exception. `tests/subprocess.rs` uses actual Linux child processes, socket pairs, pidfds and bounded host-clock waits without GPU/display access; its error-readiness case explicitly injects the readiness error into the real driver. Parent-death tests use disposable parents and temporarily make only the test process a child subreaper so it can reap its test orphans. R1 also exercises duplicate completions, malformed headers, oversized declarations, premature Ready, a fault before Initialize dispatch and graceful/escalated rejection of an unsent Render over actual Unix IPC. Readiness barriers and injected monotonic time establish native ordering; deterministic writers establish partial dispatch without depending on unreliable small-message socket congestion. They verify ordinary parent exit and forced SIGKILL; the tiny pre-prctl race is covered by the source guard, not a claimed exhaustively exercised interleaving.

Tests do not establish real GPU settlement, desktop responsiveness under GPU/driver faults, display correctness, SHAPE acceptance, compositor/scanout timing or production transport suitability. No new desktop manual test is required for this unwired infrastructure. Independent ChatGPT source review is the next gate. B2-D2-B, B2-D2-C and B2-D3 remain unauthorized.
