# A01-C1-A — Modular Rust Foundation acceptance

The owner technically accepted the standalone DesktopRoomie executable after local Codex verification and independent ChatGPT source review. A01-C1-A-F documents this acceptance and updates the architecture, project status and progress-site milestones.

## Source identities

The public baseline is `791a0636e7336d1df91efc5cb069b3790277213c`. The accepted C1-A implementation is the local, uncommitted source delta over that baseline. Its three changed files have these Git blob IDs, with paths relative to `experiments/desktop-probe/`.

| Path | Accepted Git blob |
| --- | --- |
| `src/lib.rs` | `33a910958ef7e1d11c45ef4f63145989f424d156` |
| `src/main.rs` | `e9ca15232c055fda72497718209ab7162db62fa1` |
| `src/probe_tests/here_service_tests.rs` | `3493cd9519db51a424f5f17c6036d43b8fb08948` |

The retained implementation review patch `DesktopRoomie-A01-C1-A.patch` has SHA-256 `8a72e12d438174116d5823a6871bb269edaeb2793bac07cfbccaa341d00eb523`. The baseline commit and the three blobs identify the accepted source together. A00 acceptance and its CI remain tied to their recorded revisions. [A00 integrated acceptance](DesktopRoomie-A00-M03.4-integrated-acceptance.md)

## Implemented scope

The binary-only crate moved to Cargo's conventional library and binary layout. [`src/main.rs`](../experiments/desktop-probe/src/main.rs) passes the process arguments after the executable name to `desktop_probe::run_cli`, the single public entry point in [`src/lib.rs`](../experiments/desktop-probe/src/lib.rs). The library crate root now owns CLI dispatch and the existing startup, event loop, owner-service scheduling, fatal handling and shutdown. Pure logic, control protocol, and native X11 code retain their existing module owners. The standalone executable name and invocation stay the same.

Help, Diagnose, Owner and Control retain their output, status and dispatch behavior. The procedural X11 runtime retains owner admission, control semantics, event ordering and explicit resource cleanup. `run_cli` retains `process::exit` on Control and error paths, including nonzero owner results after existing cleanup. Help, successful diagnostics and clean owner shutdown return normally. It is a process entry point and can terminate the calling process. [Implemented architecture](ARCHITECTURE.md)

The two source-sensitive tests still inspect the production source. Their `include_str!` targets now point at `lib.rs`, which contains `run_probe` and the owner service, with the substantive assertions unchanged.

- `every_production_async_error_return_is_inside_the_shared_fatal_loop_boundary`
- `production_owner_wait_boundaries_remain_reachable_as_specified`

## Verification and approval

| Evidence | Recorded result |
| --- | --- |
| Codex local automated verification | Locked build, formatting and all-target Clippy passed. The full suite passed 760/760 tests. All 13 CLI comparisons against the baseline passed. |
| Independent ChatGPT source review | Approved the C1-A implementation source. The documentation and site diffs have their own review gate. |
| Owner-native observation | The owner confirmed that the updated executable works correctly. No comprehensive itemized regression checklist or retained native transcript was supplied, so per-scenario native coverage is unverified at this checkpoint. |

These are implementation-stage results. The documentation closeout checks source identity and documentation consistency, with Cargo and native execution outside its verification scope. Earlier A00 host results and qualifications retain their original evidence boundaries.

## Next gate

C1-B is the next separately authorized implementation slice. It must define and review the backend-neutral presentation contract and synthetic procedural compatibility path, including host ownership, frame identity, safe input coverage and scheduling. Its interfaces remain unimplemented. Vulkan/X11 transport selection remains open and depends on separately authorized synthetic native proof. The private Spine adapter and integration remain separate and unimplemented.
