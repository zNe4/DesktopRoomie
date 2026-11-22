# R00-M13-P4-D acceptance

**Technical acceptance complete. Independent ChatGPT review found the P4-D acceptance record and milestone updates substantively correct and requested bounded final corrections before publication.**

## Accepted source and preservation

The accepted D2 preview uses the private P4-D1-B derivative. The public repository baseline is `03786080ef9777cb3bdd71e3d1fd7ae192500149`.

| Private evidence | SHA-256 |
| --- | --- |
| Accepted C1 source manifest | `325d8d78f4d8bbaba9d4dd8607bfbe9ff0241cdc3738a84226733aa8b0e59c85` |
| Reviewed D1-A source manifest | `b28c5dc6fa652593ee626811f4a4976f1dddad6a2222428e01db270885c037a6` |
| Accepted D1-B source manifest | `bd63117ffde028648c14f4f7e2fc32b3f677b40f8e3fb4d57939cbd785c893c4` |
| D1-B source patch | `524b5a7bc5e9e303ca54a81f047c802697ecc4a5f3f49f5122bfeafa3c719548` |

Closeout verified the D1-B manifest and patch hashes and all 219 manifest entries against the local derivative. The original skeleton, atlas and texture matched the D0 inventory. The prepared PMA texture, atlas and preparation manifest matched the D1-A inventory. The private review bundle retains the source, checksums and logs. The native log identifies the scripted launch from this derivative's release target. It contains no executable hash.

## Acceptance evidence

- **D0 compatibility and D1-A CPU characterization.** D0 inventoried the approved inputs. D1-A sampled full cycles and directional round trips and measured the back-facing idle seam.
- **D1-B CPU verification.** Locked offline build, formatting and all-target Clippy passed for the extension, preview and S2-C renderer dependency. Tests passed with 35 extension tests, 7 preview tests and 86 renderer tests. The extension suite's one ignored test passed in the explicit accepted-asset run, covering 1,801 scripted frames, 3,416 full-cycle poses and 66 walking direction round trips. The locked offline release preview build passed.
- **Independent source review.** D1-A and D1-B review gates are complete per the owner’s closeout assignment. The private D1-B source packet covers phase and clock policy, staged geometry admission, persistent renderer resources and preservation of the default C1 preview path.
- **Owner-native D2.** The owner confirmed that playback, directional transitions, performance and lifecycle tests passed. This sign-off covers the scripted idle-to-walk-to-idle sequence, including front-left and back-right walking cuts and return to front-right.
- **Retained numerical telemetry.** The local `owner-scripted-native.log` records a partial run on NVIDIA GeForce RTX 3050 6GB Laptop GPU / Vulkan, with FIFO presentation, 1,473 submissions and returned presents, and exit 0. It reaches the 22.302 s return to front-right walking and ends before the 27 s return to idle. Its shutdown window reports 59.79 application FPS, a 16.637 ms median and 17.770 ms p95. It reports one texture validation and upload, one creation per pipeline, two geometry allocations, zero growth, zero diagnostic readbacks and zero reported GPU or surface errors. This log measures application pacing. Full-sequence and lifecycle acceptance come from the owner report. Display scanout and native device loss remain outside the evidence.

## Accepted transition behavior

Animation and direction changes use hard cuts. Clip changes start the destination at time zero and direction changes preserve walking phase. The additional gameplay-video comparison found no clear evidence of crossfading. The original game's internal mixing configuration remains unknown.

Back-facing idle remains outside the accepted script. D1-A measured up to 8.34348 world units of shared-vertex displacement across its loop seam, approximately 7.32 C1 pixels, with stable attachments and draw order. This qualification applies to back-facing idle. The script uses back-right only during walking.

Interactive desktop integration is the next milestone and requires a separate assignment and acceptance. Runtime licensing clarification remains pending.
