# R00-M01 — NekoAI Linux host refresh and A00-G3 host findings

**Status:** targeted A00 pass complete; A01-G1 revisit only if new host/recovery questions arise  
**Recorded:** 2026-10-08  
**Roadmap gate:** A00-G3 — prove placement and select the host  
**DesktopRoomie baseline:** native Rust + x11rb managed X11 probe, with A00-M02 accepted on Openbox/X11/Picom

## Decision we are about to make

A00-G3 must establish whether the current native X11 host can support the remaining presentation/recovery requirements without a framework change:

1. deliberate Above / Normal / Below placement around ordinary application windows;
2. correct coexistence with focused fullscreen applications;
3. hide/show with a recovery path that does not require clicking the hidden body and does not steal keyboard focus;
4. moving the same instance to another virtual desktop without switching the user's current desktop;
5. a final host decision record that A01 may depend on.

This study is deliberately host-scoped. It does not choose the A01 animation renderer, physics model, Spine integration, or later cross-platform architecture.

## Source audit

### NekoAI

- Repository: https://github.com/nucket/NekoAI
- Revision reviewed: `a85315d9bec0183de0c54d1a4c46eb949e811dcd` (2026-10-07)
- License: MIT
- Relevant paths:
  - `src-tauri/src/lib.rs`
  - `src-tauri/src/cursor_tracker.rs`
  - `src-tauri/src/desktop_monitor.rs`
  - `src-tauri/tauri.conf.json`
  - `src-tauri/tauri.linux.conf.json`
  - `src/PanelWindow.tsx`
  - `src/HouseWindow.tsx`

This refresh supersedes the earlier A00/M02 chat-only snapshot for the host questions below.

### Standards / target-WM references

- Openbox 3.6 actions: https://openbox.org/help/Actions
- EWMH current single-page specification: https://specifications.freedesktop.org/wm/latest-single/
- ICCCM current HTML: https://xorg.freedesktop.org/archive/current/doc/xorg-docs/icccm/icccm.html
- X11 protocol selection ownership: https://www.x.org/releases/X11R7.5/doc/x11proto/proto.pdf
- Xlib `XSendEvent` / selection behavior reference: https://www.x.org/releases/X11R7.6/doc/libX11/specs/libX11/libX11.html

Official specifications establish mechanisms; the Openbox/Picom desktop remains the acceptance authority.

## NekoAI reconstructed host path

The current NekoAI desktop surface is Tauri/WebView-based. Its main and house windows are configured as always-on-top and skip-taskbar. On Linux, the configuration disables native transparency and the Rust side applies GTK visual/input shape regions derived from sprite alpha as a workaround for WebKitGTK transparent-frame ghosting.

The main window exposes Rust commands for position, size, always-on-top and ignore-cursor-events. A separate Tauri panel window is used for the context menu/settings flow. That panel is created focused/always-on-top, and reopening it explicitly calls `set_focus()`.

NekoAI provides independent recovery through a tray icon/menu. Its Show/Hide action hides the main Tauri window or calls a helper that shows it and explicitly focuses it. This is a valid product pattern for NekoAI, but the focus request conflicts with DesktopRoomie's established non-activation requirement.

The source also includes a 500 ms notification-monitor loop and cursor-query machinery, including a Wayland evdev fallback. Those solve NekoAI-specific observation requirements but are not evidence that DesktopRoomie should replace its proven event-driven X11 host.

## Findings for A00-G3

### 1. Above / Normal / Below should be WM policy, not manual restacking

Openbox exposes top, normal and bottom layers. Its top layer is documented as above ordinary windows except fullscreen windows. EWMH standardizes `_NET_WM_STATE_ABOVE` and `_NET_WM_STATE_BELOW`, with normal windows between them.

For an already managed client, DesktopRoomie should request state changes through the normal EWMH `_NET_WM_STATE` client-message path and then observe the resulting window state rather than assuming success.

**Disposition:** adopt the EWMH layer mechanism. Do not build a repeated `ConfigureWindow`/restack fight with the WM.

### 2. Fullscreen probably requires no polling

EWMH's recommended stacking order places focused fullscreen windows above `_NET_WM_STATE_ABOVE`. Openbox documents its top layer as remaining below fullscreen.

This naturally matches the desired A01 policy: a foreground companion may sit above ordinary windows but yield to a focused fullscreen application. When fullscreen ends, the companion can remain Above and naturally become visible over ordinary windows again.

**Disposition:** test native WM behavior first. Do not add fullscreen polling, active-window surveillance, or a focus loop unless real-host evidence disproves this model.

### 3. Workspace movement should use `_NET_WM_DESKTOP`, not change the user's desktop

EWMH separates the window's `_NET_WM_DESKTOP` from root `_NET_CURRENT_DESKTOP`. The A00 operation is "send this existing body to desktop N", not "switch the user to desktop N".

`_NET_WORKAREA` is indexed per desktop. Moving the body to another desktop therefore must validate/clamp against the selected monitor intersected with the target desktop's workarea, rather than reusing only the currently visible desktop's rectangle.

**Disposition:** request the body's target desktop, observe the resulting property/state, and never mutate `_NET_CURRENT_DESKTOP` as part of this action.

### 4. Ordinary Hide should preserve managed-window semantics

EWMH explicitly treats `_NET_WM_STATE_HIDDEN` as WM-derived rather than a normal application-controlled toggle. ICCCM defines the managed transition from Normal to Iconic via `WM_CHANGE_STATE`, and the return from Iconic to Normal by mapping the window again. A raw client withdrawal has additional ICCCM semantics and may cause WM properties/state to be removed.

**Disposition:** the first A00 hide experiment should use managed iconification and checked restoration, not a withdrawn/unmanaged lifecycle. Cancellation must release any pointer/menu ownership before hiding. After restore, wait for actual map/state evidence and re-check geometry/bounds.

Whether `SKIP_TASKBAR` / `SKIP_PAGER` should accompany the companion is an A00-M03 real-host test question, not an assumed requirement.

### 5. Independent recovery can remain X11-native for A00

A hidden or completely covered body cannot rely on its own right-click menu. X11 selections offer a server-global owner that is automatically cleared if the owning client/window disappears. A tiny dedicated control window owned by the running DesktopRoomie instance can own a private selection such as `_DESKTOPROOMIE_INSTANCE_S0`.

A second invocation could query that selection owner and, if present, send a small private `ClientMessage` directly to the control window. X11 direct `SendEvent` with an empty event mask targets the client that created the destination window.

Candidate A00 developer commands:

```text
desktop-probe --show
desktop-probe --hide
desktop-probe --recover
desktop-probe --layer above
desktop-probe --layer normal
desktop-probe --layer below
desktop-probe --desktop <N>
```

`--recover` should be allowed to mean a stronger explicit-user operation than Show: make the existing instance visible, move it to the current desktop if necessary, choose a safe placement, and put it in a recoverable foreground layer without requesting keyboard activation.

**Disposition:** this is the preferred A00 experiment over a custom global hotkey, a new daemon, or a portable IPC framework. It remains an X11 host protocol, not a commitment to the future Android/Wayland control architecture.

### 6. NekoAI reinforces, rather than challenges, the native-host choice

NekoAI demonstrates useful product patterns—especially an independent tray recovery path—but its present host also brings behavior DesktopRoomie does not need or actively rejects for the target:

- Tauri/WebView/GTK surface ownership instead of the already proven native X11 body;
- Linux shape/chroma-key workarounds for WebKitGTK transparency;
- always-on-top as the default presentation model;
- focused panel and focused Show behavior;
- background polling for unrelated observation features.

DesktopRoomie already has real-host evidence for native alpha rendering, input shaping, non-activation, pointer capture/release, event-driven idle waiting, checked cleanup, and selected-monitor bounds.

**Decision:** retain Rust + x11rb + a managed native X11 body through A00 and into A01. Do not switch to Tauri/GTK on current evidence. Re-open the host decision only if A00-G3 real-host evidence reveals an unresolvable missing capability.

## Proposed A00-M03 validation

### M03.1 — layers and fullscreen

- request Above / Normal / Below on the managed body;
- verify resulting EWMH state and real stacking against at least two ordinary windows;
- preserve typing focus throughout;
- put an ordinary application into real fullscreen and confirm Above yields naturally;
- exit fullscreen and confirm the prior layer behavior returns without polling or activation;
- record unsupported WM behavior rather than compensating with a restack loop.

### M03.2 — hide/show and independent recovery

- cancel an active body/menu gesture before Hide;
- request managed iconification and verify the body becomes non-viewable without orphan capture;
- recover from an independent second invocation/control command while focus stays with the previous app;
- test fully covered and hidden cases;
- test stale/dead selection ownership and duplicate-launch behavior;
- decide whether skip-taskbar / skip-pager states improve the companion experience on this Openbox target.

### M03.3 — workspace placement

- send one existing instance to another of the five desktops without changing `_NET_CURRENT_DESKTOP`;
- verify no duplicate body and no activation;
- compute target-desktop usable bounds and reconcile placement;
- recover the same instance deliberately to the current workspace;
- test stale/invalid desktop indices and workspace-count changes through deterministic seams where safe.

### M03.4 — integrated host decision

Record the supported Openbox/X11/Picom contract, required configuration, layer/fullscreen behavior, hide/recovery path, workspace behavior, known limitations, and reusable X11 host boundaries. A00 completes only after this evidence passes.

## Deferred / explicitly out of scope

- A01 user-vs-character layer policy and persistence;
- general-purpose IPC or cross-platform control architecture;
- Wayland support;
- window-platform/perching behavior;
- A01 gravity, autonomous motion or animation scheduling;
- Spine 2D integration.

The newly available private official Vanilla Spine 2D export is important for A01-G2 but does not alter the A00 host proof. Its feasibility is now tracked separately as R00-M12.
