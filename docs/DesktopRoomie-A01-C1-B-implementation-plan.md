# DesktopRoomie A01-C1-B implementation plan

**Status** Local planning record. B1 is the next proposed coding assignment and requires separate owner authorization. Review each checkpoint before authorizing the next one.

The common contract describes the character’s presentation in client-local coordinates. Backend adapters own their coordinate systems, skeletons, animation repertoires, rendering and coverage construction. A future 3D adapter for Hina and a Spine adapter for Vanilla can make different choices in each of those areas. The public host owns dragging, positioning, lifecycle, visibility, layers and interaction policy. The plan builds on the [C1-A acceptance](A01-C1-A-ACCEPTANCE.md) and refines the C1-B portion of the [C1 architecture plan](DesktopRoomie-A01-C1-architecture-plan.md#5-c1-b--neutral-presentation-contract-and-synthetic-fixture-later-mission).

## Prior review findings

| Area | Assessment | Decision |
| --- | --- | --- |
| Ownership boundaries | Preserves one X11 owner, event loop, layer controller and pointer owner. | Accept |
| Backend-neutral contract | An associated payload type accommodates representations beyond CPU pixel buffers. | Accept with refinement |
| Procedural adapter | Preserves the exact fixture and shares hit coverage across consumers. | Accept |
| Committed extent | Identifies the existing 160×160 assumptions across the host. | Accept |
| Frame scheduling | Fits the existing event-loop priority and bounded batches. | Accept with refinement |
| Publication safety | Distinguishes X11 request completion from display acknowledgment. | Accept with refinement |
| External extension boundary | An internal factory leaves the separate-crate integration unresolved. | Refine for B3b |
| Implementation stages | Separate B3a and B3b checkpoints reduce integration risk. | Accept |

## Architecture refinements

### Frame production and publication

B1 defines `FrameProducer` with an associated payload type, frame identity, metadata, validated coverage and a small pure publication-state vocabulary. Finalize the publisher’s operational interface in B2/B3 against the synchronous X11 path. Tickets and polling depend on a concrete asynchronous publisher and transport decision.

### External entry point

B1 exposes the producer abstraction and its supporting data types for downstream crates. B3b demonstrates an entry point that lets a separate executable supply a presentation factory while using the public host. Only the admitted owner invokes the factory. Use a synthetic external-crate compile test if feasible. Native connection handles remain host-owned. Spine integration and transport selection have separate gates.

### Frame generations and scheduling

A single target epoch can track lifecycle or presentation-context invalidation. Ordinary root-position changes during dragging preserve the epoch when the client-local presentation remains valid. The one-step-per-turn limit applies to scheduled animation work. Click feedback and `Expose` remain event-driven.

### Procedural startup and publication failures

Preserve A00’s static startup sequence. The host installs the nonempty native SHAPE input region before issuing the map request and uploads and paints the initial procedural frame afterward. For the current renderer, a publication failure with uncertain or partial visible effects goes through the existing fatal cleanup path. A checked `CopyArea` establishes X11 request completion. Physical scanout requires its own evidence. Changing-coverage atomicity remains open for the separately authorized native transport experiment. [Window creation](../experiments/desktop-probe/src/x11/window.rs#L243-L250) · [Owner startup](../experiments/desktop-probe/src/lib.rs#L330-L358) · [Checked blit](../experiments/desktop-probe/src/x11/render.rs#L111-L130)

## Checkpoints

| Checkpoint | Deliverable |
| --- | --- |
| B1 — Pure contract and validation | Public frame types, capabilities, identity, hit-region validation and pure tests. |
| B2 — Procedural compatibility adapter | Exact premultiplied-alpha (PMA) pixel parity, palette behavior and compatibility with the existing renderer. |
| B3a — Committed geometry and input | One authoritative extent and canonical region across gesture, window, SHAPE, Show and Here/Recover code. |
| B3b — Host coordinator and scheduling | Owner-only factory, frame deadlines, commitment, lifecycle and publication integration. |
| B4 — Deterministic integration acceptance | Failure injection, event ordering, stale frames and complete A00 regression coverage. |

## Proposed B1 assignment

Paths in this section are relative to `experiments/desktop-probe/`.

| Subject | B1 scope |
| --- | --- |
| New modules | `src/presentation/mod.rs` and `src/presentation/region.rs` |
| Existing module | `src/lib.rs` for public registration of the pure interface |
| Shared geometry | Reuse the existing `Point`, `Rect` and `Size` types with checked validation. |
| Semantic intent | Define an explicit action vocabulary and supported action/facing combinations. Directional support is specific to each backend. |
| Metadata | Frame identity, extent, optional visible bounds and anchor, and canonical hit region. |
| Validation | Require nonzero extent, coordinate containment, sorted nonoverlapping spans, checked arithmetic and generation consistency. |
| Producer | Use an associated payload independent of the native renderer. |
| Publication | Pure transitions and outcomes for stale frames and failures. The operational publisher API belongs to the later integration checkpoints. |
| Unit tests | Cover malformed geometry, overflow, unsupported intents, stale generations and publication-state transitions. |

B1 is limited to pure Rust contracts and tests. Runtime parity with the accepted baseline is required. Native rendering, event-loop changes, resize, private backends, transport selection, new dependencies and CLI changes are outside B1’s scope.

### Verification for the future B1 implementation

Run the existing 760 tests plus the B1 tests, along with the locked build, formatting and all-target Clippy checks. Treat these as implementation evidence. Owner-native acceptance has a separate gate.

```sh
cargo build --locked --manifest-path experiments/desktop-probe/Cargo.toml
cargo fmt --manifest-path experiments/desktop-probe/Cargo.toml --check
cargo clippy --locked --manifest-path experiments/desktop-probe/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path experiments/desktop-probe/Cargo.toml
```

## Repository review notes

The plan fits the current source boundaries. The reviewed local `main` was at `d931862419428171600b2514520ab3738c509d4f`, matching the local `origin/main` ref. The three C1-A source blobs match the acceptance record. The parent architecture plan’s `791a0636e7336d1df91efc5cb069b3790277213c` is the historical C1-A starting point. Pin the current base explicitly in the future B1 assignment. [C1-A source identities](A01-C1-A-ACCEPTANCE.md#source-identities)

The following details should be explicit when the B1 API is specified. They are review findings for that assignment.

- **Public geometry.** `geometry` is private at the crate root. Re-exporting only `Point`, `Rect` and `Size` from `presentation` would give downstream callers a usable API while preserving the approved file scope. [Crate root](../experiments/desktop-probe/src/lib.rs#L3-L8) · [Geometry types](../experiments/desktop-probe/src/geometry.rs#L5-L78)
- **Canonical region.** Define span ordering, endpoint inclusion, the treatment of adjacent spans and the representation of empty coverage. Specify local-coordinate rules for optional bounds and anchors at the extent edges. `Size` has unsigned dimensions and `Point` and `Rect` have signed coordinates, so endpoint conversion needs an explicit overflow rule. [Geometry types](../experiments/desktop-probe/src/geometry.rs#L5-L78)
- **Identity and failure.** Define the frame ID’s scope and ordering within a target generation. Cover duplicate or out-of-order candidates within the same generation as well as older generations. A stale candidate rejected before native effects and a failed publication with uncertain native effects need distinct outcomes. Keep those outcomes pure in B1 and preserve the existing fatal handling when B3b integrates native publication. [Owner fatal handling](../experiments/desktop-probe/src/lib.rs#L384)
