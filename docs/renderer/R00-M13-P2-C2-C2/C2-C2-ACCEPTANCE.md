# DesktopRoomie R00-M13-P2-C2-C2 — accepted atlas rendering milestone

**Owner-approved technical closure on November 21, 2026.** Acceptance covers
C2-A, C2-B and C2-C on the tested synthetic fixtures and recorded native
environment. Independent ChatGPT reviews approved C2-F1 portable historical
verification and C2-F2 consolidated checkpoint preservation. The offscreen
renderer remains separate from the accepted A00 Rust/x11rb desktop host.
Production renderer integration remains pending.

## Accepted native results

All 18 successful fresh processes used the **NVIDIA GeForce RTX 3050 6GB
Laptop GPU** on **Vulkan**, vendor **0x10de**, device **0x25ec**, driver
**620.73.04**.

| Milestone | Processes | RGBA channels compared | Exact matches | One-byte deviations | Comparison violations |
| --- | ---: | ---: | ---: | ---: | ---: |
| C2-A | 3 | 3,072 | 3,072 | 0 | 0 |
| C2-B | 9 | 9,216 | 8,928 | 288 | 0 |
| C2-C | 6 | 24,576 | 22,836 | 1,740 | 0 |
| **Total** | **18** | **36,864** | **34,836** | **2,028** | **0** |

- **C2-A — Nearest atlas regions.** Two independent indexed draws shared one
  nearest-filtered atlas page using validated region-local UV remapping.
  All 3,072 channels matched the CPU reference exactly.
- **C2-B — Bilinear filtering and atlas isolation.** The fixture exercised
  horizontal, vertical and corner extrusion, page-edge clamping and
  interpolation of transparent premultiplied-alpha (PMA) texels. The protected
  atlas variants differed in unrelated texels. All six protected frames were
  byte-identical, and all three unprotected frames were byte-identical within
  their group. Unprotected contamination reproduced at 52 distinguished
  pixels. Across those pixels, the minimum of the per-pixel largest channel
  differences between protected and unprotected frames was 17 bytes, against
  a required minimum of 15. All 9,216 channels met the frozen tolerances,
  with a maximum observed deviation of one byte from the CPU reference.
- **C2-C — Integrated atlas meshes.** The fixture used the accepted complex mesh
  geometry and coverage, two independent indexed draws, one shared
  bilinearly filtered atlas and overlapping PMA source-over composition.
  All six native frames were byte-identical. All 24,576 channels met the
  frozen tolerances, with a maximum observed deviation of one byte from
  the CPU reference. The frozen mask allows up to two bytes in selected
  overlap channels.

## Numerical and lifecycle qualifications

The CPU references use exact rational arithmetic for operations such as
bilinear interpolation and PMA blending. The C2-C oracle quantizes the
first draw to stored RGBA8 before using it as the second draw's destination
and keeps the second draw's filtered source rational through blending.
The frozen engineering comparison envelopes set fixture-specific pass
thresholds. The native comparisons record the actual deviations on the
configuration above. These contracts provide no universal Vulkan precision
guarantee or implementation-independent hardware rounding rule.

The C2-C negative controls for premature source quantization and omitted
intermediate destination quantization remain within the native comparison
mask. Operation-order tests, the original WGSL identity, blend descriptor
tests and source review carry those distinctions. Recorded process durations
cover one-shot invocations. Lifecycle evidence covers destruction requests
and ownership release. Steady-state animation performance and physical
GPU-memory reclamation remain outside the measured scope.

## Frozen identities and preservation

| Artifact | SHA-256 or source digest |
| --- | --- |
| Accepted primary C2-C source | `44216cb8b338fa1ba7850f66a76d6f64f43b76e46f77b1a9df78be0316f01c69` |
| Accepted C2-C GPU worker | `bd67a380825a3458c4bb3a129e8abf388817f74a188fb3f7b168dda03f452bec` |
| Unchanged WGSL shader | `bedbb7a0b76ceb502d519ad6540359d0c84e213ca4455f8250478b1a17ab5936` |
| Approved consolidated manifest | `d395f889540b55fe0a690c1edb1bc133d47a47ac5f1f0c025a2c4cc68b10d40f` |
| Approved consolidated verifier | `5a49e91d042a151e6e957ac09fcbff3d4cb724eb29c3a3eeacf8f8390a2300ff` |
| F2 review archive | `ddfad4a6cde1b5f40f97443b45cc5ee99f3127e18f34576338907d0eb4a2340d` |

The consolidated checkpoint preserves C1, C2-A, C2-B and C2-C. Portable
historical CPU verification authenticates the preserved artifacts while
retaining their historical execution identities and original acceptance
predicates. Under F2, verification passed all six checks on the installed
and independently relocated checkpoint. The F3 installed recheck also
passed all six. Historical verification needs no GPU execution.

The consolidated checkpoint's primary C2-C project is the accepted baseline.
Any separately authorized renderer development starts from an isolated
derivative of that project.

## Scope and next proposed work

Acceptance is limited to synthetic offscreen atlas rendering. Asset parsing,
rotated and trimmed regions, Spine loading and animation, live desktop
presentation, glTF and 3D rendering, and character switching remain outside
this closure. Broader renderer research and A01 gates retain their own
acceptance criteria.

The next proposal is a bounded, read-only investigation of the actual private
Vanilla assets, subject to separate authorization. The assets and intended
first presentation will determine priority for rotation, trimming, atlas
parsing and other asset-facing requirements. Establish usable interactive
Vanilla first, then audit Hina's GLB/model requirements and validate the
independent 3D backend. Character switching follows once the required backend
contracts are established. [A01 plan](../../DesktopRoomie-A00-A01-plan-v0.1.md)
