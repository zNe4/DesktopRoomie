# A01-C1-C-B1 synthetic Vulkan readback

B1 is a standalone, headless Cargo package. From the repository root, explicitly opt in to the benchmark with:

```sh
cargo run --manifest-path experiments/gpu-readback/Cargo.toml --release --locked --features native-vulkan -- --run
```

`--help` or an empty argument list prints the command. Default Cargo tests use CPU-only code. Tests built with `native-vulkan` also use CPU-only test seams. The benchmark creates a Vulkan adapter without a surface and reports its device type and driver. Exit `0` requires exact parity in every iteration. Exit `1` indicates preparation, readback or parity failure. Exit `2` indicates unavailable adapter capabilities or invalid CLI use.

The package pins `wgpu` to `29.0.1` with `std`, `vulkan` and `wgsl`. The wgpu crate declares Rust 1.87. The package requires Rust 1.88 because the accepted CPU oracle uses `slice::as_chunks`. The lockfile resolves `wgpu-core` to 29.0.4 and retains `ordered-float` 5.4.0 because 5.5.x requires Rust 1.90. The experiment compiles the accepted `synthetic.rs` verbatim against the public presentation types from `desktop-probe` and invokes the fixture through `FrameProducer`.

## Exact comparison

Each sweep evaluates `Idle/Neutral` at eligible times 0, 300, 600, 900 and 1200 ms. The benchmark advances through those logical samples without sleeping. WGSL computes the image from integer pixel coordinates and the phase number. A full-screen triangle writes semantic RGBA into a linear, single-sample `Bgra8Unorm` target with a transparent clear and no blending. The persistent target uses `RENDER_ATTACHMENT | COPY_SRC`. The CPU reference enters only the comparison path.

The single `COPY_DST | MAP_READ` staging allocation holds 122,880 bytes. Each of the 160 rows has a 768-byte copy pitch, of which 640 bytes are pixels. The owned 102,400-byte result preserves top-left row order and BGRA channel order. Comparison checks every byte, rejects channels above alpha, derives the mask from `A != 0`, and checks exact bounds and half-open hit spans against the CPU fixture. A parity diagnostic gives the first differing pixel and channel, both byte values, total differing bytes and pixels, both SHA-256 hashes, and coverage diagnostics.

## Resource ownership and failure

1. The sole staging slot starts available and moves to in flight when taken. The renderer encodes the render pass and texture copy into one command buffer and submits once. It waits using that call's opaque wgpu submission index.
2. Only after that submission is confirmed does the renderer request mapping. The mapping callback can send only a tagged status to that attempt's private receiver. The renderer polls using the same submission index and accepts only the current generation, phase and experiment-local submission serial before the deadline.
3. After successful mapping the renderer opens a read-only view, copies the pixel prefixes, checks the packed length, drops the view and unmaps. A final device-error check precedes return to the available state. The output owns its bytes independently.
4. A failed attempt retires staging and returns no bytes. A pending or failed map with confirmed GPU completion is canceled by destroying staging. Dropping its receiver isolates late callbacks. The benchmark exits on the first error and has no retry path.
5. A submission that cannot be confirmed is left to process teardown after the failure is reported. The same path handles a caught fatal native poll error. At this point the attempt has created no CPU view. The process exits with the queue alive because this version of wgpu-core can wait again and panic in the queue destructor after a hung submission. Normal shutdown drops the settled resources and reports `released`.

The frame wait budget is 2 seconds, shared by submission and mapping from the start of submit. It is specific to this experiment. Both wgpu waits receive the remaining duration and the same submission index. Adapter and device requests are immediately ready in the pinned native backend, so the experiment polls each future once and fails on `Pending`. The budget covers controlled waits. Synchronous driver calls, pipeline creation and driver or OS resource teardown have no hard wall-clock guarantee here. A device-lost callback and the uncaptured error handler preserve the first reported fault. The pinned native `Device::poll` may panic after reporting a HAL device failure. The standalone command catches that unwind, reports the recorded fault or the panic diagnostic, and takes the uncertain-queue exit path.

## Telemetry

The output is line-oriented. Five warmup sweeps produce 25 warmup frames. Twenty measured sweeps produce 100 frames, 20 per phase. Each successful frame is compared before its timing is printed.

| Record | Fields |
| --- | --- |
| `experiment`, `layout` | Compiler, pinned wgpu version and features, logical period, wait budget, counts and byte layout |
| `adapter`, `capabilities` | Backend, adapter name, vendor and device IDs, driver, hardware/software class, format usages and relevant limits |
| `initialization` | Device setup, shader and pipeline creation, and total setup wall time |
| `parity` | Each phase's first verified CPU and GPU pixel hashes, mask hash, covered count, bounds, span count and mismatch counts |
| `frame` | Phase, submission, sample class, sweep index, and elapsed microseconds for encode, submit, GPU wait, map, repack, comparison, total and CPU blocking |
| `summary` | Per-phase, per-metric measured count, min, median, nearest-rank p95 and p99, and max. With 20 samples, nearest-rank p99 is the max. |
| `shutdown` | Frame submissions, map requests, validated mappings, explicit unmaps, staging allocations/reuses/retirements, peak outstanding submissions and shutdown state |

Encode includes both render and copy recording. GPU wait is host-observed completion latency for render plus copy. Mapping elapsed time covers the map request, its poll and callback receipt. CPU blocking is submit, GPU wait and map wall time added together. Total covers preparation through comparison. All recorded times are host wall times and specific to this experiment.

## Deterministic test matrix

| Case | Expected result |
| --- | --- |
| 256-byte copy alignment, nonzero padding, malformed size, zero extent and overflow | Exact row prefixes, with rejected malformed inputs |
| Accepted phase edges, wrap, recolor, hole, disjoint and empty frames | Accepted fixture metadata and bytes preserved |
| Channel swap, row inversion, malformed PMA and oversized output | Parity failure with a specific diagnostic |
| Wrong generation, phase or submission | Stale completion rejected |
| Delayed map or attempt to acquire in-flight staging | Staging unavailable until matching completion |
| Failed map, expired deadline or callback after timeout | Attempt fails, staging stays retired and late completion cannot reach a later receiver |
| Callback after success or cancellation | Attempt receiver is gone |
| Successful read and failed repack | Read-only view drops before unmap in both cases |
| Device loss, validation failure, internal device error or native poll unwind | Explicit failure, even with a successful map message present |

## Recorded verification

At base `ea8f2a6e1264fdc0dbee65ca1dae4a713d2892b7`, all 851 baseline test identities matched their preflight inventory. The probe's documented Cargo checks exited `0`. B1 passed all 16 CPU-only tests, including all rows in the fault matrix, in both default and `native-vulkan` builds. B1 formatting, locked build, all-target check and all-target Clippy exited `0`. The pinned wgpu stack separately built on Rust 1.87.0, and the full B1 all-target check with Vulkan enabled passed on Rust 1.88.0. An intentionally missing ICD produced `BLOCKED`, exit `2`, and zero submissions.

The sandbox software run used llvmpipe with Mesa 26.3.1 and LLVM 23.1.5, Vulkan vendor `0x10005`, device `0x0000`, and Rust 1.99.0. All 25 warmup frames and 100 measured frames matched, with zero differing bytes or pixels. The CPU and GPU hashes below were identical. Mask, bounds, and complete span sequences matched in every frame.

| Phase | Eligible ms | Covered pixels | Bounds `(x, y, width, height)` | Spans | CPU and GPU BGRA8 SHA-256 |
| --- | ---: | ---: | --- | ---: | --- |
| 0 | 0 | 3136 | `(17, 40, 56, 56)` | 56 | `502407c0dfe82bed6c8afa7cace8ea8cfcb2901b6a5a4f1f8c1422eed3962344` |
| 1 | 300 | 3136 | `(17, 40, 56, 56)` | 56 | `bb1671aa10a1004cdac43424ee32bd26df32d223f8963bd10f4b4011d03aa04f` |
| 2 | 600 | 2303 | `(51, 53, 54, 57)` | 88 | `747f47acfa87e37989c564942dff2235a6ce67a83310127f3c20720965be7de4` |
| 3 | 900 | 1446 | `(113, 24, 40, 107)` | 64 | `1a919a32707c522bc1be04a9163c23ffde8663959f80a3afb48d0cdc0648e2fe` |
| 4 | 1200 | 0 | Empty | 0 | `f627ca4c2c322f15db26152df306bd4f983f0146409b81a4341b9b340c365a16` |

The software run measured 168.210 ms for device setup, 4.129 ms for shader and pipeline creation, and 172.376 ms total initialization. Per-phase median frame totals ranged from 0.832 to 0.870 ms. It exited `0` and reported 125 submissions, successful maps and unmaps, one staging allocation, 124 reuses, zero retirements, peak outstanding of one, and `shutdown state=released`.

For owner-native verification, run the opt-in command on the hardware Vulkan host and retain stdout, stderr and the exit status. Check the reported adapter class, all five parity records, the final `PASS`, and the shutdown counts. Hardware Vulkan remains pending. Physical device loss and a hung driver have injected test coverage only. B1 exercises offscreen rendering and readback independently of X11.
