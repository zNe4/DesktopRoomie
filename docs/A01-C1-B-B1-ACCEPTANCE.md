# A01-C1-B-B1 — Backend-neutral presentation contract

**TECHNICALLY ACCEPTED — OWNER APPROVED — 2026-11-22**

## Source identity

The pinned public baseline is `d931862419428171600b2514520ab3738c509d4f`. Local `main` and the local `origin/main` tracking ref both pointed to that commit during F1 verification. The B1-R1 implementation was verified as a local patch against that baseline.

Cumulative B1-R1 patch SHA-256

`f377ce4f84c1bb6e9519a1a9b8e1b4d1dcfde874f71dfab214d25f8d32df1a48`

The patch applied to a fresh Git archive of the pinned baseline and reproduced all four implementation files byte for byte. Paths below are relative to `experiments/desktop-probe/`.

| Path | SHA-256 | Git blob SHA-1 |
| --- | --- | --- |
| `src/lib.rs` | `fb907ae50d61a77217fdbaf94a9833b17d199e60253f264385be5d51e5b78fc6` | `b60e67c5238b0f3114049c6163504196db38f268` |
| `src/presentation/mod.rs` | `c70bf9bbc8cb9404bc0f83a68de94357f9555fefc776a74d4c9fe947a5206598` | `836a99d46b3b675158d340d8874f9568feebbec1` |
| `src/presentation/region.rs` | `5d7a48767b856895ef3252e812d1dc0070fe29c45e0037bee8ddcfbc9bbd8a5e` | `2d7c3b8cb0f79ccec85678a627e5782f7a0753c7` |
| `tests/presentation_contract.rs` | `2d205716b88a8d6dd2ea06b21133fa742c0265c30f2581ebb3f3c328d9817fff` | `4d27af93382565717141d1c715d9f4a1c4422413` |

## Contract and geometry

B1 adds a public, pure Rust `presentation` module for frame production and publication validation for one client target. The existing X11 runtime continues to use its procedural presentation path. The host owns timing, lifecycle, input policy, native limits and validation that coverage matches rendered content.

- `ClientExtent` requires positive dimensions with exclusive right and bottom edges representable as signed 32-bit coordinates. Geometry uses client-local pixels with a top-left origin and Y increasing downward.
- `HitRegion` validates row spans in ascending `(y, start_x)` order with half-open horizontal coverage `[start_x, end_x)`. It rejects unsorted, overlapping, empty and out-of-bounds spans. It coalesces adjacent spans on the same row into maximal spans, preserving holes and row boundaries. An empty region covers no pixels.
- `FrameMetadata` requires the hit region to share the frame extent. Optional visible bounds must have checked edges, a nonempty area contained by the extent and cover all hit spans. An optional anchor must identify a client pixel and may lie outside the visible bounds.

## Identity and production

The host issues `FrameIdentity` values scoped to one target lifetime. Sequences increase strictly within a generation and stop at exhaustion. Invalidation advances the generation with checked arithmetic, and generation exhaustion permanently closes admission and commitment. Ordinary root-position dragging preserves the generation while client-local presentation remains valid.

`PresentationCapabilities` exposes a validated initial extent, exact action/facing pairs and an optional positive frame period. An omitted period requests no recurring animation. `FrameRequest` carries identity, intent, extent, elapsed active time and the host-supplied capped delta, which must be no larger than elapsed active time. `FrameProducer` exposes capabilities and prepares a frame with the request identity and extent, validated metadata and an owned backend-specific payload. The producer defines its payload and error types, which are independent of the mutable borrow used for preparation.

## Publication and independent review findings

The normal transition sequence is Prepared → Submitted → TransportCompleted → HostCommitted. Admission checks request identity, extent, generation and sequence. Newer candidates may supersede prepared work. Obsolete completions and commits are rejected. Failure before visible mutation is legal from Prepared. Submitted and transport-completed work may fail once the host has settled the native work, with visible effects classified as either absent or possibly visible.

Independent ChatGPT source review raised two findings in the original B1. R1 resolved them as follows.

1. **A newer candidate could displace the last committed identity.** R1 retains `last_committed()` separately as history. `current_committed()` exposes that identity while it belongs to the current generation and visibility is settled. A prepared candidate or a settled failure before visible mutation alone leaves the prior commit eligible. An unsettled submission or a possibly visible failure suppresses current authority while preserving history. Possibly visible failure remains latched until a successful commit, which replaces the historical identity and clears the uncertainty.
2. **Supersession or invalidation could lose an outstanding submission.** R1 keeps a submitted or transport-completed candidate until commitment or a settled failure and rejects new admission while it is outstanding. An admission rejection leaves the state and sequence authority intact. Invalidation revokes commit authority while retaining the outstanding candidate. Only a settled failure for that exact stale identity can retire it. Late completion or commit cannot restore stale authority.

The tracker carries identities and state without frame payloads. Host commitment records successful host checks. Compositor acknowledgment, display scanout and atomic visual/input-region updates remain outside the contract.

## Automated verification and review status

Codex ran these commands locally from `experiments/desktop-probe/` and recorded their exit statuses independently.

| Command | Exit status | Result |
| --- | ---: | --- |
| `cargo fmt --all -- --check` | 0 | Passed |
| `cargo build --locked` | 0 | Passed |
| `cargo clippy --locked --all-targets -- -D warnings` | 0 | Passed |
| `cargo test --locked --all-targets` | 0 | 785 passed, 0 failed |

All 760 original library tests matched the saved baseline inventory by name and passed. The binary test target contained zero tests. All 25 B1 integration tests passed. Nothing was ignored, measured or filtered out. File checksums before and after verification matched for the source, Cargo manifest and lockfile. The F1 source identity report records the baseline, patch replay and source hashes. The separate complete Cargo verification log preserves the command output and exit statuses.

Independent ChatGPT review of the B1-R1 source passed. Independent ChatGPT review of the F1 documentation passed with two editorial corrections incorporated in this record. Codex verified patch identity and local reproducibility and executed the Cargo checks. The owner approved B1 technical acceptance on November 22, 2026. The source was uncommitted at acceptance. Git publication is a separate owner-controlled action that leaves the technical acceptance evidence intact. Native X11 and GPU tests were unperformed and are outside the required evidence for this pure checkpoint.

## Remaining C1-B work

- **B2** — Procedural compatibility adapter with exact premultiplied-alpha pixel and palette parity.
- **B3a** — Committed geometry and input, using one authoritative extent and canonical region across gestures, window geometry, SHAPE, Show and Here/Recover.
- **B3b** — Host coordinator and scheduling, including owner-only factory invocation, frame deadlines, lifecycle and publication integration.
- **B4** — Deterministic integration acceptance covering failure injection, event ordering, stale frames and A00 regressions.

Vulkan/X11 transport selection and private renderer integration remain unauthorized. Each remaining checkpoint requires separate authorization.
