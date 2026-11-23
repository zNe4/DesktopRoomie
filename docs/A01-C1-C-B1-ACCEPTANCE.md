# A01-C1-C-B1 acceptance

**TECHNICALLY ACCEPTED — OWNER APPROVED — 2026-11-23.** Independent ChatGPT source review passed. The owner accepted the isolated synthetic Vulkan readback experiment on the strength of the automated checks and exact software and hardware parity. Independent ChatGPT review of the B1-F documentation and progress-site patches passed on 2026-11-23. Git publication and site deployment remain owner-controlled.

## Implementation and source identity

B1 is a standalone, opt-in headless Cargo package under [experiments/gpu-readback](../experiments/gpu-readback/README.md). The reviewed B1 source consists of eleven local files on `a01` at the published host baseline `ea8f2a6e1264fdc0dbee65ca1dae4a713d2892b7`. All eleven files match the independently reviewed `desktoproomie-b1-source-review.tar.gz` byte for byte and match the retained change inventory. The archive SHA-256 is `edf932dae2e210da396281b74965a1f5de46279ee8ee27ceb733e65ea516b99b`.

| Path relative to `experiments/gpu-readback/` | SHA-256 |
| --- | --- |
| `Cargo.lock` | `538832bfb16548f97b0a3385551703bc98752c4b54a6252ab3127754ddac5278` |
| `Cargo.toml` | `115eaea2ef94eae41648151f96c98a11ea0828eaf763f619845daa730c2ce660` |
| `README.md` | `fb7d39aeaccb5d68ed3e8e4a4a64089807dacd837eb6107c2e54a9c3d26ad129` |
| `build.rs` | `265cf8ff54e0bd27f348696de0a68c3608913150c2f9c3800b1a1c6bd54c056b` |
| `src/fixture.wgsl` | `bd194b7f6049e3ba56263bcae7466888192fd651cf75034e368628a92e5013f8` |
| `src/gpu.rs` | `f252595349af7b73726da7ed37e3376d4ff1b370ca85ac4c8610f746d172ecf7` |
| `src/layout.rs` | `adb54f05ceb35e5e4e67b8f9040650ddfd2ba51d6675caabb12856fb37aa81f8` |
| `src/main.rs` | `89a180b8df5a31ba783dc4ed671aed9f53c294192bc71ccc1ec2fc81489ce141` |
| `src/oracle.rs` | `d9aceb39269634314e96e7a3d71bfa8efd992a542e60e2d40ea14e085a1d24a2` |
| `src/readback.rs` | `150ebad9a696f376ef203488c460cd89a6fe0062f530428dac719a6ff45e8e78` |
| `src/run.rs` | `b0b3a238707cce6c76199318d0c01d5149f52c5faae6fee86cf168f0d06aed1c` |

The experiment compiles the accepted [CPU fixture](../experiments/desktop-probe/src/presentation/synthetic.rs) verbatim against the public presentation types. Its SHA-256 is `9bcca46707113b45f897e2ec688e4ff2467efb14d4416335a8d5729238ebb099`. The desktop-probe host at the published baseline is unchanged. B1 uses synthetic inputs and has no private Vanilla assets or X11 integration.

The optional `native-vulkan` feature pins `wgpu` to 29.0.1 with `std`, `vulkan` and `wgsl`. The pinned wgpu stack passed a locked compatibility check on Rust 1.87.0. B1 requires Rust 1.88 because the accepted CPU oracle uses `slice::as_chunks`. The full Vulkan-enabled B1 all-target check passed on Rust 1.88.0. The lockfile resolves `wgpu-core` to 29.0.4 and retains `ordered-float` 5.4.0 for compiler compatibility.

## Accepted technical contract

- Each sweep exercises the five accepted 160×160 CPU animation phases at 0, 300, 600, 900 and 1200 ms of logical eligible time. GPU rendering independently generates the phase in WGSL using a linear, single-sample `Bgra8Unorm` target with blending disabled. Readback must match every CPU byte and retain exact premultiplied-alpha semantics.
- A single staging buffer holds 160 rows at a 768-byte aligned pitch. Repacking copies each 640-byte pixel prefix in top-left row order into an owned 102,400-byte BGRA8 result. The comparison derives the alpha mask, bounds and half-open hit spans from the completed readback bytes.
- Submission and mapping share a finite two-second experimental deadline and target the specific wgpu submission. Checked staging ownership prevents reuse while in flight. Tagged callback completion is validated before access, and the read view is dropped before explicit unmap. Failure retires the slot and returns no bytes. Uncertain submission completion or a fatal poll error takes the standalone process exit path.

## Verification and evidence

| Evidence | Recorded result |
| --- | --- |
| Independent ChatGPT source review | PASS on the archive identified above. |
| Published desktop-probe baseline | Formatting, build, Clippy and 851 tests passed on the matching published SHA. [CI 45258825906](https://github.com/zNe4/DesktopRoomie/actions/runs/45258825906) |
| Codex-reported desktop-probe checks | Formatting, build, all-target check, Clippy and tests exited `0`. The retained preflight and final inventories match across all 851 test identities. |
| Codex-reported B1 checks | Formatting, locked build, all-target check and all-target Clippy passed. The Vulkan-feature check and Clippy also passed. All 16 CPU-only tests passed under each of the default and `native-vulkan` configurations. The retained successful commands exited `0`. An intentionally unavailable Vulkan ICD exited `2` with zero submissions. |
| Software Vulkan | The retained implementation verification log records llvmpipe with Mesa 26.3.1 and LLVM 23.1.5. All 125/125 frames achieved exact parity. The retained exit-status record reports `0`. |
| Hardware Vulkan | The owner ran the experiment on an NVIDIA GeForce RTX 3050 6GB Laptop GPU using NVIDIA Vulkan driver 620.73.04. All 125/125 frames achieved exact parity. The complete output records PASS and settled shutdown. The owner separately confirmed shell exit status `0`. |

Each Vulkan run comprises 25 warmup frames and 100 measured frames. Both logs report identical five-phase pixel and mask hashes, bounds and span counts, with zero differing bytes or pixels. On the hardware run, the final `released` shutdown reports 125 submissions, 125 successful maps, 125 explicit unmaps, one staging allocation, 124 reuses, zero retirements and peak outstanding work of one. Shutdown settled with zero outstanding operations.

| Retained evidence | SHA-256 |
| --- | --- |
| `desktoproomie-b1-baseline-tests.txt` and `desktoproomie-b1-final-baseline-tests.txt` | `c1a3bb021e1943005808df06caa3723aa3bf9f5198d6c44c49ff3a59b11d0b29` |
| `software-vulkan.log` | `7cd625fb8a188306833fb13793e5abfe144513869562b71d973b49d938159c03` |
| `desktoproomie-b1-hardware-20261123T192645.log` | `353c34e8e3e49855c62cddc4050268c7a5c6da8ae7b1436bcc8fd3f6c21576dd` |

The experiment README records the earlier checkpoint before the owner's hardware run. This acceptance record includes the later hardware evidence. The published CI covers the desktop-probe baseline. The separate B1 Cargo and Vulkan results come from the retained experiment evidence.

## Hardware performance observations

Device and pipeline initialization took approximately **506 ms total**. Across the 100 measured frames pooled over all five phases, the median total was **0.256 ms**, nearest-rank p95 was **0.296 ms**, and the maximum was **0.341 ms**. Frame total covers preparation through comparison. These are host wall-clock observations of the isolated experiment. Production performance remains unmeasured.

## Qualifications and decision boundaries

- Physical device loss and hung-driver scenarios have injected test coverage only. Synchronous driver calls and teardown have no guaranteed two-second wall-clock bound.
- The offscreen runs establish parity for this fixture. Production host responsiveness, compositor presentation, physical scanout and atomic pixel/SHAPE publication remain unverified.
- Production acceptance of the empty-SHAPE cutover remains a separate A1/A2 decision. [A1/A2 acceptance record](A01-C1-C-A1-A2-ACCEPTANCE.md)
- B2 remains pending and unimplemented. Production GPU transport selection remains open.
