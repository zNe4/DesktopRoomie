mod geometry;
mod interaction;
mod x11;

use std::env;
use std::os::unix::io::AsRawFd;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::render::ConnectionExt as RenderExt;
use x11rb::protocol::shape::ConnectionExt as ShapeExt;
use x11rb::protocol::xproto::{GrabStatus, VisualClass};
use x11rb::protocol::Event;

use crate::geometry::{calculate_centered_origin, compute_valid_origin_bounds, Point, Size};
use crate::interaction::{HostAction, InteractionManager};
use crate::x11::monitors::query_desktop_layout;
use crate::x11::pointer::{grab_pointer, PointerCaptureTracker};
use crate::x11::render::{Renderer, WINDOW_HEIGHT, WINDOW_WIDTH};
use crate::x11::visual::find_alpha_visual;
use crate::x11::window::{ConfigureReconciliation, ManagedProbeWindow};

struct Config {
    diagnose: bool,
    duration_secs: Option<u64>,
    delay_secs: Option<u64>,
}

#[repr(C)]
struct PollFd {
    fd: std::os::raw::c_int,
    events: std::os::raw::c_short,
    revents: std::os::raw::c_short,
}

const POLLIN: std::os::raw::c_short = 0x0001;
const CONFIRMATION_TIMEOUT: Duration = Duration::from_millis(150);

extern "C" {
    fn poll(fds: *mut PollFd, nfds: usize, timeout: std::os::raw::c_int) -> std::os::raw::c_int;
}

fn parse_args() -> Result<Config, String> {
    let mut diagnose = false;
    let mut duration_secs = None;
    let mut delay_secs = None;

    let args: Vec<String> = env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_help();
                process::exit(0);
            }
            "--diagnose" => {
                diagnose = true;
            }
            "--delay" => {
                i += 1;
                if i >= args.len() {
                    return Err("Missing value for --delay <seconds>".to_string());
                }
                let val: u64 = args[i]
                    .parse()
                    .map_err(|_| format!("Invalid delay value: '{}'", args[i]))?;
                delay_secs = Some(val);
            }
            "--duration" => {
                i += 1;
                if i >= args.len() {
                    return Err("Missing value for --duration <seconds>".to_string());
                }
                let val: u64 = args[i]
                    .parse()
                    .map_err(|_| format!("Invalid duration value: '{}'", args[i]))?;
                duration_secs = Some(val);
            }
            unknown => {
                return Err(format!(
                    "Unknown option: '{}'. Use --help for usage.",
                    unknown
                ));
            }
        }
        i += 1;
    }

    Ok(Config {
        diagnose,
        duration_secs,
        delay_secs,
    })
}

fn print_help() {
    println!("DesktopRoomie - A00-M01: Desktop Probe");
    println!();
    println!("USAGE:");
    println!("  desktop-probe [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("  -h, --help              Show this help message and exit");
    println!("  --diagnose              Run pure X11 environment diagnostics and exit");
    println!("  --delay <SECONDS>       Delay in seconds before mapping window (for typing test)");
    println!("  --duration <SECONDS>    Run for a specified duration in seconds, then exit");
}

fn main() {
    let config = match parse_args() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("[ERROR] {}", err);
            process::exit(1);
        }
    };

    println!("=== DesktopRoomie A00-M01: Desktop Probe ===");

    // Connect to X11 display session
    let (conn, screen_num) = match x11rb::connect(None) {
        Ok(res) => res,
        Err(err) => {
            eprintln!("\n[ERROR] Failed to connect to X11 display: {}", err);
            match env::var("DISPLAY") {
                Ok(val) if !val.is_empty() => {
                    eprintln!("  DISPLAY is set to '{}'.", val);
                    eprintln!("  Possible causes: X server is not running, DISPLAY is incorrect, or XAUTHORITY permissions failed.");
                }
                _ => {
                    eprintln!("  DISPLAY environment variable is not set or empty.");
                    eprintln!("  Ensure you are running within an active X11 graphical session.");
                }
            }
            process::exit(1);
        }
    };

    let setup = conn.setup();
    let screen = match setup.roots.get(screen_num) {
        Some(s) => s,
        None => {
            eprintln!("[ERROR] Invalid default screen number: {}", screen_num);
            process::exit(1);
        }
    };

    let atoms = match x11::monitors::LayoutAtoms::subscribe(&conn, screen.root) {
        Ok(atoms) => atoms,
        Err(error) => {
            eprintln!("[ERROR] Geometry subscription failed: {error}");
            process::exit(1);
        }
    };
    let layout = match query_desktop_layout(&conn, screen.root, &atoms, None) {
        Ok(layout) => layout,
        Err(error) => {
            eprintln!("[ERROR] Desktop geometry query failed: {error}");
            process::exit(1);
        }
    };
    let selected_monitor = layout
        .primary_monitor
        .as_ref()
        .expect("validated monitor")
        .name_atom;

    // If pure diagnosis requested, print diagnostics and exit cleanly (M01.1 mode)
    if config.diagnose {
        run_diagnostics(&conn, screen, &layout);
        println!("\n[Result]");
        println!("  M01.1: X11 connection established; diagnostics completed.");
        return;
    }

    // M02.5 retains bounded dragging and adds interruption recovery.
    println!("Mission M02.5: interruption recovery and changing bounds...");

    // 1. Discover 32-bit alpha Render visual
    let alpha_vis = match find_alpha_visual(&conn) {
        Ok(vis) => vis,
        Err(err) => {
            eprintln!("\n[ERROR] Alpha visual discovery failed: {}", err);
            process::exit(1);
        }
    };

    println!("\n[Visual Selection]");
    println!(
        "  Discovered 32-bit ARGB visual: 0x{:x} (Render PictFormat: 0x{:x})",
        alpha_vis.visual_id, alpha_vis.pict_format
    );
    println!(
        "  Verified channel layout: ARGB (A: mask=0x{:x} shift={}, R: mask=0x{:x} shift={}, G: mask=0x{:x} shift={}, B: mask=0x{:x} shift={})",
        alpha_vis.alpha_mask,
        alpha_vis.alpha_shift,
        alpha_vis.red_mask,
        alpha_vis.red_shift,
        alpha_vis.green_mask,
        alpha_vis.green_shift,
        alpha_vis.blue_mask,
        alpha_vis.blue_shift,
    );

    // 2. Validate geometry & bounds
    let body_size = Size::new(WINDOW_WIDTH as u32, WINDOW_HEIGHT as u32);
    let valid_bounds = match checked_probe_bounds(layout.usable_area) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("\n[ERROR] Bounded placement validation failed: {}", e);
            process::exit(1);
        }
    };

    let requested_origin = match calculate_centered_origin(layout.usable_area, body_size) {
        Ok(pt) => pt,
        Err(e) => {
            eprintln!("\n[ERROR] Failed to calculate centered origin: {}", e);
            process::exit(1);
        }
    };

    if let Some(ref pm) = layout.primary_monitor {
        println!(
            "  Primary display: '{}' ({}x{} at +{}+{})",
            pm.name, pm.width, pm.height, pm.x, pm.y
        );
    }
    println!(
        "  Usable area on main screen (desktop {}): x={}, y={}, w={}, h={}",
        layout.current_desktop,
        layout.usable_area.x,
        layout.usable_area.y,
        layout.usable_area.width,
        layout.usable_area.height
    );
    println!("  Valid origin bounds: {}", valid_bounds);
    if let Some(monitor) = &layout.primary_monitor {
        println!(
            "  Selected monitor identity: atom=0x{:x}, outputs={:?}",
            monitor.name_atom, monitor.outputs
        );
    }

    // Optional startup delay (for testing focus behavior during mapping)
    if let Some(delay) = config.delay_secs {
        println!("\n[Startup Delay]");
        println!(
            "  Waiting {} second(s) before creating and mapping probe window...",
            delay
        );
        thread::sleep(Duration::from_secs(delay));
    }

    // 3. Create managed borderless window
    let mut probe_window =
        match ManagedProbeWindow::create(&conn, screen.root, alpha_vis.visual_id, requested_origin)
        {
            Ok(w) => w,
            Err(e) => {
                eprintln!("[ERROR] Failed to create managed probe window: {}", e);
                process::exit(1);
            }
        };

    println!("\n[Window Creation]");
    println!(
        "  Window created: ID=0x{:x}, initial configured geometry={}x{} at +{}+{}",
        probe_window.window,
        probe_window.width,
        probe_window.height,
        probe_window.x(),
        probe_window.y()
    );
    println!("  Requested startup placement: {}", requested_origin);
    println!("  Window properties: borderless (_MOTIF_WM_HINTS), managed, type=UTILITY");
    println!("  Input shape: X11 Shape extension applied to body silhouette and test patch");
    println!("  Awaiting MapNotify confirmation from window manager...");

    // 4. Initialize double-buffered renderer
    let mut renderer = match Renderer::new(&conn, probe_window.window, probe_window.colormap) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[ERROR] Failed to initialize renderer: {}", e);
            if let Err(cleanup) = probe_window.destroy(&conn) {
                eprintln!("[ERROR] Window cleanup failed: {cleanup}");
            }
            process::exit(1);
        }
    };

    // Perform initial paint (fatal on failure)
    if let Err(e) = renderer.paint(&conn, probe_window.window) {
        eprintln!("[ERROR] Initial paint failed: {}", e);
        let _ = x11::resource::cleanup_all([
            Box::new(|| renderer.destroy(&conn)),
            Box::new(|| probe_window.destroy(&conn)),
        ]);
        process::exit(1);
    }

    println!("\n[Running Probe]");
    if let Some(sec) = config.duration_secs {
        println!("  Probe running for {} seconds (or until closed)...", sec);
    } else {
        println!(
            "  Probe is visible on desktop. Left-click to toggle color, right-click to exit, or press Ctrl+C."
        );
    }

    let mut runtime = ProbeRuntime::new();
    let result = run_probe(
        &conn,
        screen.root,
        &config,
        &atoms,
        selected_monitor,
        &mut probe_window,
        &mut renderer,
        &mut runtime,
        valid_bounds,
    );
    let outcome = finish_probe_with(result, || {
        runtime.correction = None;
        let cancellation = runtime.cancel(&conn, &mut probe_window, "shutdown");
        let cleanup = x11::resource::cleanup_all([
            Box::new(|| cancellation),
            Box::new(|| renderer.destroy(&conn)),
            Box::new(|| probe_window.destroy(&conn)),
        ]);
        if runtime.pointer.is_grabbed() {
            eprintln!("[ERROR] Pointer release remains unconfirmed; connection teardown is best effort only");
        }
        cleanup
    });
    if outcome.is_err() {
        process::exit(1);
    }
    println!("  Probe exited cleanly.");
}

/// Cleanup always runs; a fatal host error retains diagnostic priority over cleanup failures.
fn finish_probe_with(
    result: Result<(), HostError>,
    cleanup: impl FnOnce() -> Result<(), HostError>,
) -> Result<(), HostError> {
    if let Err(error) = &result {
        eprintln!("[ERROR] {error}");
    }
    let cleanup_result = cleanup();
    result.and(cleanup_result)
}

fn checked_probe_bounds(
    area: crate::geometry::Rect,
) -> Result<crate::geometry::ValidOriginBounds, HostError> {
    let bounds =
        compute_valid_origin_bounds(area, Size::new(WINDOW_WIDTH.into(), WINDOW_HEIGHT.into()))?;
    for coordinate in [bounds.min_x, bounds.max_x, bounds.min_y, bounds.max_y] {
        i16::try_from(coordinate)
            .map_err(|_| "Selected-monitor placement exceeds supported X11 coordinate range")?;
    }
    Ok(bounds)
}

type HostError = Box<dyn std::error::Error>;
const POINTER_CHECK_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyAvailability {
    AwaitingMap,
    Unavailable,
    Validating,
    Ready,
    Destroyed,
}

#[derive(Debug, PartialEq, Eq)]
enum SafetyDecision {
    NoChange,
    HoldConfirmed,
    Cancel,
}

#[derive(Default)]
struct GestureSafety {
    generation: u64,
    deadline: Option<Instant>,
    observation: Option<(u64, bool, u64)>,
}
impl GestureSafety {
    fn acquired(&mut self, now: Instant) {
        self.generation = self.generation.wrapping_add(1);
        self.deadline = Some(now + POINTER_CHECK_INTERVAL);
        self.observation = None;
    }
    fn clear(&mut self) {
        self.deadline = None;
        self.observation = None;
    }
    fn observed(&mut self, pressed: bool, sequence: u64) {
        self.observation = Some((self.generation, pressed, sequence));
        self.deadline = None;
    }
    // Events with a last-processed request sequence below QueryPointer precede the
    // observation. Apply it before later events, or once the queue is empty.
    fn apply(&mut self, active: bool, now: Instant, next_sequence: Option<u64>) -> SafetyDecision {
        if self
            .observation
            .is_some_and(|(_, _, sequence)| next_sequence.is_some_and(|next| next < sequence))
        {
            return SafetyDecision::NoChange;
        }
        let Some((generation, pressed, _)) = self.observation.take() else {
            return SafetyDecision::NoChange;
        };
        if !active || generation != self.generation {
            return SafetyDecision::NoChange;
        }
        if pressed {
            self.deadline = Some(now + POINTER_CHECK_INTERVAL);
            SafetyDecision::HoldConfirmed
        } else {
            self.clear();
            SafetyDecision::Cancel
        }
    }
}

struct ProbeRuntime {
    interaction: InteractionManager,
    pointer: PointerCaptureTracker,
    pending_move: Option<Point>,
    final_target: Option<Point>,
    confirmation_deadline: Option<Instant>,
    safety: GestureSafety,
    body: BodyAvailability,
    correction: Option<(Point, Instant)>,
}
impl ProbeRuntime {
    fn new() -> Self {
        Self {
            interaction: InteractionManager::new(),
            pointer: PointerCaptureTracker::new(),
            pending_move: None,
            final_target: None,
            confirmation_deadline: None,
            safety: GestureSafety::default(),
            body: BodyAvailability::AwaitingMap,
            correction: None,
        }
    }

    fn apply_safety(&mut self, now: Instant, next_sequence: Option<u64>) -> bool {
        match self
            .safety
            .apply(self.interaction.has_left_gesture(), now, next_sequence)
        {
            SafetyDecision::NoChange => false,
            SafetyDecision::HoldConfirmed => {
                self.interaction.confirm_button_held();
                false
            }
            SafetyDecision::Cancel => true,
        }
    }

    fn cancel_with(
        &mut self,
        window: &mut ManagedProbeWindow,
        release: impl FnOnce(&mut PointerCaptureTracker) -> Result<(), HostError>,
    ) -> Result<(), HostError> {
        self.safety.clear();
        cancel_movement_with(
            &mut self.interaction,
            &mut self.pending_move,
            &mut self.final_target,
            &mut self.confirmation_deadline,
            window,
            |_| release(&mut self.pointer),
        )
    }

    fn cancel(
        &mut self,
        conn: &impl Connection,
        window: &mut ManagedProbeWindow,
        reason: &str,
    ) -> Result<(), HostError> {
        if self.interaction.has_left_gesture() || self.pointer.is_grabbed() {
            println!("[INPUT] Cancelling gesture: {reason}");
        }
        self.cancel_with(window, |pointer| {
            pointer
                .release_if_held(conn, x11rb::CURRENT_TIME)
                .map(|_| ())
        })
    }
    fn unavailable(&mut self, destroyed: bool) {
        self.body = if destroyed {
            BodyAvailability::Destroyed
        } else {
            BodyAvailability::Unavailable
        };
        self.correction = None;
    }
    fn flush_move(
        &mut self,
        conn: &impl Connection,
        window: &mut ManagedProbeWindow,
    ) -> Result<(), HostError> {
        if let Some(target) = self.pending_move.take() {
            if target != window.requested_origin() {
                window.configure_position(conn, target)?;
                self.confirmation_deadline = Some(Instant::now() + CONFIRMATION_TIMEOUT);
            }
        }
        Ok(())
    }
    fn accepts_input(&self) -> bool {
        self.body == BodyAvailability::Ready
    }
    fn can_reconcile(&self) -> bool {
        !matches!(
            self.body,
            BodyAvailability::AwaitingMap
                | BodyAvailability::Unavailable
                | BodyAvailability::Destroyed
        ) && self.correction.is_none()
    }
    fn placement(
        &mut self,
        conn: &impl Connection,
        root: u32,
        window: &mut ManagedProbeWindow,
        bounds: &crate::geometry::ValidOriginBounds,
    ) -> Result<(), HostError> {
        if !self.can_reconcile() {
            return Ok(());
        }
        let actual = window.query_actual_root_geometry(conn, root)?;
        self.placement_with(window, actual, bounds, |window, target| {
            window.configure_position(conn, target)
        })
    }
    fn placement_with(
        &mut self,
        window: &mut ManagedProbeWindow,
        actual: crate::geometry::Rect,
        bounds: &crate::geometry::ValidOriginBounds,
        request: impl FnOnce(&mut ManagedProbeWindow, Point) -> Result<(), HostError>,
    ) -> Result<(), HostError> {
        if !self.can_reconcile() {
            return Ok(());
        }
        if actual.size() != Size::new(WINDOW_WIDTH.into(), WINDOW_HEIGHT.into()) {
            return Err(format!(
                "Unsupported body dimensions on placement verification: {}",
                actual.size()
            )
            .into());
        }
        window.confirmed_origin = actual.origin();
        window.cancel_in_flight_moves();
        let target = bounds.clamp(actual.origin());
        if target == actual.origin() {
            self.body = BodyAvailability::Ready;
        } else {
            self.body = BodyAvailability::Validating;
            request(window, target)?;
            self.correction = Some((target, Instant::now() + CONFIRMATION_TIMEOUT));
            println!("[PLACEMENT] Changed bounds require one corrective placement to {target}; awaiting verification");
        }
        Ok(())
    }
    fn check_correction(
        &mut self,
        now: Instant,
        actual: Point,
        bounds: &crate::geometry::ValidOriginBounds,
    ) -> Result<bool, HostError> {
        let Some((target, deadline)) = self.correction else {
            return Ok(false);
        };
        if now < deadline {
            return Ok(false);
        }
        self.correction = None;
        if actual != target {
            return Err(format!("WM refused necessary bounds correction: requested {target}, confirmed {actual}; will not retry").into());
        }
        self.body = if bounds.clamp(actual) == actual {
            BodyAvailability::Ready
        } else {
            BodyAvailability::Validating
        };
        Ok(true)
    }
}

fn relevant_layout_event(
    event: &Event,
    root: u32,
    window: u32,
    atoms: &x11::monitors::LayoutAtoms,
) -> bool {
    match event {
        Event::RandrScreenChangeNotify(_) | Event::RandrNotify(_) => true,
        Event::ConfigureNotify(ev) => ev.window == root,
        Event::PropertyNotify(ev) => {
            (ev.window == root && (ev.atom == atoms.workarea || ev.atom == atoms.current_desktop))
                || (ev.window == window && ev.atom == atoms.wm_desktop)
        }
        _ => false,
    }
}
fn interrupts_pending_movement(
    event: &Event,
    root: u32,
    window: u32,
    atoms: &x11::monitors::LayoutAtoms,
) -> bool {
    relevant_layout_event(event, root, window, atoms)
        || match event {
            Event::ButtonPress(ev) => ev.event == window && ev.detail == 3,
            Event::UnmapNotify(ev) => ev.window == window,
            Event::DestroyNotify(ev) => ev.window == window,
            Event::ClientMessage(ev) => ev.window == window,
            Event::Error(_) => true,
            _ => false,
        }
}

#[allow(clippy::too_many_arguments)]
fn run_probe(
    conn: &x11rb::rust_connection::RustConnection,
    root: u32,
    config: &Config,
    atoms: &x11::monitors::LayoutAtoms,
    selected_monitor: u32,
    window: &mut ManagedProbeWindow,
    renderer: &mut Renderer,
    runtime: &mut ProbeRuntime,
    initial_bounds: crate::geometry::ValidOriginBounds,
) -> Result<(), HostError> {
    let mut bounds = initial_bounds;
    let mut buffered_event = None;
    let start = Instant::now();
    let duration_deadline = config
        .duration_secs
        .map(|sec| {
            start
                .checked_add(Duration::from_secs(sec))
                .ok_or("Duration exceeds supported monotonic deadline")
        })
        .transpose()?;
    loop {
        let now = Instant::now();
        if duration_deadline.is_some_and(|dl| now >= dl) {
            runtime.cancel(conn, window, "duration expiry")?;
            return Ok(());
        }
        // Confirmation is bounded even during sustained incoming event traffic.
        if let Some((_, deadline)) = runtime.correction {
            if now >= deadline {
                if !window.is_viewable(conn)? {
                    runtime.cancel(
                        conn,
                        window,
                        "body unavailable during correction verification",
                    )?;
                    runtime.unavailable(false);
                    continue;
                }
                let actual = window.query_actual_root_geometry(conn, root)?;
                if actual.size() != Size::new(WINDOW_WIDTH.into(), WINDOW_HEIGHT.into()) {
                    return Err("Body resized during bounds correction".into());
                }
                runtime.check_correction(now, actual.origin(), &bounds)?;
                window.cancel_in_flight_moves();
                runtime.placement(conn, root, window, &bounds)?;
            }
        } else {
            let window_id = window.window;
            check_confirmation_timeout_with(
                &mut runtime.confirmation_deadline,
                window,
                &mut runtime.final_target,
                &mut runtime.interaction,
                &mut runtime.pending_move,
                || {
                    let actual = ManagedProbeWindow::query_window_actual_root_geometry(
                        conn, window_id, root,
                    )?;
                    if actual.size() != Size::new(WINDOW_WIDTH.into(), WINDOW_HEIGHT.into()) {
                        return Err("Body resized during movement verification".into());
                    }
                    Ok(actual.origin())
                },
                |time| runtime.pointer.release_if_held(conn, time).map(|_| ()),
            )?;
            if !runtime.interaction.has_left_gesture() {
                runtime.safety.clear();
            }
        }
        if runtime.safety.deadline.is_some_and(|dl| now >= dl) {
            let (sequence, pressed) = crate::x11::pointer::left_button_pressed(conn, root)?;
            runtime.safety.observed(pressed, sequence);
        }

        let batch = drain_events_bounded(&mut buffered_event, 64, || {
            conn.poll_for_event_with_sequence()
        })?;
        for (event, sequence) in batch {
            if matches!(&event, Event::DestroyNotify(ev) if ev.window == window.window) {
                window.window_resource.externally_destroyed();
            }
            if runtime.apply_safety(Instant::now(), Some(sequence)) {
                runtime.cancel(
                    conn,
                    window,
                    "QueryPointer confirmed initiating button released",
                )?;
            }
            let right_cancelled = matches!(&event, Event::ButtonPress(ev) if ev.event == window.window && ev.detail == 3)
                && runtime.interaction.has_left_gesture();
            if matches!(&event, Event::ButtonPress(ev) if ev.event == window.window && ev.detail == 3 && !runtime.interaction.accepts_gesture_time(ev.time))
            {
                continue;
            }
            if interrupts_pending_movement(&event, root, window.window, atoms) {
                runtime.cancel(conn, window, "host or right-button interruption")?;
            } else if !matches!(event, Event::MotionNotify(_)) {
                runtime.flush_move(conn, window)?;
            }
            if relevant_layout_event(&event, root, window.window, atoms) {
                let layout = query_desktop_layout(conn, root, atoms, Some(selected_monitor))?;
                bounds = checked_probe_bounds(layout.usable_area)?;
                if !matches!(
                    runtime.body,
                    BodyAvailability::AwaitingMap | BodyAvailability::Unavailable
                ) {
                    if window.verified_visible(conn, layout.current_desktop, atoms.wm_desktop)? {
                        runtime.body = BodyAvailability::Validating;
                        runtime.placement(conn, root, window, &bounds)?;
                    } else {
                        runtime.unavailable(false);
                    }
                }
                continue;
            }
            match event {
                Event::MapNotify(ev) if ev.window == window.window => {
                    runtime.cancel(conn, window, "map verification")?;
                    let layout = query_desktop_layout(conn, root, atoms, Some(selected_monitor))?;
                    bounds = checked_probe_bounds(layout.usable_area)?;
                    if window.verified_visible(conn, layout.current_desktop, atoms.wm_desktop)? {
                        runtime.body = BodyAvailability::Validating;
                        runtime.placement(conn, root, window, &bounds)?;
                        println!("[PLACEMENT] Verified map at {}", window.confirmed_origin());
                    } else {
                        runtime.unavailable(false);
                    }
                }
                Event::UnmapNotify(ev) if ev.window == window.window => {
                    runtime.unavailable(false);
                }
                Event::DestroyNotify(ev) if ev.window == window.window => {
                    window.window_resource.externally_destroyed();
                    runtime.unavailable(true);
                    println!("[LIFECYCLE] Body externally destroyed; shutting down");
                    return Ok(());
                }
                Event::ConfigureNotify(ev) if ev.window == window.window => {
                    if matches!(
                        runtime.body,
                        BodyAvailability::Unavailable | BodyAvailability::AwaitingMap
                    ) {
                        continue;
                    }
                    match window.handle_configure_notify(conn, root, &ev)? {
                        ConfigureReconciliation::UnexpectedSizeChange { expected, actual } => {
                            return Err(format!(
                                "WM altered body dimensions: expected {expected}, actual {actual}"
                            )
                            .into())
                        }
                        _ if runtime.correction.is_some() => {} // fixed deadline verifies the live position
                        ConfigureReconciliation::Confirmed { origin, .. } => {
                            if window.in_flight_moves.is_empty() {
                                if runtime.final_target.is_some_and(|target| target != origin) {
                                    runtime.cancel(conn, window, "final-position mismatch")?;
                                } else {
                                    runtime.final_target = None;
                                    runtime.confirmation_deadline = None;
                                }
                            }
                        }
                        ConfigureReconciliation::GenuineMismatch {
                            requested,
                            confirmed,
                        } => {
                            eprintln!("[WARN] WM adjusted movement: requested {requested}, confirmed {confirmed}");
                            runtime.cancel(conn, window, "WM movement mismatch")?;
                        }
                        ConfigureReconciliation::InFlightCatchUp { .. } => {
                            runtime.confirmation_deadline =
                                Some(Instant::now() + CONFIRMATION_TIMEOUT);
                        }
                        ConfigureReconciliation::StaleHistorical { .. } => {}
                    }
                }
                Event::MotionNotify(ev) if ev.event == window.window && runtime.accepts_input() => {
                    let was_dragging = runtime.interaction.is_dragging();
                    if let HostAction::MoveWindow { target } = runtime.interaction.handle_motion(
                        Point::new(ev.root_x.into(), ev.root_y.into()),
                        ev.time,
                        &bounds,
                    ) {
                        if !was_dragging {
                            println!("[INPUT] Drag started; target origin {target}");
                        }
                        runtime.pending_move = Some(target);
                    }
                }
                Event::ButtonPress(ev) if ev.event == window.window && runtime.accepts_input() => {
                    use x11rb::protocol::xproto::KeyButMask;
                    match ev.detail {
                        1 if !ev.state.contains(KeyButMask::BUTTON1) => {
                            let origin = window.query_actual_root_origin(conn, root)?;
                            if let HostAction::AcquireGrab { time } =
                                runtime.interaction.handle_left_press(
                                    Point::new(ev.root_x.into(), ev.root_y.into()),
                                    (ev.event_x, ev.event_y),
                                    ev.time,
                                    origin,
                                )
                            {
                                match grab_pointer(conn, window.window, time)? {
                                    GrabStatus::SUCCESS => {
                                        runtime.pointer.set_grabbed(true);
                                        runtime.interaction.on_grab_acquired();
                                        runtime.safety.acquired(Instant::now());
                                        runtime.final_target = None;
                                        println!("[INPUT] Left gesture captured");
                                    }
                                    status => {
                                        runtime.interaction.on_grab_denied();
                                        eprintln!("[WARN] Pointer capture denied: {status:?}");
                                    }
                                }
                            }
                        }
                        // The common pre-dispatch cancellation consumed right-cancel. Only a
                        // fresh idle right press with left physically up can request M02.4 quit.
                        3 if !right_cancelled
                            && !ev.state.contains(KeyButMask::BUTTON1)
                            && runtime.interaction.handle_right_press(ev.time)
                                == HostAction::ExitCleanly =>
                        {
                            return Ok(());
                        }
                        _ => {}
                    }
                }
                Event::ButtonRelease(ev) if ev.event == window.window => {
                    if ev.detail != 1 {
                        continue;
                    }
                    let action = runtime.interaction.handle_left_release(
                        Point::new(ev.root_x.into(), ev.root_y.into()),
                        (ev.event_x, ev.event_y),
                        ev.time,
                        &bounds,
                    );
                    match action {
                        HostAction::ReleaseGrabAndMoveWindow { time, target } => {
                            if target != window.requested_origin() {
                                window.configure_position(conn, target)?;
                            }
                            runtime.pointer.release_if_held(conn, time)?;
                            runtime.safety.clear();
                            if window.in_flight_moves.is_empty()
                                && window.confirmed_origin() == target
                            {
                                runtime.final_target = None;
                                runtime.confirmation_deadline = None;
                            } else {
                                runtime.final_target = Some(target);
                                runtime.confirmation_deadline =
                                    Some(Instant::now() + CONFIRMATION_TIMEOUT);
                            }
                            println!("[INPUT] Drag released at requested origin {target}");
                        }
                        HostAction::ReleaseGrabAndToggleColor { time } => {
                            runtime.pointer.release_if_held(conn, time)?;
                            runtime.safety.clear();
                            let theme = renderer.toggle_color(conn, window.window)?;
                            println!("[INPUT] Click completed: {}", theme.name());
                        }
                        HostAction::ReleaseGrab { time } => {
                            runtime.pointer.release_if_held(conn, time)?;
                            runtime.safety.clear();
                        }
                        _ => {}
                    }
                }
                Event::Expose(ev) if ev.window == window.window && ev.count == 0 => {
                    renderer.paint(conn, window.window)?;
                }
                Event::ClientMessage(ev)
                    if ev.window == window.window
                        && ev.data.as_data32()[0] == window.wm_delete_window =>
                {
                    return Ok(());
                }
                Event::Error(error) => {
                    return Err(format!("Asynchronous X11 error: {error:?}").into())
                }
                _ => {} // LeaveNotify never cancels a stationary hold.
            }
        }

        // Look one event ahead before the batch-end flush. An interruption at the
        // batch boundary must discard the coalesced move just like one within a batch.
        if let Some((event, sequence)) = conn.poll_for_event_with_sequence()? {
            if runtime.apply_safety(Instant::now(), Some(sequence)) {
                runtime.cancel(
                    conn,
                    window,
                    "QueryPointer confirmed initiating button released",
                )?;
            }
            let interrupted = interrupts_pending_movement(&event, root, window.window, atoms);
            buffered_event = Some((event, sequence));
            if !interrupted {
                runtime.flush_move(conn, window)?;
            }
            continue;
        }
        if runtime.apply_safety(Instant::now(), None) {
            runtime.cancel(
                conn,
                window,
                "QueryPointer confirmed initiating button released",
            )?;
            continue;
        }
        runtime.flush_move(conn, window)?;
        conn.flush()?;
        // reply()/check() can have buffered events. Process them before a safety observation
        // or socket wait, preserving ordinary release and fresh-press ordering across batches.
        if let Some(event) = conn.poll_for_event_with_sequence()? {
            buffered_event = Some(event);
            continue;
        }
        let now = Instant::now();
        let timeout = [
            duration_deadline,
            runtime.confirmation_deadline,
            runtime.correction.map(|(_, dl)| dl),
            runtime.safety.deadline,
        ]
        .into_iter()
        .flatten()
        .map(|dl| dl.saturating_duration_since(now))
        .min();
        let timeout_ms = timeout.map_or(-1, |duration| {
            duration.as_millis().min(i32::MAX as u128) as i32
        });
        let mut pfd = PollFd {
            fd: conn.stream().as_raw_fd(),
            events: POLLIN,
            revents: 0,
        };
        let result = unsafe { poll(&mut pfd, 1, timeout_ms) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error.into());
            }
        } else if pfd.revents & (0x0008 | 0x0010 | 0x0020) != 0 {
            return Err(
                format!("X11 socket unavailable (poll revents=0x{:x})", pfd.revents).into(),
            );
        }
    }
}

/// Drains available X11 events into a bounded batch, prepending any previously buffered event.
pub fn drain_events_bounded<T, E, F>(
    buffered_event: &mut Option<T>,
    max_batch: usize,
    mut poll_fn: F,
) -> Result<Vec<T>, E>
where
    F: FnMut() -> Result<Option<T>, E>,
{
    let mut batch = Vec::new();
    if let Some(ev) = buffered_event.take() {
        batch.push(ev);
    }
    while batch.len() < max_batch {
        match poll_fn()? {
            Some(ev) => batch.push(ev),
            None => break,
        }
    }
    Ok(batch)
}
#[allow(clippy::too_many_arguments)]
fn cancel_movement_with(
    interaction: &mut InteractionManager,
    pending_move: &mut Option<Point>,
    final_target: &mut Option<Point>,
    deadline: &mut Option<Instant>,
    window: &mut ManagedProbeWindow,
    release: impl FnOnce(u32) -> Result<(), HostError>,
) -> Result<(), HostError> {
    interaction.cancel(x11rb::CURRENT_TIME);
    *pending_move = None;
    *final_target = None;
    *deadline = None;
    window.cancel_in_flight_moves();
    release(x11rb::CURRENT_TIME)
}

#[allow(clippy::too_many_arguments)]
fn check_confirmation_timeout_with<F, R>(
    confirmation_deadline: &mut Option<Instant>,
    window: &mut ManagedProbeWindow,
    final_target: &mut Option<Point>,
    interaction: &mut InteractionManager,
    pending_move: &mut Option<Point>,
    mut query: F,
    release: R,
) -> Result<bool, HostError>
where
    F: FnMut() -> Result<Point, HostError>,
    R: FnOnce(u32) -> Result<(), HostError>,
{
    if !confirmation_deadline.is_some_and(|dl| Instant::now() >= dl) {
        return Ok(false);
    }
    let live = query()?;
    let rec =
        window.reconcile_live_snapshot(live, Size::new(window.width.into(), window.height.into()));
    if matches!(rec, ConfigureReconciliation::Confirmed { .. })
        && final_target.is_none_or(|target| target == live)
    {
        *final_target = None;
        *confirmation_deadline = None;
    } else {
        eprintln!(
            "[WARN] Movement confirmation mismatch: requested {}, confirmed {live}; cancelling",
            window.requested_origin()
        );
        cancel_movement_with(
            interaction,
            pending_move,
            final_target,
            confirmation_deadline,
            window,
            release,
        )?;
    }
    Ok(true)
}

fn run_diagnostics(
    conn: &impl Connection,
    screen: &x11rb::protocol::xproto::Screen,
    layout: &x11::monitors::DesktopLayout,
) {
    println!("\n[X11 Session & Display]");
    println!("  Root window:            0x{:x}", screen.root);
    println!(
        "  Total screen spanning:  {}x{} px",
        screen.width_in_pixels, screen.height_in_pixels
    );
    println!("  Root visual ID:         0x{:x}", screen.root_visual);
    println!("  Root depth:             {} bpp", screen.root_depth);

    println!("\n[RandR Monitors]");
    if let Some(ref pm) = layout.primary_monitor {
        println!(
            "  Primary monitor '{}':   {}x{} at +{}+{}",
            pm.name, pm.width, pm.height, pm.x, pm.y
        );
    }

    println!("\n[Visuals & Depths]");
    let mut depth_32_candidates = 0;
    for depth in &screen.allowed_depths {
        if depth.depth == 32 {
            for visual in &depth.visuals {
                if visual.class == VisualClass::TRUE_COLOR {
                    depth_32_candidates += 1;
                }
            }
        }
    }
    println!(
        "  Allowed depths:         {:?}",
        screen
            .allowed_depths
            .iter()
            .map(|d| d.depth)
            .collect::<Vec<_>>()
    );
    println!(
        "  32-bit TrueColor visuals: {} candidate(s) found (alpha-capable Render format verification will be performed in M01.2)",
        depth_32_candidates
    );

    println!("\n[Extensions]");
    if let Ok(cookie) = conn.render_query_version(0, 11) {
        if let Ok(reply) = cookie.reply() {
            println!(
                "  RENDER extension:       supported (v{}.{})",
                reply.major_version, reply.minor_version
            );
        }
    }
    if let Ok(cookie) = conn.shape_query_version() {
        if let Ok(reply) = cookie.reply() {
            println!(
                "  SHAPE extension:        supported (v{}.{})",
                reply.major_version, reply.minor_version
            );
        }
    }

    println!("\n[Window Manager / EWMH]");
    println!("  _NET_CURRENT_DESKTOP:    {}", layout.current_desktop);
    if let Some(wa) = layout.desktop_workarea {
        println!(
            "  Current desktop workarea: x={}, y={}, w={}, h={}",
            wa.x, wa.y, wa.width, wa.height
        );
    }
    println!(
        "  -> Usable area on main screen (desktop {}): x={}, y={}, w={}, h={}",
        layout.current_desktop,
        layout.usable_area.x,
        layout.usable_area.y,
        layout.usable_area.width,
        layout.usable_area.height
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb::protocol::xproto::{
        ButtonReleaseEvent, ConfigureNotifyEvent, KeyButMask, Motion, MotionNotifyEvent,
    };

    #[test]
    fn test_drain_events_bounded_preserves_buffered_event() {
        let mut buffered_event = Some(Event::MotionNotify(MotionNotifyEvent {
            response_type: 6,
            detail: Motion::NORMAL,
            sequence: 1,
            time: 100,
            root: 1,
            event: 10,
            child: 0,
            root_x: 50,
            root_y: 50,
            event_x: 5,
            event_y: 5,
            state: KeyButMask::from(0u16),
            same_screen: true,
        }));

        let mut queue = vec![Event::MotionNotify(MotionNotifyEvent {
            response_type: 6,
            detail: Motion::NORMAL,
            sequence: 2,
            time: 101,
            root: 1,
            event: 10,
            child: 0,
            root_x: 60,
            root_y: 60,
            event_x: 15,
            event_y: 15,
            state: KeyButMask::from(0u16),
            same_screen: true,
        })];

        let batch: Vec<Event> = drain_events_bounded(&mut buffered_event, 64, || {
            Ok::<_, ()>(if queue.is_empty() {
                None
            } else {
                Some(queue.remove(0))
            })
        })
        .unwrap();

        assert_eq!(batch.len(), 2);
        assert!(buffered_event.is_none());
    }

    #[test]
    fn test_buffered_events_exceeding_batch_limit_drains_release_last() {
        // Create 70 events: 69 motions followed by 1 release
        let mut queue = std::collections::VecDeque::new();
        for i in 0..69 {
            queue.push_back(Event::MotionNotify(MotionNotifyEvent {
                response_type: 6,
                detail: Motion::NORMAL,
                sequence: i as u16,
                time: 1000 + i,
                root: 1,
                event: 10,
                child: 0,
                root_x: 100 + i as i16,
                root_y: 100,
                event_x: 10,
                event_y: 10,
                state: KeyButMask::BUTTON1,
                same_screen: true,
            }));
        }
        queue.push_back(Event::ButtonRelease(ButtonReleaseEvent {
            response_type: 5,
            detail: 1,
            sequence: 70,
            time: 1070,
            root: 1,
            event: 10,
            child: 0,
            root_x: 168,
            root_y: 100,
            event_x: 10,
            event_y: 10,
            state: KeyButMask::from(0u16),
            same_screen: true,
        }));

        let mut buffered_event = None;
        let mut processed_events = Vec::new();

        // Simulate event loop across batches without sleeping on socket
        let mut loops = 0;
        while loops < 10 && (!queue.is_empty() || buffered_event.is_some()) {
            loops += 1;
            // 1. Drain batch
            let batch: Vec<Event> =
                drain_events_bounded(&mut buffered_event, 64, || Ok::<_, ()>(queue.pop_front()))
                    .unwrap();

            let batch_len = batch.len();
            for ev in batch {
                processed_events.push(ev);
            }

            // Verify batch 1 hit max_batch limit
            if loops == 1 {
                assert_eq!(batch_len, 64);
            }

            // 2. Before blocking, check if more events remain buffered in poll_fn
            if buffered_event.is_none() {
                if let Some(next_ev) = queue.pop_front() {
                    buffered_event = Some(next_ev);
                    continue;
                }
            }
        }

        assert_eq!(
            loops, 2,
            "Should drain all 70 events in exactly 2 batches without blocking"
        );
        assert_eq!(processed_events.len(), 70);

        // Verify the 70th event processed is the ButtonRelease event
        match &processed_events[69] {
            Event::ButtonRelease(ev) => {
                assert_eq!(ev.detail, 1);
                assert_eq!(ev.root_x, 168);
            }
            other => panic!("Expected ButtonRelease at end of stream, found {:?}", other),
        }
    }

    #[test]
    fn test_confirmation_timeout_with_no_further_events() {
        let mut window = ManagedProbeWindow {
            window_resource: Default::default(),
            colormap_resource: Default::default(),
            window: 100,
            colormap: 1,
            wm_delete_window: 2,
            requested_origin: Point::new(150, 150),
            confirmed_origin: Point::new(100, 100),
            in_flight_moves: std::collections::VecDeque::new(),
            superseded_moves: std::collections::VecDeque::new(),
            width: 160,
            height: 160,
        };
        window.in_flight_moves.push_back(Point::new(150, 150));

        let mut awaiting_final_confirmation = Some(Point::new(150, 150));
        let mut confirmation_deadline = Some(Instant::now() - Duration::from_millis(1));

        // When deadline has expired and no events arrived, checked live query runs.
        assert!(confirmation_deadline.unwrap() <= Instant::now());
        // Discrepancy case: live query observes refused move remaining at (100, 100)
        let live_origin = Point::new(100, 100);
        let rec = window.reconcile_live_snapshot(live_origin, Size::new(160, 160));
        assert_eq!(
            rec,
            ConfigureReconciliation::GenuineMismatch {
                requested: Point::new(150, 150),
                confirmed: Point::new(100, 100),
            }
        );
        confirmation_deadline = None;
        assert_eq!(confirmation_deadline, None);

        // Success case: live query observes target reached at (150, 150)
        window.in_flight_moves.push_back(Point::new(150, 150));
        let rec_ok = window.reconcile_live_snapshot(Point::new(150, 150), Size::new(160, 160));
        assert_eq!(
            rec_ok,
            ConfigureReconciliation::Confirmed {
                origin: Point::new(150, 150),
                size: Size::new(160, 160),
            }
        );
        if let ConfigureReconciliation::Confirmed { origin, .. } = rec_ok {
            if awaiting_final_confirmation == Some(origin) {
                awaiting_final_confirmation = None;
            }
        }
        assert_eq!(awaiting_final_confirmation, None);
    }

    #[test]
    fn test_awaiting_final_confirmation_rejects_mismatched_target() {
        let mut window = ManagedProbeWindow {
            window_resource: Default::default(),
            colormap_resource: Default::default(),
            window: 100,
            colormap: 1,
            wm_delete_window: 2,
            requested_origin: Point::new(200, 200),
            confirmed_origin: Point::new(100, 100),
            in_flight_moves: std::collections::VecDeque::new(),
            superseded_moves: std::collections::VecDeque::new(),
            width: 160,
            height: 160,
        };

        let awaiting_final = Point::new(200, 200);

        // Suppose WM confirms position (180, 200) instead of expected (200, 200)
        let confirmed_event_origin = Point::new(180, 200);
        let rec =
            window.reconcile_historical_notification(confirmed_event_origin, Size::new(160, 160));

        // It is not accepted as the awaited final target
        assert_ne!(confirmed_event_origin, awaiting_final);
        assert_eq!(
            rec,
            ConfigureReconciliation::GenuineMismatch {
                requested: Point::new(200, 200),
                confirmed: confirmed_event_origin,
            }
        );
    }

    #[test]
    fn test_expired_deadline_with_more_than_64_buffered_unrelated_events() {
        // Create 70 unrelated events (e.g. motion events on an unrelated window)
        let mut queue = std::collections::VecDeque::new();
        for i in 0..70 {
            queue.push_back(Event::MotionNotify(MotionNotifyEvent {
                response_type: 6,
                detail: Motion::NORMAL,
                sequence: i as u16,
                time: 1000 + i,
                root: 1,
                event: 999, // unrelated window
                child: 0,
                root_x: 50,
                root_y: 50,
                event_x: 50,
                event_y: 50,
                state: KeyButMask::from(0u16),
                same_screen: true,
            }));
        }

        let mut window = ManagedProbeWindow {
            window_resource: Default::default(),
            colormap_resource: Default::default(),
            window: 100,
            colormap: 1,
            wm_delete_window: 2,
            requested_origin: Point::new(150, 150),
            confirmed_origin: Point::new(100, 100),
            in_flight_moves: std::collections::VecDeque::new(),
            superseded_moves: std::collections::VecDeque::new(),
            width: 160,
            height: 160,
        };
        window.in_flight_moves.push_back(Point::new(150, 150));

        let mut awaiting_final_confirmation = Some(Point::new(150, 150));
        let mut confirmation_deadline = Some(Instant::now() - Duration::from_millis(10));
        let mut pending_move = None;
        let mut interaction = InteractionManager::new();
        // Simulate dragging state
        let bounds = crate::geometry::ValidOriginBounds::new(0, 1000, 0, 1000);
        let _ = interaction.handle_left_press(
            Point::new(100, 100),
            (20, 20),
            1000,
            Point::new(100, 100),
        );
        interaction.on_grab_acquired();
        let _ = interaction.handle_motion(Point::new(150, 150), 1010, &bounds);
        assert!(interaction.is_dragging());

        let mut grab_released = false;
        let mut buffered_event = None;
        let mut loops = 0;

        // Simulate production event loop structure
        while loops < 10 && (!queue.is_empty() || buffered_event.is_some()) {
            loops += 1;

            // Check confirmation deadline at top of loop alongside duration deadline,
            // BEFORE draining/processing another batch
            let _ = check_confirmation_timeout_with(
                &mut confirmation_deadline,
                &mut window,
                &mut awaiting_final_confirmation,
                &mut interaction,
                &mut pending_move,
                || Ok(Point::new(100, 100)), // Live query observes window remained at 100,100 (refusal)
                |_time| {
                    grab_released = true;
                    Ok(())
                },
            );

            // 1. Drain batch (64 events)
            let batch =
                drain_events_bounded(&mut buffered_event, 64, || Ok::<_, ()>(queue.pop_front()))
                    .unwrap();
            assert!(!batch.is_empty());

            // 2. Before blocking on socket, verify no events remain buffered
            if buffered_event.is_none() {
                if let Some(next_ev) = queue.pop_front() {
                    buffered_event = Some(next_ev);
                    continue;
                }
            }
        }

        // Crucial assertions:
        // 1. Confirmation deadline was processed on loop 1 BEFORE batch 2 drained, not bypassed by continue
        assert_eq!(confirmation_deadline, None);
        assert!(grab_released);
        assert!(
            !interaction.is_dragging(),
            "Interaction must be cancelled on timeout discrepancy"
        );
        assert!(awaiting_final_confirmation.is_none());
        assert!(window.in_flight_moves.is_empty());
        assert_eq!(loops, 2, "Drained 70 events in exactly 2 batches");
    }

    #[test]
    fn test_two_consecutive_drags_with_delayed_confirmation_of_first() {
        let mut window = ManagedProbeWindow {
            window_resource: Default::default(),
            colormap_resource: Default::default(),
            window: 100,
            colormap: 1,
            wm_delete_window: 2,
            requested_origin: Point::new(80, 80),
            confirmed_origin: Point::new(80, 80),
            in_flight_moves: std::collections::VecDeque::new(),
            superseded_moves: std::collections::VecDeque::new(),
            width: 160,
            height: 160,
        };

        let mut interaction = InteractionManager::new();
        let bounds = crate::geometry::ValidOriginBounds::new(0, 1000, 0, 1000);
        let mut awaiting_final_confirmation: Option<Point> = None;
        assert_eq!(awaiting_final_confirmation, None);

        let a = Point::new(100, 80);
        let b = Point::new(120, 80);

        // --- Drag 1 ---
        // Press on body at (100, 100)
        let action1 =
            interaction.handle_left_press(Point::new(100, 100), (20, 20), 1000, Point::new(80, 80));
        assert!(matches!(action1, HostAction::AcquireGrab { .. }));
        interaction.on_grab_acquired();
        awaiting_final_confirmation = None;
        assert_eq!(awaiting_final_confirmation, None);

        // Move to A
        let action_m1 = interaction.handle_motion(Point::new(120, 100), 1010, &bounds);
        assert_eq!(action_m1, HostAction::MoveWindow { target: a });
        window.in_flight_moves.push_back(a);
        window.requested_origin = a;

        // Release Drag 1 at root (120, 100)
        let action_r1 =
            interaction.handle_left_release(Point::new(120, 100), (20, 20), 1020, &bounds);
        assert_eq!(
            action_r1,
            HostAction::ReleaseGrabAndMoveWindow {
                time: 1020,
                target: a
            }
        );
        awaiting_final_confirmation = Some(a);
        assert_eq!(awaiting_final_confirmation, Some(a));

        // --- Drag 2 (starts before A is confirmed) ---
        // Fresh left press begins gesture 2
        let action2 = interaction.handle_left_press(Point::new(120, 100), (20, 20), 1050, a);
        assert!(matches!(action2, HostAction::AcquireGrab { .. }));
        interaction.on_grab_acquired();
        // Production logic: fresh gesture supersedes awaiting_final_confirmation
        awaiting_final_confirmation = None;
        assert_eq!(awaiting_final_confirmation, None);
        // Tracking of previous in-flight move A is preserved!
        assert_eq!(window.in_flight_moves.len(), 1);
        assert_eq!(window.in_flight_moves[0], a);

        // Drag 2 moves to B
        let action_m2 = interaction.handle_motion(Point::new(140, 100), 1060, &bounds);
        assert_eq!(action_m2, HostAction::MoveWindow { target: b });
        window.in_flight_moves.push_back(b);
        window.requested_origin = b;
        assert_eq!(window.in_flight_moves.len(), 2);

        // Release Drag 2 at root (140, 100)
        let action_r2 =
            interaction.handle_left_release(Point::new(140, 100), (20, 20), 1070, &bounds);
        assert_eq!(
            action_r2,
            HostAction::ReleaseGrabAndMoveWindow {
                time: 1070,
                target: b
            }
        );
        awaiting_final_confirmation = Some(b);
        assert_eq!(awaiting_final_confirmation, Some(b));

        // --- Delayed confirmation for Drag 1 (A) arrives ---
        let ev_a = ConfigureNotifyEvent {
            response_type: 22 | 0x80, // synthetic event
            sequence: 1,
            event: 100,
            window: 100,
            above_sibling: 0,
            x: a.x as i16,
            y: a.y as i16,
            width: 160,
            height: 160,
            border_width: 0,
            override_redirect: false,
        };
        let rec_a = window
            .handle_configure_notify_with(&ev_a, || unreachable!())
            .unwrap();
        // Should be in-flight catch-up, NOT Confirmed and NOT GenuineMismatch
        assert_eq!(
            rec_a,
            ConfigureReconciliation::InFlightCatchUp {
                confirmed: a,
                remaining_in_flight: 1,
            }
        );
        // Crucial: awaiting_final_confirmation remains Some(b) and Drag 2 is NOT cancelled
        assert_eq!(awaiting_final_confirmation, Some(b));
        assert!(interaction.is_idle()); // Gesture 2 released cleanly into idle

        // --- Confirmation for Drag 2 (B) arrives ---
        let ev_b = ConfigureNotifyEvent {
            response_type: 22 | 0x80,
            sequence: 2,
            event: 100,
            window: 100,
            above_sibling: 0,
            x: b.x as i16,
            y: b.y as i16,
            width: 160,
            height: 160,
            border_width: 0,
            override_redirect: false,
        };
        let rec_b = window
            .handle_configure_notify_with(&ev_b, || unreachable!())
            .unwrap();
        assert_eq!(
            rec_b,
            ConfigureReconciliation::Confirmed {
                origin: b,
                size: Size::new(160, 160),
            }
        );
        if let ConfigureReconciliation::Confirmed { origin, .. } = rec_b {
            if awaiting_final_confirmation == Some(origin) {
                awaiting_final_confirmation = None;
            }
        }
        assert_eq!(awaiting_final_confirmation, None);
        assert_eq!(window.confirmed_origin(), b);
        assert!(window.in_flight_moves.is_empty());
    }

    #[test]
    fn test_expired_deadline_query_buffers_release_processes_before_socket_wait() {
        let mut window = ManagedProbeWindow {
            window_resource: Default::default(),
            colormap_resource: Default::default(),
            window: 100,
            colormap: 1,
            wm_delete_window: 2,
            requested_origin: Point::new(150, 150),
            confirmed_origin: Point::new(100, 100),
            in_flight_moves: std::collections::VecDeque::new(),
            superseded_moves: std::collections::VecDeque::new(),
            width: 160,
            height: 160,
        };
        window.in_flight_moves.push_back(Point::new(150, 150));

        let mut interaction = InteractionManager::new();
        let bounds = crate::geometry::ValidOriginBounds::new(0, 1000, 0, 1000);
        let _ = interaction.handle_left_press(
            Point::new(100, 100),
            (20, 20),
            1000,
            Point::new(100, 100),
        );
        interaction.on_grab_acquired();
        let _ = interaction.handle_motion(Point::new(150, 150), 1010, &bounds);
        assert!(interaction.is_dragging());

        let mut awaiting_final_confirmation = None;
        let mut confirmation_deadline = Some(Instant::now() - Duration::from_millis(10));
        let mut pending_move = None;
        let mut buffered_event: Option<Event> = None;

        // Simulated x11rb internal event queue
        let mut internal_event_queue = std::collections::VecDeque::new();
        let mut socket_wait_invoked = false;
        let mut release_processed = false;

        // Loop iteration 1:
        // Batch draining finds no events in queue initially
        let batch: Vec<Event> = drain_events_bounded(&mut buffered_event, 64, || {
            Ok::<_, ()>(internal_event_queue.pop_front())
        })
        .unwrap();
        assert!(batch.is_empty());

        // Now post-batch handling in loop runs:
        // 1. All X11 operations executed BEFORE final empty-event check:
        // Check confirmation timeout: query runs and buffers an incoming ButtonRelease
        let query_buffers_release = || {
            // When query executes synchronous round-trip (.reply()), server event arrives on socket
            internal_event_queue.push_back(Event::ButtonRelease(ButtonReleaseEvent {
                response_type: 5,
                detail: 1,
                sequence: 10,
                time: 1050,
                root: 1,
                event: 100,
                child: 0,
                root_x: 150,
                root_y: 150,
                event_x: 20,
                event_y: 20,
                state: KeyButMask::BUTTON1,
                same_screen: true,
            }));
            Ok(Point::new(100, 100)) // Live query observes discrepancy
        };

        check_confirmation_timeout_with(
            &mut confirmation_deadline,
            &mut window,
            &mut awaiting_final_confirmation,
            &mut interaction,
            &mut pending_move,
            query_buffers_release,
            |_time| Ok(()),
        )
        .unwrap();

        // 2. Final empty-event check BEFORE waiting on socket:
        let should_continue = if buffered_event.is_some() {
            true
        } else {
            match internal_event_queue.pop_front() {
                Some(ev) => {
                    buffered_event = Some(ev);
                    true
                }
                None => false,
            }
        };

        if !should_continue {
            socket_wait_invoked = true;
        }

        // Crucial: socket wait was NOT invoked because the buffered release was detected!
        assert!(
            !socket_wait_invoked,
            "Socket wait must not be invoked when release was buffered"
        );
        assert!(should_continue);
        assert!(buffered_event.is_some());

        // Loop iteration 2:
        // Drain batch picks up the buffered ButtonRelease event immediately
        let batch2: Vec<Event> = drain_events_bounded(&mut buffered_event, 64, || {
            Ok::<_, ()>(internal_event_queue.pop_front())
        })
        .unwrap();
        assert_eq!(batch2.len(), 1);

        for ev in batch2 {
            if let Event::ButtonRelease(rel) = ev {
                assert_eq!(rel.detail, 1);
                let _ = interaction.handle_left_release(
                    Point::new(rel.root_x as i32, rel.root_y as i32),
                    (rel.event_x, rel.event_y),
                    rel.time,
                    &bounds,
                );
                release_processed = true;
            }
        }

        assert!(
            release_processed,
            "Buffered release must be processed before any socket wait"
        );
        assert!(interaction.is_idle());
    }
    fn test_window() -> ManagedProbeWindow {
        ManagedProbeWindow {
            window_resource: Default::default(),
            colormap_resource: Default::default(),
            window: 100,
            colormap: 1,
            wm_delete_window: 2,
            requested_origin: Point::new(100, 100),
            confirmed_origin: Point::new(100, 100),
            in_flight_moves: Default::default(),
            superseded_moves: Default::default(),
            width: 160,
            height: 160,
        }
    }
    fn captured_runtime() -> ProbeRuntime {
        let mut runtime = ProbeRuntime::new();
        runtime.body = BodyAvailability::Ready;
        runtime.interaction.handle_left_press(
            Point::new(180, 180),
            (80, 80),
            1000,
            Point::new(100, 100),
        );
        runtime.interaction.on_grab_acquired();
        runtime.pointer.set_grabbed(true);
        runtime.safety.acquired(Instant::now());
        runtime
    }

    #[test]
    fn cancellation_invalidates_movement_before_failed_release_and_cleanup_retries() {
        let mut runtime = captured_runtime();
        let mut window = test_window();
        runtime.pending_move = Some(Point::new(200, 200));
        runtime.final_target = Some(Point::new(220, 220));
        runtime.confirmation_deadline = Some(Instant::now());
        window.in_flight_moves.push_back(Point::new(190, 190));
        assert!(runtime
            .cancel_with(&mut window, |pointer| pointer
                .release_with(|| Err("injected ungrab failure".into()))
                .map(|_| ()))
            .is_err());
        assert!(runtime.pending_move.is_none());
        assert!(runtime.final_target.is_none());
        assert!(runtime.confirmation_deadline.is_none());
        assert!(runtime.safety.deadline.is_none());
        assert!(window.in_flight_moves.is_empty());
        assert!(window.superseded_moves.contains(&Point::new(190, 190)));
        assert!(runtime.pointer.is_grabbed());
        assert_eq!(
            runtime.interaction.handle_left_release(
                Point::new(180, 180),
                (80, 80),
                1010,
                &crate::geometry::ValidOriginBounds::new(0, 1000, 0, 1000)
            ),
            HostAction::None
        );
        runtime
            .cancel_with(&mut window, |pointer| {
                pointer.release_with(|| Ok(())).map(|_| ())
            })
            .unwrap();
        assert!(!runtime.pointer.is_grabbed());
    }

    #[test]
    fn stationary_hold_rearms_safety_indefinitely_and_release_cancels_without_position() {
        let mut runtime = captured_runtime();
        let mut now = Instant::now();
        for sequence in 1..1000 {
            now += POINTER_CHECK_INTERVAL;
            runtime.safety.observed(true, sequence);
            assert!(!runtime.apply_safety(now, None));
            assert_eq!(runtime.safety.deadline, Some(now + POINTER_CHECK_INTERVAL));
            assert!(runtime.interaction.has_left_gesture());
        }
        runtime.safety.observed(false, 1000);
        assert!(runtime.apply_safety(now, None));
        let mut window = test_window();
        runtime
            .cancel_with(&mut window, |pointer| {
                pointer.release_with(|| Ok(())).map(|_| ())
            })
            .unwrap();
        assert!(runtime.pending_move.is_none());
        assert!(runtime.final_target.is_none());
    }

    #[test]
    fn safety_observation_waits_for_pre_query_release_but_not_later_events() {
        let mut runtime = captured_runtime();
        let now = Instant::now();
        runtime.safety.observed(false, 70000); // extended sequence, beyond 16-bit wrap
        assert!(!runtime.apply_safety(now, Some(69999)));
        let action = runtime.interaction.handle_left_release(
            Point::new(180, 180),
            (80, 80),
            1010,
            &crate::geometry::ValidOriginBounds::new(0, 1000, 0, 1000),
        );
        assert_eq!(action, HostAction::ReleaseGrabAndToggleColor { time: 1010 });
        assert!(!runtime.apply_safety(now, Some(70000)));
        assert!(runtime.safety.observation.is_none());

        let mut runtime = captured_runtime();
        runtime.safety.observed(false, 70000);
        assert!(runtime.apply_safety(now, Some(70000)));
        assert!(runtime.safety.deadline.is_none());
    }

    #[test]
    fn buffered_new_press_invalidates_prior_safety_observation() {
        let mut runtime = captured_runtime();
        let now = Instant::now();
        runtime.safety.observed(false, 900);
        // The queued release and fresh press both preceded the query response boundary.
        runtime.interaction.handle_left_release(
            Point::new(180, 180),
            (80, 80),
            1010,
            &crate::geometry::ValidOriginBounds::new(0, 1000, 0, 1000),
        );
        assert!(!runtime.apply_safety(now, Some(899)));
        runtime.interaction.handle_left_press(
            Point::new(180, 180),
            (80, 80),
            1020,
            Point::new(100, 100),
        );
        runtime.interaction.on_grab_acquired();
        runtime.safety.acquired(now);
        assert!(!runtime.apply_safety(now, Some(900)));
        assert!(runtime.safety.deadline.is_some());
        assert!(runtime.interaction.has_left_gesture());
    }

    #[test]
    fn interruption_discards_coalesced_move_and_release_preserves_flush_order() {
        use x11rb::protocol::xproto::{ButtonPressEvent, UnmapNotifyEvent, UNMAP_NOTIFY_EVENT};
        let atoms = x11::monitors::LayoutAtoms {
            workarea: 3,
            current_desktop: 4,
            wm_desktop: 5,
        };
        let release = Event::ButtonRelease(ButtonReleaseEvent {
            response_type: 5,
            detail: 1,
            sequence: 1,
            time: 1010,
            root: 1,
            event: 100,
            child: 0,
            root_x: 200,
            root_y: 200,
            event_x: 80,
            event_y: 80,
            state: KeyButMask::BUTTON1,
            same_screen: true,
        });
        assert!(!interrupts_pending_movement(&release, 1, 100, &atoms));
        let unmap = Event::UnmapNotify(UnmapNotifyEvent {
            response_type: UNMAP_NOTIFY_EVENT,
            sequence: 1,
            event: 100,
            window: 100,
            from_configure: false,
        });
        assert!(interrupts_pending_movement(&unmap, 1, 100, &atoms));
        let right = Event::ButtonPress(ButtonPressEvent {
            detail: 3,
            ..match release {
                Event::ButtonRelease(ev) => ev,
                _ => unreachable!(),
            }
        });
        assert!(interrupts_pending_movement(&right, 1, 100, &atoms));
        let mut runtime = captured_runtime();
        runtime.pending_move = Some(Point::new(200, 200));
        let mut window = test_window();
        runtime
            .cancel_with(&mut window, |pointer| {
                pointer.release_with(|| Ok(())).map(|_| ())
            })
            .unwrap();
        assert!(runtime.pending_move.is_none());
    }

    #[test]
    fn unavailable_body_defers_placement_and_verified_remap_requires_correction_first() {
        let mut runtime = ProbeRuntime::new();
        let mut window = test_window();
        let bounds = crate::geometry::ValidOriginBounds::new(0, 50, 0, 50);
        let actual = crate::geometry::Rect::new(100, 100, 160, 160);
        runtime.unavailable(false);
        runtime
            .placement_with(&mut window, actual, &bounds, |_, _| {
                panic!("unavailable body must not move")
            })
            .unwrap();
        assert!(!runtime.accepts_input());
        runtime.body = BodyAvailability::Validating; // production verified map gate
        let calls = std::cell::Cell::new(0);
        runtime
            .placement_with(&mut window, actual, &bounds, |_, target| {
                assert_eq!(target, Point::new(50, 50));
                calls.set(calls.get() + 1);
                Ok(())
            })
            .unwrap();
        assert!(!runtime.accepts_input());
        runtime
            .placement_with(&mut window, actual, &bounds, |_, _| {
                panic!("must verify before requesting again")
            })
            .unwrap();
        assert_eq!(calls.get(), 1);
        let deadline = runtime.correction.unwrap().1;
        assert!(!runtime
            .check_correction(
                deadline - Duration::from_millis(1),
                Point::new(100, 100),
                &bounds
            )
            .unwrap());
        assert!(runtime
            .check_correction(deadline, Point::new(50, 50), &bounds)
            .unwrap());
        assert!(runtime.accepts_input());
        runtime.unavailable(true);
        runtime
            .placement_with(&mut window, actual, &bounds, |_, _| {
                panic!("destroyed body must not move")
            })
            .unwrap();
        assert!(!runtime.accepts_input());
    }

    #[test]
    fn refused_correction_fails_without_retry_and_movement_failure_still_cleans_up() {
        let mut runtime = captured_runtime();
        let mut window = test_window();
        let bounds = crate::geometry::ValidOriginBounds::new(0, 50, 0, 50);
        let deadline = Instant::now();
        runtime.correction = Some((Point::new(50, 50), deadline));
        let result = runtime
            .check_correction(deadline, Point::new(100, 100), &bounds)
            .map(|_| ());
        let calls = std::cell::Cell::new(0);
        let error = finish_probe_with(result, || {
            runtime.cancel_with(&mut window, |pointer| {
                pointer
                    .release_with(|| {
                        calls.set(1);
                        Ok(())
                    })
                    .map(|_| ())
            })?;
            x11::resource::cleanup_all([Box::new(|| {
                assert_eq!(calls.get(), 1);
                calls.set(2);
                Err("injected cleanup failure".into())
            })])
        })
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("WM refused necessary bounds correction"));
        assert_eq!(calls.get(), 2);
        assert!(!runtime.pointer.is_grabbed());

        runtime.body = BodyAvailability::Validating;
        let error = runtime
            .placement_with(
                &mut window,
                crate::geometry::Rect::new(100, 100, 160, 160),
                &bounds,
                |_, _| Err("injected movement failure".into()),
            )
            .unwrap_err();
        assert_eq!(error.to_string(), "injected movement failure");
        assert!(runtime.correction.is_none());
    }

    #[test]
    fn workspace_workarea_and_root_geometry_are_relevant_but_unrelated_properties_are_not() {
        use x11rb::protocol::xproto::{Property, PropertyNotifyEvent};
        let atoms = x11::monitors::LayoutAtoms {
            workarea: 3,
            current_desktop: 4,
            wm_desktop: 5,
        };
        let property = |window, atom| {
            Event::PropertyNotify(PropertyNotifyEvent {
                response_type: 28,
                sequence: 1,
                window,
                atom,
                time: 1000,
                state: Property::NEW_VALUE,
            })
        };
        for event in [property(1, 3), property(1, 4), property(100, 5)] {
            assert!(relevant_layout_event(&event, 1, 100, &atoms));
            assert!(interrupts_pending_movement(&event, 1, 100, &atoms));
        }
        assert!(!relevant_layout_event(&property(100, 99), 1, 100, &atoms));
        assert!(!relevant_layout_event(&property(999, 3), 1, 100, &atoms));
    }
    #[test]
    fn safety_boundary_survives_multiple_batches_without_swallowing_ordinary_release() {
        let mut runtime = captured_runtime();
        let now = Instant::now();
        runtime.safety.observed(false, 70000);
        let mut queue = std::collections::VecDeque::new();
        for _ in 0..69 {
            queue.push_back((None, 69999));
        }
        queue.push_back((Some(1010), 69999)); // ordinary release buffered by QueryPointer
        let mut buffered = None;
        let mut completions = 0;
        for batch_index in 0..2 {
            let batch =
                drain_events_bounded(&mut buffered, 64, || Ok::<_, ()>(queue.pop_front())).unwrap();
            if batch_index == 0 {
                assert_eq!(batch.len(), 64);
            }
            for (release, sequence) in batch {
                assert!(!runtime.apply_safety(now, Some(sequence)));
                if let Some(time) = release {
                    assert_eq!(
                        runtime.interaction.handle_left_release(
                            Point::new(180, 180),
                            (80, 80),
                            time,
                            &crate::geometry::ValidOriginBounds::new(0, 1000, 0, 1000)
                        ),
                        HostAction::ReleaseGrabAndToggleColor { time }
                    );
                    completions += 1;
                }
            }
        }
        assert!(!runtime.apply_safety(now, None));
        assert_eq!(completions, 1);
        assert!(runtime.safety.deadline.is_none());
    }

    #[test]
    fn failed_corrective_move_preserves_primary_error_and_attempts_release_before_destruction() {
        let mut runtime = captured_runtime();
        let mut window = test_window();
        runtime.body = BodyAvailability::Validating;
        let bounds = crate::geometry::ValidOriginBounds::new(0, 50, 0, 50);
        let result = runtime.placement_with(
            &mut window,
            crate::geometry::Rect::new(100, 100, 160, 160),
            &bounds,
            |_, _| Err("injected move failure".into()),
        );
        let trace = std::cell::RefCell::new(Vec::new());
        let error = finish_probe_with(result, || {
            let cancellation = runtime.cancel_with(&mut window, |pointer| {
                pointer
                    .release_with(|| {
                        trace.borrow_mut().push("ungrab");
                        Err("injected release failure".into())
                    })
                    .map(|_| ())
            });
            x11::resource::cleanup_all([
                Box::new(|| cancellation),
                Box::new(|| {
                    trace.borrow_mut().push("renderer");
                    Ok(())
                }),
                Box::new(|| {
                    trace.borrow_mut().push("window");
                    Ok(())
                }),
            ])
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "injected move failure");
        assert_eq!(*trace.borrow(), ["ungrab", "renderer", "window"]);
        assert!(runtime.pointer.is_grabbed()); // explicit failed release obligation
        assert!(runtime.pending_move.is_none());
        assert!(runtime.final_target.is_none());
    }

    #[test]
    fn unsupported_x11_coordinate_range_is_rejected_before_placement() {
        assert!(checked_probe_bounds(crate::geometry::Rect::new(32700, 0, 1920, 1080)).is_err());
        assert!(checked_probe_bounds(crate::geometry::Rect::new(-40000, 0, 1920, 1080)).is_err());
        assert!(checked_probe_bounds(crate::geometry::Rect::new(-1920, 0, 1920, 1080)).is_ok());
    }
}
