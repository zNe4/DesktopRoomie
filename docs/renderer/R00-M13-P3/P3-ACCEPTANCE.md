# DesktopRoomie R00-M13-P3 — static Vanilla rendering acceptance

The owner accepted corrected directional rendering on **2026-11-22**. The accepted scope is private offscreen rendering of `Body_Idle_01` at time zero. **P4 — Continuous animation playback** is the next renderer milestone and requires a separate assignment. Runtime licensing clarification remains pending.

## Retained technical evidence

- **P3-N1:** `Back_R` matched the historical idle counts of **37 draws, 1,047 vertices and 1,232 triangles**. Strict encoded premultiplied alpha (PMA) validation rejected the original texture before GPU rendering because encoded RGB exceeded alpha.
- **P3-N2:** The successful retry used the reviewed, hash-checked clamp `RGB = min(RGB, alpha)`, preserving alpha. It corrected 534 pixels and 1,079 channels (maximum correction 28). The native NVIDIA/Vulkan log records the same geometry counts and a framebuffer hash for the first authentic `Back_R` idle frame. A separate earlier retry log records an output-directory validation failure.
- **P3-B:** The CPU-only preparation utility persisted the lossless RGBA8 texture, a byte-identical atlas copy and a manifest of hashes and correction statistics. The renderer then consumed the prepared texture through strict PMA validation. Native logs for both front skins report 62 draws, 1,890 vertices and 2,463 triangles and distinct framebuffer hashes. The independent review reported pixel differences confined to the chest decoration, with identical alpha and the same geometric orientation before the correction.
- **P3-B-R1:** `Front_L` and `Back_L` reflect by negating horizontal scale and offset in the fitted presentation transform. `Front_R` and `Back_R` keep native orientation. The transform leaves skeletal evaluation, vertex data and texture bytes intact. Retained CPU test output covers all four mappings. Retained native summaries report the expected reflection flags and framebuffer hashes for `Front_L`, `Front_R` and `Back_L`. The corrected `Back_R` mapping is covered by CPU tests, with the earlier P3-N2 frame as the native reference.

## Owner observations and preservation

The owner reported a successful visual comparison of the first `Back_R` frame with the reference viewer and visually correct poses from the first front-skin runs. The owner's November 22 approval covers the corrected directional presentation. The native logs record diagnostics and framebuffer hashes. The visual judgments come from the owner, and the available logs do not provide a complete shell exit-status transcript. Closeout used the retained evidence without rerunning the GPU.

The private checkpoint preserves the extension and its S2-C path dependency. The complete source trees and Cargo files were compared byte for byte with the P3-B-R1 review copy. The source checksum list has SHA-256 `d65415827310f90138c741bdc7923449f83060f0918d2fff6ef5657992fed805`. The existing prepared atlas, PNG and manifest were retained in place and checked by hash. A01 host integration and continuous playback have their own acceptance gates.
