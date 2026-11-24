# D2-A experimental wire format, version 1

This is a synthetic frame/control protocol, not a production GPU pixel-transport decision. Both endpoints use a private Unix stream socket pair. The parent owns one endpoint; the child's unique endpoint is fd 0, used bidirectionally. There is no named socket, discovery service, shared memory, X11 connection, GPU handle or pointer in this protocol. stdout and stderr go to `/dev/null`; human-readable streams are never parsed as binary traffic or buffered by the supervisor. Diagnostic strings inside typed control reports have the separate bounds below.

All integers use **little-endian** byte order, without padding. Every message starts with this 24-byte header:

| Offset | Bytes | Field |
| --- | --- | --- |
| 0 | 4 | ASCII `DRWK` |
| 4 | 2 | Protocol version, exactly 1 |
| 6 | 2 | Message tag, 1–10 |
| 8 | 4 | Body length, excluding header |
| 12 | 4 | Reserved, exactly zero |
| 16 | 8 | Owner-issued backend session identity |

A session identity must be fresh wherever reports from an earlier session might still exist. Zero is legal. Every job body starts with a 32-byte `JobKey`: four consecutive u64 values, **eligibility epoch, job serial, frame generation, frame sequence**. Serial must be nonzero. Together with the header's session, this is the complete D1 job identity; every field must match. The supervisor requires strictly increasing admitted serials within its one session. Epoch, generation and sequence authority remain with the desktop owner; the supervisor does not invent them.

A diagnostic is a u16 UTF-8 byte count followed by exactly that many bytes, **0–512 bytes**. Invalid UTF-8, an inconsistent count, oversized diagnostics, unknown enums, unexpected trailing bytes and noncanonical durations are rejected.

| Tag | Message / direction | Body, in order | Exact or maximum total bytes |
| --- | --- | --- | --- |
| 1 | Initialize, parent → worker | u16 width=160, u16 height=160, u8 format=1 | 29 |
| 2 | Ready, worker → parent | Empty | 24 |
| 3 | Render, parent → worker | JobKey; u16 width=160; u16 height=160; u8 action; u8 facing; elapsed; delta | 86 |
| 4 | Frame, worker → parent | JobKey; u32 pixel byte count=102400; exactly 102400 pixel bytes | 102460 |
| 5 | RejectedBeforeSubmission, worker → parent | JobKey; diagnostic | ≤570 |
| 6 | JobFailed, worker → parent | JobKey; diagnostic | ≤570 |
| 7 | Unresolved, worker → parent | JobKey; diagnostic | ≤570 |
| 8 | BackendFault, worker → parent | Diagnostic; session remains in header | ≤538 |
| 9 | Shutdown, parent → worker | Empty | 24 |
| 10 | Stopped, worker → parent | Empty | 24 |

Format 1 means top-left row order, tightly packed premultiplied BGRA8, 160×160, 640 bytes per row, **102400 bytes total**. The codec validates extent/format/length, not pixel provenance or premultiplied channel values. A future presentation adapter must perform the existing payload/metadata validation before publication. No SHAPE, hit spans or presentation commitment travel over this experimental protocol.

Action values are Idle=0, Walk=1. Facing values are Neutral=0, FrontRight=1, FrontLeft=2, BackRight=3, BackLeft=4. Each active-time duration is u64 seconds followed by u32 nanoseconds, with nanoseconds <1000000000. Delta must not exceed elapsed. These are request-relative active times, not wall-clock timestamps or serialized native `Instant` values. Supervisor deadlines never travel on the wire. The original authoritative `FrameRequest` remains with the owner.

The maximum body is **102436 bytes**; the maximum complete message is **102460 bytes**. Per-tag length is checked as soon as all 24 header bytes arrive, before any claimed body is accepted. The decoder reserves one fixed maximum message buffer, accepts only up to the current header/body boundary, and requires consumption before receiving another complete message. It has no unbounded message queue; malformed input poisons the decoder. Its `feed` return count leaves following/coalesced bytes with the caller. EOF with buffered incomplete bytes is truncation.

The transmitter has one complete-message slot and one offset. Enqueuing while bytes remain returns Backpressure and preserves the partial message. Short writes advance only that offset; Interrupted and WouldBlock preserve bytes; a zero write is WriteZero. Supervisor-side read/write syscalls are nonblocking. The fake worker may block on its private channel; it never runs on the desktop owner thread.

## Response ordering and dispatch (D2-A-R1)

The wire version, tag set, sizes and encodings are unchanged. The parent records complete Initialize transmission only when its final byte is accepted by the native writer. Ready before that fact is a protocol-order failure and cannot establish initialization. Complete transmission is not proof that the worker has read the command, submitted work or settled GPU operations.

Each Render is admitted as entirely unsent. Its sole outgoing slot records positive short writes and the final byte separately. Frame, RejectedBeforeSubmission, JobFailed and Unresolved require both the complete job identity and **complete transmission of that Render**. A premature matching report is a protocol failure and cannot discharge the slot. A same-session BackendFault is independently authoritative regardless of initialization or job-dispatch progress.

When admission closes, a Render with zero written bytes is withdrawn only with an explicit local `RejectedBeforeDispatch` report backed by that zero-offset proof. This local report is not a wire message or a worker settlement assertion. It never applies to a partial or complete Render. During graceful Closing, an existing Render prefix may finish before Shutdown is appended; no new Render begins. The original escalation/close deadlines are fixed throughout. Once TerminationRequested or ShutdownUnconfirmed begins, **no further protocol writes of any kind** are allowed, even to complete a partial message. Its remaining bytes are not replaced with Shutdown. Reads can still accept valid terminal evidence for a completely transmitted job, and process exit remains independently observed.

## Settlement contract

This protocol trusts a worker's assertions only after the response-order checks above, not the existence of a packet alone. D2-A's fake worker has no GPU work and exercises those assertions synthetically. A future B1-derived worker must uphold the following contract before using these message tags:

| Report / event | Physical job settlement | Session/result consequence |
| --- | --- | --- |
| Local RejectedBeforeDispatch (not a wire tag) | Sole transmitter proves no Render byte was written and withdraws the entire queued message; no submitted physical work is claimed | Retires only an entirely unsent obligation; never relies on process/deadline events |
| Frame | Certifies GPU work and staging access settled; owned bytes independent of staging | Matching fresh, uncanceled result can become a candidate, subject to fault/drain barrier |
| RejectedBeforeSubmission | Certifies the job never submitted work and has no outstanding job resources | Releases job slot; retains a correlated rejection report |
| JobFailed | Certifies all job GPU/staging work settled, including map/view cleanup or cancellation after confirmed GPU completion | Releases job slot; retains a correlated failure report |
| Unresolved | Explicitly **does not** certify settlement | Preserves exact obligation, permanently closes admission, begins graceful close; a later matching terminal report may discharge debt but cannot reopen session |
| BackendFault | No job settlement assertion | Correlated current-session fault revokes ready bytes, closes session; matching terminal job evidence is still required separately |
| Stopped | Asserts controlled resource shutdown, only legal during closing/termination with no outstanding job | Shutdown confirmed only after this assertion **and** successful child exit/reaping, without forced termination |
| Freshness / escalation / shutdown deadlines | No settlement assertion | Revoke eligibility or advance process policy only; retiring unsent work additionally requires explicit zero-offset rejection |
| EOF, disconnection, malformed input, unexpected exit | No settlement assertion | Terminal session failure; retain any unresolved job identity |
| Successful kill request or child reaping | No settlement assertion | Process facts only; shutdown remains unconfirmed without the required controlled acknowledgment |

In particular, the B1 two-second controlled GPU wait and its uncertain-submission process-exit path must not be reclassified as JobFailed. Such a future worker owes Unresolved when it cannot prove settlement. D2-A changes none of B1's code or evidence.
