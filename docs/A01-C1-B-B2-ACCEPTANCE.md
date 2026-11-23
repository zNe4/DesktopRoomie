# A01-C1-B-B2 acceptance

The owner technically accepted the B2 procedural compatibility adapter on November 22, 2026, after independent source review, automated verification and owner-native confirmation.

## Accepted source

B2 was uncommitted at acceptance. Its base was `fd394a5666492d3c1baf0b164d3092b375f9a467` (B1). The retained final implementation patch, `A01-C1-B-B2-I.patch`, has SHA-256 `2a1b84d0a084d97ea6e0bf488a15f83aa38ec0b8913a9c734c7182358ec4e350`. The five files below matched the final verification replay by SHA-256 during closeout.

| Path under `experiments/desktop-probe/` | SHA-256 |
| --- | --- |
| `src/presentation/mod.rs` | `307041044c89e568b08dcb5f2122038fa36cb87fd62e44129fe41c9b722db89a` |
| `src/presentation/procedural.rs` | `7f5974b1f148d183224aaf9ffa9c7f4bc28f816c779069c97eceb024ba234eb7` |
| `src/x11/render.rs` | `3e9121f2a376d8bb288870f0e78ee9b83d11baa1e7f447d9c0dc1e79ebc82bb7` |
| `src/x11/shape.rs` | `71dac83aad9566b4a1a61b0f17d37a82c7923c2db40be1f8c3324bc1c06ec02f` |
| `tests/procedural_adapter.rs` | `4d61e1a7ce20b2f8ad8f3eadf86bb0706e6e8618d5fb7e4013fa3518a46ab5ab` |

## Verification and review

| Evidence | Accepted result |
| --- | --- |
| Independent ChatGPT source review | PASS with no blocking findings. |
| Cargo verification | Build, formatting and all-target Clippy passed. Tests reported 795 passed and zero failures (762 unit, 25 B1 integration and 8 procedural adapter integration). These are implementation results. |
| Procedural pixel parity | Exact byte-for-byte match against the frozen B1 renderer for all 25,600 premultiplied BGRA8 pixels per palette, Lavender and Coral. |
| Hit-region parity | Exact agreement between nonzero alpha, canonical spans, legacy geometry and the X11 SHAPE rectangles for both palettes, including boundary checks. |
| Owner-native confirmation | The owner confirmed that the updated application works correctly on Arch Linux/X11/Openbox. The confirmation is a summary without a scenario-by-scenario transcript. |

## Scope and next checkpoint

B2 covers the fixed 160×160 Idle/Neutral procedural frame and the two existing palettes through the synchronous X11 renderer. Native confirmation is scoped to the owner's host. The pixel and region comparisons verify generated buffers and coverage. Other window managers, Wayland and display-scaling variants are outside this acceptance.

Committed geometry and input (B3a) is the next checkpoint and requires its own assignment. B3b, B4, C1-C, Vulkan/X11 transport selection and private renderer integration remain outside B2. Technical acceptance applies to the source identity above. Publication requires separate owner authorization.
