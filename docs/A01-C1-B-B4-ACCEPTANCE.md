# A01-C1-B-B4 acceptance

**TECHNICALLY ACCEPTED — OWNER APPROVED — 2026-11-23**

## Accepted implementation

B4-I1 is an uncommitted test-only patch on `a01` against the published B3b baseline `41bc4f8d8779737a7ba3c1381577e2edd750de88`. The retained full-index binary patch, `A01-C1-B-B4-I1.patch`, has SHA-256 `8d588e4678782f12c81242fc59958bf78272a21ebbb210781b1feef543c8e6e1`. Patch replay against that baseline reproduced all 50 probe source and Cargo files byte for byte. Closeout verified the same patch and all 50 files against the retained source manifest by SHA-256 and Git blob ID.

| Path under `experiments/desktop-probe/` | SHA-256 | Git blob ID |
| --- | --- | --- |
| `src/probe_tests/here_service_tests.rs` | `0cf30f4865fca0750c22ac68f7b9a15a2d96a753b2973620b4fdcd13cb57feb8` | `08e052fa6f5fcbe7d63dc64d1d2b1ad8cdb3f173` |
| `src/probe_tests/presentation_runtime_tests.rs` | `2b332066cd9c7d2580869a728413c169399506e8e330f634643c2b4989553e60` | `10bbaca58a737b0248ab4cfbfd3c96ce01348a74` |
| `src/x11/shape.rs` | `f6a6c200f21d32aee61c82bdc466cc1f0992e2a86024da3f21e6b1d578b1aefe` | `15111c631249d46816f0745229b2a958147339c5` |

Seven tests exercise presentation and input through runtime seams registered under `x11::shape` with `#[cfg(test)]`. The eighth exercises Expose failure during Recover, preserving confirmed relocation and restoration history and limiting terminal reply handling to one attempt. B4-I1 preserves production behavior and the B3b source ownership boundaries.

The deterministic coverage includes procedural pixel and input SHAPE agreement, committed-frame Expose repaint, injected presentation and cleanup failures, rejection of stale and unsettled frames, post-map SHAPE failure, and Unmap ordering at the event-batch boundary. A source-order assertion checks Expose routing and buffered-event handling before frame waiting in the native loop. The timing checks use a synthetic producer.

## Verification and review

| Evidence | Result |
| --- | --- |
| Independent ChatGPT source review | **PASS**, with no blocking findings. |
| Codex-reported formatting (`cargo fmt --all -- --check`) | Exit 0 |
| Codex-reported build (`cargo build --locked`) | Exit 0 |
| Codex-reported Clippy (`cargo clippy --locked --all-targets -- -D warnings`) | Exit 0 |
| Codex-reported tests (`cargo test --locked --all-targets`) | Exit 0. **839 passed** (806 library, 25 presentation-contract, 8 procedural-adapter). The binary target contained zero tests. Zero failed, ignored, measured or filtered. |

All 831 previously accepted tests matched by Cargo target and test name. The eight additions are library tests. The Cargo results apply to B4-I1. Closeout verification covered documentation and source identity.

## Owner-native acceptance

The owner reported **6/6 PASS** on the Arch Linux/X11/Openbox host. The results below are owner-reported observations grouped by the acceptance scenarios.

| Native acceptance group | Owner-reported result |
| --- | --- |
| Startup and relaunch | PASS |
| Palette, Expose and click-through | PASS |
| Gestures, popup and focus | PASS |
| Visibility, remap and workspaces | PASS |
| Bring Top, Bring Here and Recover, including minimized state | PASS |
| Idle, Quit and clean shutdown | PASS |

## Limits and review gate

- Native fault injection remains unverified. Injected failures were exercised through deterministic test seams.
- Checked X11 transport and host commitment leave live compositor acknowledgment, GPU scanout and atomic pixel/SHAPE updates unverified.
- Sustained 60 FPS playback, display-scaling variants including mixed DPI, other window managers and Wayland remain unverified.

B4 technical acceptance is complete. Independent ChatGPT review of this documentation closeout passed on 2026-11-23. Git publication remains owner-controlled. C1-C, Vulkan/X11 transport selection and private renderer integration require separate authorization.
