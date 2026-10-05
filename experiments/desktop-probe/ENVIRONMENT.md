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

---

## 2. Display & Geometry

| Property | Value | Notes |
| --- | --- | --- |
| Root Window | `0x6ae` | Default root on screen index 0 |
| Root Visual | `0x21` (depth 24 bpp) | Default 24-bit root visual |
| Current Dimensions | 2944×1280 px (779×339 mm) | Combined desktop display geometry |
| Workarea (`_NET_WORKAREA`) | `x=10, y=48, w=2924, h=1222` | Accounts for 48px top Polybar and 10px screen margins |

---

## 3. Toolchain & Dependencies

| Tool / Library | Version | Notes |
| --- | --- | --- |
| `rustc` | 1.99.0 (b940084d7 2026-09-28) | Host toolchain |
| `cargo` | 1.99.0 (5f94df478 2026-08-27) | Host build tool |
| `~/.cargo/bin` | Present in user `PATH` via `~/.zshrc` | User environment verified |
| `x11rb` crate | 0.14.0 | Features enabled: `render`, `shape` |

---

## 4. Host Capabilities Discovered

Running the M01.1 desktop-probe verifies:
- **X11 Server Connection:** Successfully connects via Unix domain socket to `DISPLAY=:0`.
- **Root Screen:** Default screen index 0, Root window 0x6ae, Depth 24 bpp.
- **Allowed Depths:** `[24, 1, 4, 8, 15, 16, 32]`
- **32-bit TrueColor Visuals:** 16 found (confirmed suitable for M01.2 ARGB transparent window composition).
- **RENDER Extension:** Supported (v0.11), required for alpha composition and blending in M01.2.
- **SHAPE Extension:** Supported (v1.1), required for bounding and input shaping in M01.3.

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
