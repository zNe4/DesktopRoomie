# A01-C1-C-B2-D1 — Pure GPU-to-X11 handoff contract

**TECHNICALLY ACCEPTED — OWNER APPROVED — 2026-11-23.** Independent ChatGPT source review passed. No R1 correction was required.

## Reviewed implementation

D1 adds a pure owner-side coordinator for asynchronous GPU frame preparation under `experiments/desktop-probe/src/presentation/gpu_handoff.rs` and exports it from the existing presentation module. It preserves the original immutable `FrameRequest` and `FrameIdentity`, with a distinct checked job serial, eligibility epoch and owner-selected session identity. The coordinator has one exclusive slot containing either a physically outstanding job or a completed ready frame, never both. It rejects mismatched and duplicate reports, and prevents a canceled, timed-out or obsolete success from becoming ready for publication.

Logical invalidation does not settle physical GPU work. Timeout or unresolved completion revokes the frame's readiness but retains the outstanding slot until matching terminal settlement. Recoverable dispatch rejection, worker failure, device-wide failure and unresolved completion remain distinct outcomes. Device failure closes admission independently of frame freshness. Epoch and serial exhaustion close admission without wrapping. The owner supplies monotonic time and deadlines. No worker, GPU transport or native scheduling mechanism is added.

GPU readiness remains a candidate handoff. The existing host continues to validate and commit through `FrameGeometry` and its `Publication` tracker. D1 does not make `PublicationEvent::Submit`, `TransportComplete` or `Commit` represent GPU completion.

## Verification and source identity

The reviewed implementation is based on branch `a01` at pinned baseline `6ae028d7e482483eb326d4b598607e85d7dbabd9`. Its patch SHA-256 is `43ee2e8fc363f0a6b6f1c7bc67410c66108535b85644c534ee03abbe4f5ebb9f`. The exact patch was reconstructed and its SHA-256 verified for this closeout. All three implementation files match the reviewed SHA-256 values:

| File relative to `experiments/desktop-probe/` | SHA-256 |
| --- | --- |
| `src/presentation/mod.rs` | `938f7f666511d74f6fe26e1dd1ce9ab5d22f1b6214b0696ee7ae393970f1c8c4` |
| `src/presentation/gpu_handoff.rs` | `79af6f9f7e212a3d875888b88f4f6d28a05337e692762a846edde12134b2e56c` |
| `src/presentation/gpu_handoff/tests.rs` | `612613f549fda8362aa55711d06052f05c91b71636b49b1fd38d7ac90056e58e` |

The retained D1 implementation verification records all four required Cargo commands passing from `experiments/desktop-probe`:

| Command | Exit | Result |
| --- | ---: | --- |
| `cargo fmt --all -- --check` | 0 | PASS |
| `cargo build --locked` | 0 | PASS |
| `cargo clippy --locked --all-targets -- -D warnings` | 0 | PASS |
| `cargo test --locked --all-targets` | 0 | 867 passed, 0 failed |

The test inventory preserved all 851 baseline identities and added 16 deterministic handoff tests. The documentation-only closeout did not rerun Cargo or native checks. No native acceptance was required or performed for this unwired pure checkpoint.

## Binding integration requirements

The independent source review established these requirements for future integration:

1. Consume and validate ready frames within the same owner turn.
2. Require actual GPU settlement evidence before releasing resources or admitting replacement work.
3. Correlate backend-wide faults with the correct backend session and process them before consuming affected ready frames.
4. Define explicit escalation for unresolved work and explicit shutdown policy.
5. Preserve `FrameTiming` as the sole animation scheduling authority.

## Scope boundaries and next mission

**A01-C1-C-B2-D2-P — GPU Worker Execution and Failure Isolation Architecture Review** is pending as a separately authorized mission. Production transport selection, live GPU integration, production SHAPE acceptance and private Vanilla integration remain open. This acceptance records the pure D1 contract and does not resolve those decisions.
