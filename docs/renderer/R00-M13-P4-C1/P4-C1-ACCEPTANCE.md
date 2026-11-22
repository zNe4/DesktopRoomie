# DesktopRoomie R00-M13-P4-C1 — native animation preview acceptance

**Technically accepted on November 22, 2026.** Independent source review passed, and the owner accepted the private, managed X11 preview on the tested NVIDIA/Vulkan setup. The owner observed authentic front-facing Vanilla playing and looping continuously for at least 30 seconds without visible rendering corruption. Resize, maximize, minimize, restore, close and relaunch behaved correctly.

## Owner-reported telemetry

| Application measurement | Observed value |
| --- | --- |
| Submitted frames and returned present calls | 4,409 each |
| FPS across the three reported measurement windows | 60.01, 59.98, 59.94 |
| Median frame interval (ms), in the same order | 16.703, 16.680, 16.700 |
| p95 frame interval (ms), in the same order | 19.287, 17.581, 19.493 |
| Combined time per frame for CPU animation evaluation, geometry validation and packing, and upload staging and encoding | Approximately 0.98 ms |
| Reported GPU and surface errors | 0 |
| Texture validations and uploads | 1 each |
| Offscreen and presentation pipeline creations | 1 each |
| Geometry buffer allocations and growths | 2 allocations, 0 growths |
| Diagnostic framebuffer readbacks | 0 |

Application counters support persistent texture, pipeline and geometry resource use over the exercised session. Frame intervals and FPS describe application activity. Actual display scanout, compositor timing and GPU completion timing remain unmeasured. The CPU phase total excludes queue submission and presentation. Native device loss was untested and is terminal in this preview. Device recreation after loss is outside the accepted scope.

## Evidence and preservation

The private checkpoint contains the exact 216-file reviewed source snapshot, including the local dependency source, Cargo manifests and lockfiles. All files matched the review manifest. The source manifest SHA-256 is `325d8d78f4d8bbaba9d4dd8607bfbe9ff0241cdc3738a84226733aa8b0e59c85`. The reviewed patch SHA-256 is `4305fd9e3264ff858add89f00315690bc5fed4ea003900afa384ad97cd9924bc`. The checkpoint retains the review candidate report, its check logs and the owner's November 22 native findings. The owner supplied three lines of native telemetry. The original terminal log and complete shell exit-status transcript were not retained.

The retained source-review logs record passing formatting, build and all-target Clippy checks for the preview, extension and offscreen dependency, with 5, 27 and 85 tests passed respectively. The locked offline release preview build also passed. Closeout verified the source and evidence identities without rerunning native playback.

**Next is P4-D — animation and directional transitions**, followed by interactive desktop integration. Each requires its own assignment and acceptance. Spine runtime licensing clarification remains pending. Independent ChatGPT review approved the documentation closeout on November 22, 2026.
