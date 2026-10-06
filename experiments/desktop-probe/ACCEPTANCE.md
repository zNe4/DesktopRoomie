# DesktopRoomie — Adventure A00, Mission M01 Acceptance Report

This document records the official acceptance verification results for **Adventure A00 (Mission M01 Desktop Probe)**, as specified in `docs/DesktopRoomie-A00-M01-desktop-probe.md`.

- **Environment**: Arch Linux (Kernel 6.x), Openbox 3.6.1, Picom v13 (d87a5ba), Rust 1.99.0
- **Primary Display**: `eDP-2` (1920×1080 at +0+0)
- **Target Executable**: `experiments/desktop-probe` (git commit `feat(desktop-probe): add exit controls and record acceptance results`)
- **Verification Date**: 2026-10-06

---

## Acceptance Checklist Matrix

| # | Check | Requirement / Expected Result | Verification Evidence & Observed Behavior | Status |
|---|---|---|---|:---:|
| 1 | **Build** | Documented command succeeds using recorded toolchain (`rustc 1.99.0`, `cargo 1.99.0`, `x11rb 0.14.0`). | `cargo fmt --check` passes with zero diffs.<br>`cargo clippy -- -D warnings` completes with 0 warnings.<br>`cargo build` compiles binary `desktop-probe` in <1s. | **PASS** |
| 2 | **Mapping** | Body appears during delayed-start typing test (`--delay <sec>`) without stealing or diverting keyboard focus. | Executed with `--delay 2`. Active window (`_NET_ACTIVE_WINDOW`) remained on foreground editor/browser `37748739` before launch, during the 2s delay, and after probe mapped at (880, 479). Probe window (`35651585`) never became active (`_NET_WM_USER_TIME = 0`). | **PASS** |
| 3 | **Transparency** | Empty area is genuinely transparent; 50% translucent cyan patch blends smoothly; body is solid; no borders or frame. | Visual verification under Picom compositor: transparent padding shows underlying wallpaper/windows, 50% cyan patch (`#00C8FF80`) blends composited, opaque body (`#A55FE1`) is crisp. `_MOTIF_WM_HINTS` decorations=0 removed titlebar/borders. | **PASS** |
| 4 | **Body click** | Pointer clicks on body provide visual/event feedback while keyboard focus remains with prior application. | Left click (Button 1) on body toggles color between Lavender (`#A55FE1`) and Coral (`#F06E50`) with instant double-buffered repaint. Foreground window retains keyboard focus throughout (`WM_HINTS` `input = false`). | **PASS** |
| 5 | **Hover** | Pointer enters/leaves body without activating window. | `EnterNotify` and `LeaveNotify` log pointer coordinates. No window activation occurs; `_NET_ACTIVE_WINDOW` is unaffected. | **PASS** |
| 6 | **Empty-region click** | Transparent padding clicks pass through to underlying buttons/text fields. | Carved input hit region using X11 Shape extension (`SK::INPUT`, `SO::SET`). Clicking in transparent padding (e.g. offset 10, 100) generated 0 probe events and passed through to background window. | **PASS** |
| 7 | **Exit** | Right-click quits; finite-duration fallback (`--duration <sec>`) and `WM_DELETE_WINDOW` also work cleanly. | Button 3 (Right click) on body triggers immediate clean exit (exit code 0). `--duration <sec>` automatically exits at timeout. `WM_DELETE_WINDOW` handled cleanly. Wheel scroll (buttons 4/5) and middle click (button 2) are safely ignored. | **PASS** |
| 8 | **Idle** | Probe waits for events/timers rather than spinning continuously. | Monitored process status: 20ms paced poll loop yields CPU time, registering 0.0% CPU and 0.0% memory during idle state. | **PASS** |
| 9 | **Cleanup** | Resources released cleanly on shutdown; desktop interaction restored. | Server-side `Window`, `Colormap`, double-buffered `Pixmap`, and `Gcontext` explicitly destroyed via `.check()?`. Background windows receive input normally without lingering grabs or artifacts. | **PASS** |

---

## Technical Baseline & Nonblocking Findings

1. **Window Manager Integration**:
   - The probe operates successfully as a **managed window** (`override_redirect = false`) under Openbox.
   - Setting `WM_HINTS.input = false`, omitting `WM_TAKE_FOCUS`, and setting `_NET_WM_USER_TIME = 0` provides clean focus immunity in standard click-to-focus Openbox configurations without modifying system-wide settings.
2. **Hit Masking via Shape Extension**:
   - X11 Shape extension 1.1 (`shape_rectangles` on `SK::INPUT`) successfully isolates pointer input to deliberate visual pixels without clipping composited alpha visuals.
3. **Color Space & Premultiplied Alpha**:
   - 32-bit TrueColor visual `0x27f` with Render format `0x25` uses little-endian byte ordering `[B, G, R, A]` matching `Renderer`'s premultiplied memory buffer.
4. **Scope Boundaries**:
   - As required by the specification, passing these checks establishes the X11/Openbox technical foundation for desktop character probes, without claiming that later workspace or multi-layer window ordering tests (A01) are complete.
