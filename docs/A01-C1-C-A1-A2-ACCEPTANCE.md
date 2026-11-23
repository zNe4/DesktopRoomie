# A01-C1-C-A1/A2 acceptance

**2026-11-23.** A1 independent source review passed. The owner reported **6/6 PASS** for the A2 native experiment. Independent ChatGPT A3 documentation review passed on 2026-11-23.

## Implementation identity and scope

The reviewed A1 implementation is the uncommitted source patch on `a01` against published baseline `d54508a45351e58af7fd6ded54cb2e8f1c72d99c`. Its full-index binary patch SHA-256 is `c895bf68d264c58d7c13ca34af84c9182471d8b3cb1010eacee3965c5652cb38`. The patch includes eight modified source files and two new source files. A3 reconstructed that exact patch from the working tree and verified that its digest matched the reviewed digest.

A1 adds an opt-in owner launch with `--synthetic-animation`. Its CPU fixture loops through five fixed-canvas 160×160 premultiplied BGRA8 frames, with a nominal 300 ms period on the eligible-time clock. The fixture derives hit coverage from nonzero alpha and includes a transparent hole, translucent pixels, disjoint coverage and a fully transparent phase. The default launch remains the nonperiodic procedural Lavender/Coral body.

For a frame with changed coverage, the host preflights the candidate, blocks logical body input and uploads to the backing pixmap. It then checks empty input SHAPE and pointer release, blits and flushes the image, and checks installation of the exact candidate SHAPE before committing the frame. The runtime installs the full enabling sequence and unblocks dispatch after timing succeeds. Equal coverage uses the checked image-only path. Expose repaints the committed pixmap. A failed or uncertain native publication, or a timing error after commit, revokes usable frame authority and invokes the existing safety fence and terminal path.

## Automated verification and source review

Codex reported all four A1 checks passing from `experiments/desktop-probe/`.

| Check | Reported result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo build --locked` | PASS |
| `cargo clippy --locked --all-targets -- -D warnings` | PASS |
| `cargo test --locked --all-targets` | PASS, **851 tests passed** |

The A1 test inventory preserved all **839 baseline test identities** by Cargo target and test name and added 12 tests. The additions cover owner-only CLI opt-in, producer selection, scheduled playback and suspension, checked cutover ordering, candidate preflight, failures and stale input, and startup/deferred restoration with Expose kept independent. Failure and event-ordering tests use deterministic seams. Independent ChatGPT source review passed for the patch identified above. A3 checked source identity and documentation without repeating Cargo or native tests.

## Owner-native verification

The owner reported **6/6 PASS** on **Arch Linux / X11 / Openbox / Picom**. This is the owner's report on the reviewed A1 implementation. The supplied evidence gives the aggregate result and this specific observation.

- **A2-N3.** Animation paused during grabbing/dragging and menu interaction, then resumed when eligible. The pause is expected. The owner observed that the underlying `xev` receiver received clicks only in transparent regions, with no unexpected clicks through visibly occupied pixels.

## Limitations and review gate

- X11 pixel and input SHAPE updates are not atomic. During the empty-SHAPE interval, an underlying application can receive a click while old pixels remain visible. Checked X11 requests establish host-side completion. Compositor presentation and physical scanout remain unverified.
- The absence of observed unexpected clicks does not prove their impossibility. The A2 result describes the exercised native session.
- Precise native cutover timings, including the duration of the empty-SHAPE interval, and sustained GPU animation performance were not measured. The fixture period is a scheduling parameter.
- Production acceptance of the empty-SHAPE cutover remains a separate decision. GPU transport selection and private Vanilla integration are outside this experiment.

Independent ChatGPT A3 documentation review passed on 2026-11-23. Git publication remains owner-controlled. The implementation and closeout remain local and uncommitted.
