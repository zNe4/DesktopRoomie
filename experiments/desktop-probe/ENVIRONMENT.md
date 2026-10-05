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

---

## 3. Toolchain & Dependencies

| Tool / Library | Version | Notes |
| --- | --- | --- |
| `rustc` | 1.99.0 (b940084d7 2026-09-28) | Host toolchain |
| `cargo` | 1.99.0 (5f94df478 2026-08-27) | Host build tool |
| `~/.cargo/bin` | Present in user `PATH` via `~/.zshrc` | User environment verified |
| `x11rb` crate | 0.14.0 | Features enabled: `render`, `shape`, `randr` |

---

## 4. Host Capabilities Discovered

Running the M01.1 desktop-probe verifies:
- **X11 Server Connection:** Successfully connects via Unix domain socket to `DISPLAY=:0`.
- **Primary Display Discovery:** RandR extension isolates primary screen `eDP-2` (1920×1080 at +0+0).
- **Candidate 32-bit Visuals:** 16 candidate 32-bit TrueColor visuals detected (alpha-capable Render format verification will be performed in M01.2).
- **RENDER Extension:** Supported (v0.11), required for alpha composition and blending in M01.2.
- **SHAPE Extension:** Supported (v1.1), required for bounding and input shaping in M01.3.
- **Workarea Selection:** Properly indexes `_NET_WORKAREA` by `_NET_CURRENT_DESKTOP` and computes usable bounds on the primary display.

---

## 5. Build and Execution

From the repository root:
```bash
# Build the probe
cargo build --manifest-path experiments/desktop-probe/Cargo.toml

# Run the probe diagnostics
cargo run --manifest-path experiments/desktop-probe/Cargo.toml

# Negative test (simulated failure)
DISPLAY=:99 cargo run --manifest-path experiments/desktop-probe/Cargo.toml
```

Alternatively, from within the experiment directory:
```bash
cd experiments/desktop-probe
cargo run
```
