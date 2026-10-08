# DesktopRoomie — Target Environment Record

This document records the verified runtime host environment on the development machine for **Adventure A00 (Mission M01 Desktop Probe)**, as specified in `docs/DesktopRoomie-A00-M01-desktop-probe.md`.

Recorded: 2026-10-05.

---

## 1. Operating System & Desktop Session

| Property | Value | Notes |
| --- | --- | --- |
| Distribution | Arch Linux (Archcraft) | Kernel 6.x Linux |
| Session Type | X11 | `DISPLAY=:0`, `XDG_SESSION_TYPE=x11` |
| Window Manager | Openbox 3.6.1 | Running via `/usr/bin/openbox` |
| Compositor | Picom v13 (revision d87a5ba) | Running in background (`picom`) |
| Top Panel / Bar | Polybar | Configured via Archcraft default theme |
| Focus Policy | Click-to-focus | Hover alone does not activate or steal focus |
| Virtual Desktops | 5 | Reported by `_NET_NUMBER_OF_DESKTOPS` |
| Current Desktop | 0 | Reported by `_NET_CURRENT_DESKTOP` |

---

## 2. Display & Geometry

| Property | Value | Notes |
| --- | --- | --- |
| Target Main Screen | `eDP-2` (1920×1080 at +0+0) | Primary display for DesktopRoomie |
| Secondary Screen | `HDMI-1-0` (1024×1280 at +1920+0) | External vertical display |
| Combined X11 Spanning | 2944×1280 px | Total root window spanning area |
| Root Window | `0x6ae` | Default root on screen index 0 |
| Root Visual | `0x21` (depth 24 bpp) | Default 24-bit root visual |
| Current Desktop Workarea | `x=10, y=48, w=2924, h=1222` | Spanning workarea for `_NET_CURRENT_DESKTOP` (desktop 0) |
| Usable Area on Main Screen | `x=10, y=48, w=1910, h=1032` | Intersected bounds of main monitor (`eDP-2`) and desktop workarea |
| Probe Initial Geometry | 160×160 at +885+484 | Centered on primary monitor within usable workarea |

---

## 3. Toolchain & Dependencies

| Tool / Library | Version | Notes |
| --- | --- | --- |
| `rustc` | 1.99.0 (b940084d7 2026-09-28) | Host toolchain |
| `cargo` | 1.99.0 (5f94df478 2026-08-27) | Host build tool |
| `~/.cargo/bin` | Present in user `PATH` via `~/.zshrc` | User environment verified |
| `x11rb` crate | 0.14.0 | Features enabled: `render`, `shape`, `randr` |

---

## 4. Host Capabilities Discovered & Verified

- **X11 Server Connection:** Successfully connects via Unix domain socket to `DISPLAY=:0`.
- **Primary Display Discovery:** RandR extension isolates primary screen `eDP-2` (1920×1080 at +0+0).
- **Alpha-Capable Render Visual (M01.2):** Discovered TrueColor 32-bit visual `0x27f` with Render format `0x25` (`direct.alpha_mask == 0xff`).
- **Managed Borderless Window (M01.2):** Created managed window (`override_redirect = false`) with `_MOTIF_WM_HINTS` (`decorations = 0`) and `_NET_WM_WINDOW_TYPE_UTILITY`. Openbox manages window without borders or titlebar.
- **Alpha Blending & Compositing (M01.2):** Verified under Picom: empty region is genuinely transparent, translucent cyan patch blends with underlying desktop windows, and opaque body is crisp.
- **RENDER Extension:** Supported (v0.11), required for alpha composition and blending.
- **SHAPE Extension & Hit Testing (M01.3):** Verified `shape_rectangles` with `SK::INPUT` and `SO::SET` (v1.1). Transparent padding rejects pointer clicks (passing through to underlying windows/desktop), while solid body and translucent patch capture pointer events.
- **Focus Isolation (M01.3):** Verified ICCCM No-Input model (`WM_HINTS` with `input = false` and omitting `WM_TAKE_FOCUS`). Clicking on the probe body does not steal keyboard focus (`_NET_ACTIVE_WINDOW` remains unchanged).
- **Pointer Feedback & Repaint (M01.3):** Handles `EnterNotify`, `LeaveNotify`, `ButtonPress`, and `ButtonRelease`. Left clicking the body toggles palette between Lavender (`#A55FE1`) and Coral (`#F06E50`) with immediate double-buffered blit.

---

## 5. Build and Execution

From the repository root:
```bash
# Run the transparent body probe (indefinite until closed or Ctrl+C)
cargo run --manifest-path experiments/desktop-probe/Cargo.toml

# Run probe with automatic duration timeout (e.g. 5 seconds)
cargo run --manifest-path experiments/desktop-probe/Cargo.toml -- --duration 5

# Run pure diagnostics without rendering (M01.1 mode)
cargo run --manifest-path experiments/desktop-probe/Cargo.toml -- --diagnose
```

## M02 environment reconciliation — recorded 2026-10-07

The owner rediscovered the host during final M02 acceptance at frozen revision `d8dc1abef8b6a8d9395342b234f2f0beb912bc7b`: Archcraft rolling / Linux `7.2.8-arch1-1`, X11 (`DISPLAY=:0`), Openbox 3.6.1, Picom v13 revision `d87a5ba`, and Rust/Cargo 1.99.0. Openbox, Picom, and Polybar were active. The final snapshot had one active monitor, `eDP-2` primary, 1920×1080 at +0+0 (144 Hz), with root 1920×1080, 5 desktops (current 0), and `_NET_WORKAREA` / selected usable area x=10, y=48, w=1900, h=1022.

A secondary monitor was temporarily available during the actual T09 cross-monitor release test, then disconnected before the final single-monitor snapshot. Earlier movement logs used a different desktop configuration/workarea; the final snapshot does not describe the entire acceptance session. The M01 observations above, including their monitor/root/workarea history, remain historical evidence. See [ACCEPTANCE-M02.md](ACCEPTANCE-M02.md) for the integrated results and limitations.

The historical Ctrl+C launch comment describes development/emergency termination, not checked ordinary M02 cleanup; normal checked shutdown is through Quit or configured duration expiry.
