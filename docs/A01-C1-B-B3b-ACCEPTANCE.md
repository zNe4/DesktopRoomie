# A01-C1-B-B3b acceptance

**TECHNICALLY ACCEPTED — OWNER APPROVED — 2026-11-23**

## Accepted implementation

B3b-R1 is an uncommitted implementation on `a01` against the B3a baseline `f164f3c4b9326f3b9c2a4f9f664cb01dc9404ea1`. The cumulative implementation patch has SHA-256 `708b370e5f38dffaf59e03b13babde22a4f882d13ca69746855666978b95a878`. The closeout reproduced the full-index binary patch for all six implementation and test paths, including the two new files. All 49 probe source and build files matched the retained verification manifest by SHA-256 and Git blob ID.

The host creates one persistent `ProceduralProducer` after owner admission. `BodyPresentation` coordinates frame preparation for the body lifetime, and `ProbeRuntime` owns timing, eligibility and event-loop scheduling. The producer supplies pixels and metadata. The host reuses B3a `FrameGeometry` and the B1 publication authority for frame identity, admission and commitment. Committed geometry continues to drive placement and input.

Active elapsed time accumulates while the host is eligible. Body unavailability, unsettled input, pending layer transitions, movement in flight and active owner control operations suspend timing. For a positive frame period, the host uses checked monotonic deadlines and caps the scheduled delta at one period. Successful presentation rearms from completion, and resumption starts a full new period. Missed periods produce no catch-up bursts.

The built-in procedural producer is nonperiodic. Startup and palette changes prepare frames on demand, and Expose repaints the committed pixmap. B3b-R1 corrected elapsed time for nonperiodic producers by tracking eligible time independently of the recurring deadline. Nonperiodic redraw requests now carry accumulated active elapsed time, excluding suspension, and zero delta. They arm no recurring deadline.

Before submitting native work, the host validates frame identity, fixed extent and coverage, native dimensions and initial SHAPE limits. It checks tightly packed BGRA8 byte length, premultiplication and exact agreement between nonzero alpha and the hit region. Checked upload and blit precede host commitment. Failure stops further frame production. If native publication remains unsettled, the host invalidates frame authority and attempts checked input fencing, preserving the original error for fatal handling and cleanup.

## Verification and review

| Evidence | Result |
| --- | --- |
| Codex-reported Cargo verification | All four checks passed on B3b-R1: formatting, locked build, locked all-target Clippy with warnings denied and locked all-target tests. |
| Codex-reported tests | **831 passed, zero failed.** All 809 B3a tests were preserved and 22 B3b tests were added. |
| Independent ChatGPT source review | **PASS.** The R1 correction for nonperiodic active elapsed time was resolved. |

These Cargo results apply to the accepted implementation patch. Closeout verification covered documentation and source identity.

## Owner-native acceptance

The owner reported **native PASS** for all six groups on the Arch Linux/X11/Openbox host. This record relies on the owner's group-level report. Independently retained native logs and per-scenario transcripts are unavailable.

| Native acceptance group | Owner-reported result |
| --- | --- |
| Startup and remap | PASS |
| Palettes and click-through | PASS |
| Gestures and focus | PASS |
| Visibility and recovery | PASS |
| Interrupted states | PASS |
| Idle and shutdown | PASS |

## Limits and review gate

- Timed producer behavior was exercised in synthetic tests. Live animation remains unverified.
- Checked X11 transport and host commitment leave compositor acknowledgment, physical scanout and atomic pixel/SHAPE updates unverified.
- Display-scaling variants, including mixed DPI, other window managers and Wayland remain unverified.

B3b technical acceptance is complete. This documentation closeout passed independent ChatGPT review on 2026-11-23. Git publication remains owner-controlled. B4, C1-C and transport selection require separate authorization.
