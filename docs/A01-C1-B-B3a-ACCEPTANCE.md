# A01-C1-B-B3a acceptance

The owner technically accepted B3a committed geometry and input on 2026-11-22 after independent ChatGPT source review and owner-native acceptance on Arch Linux/X11/Openbox.

## Accepted implementation

B3a was uncommitted on `a01` at acceptance, based on B2 commit `4fe0ba519607626324be64210cae446b0dee9b92`. The accepted implementation patch has SHA-256 `9d41e1802581da3d7852a16b7751be3c0d26d34943d7f96d581e0d02257d01af`. The full-index binary diff against that base under `experiments/desktop-probe/` matches the retained implementation patch byte for byte.

The managed X11 host starts with a fixed 160×160 client extent and empty input SHAPE. After initial publication, committed frame metadata supplies the extent for placement and drag bounds and the canonical client-local hit region for gesture admission and completion. The host restores input SHAPE from that region when placement and input obligations are settled. Show and Here/Recover use the same committed geometry. Lavender and Coral share the same extent and hit region.

## Verification and review

| Evidence | Result |
| --- | --- |
| Codex-reported locked Cargo verification | **809 passed, zero failed.** All 795 original tests were preserved and 14 were added. |
| Independent ChatGPT source review | **PASS**, with no blocking findings. |

The Cargo result belongs to the accepted implementation patch. This closeout adds documentation verification only.

## Owner-native acceptance

The owner reported **PASS** for all six groups on Arch Linux/X11/Openbox. This table records the owner's group-level results. Independently retained native logs or per-scenario transcripts are unavailable for this record.

| Native acceptance group | Owner-reported result |
| --- | --- |
| Startup and remap | PASS |
| Both palettes (Lavender and Coral) | PASS |
| Gestures and focus | PASS |
| Visibility controls | PASS |
| Cross-workspace recovery | PASS |
| Shutdown and relaunch | PASS |

## Limits and review gate

- The checked X11 upload and blit establish completion at the X11 transport boundary. Compositor acknowledgment and physical scanout remain unverified.
- Atomic visual/input updates for changing extent or hit coverage remain unverified. B3a uses a fixed extent and rejects coverage changes during publication.
- Display-scaling variants, including mixed DPI, other window managers and Wayland remain outside the verified host scope.

B3a technical acceptance is complete. Independent ChatGPT review of this documentation closeout passed on 2026-11-22. Publication and any work on B3b, B4, C1-C or transport selection require separate owner authorization.
