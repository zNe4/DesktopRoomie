# DesktopRoomie R00-M13-P2-C2-B2 — accepted complex mesh rendering milestone

**Owner-approved milestone closure: November 19, 2026.** D1 independent source
review passed, and N1 owner-native NVIDIA/Vulkan acceptance passed. The owner's
B2-F assignment states those review/acceptance outcomes and approves closure.
Codex reverified the preserved identities and all three retained native runs
using CPU-only checkers. It did not repeat the independent source review or
launch a native worker. Independent review of this new checkpoint is a separate
pending gate from the already approved B2 technical acceptance.

B2 establishes a bounded synthetic offscreen renderer result for complex mesh
boundaries and intersections. It remains separate from the accepted A00
Rust/x11rb desktop host and is not integrated into the production application.

## D1 implementation and synthetic verification

D1 added a closed 32x32 single-sample Rgba8Unorm fixture: a non-axis-aligned
hexagon A with six indexed fan triangles and a partially overlapping
non-axis-aligned pentagon B with five. Internal edges and fan vertices share
indices. A draws before B in one pass and one submission. Constant partially
transparent PMA colors use the existing shader/input convention and original
One/OneMinusSrcAlpha/Add source-over blending for RGB and alpha.

The independent CPU oracle uses exact doubled integer coordinates, sample
positions (x+1/2,y+1/2), and top-left triangle coverage. It checks zero or one
contribution per mesh. A separate convex-polygon half-plane construction agrees
over the entire frame. Exact rational blending gives overlap (34,89,43,196).
The reference was frozen before native execution; no expected pixel was derived
from observed GPU output.

The geometry exercises 19 distinct non-vertex internal-edge sample centers in
A, six in B, shared fan/outer vertices, included and excluded outer boundaries,
and eight proper A/B edge crossings. Two crossings involve both outer
boundaries; one is exactly at pixel center (21,9), assigned B-only by top-left
coverage. The preserved numerical note and geometry specification list all
coordinates and owners.

| Pixel class | Exact count | Reference RGBA8 |
| --- | ---: | --- |
| Transparent background | 531 | (0,0,0,0) |
| A-only | 210 | (48,24,8,96) |
| B-only | 208 | (16,80,40,160) |
| A-overlapped-by-B | 75 | (34,89,43,196) |

Every frame has 1,024 pixels and 4,096 row-major RGBA8 bytes, top row first.
The worker and independent postchecker require full-frame exact equality, with
coordinates, class and actual/expected bytes on mismatch.

D1's preserved logs record formatting, Rust build and all-target Clippy PASS,
58 Rust tests PASS and 136 Python tests PASS. All prior C1/C2 regressions,
every-channel mutation checks, missing/duplicate triangles, reversed mesh
order, incorrect edge rules and mocked native-evidence failures are included.
All six prior fixture-check receipts matched B1 byte for byte. These are D1
synthetic/build results, not new B2-F test executions or native proofs. D1's
initial Serde diagnostic failure and correction remain in its original logs.
Independent D1 source review PASS is recorded on the owner's authority.

## N1 owner-native acceptance

All three fresh-process runs selected the same discrete hardware adapter:
**NVIDIA GeForce RTX 3050 6GB Laptop GPU**, Vulkan backend, vendor **4318
(0x10de)**, device **9708**, driver **NVIDIA**, driver information **620.70.06**.
This identification comes from retained receipts, not a new system/driver probe.

| Retained run | Recorded UTC start, 2026-11-19 | Wall seconds | Exit | Exact pixels | Mismatches |
| --- | --- | ---: | ---: | ---: | ---: |
| owner-d1-vulkan-1 | 21:45:22.284158 | 0.323356621999892 | 0 | 1,024 | 0 |
| owner-d1-vulkan-2 | 21:45:22.946357 | 0.1837859859999753 | 0 | 1,024 | 0 |
| owner-d1-vulkan-3 | 21:45:23.590815 | 0.1721006610000586 | 0 | 1,024 | 0 |

Every raw frame equals the independent CPU reference, including all four class
counts. Each has SHA-256
`00968b049c3ca08ff537bccafcb1a879f9c0ead69a232fb05531bbdb5f3d5284`.
The stdout arrays, raw .rgba8 files and hashes agree. The independent postcheck
also verifies ordered draws, upload/layout identities, process bounds and clean
exit, validation/OOM scopes, map/unmap ordering and resource/device cleanup.
All three passed the CPU-only B2-F re-read. Cleanup receipts establish requests
and ownership release, not physical GPU-memory reclamation.

## Frozen identities and preserved evidence

| Artifact | SHA-256 / recorded source digest |
| --- | --- |
| Accepted worker, 97,216,280 bytes | `51baccada81fe9759bf93b9cfb6202f91c8f798f6e0eaba0b933adf28a907cc9` |
| Accepted D1 source, 119 files | `1b78996b9839a525a337a2036afdc22684627042178938d5e35a906236b591a5` |
| CPU reference and each N1 framebuffer | `00968b049c3ca08ff537bccafcb1a879f9c0ead69a232fb05531bbdb5f3d5284` |
| Unchanged WGSL | `bedbb7a0b76ceb502d519ad6540359d0c84e213ca4455f8250478b1a17ab5936` |
| Original D1 review archive | `fe49380ebc49bb74bcab308d11e259d77dc9edd079e89d6c5f3b00d58365c113` |
| Original submitted N1 archive, nv.tar.gz | `83f89d8968b3b76623f61a2e4bb1212d8a0defb23684699ed004213db53adcf9` |
| Preserved accepted B1 worker | `c9f18ac1bb49634c24fa9bb4b22b0adb7744475fdf63699104127f5b003a275a` |
| Preserved accepted B1 source | `67591071793749f5f40666eee67a161f390dc048ec59f543d5778653af0c5af4` |

The recorded source digest method is SHA-256 over sorted UTF-8 lines
`<file SHA-256>  <project/path>\n`, with two spaces, the project/ prefix, and
every project file. It is not a Git commit identity. The original manifest,
patch, review/build/test logs and historical B1 project remain byte-identical.

Accepted preservation entry point:

`/home/zzzne4/Downloads/DesktopRoomie-R00-M13-P2-C2-B2-checkpoint-20261119`

Within it, reviewed source is `project/`, the exact executable is
`target/debug/gpu-smoke`, and the oracle is
`project/fixtures/complex-mesh.expected.rgba8`. Native evidence remains at
`native-evidence.C8t8yAPN/`, containing the exact submitted `nv.tar.gz` and its
23 member files, including all three process/identity/stdout/stderr/raw-frame
records. The exact D1 review archive is separately retained under `archives/`.
No newly created archive is assigned the submitted N1 archive identity.

The checkpoint README, inventory, manifest and CPU-only verification procedure
distinguish reviewed source, executable, review evidence, native evidence and
historical baselines. B2-F reports preservation integrity separately from owner
technical acceptance.

Original B1 remains at
`/home/zzzne4/Downloads/DesktopRoomie-R00-M13-P2-C2-B1-checkpoint-20261116`.
Original D1 remains at
`/home/zzzne4/Desktop/Utils/ProjectVanilla/.private-reviews/DesktopRoomie-R00-M13-P2-C2-B2-D1-20261119`.
D1's REPORT.md, D1-OWNER.md and pending-native statements remain unchanged and
historically correct for that handoff. This new acceptance record supersedes
their milestone status without rewriting earlier evidence.

## Proven scope and exclusions

Accepted behavior is exact full-frame rendering of this bounded pair of convex
indexed meshes, their exercised shared edges/vertices and intersections,
transparent background, distinct PMA colors and ordered source-over overlap on
the recorded NVIDIA/Vulkan adapter and driver. Prior C1/C2 fixture identities
and synthetic regressions remain preserved; N1 is not an exhaustive native
rerun of every historical fixture.

Excluded: arbitrary mesh topology, concavity/self-intersection and general
winding guarantees; Spine integration; animation; arbitrary transformations
and clipping; MSAA; other backends/adapters/drivers and color spaces; live A00
host/X11 integration; private assets; and complete companion behavior. This
does not select a new production host or authorize future rendering features.

**B2-F is documentation and preservation only.** There are no native reruns,
accepted executable rebuilds, source/shader/fixture/test changes, dependency
installations, Git staging, commits, pushes, merges or publication during this
mission. The owner controls publication and subsequent missions.
