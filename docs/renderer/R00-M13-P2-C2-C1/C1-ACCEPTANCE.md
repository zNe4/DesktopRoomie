# DesktopRoomie R00-M13-P2-C2-C1 — accepted textured mesh rendering milestone

**Owner-approved technical closure on November 21, 2026.** Independent D1
renderer and numerical review, independent D1-R1 correction review, and
independent review of the successful N1 retry evidence all passed, as recorded
in the owner assignment. Codex rechecked the retained evidence using CPU-only
verification. **C2-C1-F is prepared, pending independent checkpoint review.**
This offscreen renderer remains separate from the accepted A00 Rust/x11rb
desktop host.

## D1 implementation and synthetic verification

The 32×32 single-sample fixture reuses the accepted B2 indexed mesh geometry.
Mesh A is a hexagon with six indexed fan triangles. Mesh B is a partially
overlapping pentagon with five. Each mesh samples its own nonconstant 8×8
RGBA8 premultiplied-alpha (PMA) texture page with nearest-neighbor filtering
and clamp-to-edge addressing. The affine UV mappings are `u=x/32, v=y/32`
for A and `u=1−y/32, v=x/32` for B.

The existing WGSL shader and encoded-domain PMA source-over blend apply in
draw order A then B. The independent per-pixel CPU reference reconstructs
top-left coverage, UV interpolation, nearest texel selection and compositing.
The frozen texture, vertex/index, coverage, tolerance and reference bytes
are preserved with the project. The D1 logs record **61 Rust tests and 163
Python tests passed**, with formatting, build and all-target Clippy passing.

## D1-R1 reproducibility correction

The original gate could pass three individually valid framebuffers that
differed from one another. The corrected runner compares each later
framebuffer hash with the first successful run and stops immediately on a
difference, preserving the evidence and leaving the summary incomplete.
The unchanged review procedure for each individual frame still applies.
The corrected independent postcheck additionally requires all three
independently verified framebuffer hashes to match.

Independent R1 CPU-only verification passed **31/31 tests**. The retained
`logs/r1-python-tests.log` also records 31 passing tests (19 native-gate tests
and 12 numerical tests), including byte-identical receipts for all seven
prior CPU fixtures. Against D1, the retained negative-control log records
19 gate tests with three expected failures. They reproduced acceptance of
a differing second frame, acceptance of mutually different frames in a
complete evidence set, and a failed final reread leaving a complete summary.
R1 changed the Python gate, its tests and the owner instructions. The renderer,
WGSL shader, fixture, oracle and compiled worker retained their D1 identities.

## Initial N1 attempt — ENVIRONMENT_BLOCKED

At 03:46:40 UTC on November 21, the worker passed CPU fixture and shader
preflight. Explicit Vulkan discovery enumerated zero adapters. The worker
allocated zero GPU resources, submitted zero GPU commands and produced no
framebuffer. It exited with code 2 and the runner stopped after that first
attempt. The summary reported a missing framebuffer result, consistent with
the discovery blocker. This attempt contributes zero accepted native runs.

B2 had used NVIDIA 620.70.06. The blocked process record shows mappings of
updated NVIDIA 620.73.04 userspace libraries. A pending reboot after the
update and a possible kernel/userspace driver mismatch are a plausible
explanation. The retained evidence does not establish the loaded kernel
module version or conclusively establish that mismatch. The original archive
and all its contents are preserved as historical evidence.

## Successful N1 retry

All three fresh supervised processes selected the **NVIDIA GeForce RTX 3050
6GB Laptop GPU** on **Vulkan**, with vendor **0x10de**, device **0x25ec**, and
NVIDIA driver **620.73.04**.

| Run | UTC start on 2026-11-21 | Exit | Exact pixels | Nonexact pixels | Tolerance violations |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 | 17:57:43.529541 | 0 | 1,024 | 0 | 0 |
| 2 | 17:57:44.425589 | 0 | 1,024 | 0 | 0 |
| 3 | 17:57:45.280783 | 0 | 1,024 | 0 | 0 |

All three raw framebuffers were identical and exactly equal to the independent
CPU reference. **12,288/12,288 RGBA channels matched exactly** across the three
runs. Each raw file is 4,096 bytes of row-major RGBA8, top row first. The retained
receipts and unchanged R1 postcheck show clean validation and out-of-memory
scopes, completed map/view-drop/unmap, resource and device cleanup, and clean
bounded process termination.

| Per-frame pixel class | Count |
| --- | ---: |
| Background | 531 |
| A-only | 210 |
| B-only | 208 |
| A overlapped by B | 75 |

The 300 overlap channels per frame permitted a maximum one-byte deviation.
The observed maximum was zero. All channels outside the overlap required exact
equality.

## Frozen identities and preservation

| Artifact | SHA-256 or source digest |
| --- | --- |
| Original D1 review archive | `b0ea048ed7921c2f7f4bd951a64e04bbaf1cd263db0dd612bc98a1fc2ca6183d` |
| Historical D1 source | `950221eee9e1dc339a8e84551b39d901189970cef5c6516c986e9fd7bb3272c8` |
| Accepted R1 review archive | `c027de61178ed1059c4e1a70e4276738fe996bdb1c275f0dd3a6c96eea596a95` |
| Accepted R1 source, 139 files | `be02d587b2504a6fb9c247e237fb1333fb26c2b8b0e8b5e56c03549d175aae4a` |
| Accepted GPU worker, 97,556,184 bytes | `a2674fcf042e9373f0ac8395091d048f4973a33642ec56d64b27ec15ed1af9aa` |
| CPU reference and each successful framebuffer | `98f2978a9a7c447d1f85d7570d2114bfe55866b0543f8c76fe78234994f4c1eb` |
| Original environment-blocked N1 archive | `8a17035adcc3969a85edcf42deb0460c25a2ebcb40750c1583e10adee686b56e` |
| Original successful N1 archive | `904a0ccc19329e4c16662fb91a7b50f4fec07505712a2be4a73244734bde81a6` |
| Sealed B2 checkpoint manifest | `d9f62fe98e6a7f1ed3f190e9b8a18e5d404b1e93f72a50734f57730a78026ad6` |
| Accepted B2 source | `1b78996b9839a525a337a2036afdc22684627042178938d5e35a906236b591a5` |

Source digests hash sorted UTF-8 lines of `<file SHA-256>  <project/path>\n`,
with two spaces, the `project/` prefix and one line for every project file.
The original D1, R1 and N1 status statements remain historical records.

Checkpoint path

`/home/zzzne4/Downloads/DesktopRoomie-R00-M13-P2-C2-C1-checkpoint-20261121`

The accepted R1 project is under `project/`, its worker is
`target/debug/gpu-smoke`, and the independent reference is
`project/fixtures/textured-complex-mesh.expected.rgba8`. The original review
archives are under `archives/`. The blocked evidence is under
`native-evidence.d0uHeASv/` and the successful retry is under
`native-evidence.4FdaMxqF/`. Each retains its own original `nv.tar.gz`
and all archive members. The checkpoint manifest and receipt record the
CPU-only integrity verification, which remains separate from independent
checkpoint review.

## Proven scope and exclusions

Acceptance covers the tested nonconstant textured meshes, affine UV
interpolation, nearest sampling, multiple texture pages, shared-edge ownership,
varying PMA transparency, ordered compositing and deterministic readback on
the recorded NVIDIA/Vulkan configuration.

Excluded from acceptance are arbitrary mesh topology and UV mappings, texture
atlas regions/rotation/trimming, bilinear filtering over complex meshes, atlas
padding and neighboring-region isolation, mipmapping, MSAA, clipping, Spine
asset loading/animation/deformation, other backends/adapters/drivers, and
production DesktopRoomie integration. C1-F is a documentation and preservation
mission. The owner retains publication authority and future implementation
requires a separate assignment.
