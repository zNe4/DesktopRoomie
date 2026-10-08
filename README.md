# DesktopRoomie

DesktopRoomie is the reusable software foundation for a desktop companion and character simulation. Project Vanilla is its private character implementation; character assets, dialogue, voices, models, and private sensor data belong outside this public repository.

The current executable is an experimental Rust/X11 probe in [`experiments/desktop-probe/`](experiments/desktop-probe/), not the complete companion. It provides a procedural transparent body, bounded dragging, a mouse-only menu, and explicit Above / Normal / Below layers without taking typing focus.

The first supported target is **Openbox 3.6.1 on X11 with Picom v13**, recorded on Archcraft. Other window managers, Wayland, and other desktop platforms are unverified. Rendering needs an alpha-capable X11 visual and compositing; the probe uses RENDER, SHAPE, and RandR monitor discovery.

## Build and run

From the repository root with Rust/Cargo installed, in the target X11 session:

```sh
cargo build --manifest-path experiments/desktop-probe/Cargo.toml
cargo run --manifest-path experiments/desktop-probe/Cargo.toml -- --duration 30
cargo run --manifest-path experiments/desktop-probe/Cargo.toml -- --diagnose
```

Left click toggles the test color; left drag moves the body. Right press/release opens Above / Normal / Below / Dismiss / Quit. A matching left press/release selects a row; an outside left click dismisses without replay. A fully covered body has no independent recovery control yet, so finite duration is useful for layer experiments. `--delay 2` delays mapping for a typing-focus check.

Start at the [documentation index](docs/INDEX.md) for current status, implemented architecture, acceptance evidence, and future plans. Contributors should read [AGENTS.md](AGENTS.md) and the assigned mission before editing.
