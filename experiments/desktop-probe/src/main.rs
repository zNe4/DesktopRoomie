mod control;
mod geometry;
mod interaction;
mod layer;
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
use crate::interaction::{HostAction, InteractionManager, InteractionState, MenuOutcome};
use crate::layer::{Layer, LayerController, Mutation, ObservedLayer, Step};
use crate::x11::menu::{MenuPopup, PopupInputGate};
use crate::x11::monitors::query_desktop_layout;
use crate::x11::pointer::{grab_pointer, CaptureOwner, PointerCaptureTracker};
use crate::x11::render::{Renderer, WINDOW_HEIGHT, WINDOW_WIDTH};
use crate::x11::state::{LayerAtoms, LayerSupport, PropertyResult};
use crate::x11::visual::find_alpha_visual;
use crate::x11::window::{ConfigureReconciliation, ManagedProbeWindow};

#[derive(Debug, PartialEq, Eq)]
enum CliMode {
    Help,
    Diagnose,
    Owner {
        delay_secs: Option<u64>,
        duration_secs: Option<u64>,
    },
    Control(control::Command),
}

#[derive(Debug, PartialEq, Eq)]
struct UsageError(String);

impl UsageError {
    fn exit_code(&self) -> control::ExitCode {
        control::ExitCode::Usage
    }
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

// Arguments exclude argv[0]. Parsing has no process, output, or host side effects.
fn parse_args<I, S>(args: I) -> Result<CliMode, UsageError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = args.into_iter();
    let mut help = false;
    let mut diagnose = false;
    let mut duration_secs = None;
    let mut delay_secs = None;
    let mut command = None;
    while let Some(arg) = args.next() {
        match arg.as_ref() {
            "-h" | "--help" => help = true,
            "--diagnose" => diagnose = true,
            flag @ ("--delay" | "--duration") => {
                let value = args
                    .next()
                    .ok_or_else(|| UsageError(format!("Missing value for {flag} <seconds>")))?;
                let value = value.as_ref().parse::<u64>().map_err(|_| {
                    UsageError(format!("Invalid value for {flag}: '{}'", value.as_ref()))
                })?;
                if flag == "--delay" {
                    delay_secs = Some(value);
                } else {
                    duration_secs = Some(value);
                }
            }
            flag @ ("--hide" | "--show" | "--bring-top") => {
                let next = match flag {
                    "--hide" => control::Command::Hide,
                    "--show" => control::Command::Show,
                    _ => control::Command::BringTop,
                };
                if command.replace(next).is_some() {
                    return Err(UsageError("Control commands are mutually exclusive".into()));
                }
            }
            unknown => {
                return Err(UsageError(format!(
                    "Unknown option: '{unknown}'. Use --help for usage."
                )))
            }
        }
    }
    if let Some(command) = command {
        if diagnose || delay_secs.is_some() || duration_secs.is_some() {
            return Err(UsageError(
                "Control commands cannot combine with --diagnose, --delay, or --duration".into(),
            ));
        }
        if !help {
            return Ok(CliMode::Control(command));
        }
    }
    if help {
        Ok(CliMode::Help)
    } else if diagnose {
        Ok(CliMode::Diagnose)
    } else {
        Ok(CliMode::Owner {
            delay_secs,
            duration_secs,
        })
    }
}

// Replace this isolated stub with caller transport in Stage C.
fn unsupported_control(command: control::Command) -> (control::ExitCode, String) {
    (control::ExitCode::OperationFailed, format!("{command}: not implemented in this stage (M03.2-A); control transport is scheduled for Stage C"))
}

fn print_help() {
    println!("DesktopRoomie - A00-M03.1: Desktop Probe");
    println!();
    println!("Left click toggles color; left drag moves the body.");
    println!("Right click opens Above/Normal/Below/Dismiss/Quit after release. Outside left click dismisses.");
    println!();
    println!("USAGE:");
    println!("  desktop-probe [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("  -h, --help              Show this help message and exit");
    println!("  --diagnose              Run pure X11 environment diagnostics and exit");
    println!("  --delay <SECONDS>       Delay in seconds before mapping window (for typing test)");
    println!("  --duration <SECONDS>    Run for a specified duration in seconds, then exit");
    println!("  --hide | --show | --bring-top  Control commands (not operational yet; exit 7)");
}

fn main() {
    let mode = match parse_args(env::args().skip(1)) {
        Ok(mode) => mode,
        Err(err) => {
            eprintln!("[ERROR] {}", err.0);
            process::exit(err.exit_code() as i32);
        }
    };
    let (diagnose, delay_secs, duration_secs) = match mode {
        CliMode::Help => {
            print_help();
            return;
        }
        CliMode::Control(command) => {
            let (exit, diagnostic) = unsupported_control(command);
            eprintln!("[ERROR] {diagnostic}");
            process::exit(exit as i32);
        }
        CliMode::Diagnose => (true, None, None),
        CliMode::Owner {
            delay_secs,
            duration_secs,
        } => (false, delay_secs, duration_secs),
    };

    println!("=== DesktopRoomie A00-M03.1: Desktop Probe ===");

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
    if diagnose {
        run_diagnostics(&conn, screen, &layout);
        println!("\n[Result]");
        println!("  M01.1: X11 connection established; diagnostics completed.");
        return;
    }

    // M02.5 retains bounded dragging and adds interruption recovery.
    println!("Mission M03.1: managed layers and mouse-only controls...");

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
    if let Some(delay) = delay_secs {
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
    if let Some(sec) = duration_secs {
        println!("  Probe running for {} seconds (or until closed)...", sec);
    } else {
        println!(
            "  Left-click toggles body color; right-click opens Above/Normal/Below/Dismiss/Quit. Launch with --duration <SECONDS> for automatic termination."
        );
    }

    let mut runtime = ProbeRuntime::new();
    let result = run_probe(
        &conn,
        screen,
        duration_secs,
        &atoms,
        selected_monitor,
        &mut probe_window,
        &mut renderer,
        &mut runtime,
        valid_bounds,
        layout.usable_area,
    );
    let outcome = finish_probe_with(result, || {
        abort_layer(&mut runtime.layers, "shutdown");
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
    Chord,
}

#[derive(Clone, Copy)]
enum ButtonObservation {
    Held,
    Released,
    Chord,
}

#[derive(Default)]
struct GestureSafety {
    generation: u64,
    deadline: Option<Instant>,
    observation: Option<(u64, ButtonObservation, u64)>,
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
        self.observation = Some((
            self.generation,
            if pressed {
                ButtonObservation::Held
            } else {
                ButtonObservation::Released
            },
            sequence,
        ));
        self.deadline = None;
    }
    fn observed_buttons(&mut self, button: u8, buttons: u16, preserve_chords: bool, sequence: u64) {
        let pressed = if button == 0 {
            buttons != 0
        } else {
            buttons & (1 << (button - 1)) != 0
        };
        self.observed(pressed, sequence);
        if preserve_chords && button != 0 && buttons & !(1 << (button - 1)) != 0 {
            self.observation = Some((self.generation, ButtonObservation::Chord, sequence));
        }
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
        let Some((generation, observation, _)) = self.observation.take() else {
            return SafetyDecision::NoChange;
        };
        if !active || generation != self.generation {
            return SafetyDecision::NoChange;
        }
        match observation {
            ButtonObservation::Held => {
                self.deadline = Some(now + POINTER_CHECK_INTERVAL);
                SafetyDecision::HoldConfirmed
            }
            ButtonObservation::Released => {
                self.clear();
                SafetyDecision::Cancel
            }
            ButtonObservation::Chord => {
                self.clear();
                SafetyDecision::Chord
            }
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
    menu: Option<MenuPopup>,
    menu_generation: u64,
    layers: LayerController,
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
            menu: None,
            menu_generation: 0,
            layers: LayerController::default(),
        }
    }

    fn apply_safety(&mut self, now: Instant, next_sequence: Option<u64>) -> bool {
        match self.safety.apply(
            self.interaction.active_button().is_some(),
            now,
            next_sequence,
        ) {
            SafetyDecision::NoChange => false,
            SafetyDecision::HoldConfirmed => {
                self.interaction.confirm_button_held();
                false
            }
            SafetyDecision::Cancel => true,
            SafetyDecision::Chord => {
                self.interaction.suppress_observed_chord();
                self.safety.acquired(now);
                self.interaction.confirm_button_held();
                false
            }
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
        self.cancel_at(conn, window, x11rb::CURRENT_TIME)
    }
    fn cancel_at(
        &mut self,
        conn: &impl Connection,
        window: &mut ManagedProbeWindow,
        time: u32,
    ) -> Result<(), HostError> {
        self.cancel_menu_with(
            window,
            |pointer| pointer.release_if_held(conn, time).map(|_| ()),
            |popup| popup.destroy(conn),
        )
    }
    fn cancel_menu_with(
        &mut self,
        window: &mut ManagedProbeWindow,
        release: impl FnOnce(&mut PointerCaptureTracker) -> Result<(), HostError>,
        destroy: impl FnOnce(&MenuPopup) -> Result<(), HostError>,
    ) -> Result<(), HostError> {
        let release = self.cancel_with(window, release);
        // Even failed release must not skip popup cleanup. Retain resources on failure.
        cleanup_menu_with(&mut self.menu, release, destroy)
    }
    /// The selecting popup's obligations must be settled before dispatch. Later
    /// pointer ownership is deliberately absent from the layer controller.
    fn complete_menu_with(
        &mut self,
        window: &mut ManagedProbeWindow,
        outcome: MenuOutcome,
        release: impl FnOnce(&mut PointerCaptureTracker) -> Result<(), HostError>,
        destroy: impl FnOnce(&MenuPopup) -> Result<(), HostError>,
        dispatch: impl FnOnce(&mut LayerController, Layer) -> Result<(), HostError>,
    ) -> Result<bool, HostError> {
        self.cancel_menu_with(window, release, destroy)?;
        if self.pointer.is_grabbed() || self.menu.is_some() {
            return Err("Menu obligations remain before action dispatch".into());
        }
        match outcome {
            MenuOutcome::Dismiss => println!("[MENU] Dismissed"),
            MenuOutcome::Quit => {
                println!("[MENU] Quit selected");
                abort_layer(&mut self.layers, "Quit");
                return Ok(true);
            }
            MenuOutcome::SetLayer(target) => {
                if self.body != BodyAvailability::Ready {
                    eprintln!("[LAYER] Request {target:?} rejected: body placement is not ready");
                } else {
                    dispatch(&mut self.layers, target)?;
                }
            }
        }
        Ok(false)
    }
    fn sync_safety(&mut self, previous: Option<u8>) {
        let active = self.interaction.active_button();
        if active != previous {
            if active.is_some() {
                self.safety.acquired(Instant::now());
            } else {
                self.safety.clear();
            }
        }
    }
    fn open_menu(
        &mut self,
        conn: &impl Connection,
        screen: &x11rb::protocol::xproto::Screen,
        window: &mut ManagedProbeWindow,
        area: crate::geometry::Rect,
        anchor: Point,
        time: u32,
    ) -> Result<(), HostError> {
        if self.pointer.is_grabbed() || self.menu.is_some() {
            return Err("Cannot open popup with an existing capture/resource owner".into());
        }
        self.menu_generation = self.menu_generation.wrapping_add(1);
        let Some(popup) = MenuPopup::create(conn, screen, area, anchor, self.menu_generation)?
        else {
            return Ok(());
        };
        let popup_window = popup.window;
        let status =
            self.acquire_menu_with(popup, anchor, || grab_pointer(conn, popup_window, time))?;
        if status != GrabStatus::SUCCESS {
            eprintln!("[WARN] Menu pointer capture denied: {status:?}");
            return self.cancel(conn, window, "menu acquisition denied");
        }
        let popup = self.menu.as_mut().unwrap();
        popup.paint(conn, self.interaction.state())?;
        println!("[MENU] Opened popup 0x{popup_window:x}");
        Ok(())
    }
    fn acquire_menu_with(
        &mut self,
        popup: MenuPopup,
        anchor: Point,
        acquire: impl FnOnce() -> Result<(GrabStatus, u64), HostError>,
    ) -> Result<GrabStatus, HostError> {
        if self.menu.is_some() || self.pointer.is_grabbed() {
            return Err("Cannot acquire a second popup/capture owner".into());
        }
        let owner = CaptureOwner::Menu {
            window: popup.window,
            generation: popup.input.generation,
        };
        self.menu = Some(popup);
        let (status, sequence) = self.pointer.acquire_with(owner, acquire)?;
        if status == GrabStatus::SUCCESS {
            let popup = self.menu.as_mut().unwrap();
            popup.input.acquisition_sequence = sequence;
            self.interaction.menu_acquired(popup.hit(anchor, true));
        }
        Ok(status)
    }
    fn unavailable(&mut self, destroyed: bool) {
        abort_layer(&mut self.layers, "body unavailable");
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
            && self.menu.is_none()
            && !matches!(self.pointer.owner(), Some(CaptureOwner::Menu { .. }))
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

fn abort_layer(layers: &mut LayerController, reason: &str) {
    let pending = layers.abort();
    if pending.is_some() {
        eprintln!(
            "[LAYER] Operation ended: {reason}; target={:?}, last readable={:?}, phase={:?}",
            layers.desired,
            layers.observed,
            pending.map(|p| p.phase)
        );
    }
}

fn read_layer_with(
    layers: &mut LayerController,
    read: &mut impl FnMut() -> Result<PropertyResult<ObservedLayer>, HostError>,
) -> Result<Option<ObservedLayer>, HostError> {
    match read()? {
        Ok(observed) => Ok(Some(observed)),
        Err(reason) => {
            eprintln!(
                "[LAYER] Unverifiable state: {reason}; last readable={:?}",
                layers.observed
            );
            abort_layer(layers, reason);
            Ok(None)
        }
    }
}

fn validate_layer_support(
    layers: &mut LayerController,
    advertised: PropertyResult<LayerSupport>,
) -> bool {
    let Some(pending) = layers.pending() else {
        return true;
    };
    match advertised {
        Ok(support) if support.allows(pending.required) => true,
        support => {
            eprintln!(
                "[LAYER] Required advertised support {:?} unavailable: {support:?}",
                pending.required
            );
            abort_layer(layers, "unsupported or malformed _NET_SUPPORTED");
            false
        }
    }
}

/// Bounded orchestration shared by real requests and deterministic host fakes.
/// Reads return protocol errors separately from recoverable property problems.
fn service_layer_with(
    layers: &mut LayerController,
    request: Option<Layer>,
    mut read: impl FnMut() -> Result<PropertyResult<ObservedLayer>, HostError>,
    mut support: impl FnMut() -> Result<PropertyResult<LayerSupport>, HostError>,
    mut send: impl FnMut(Mutation) -> Result<(), HostError>,
    mut now: impl FnMut() -> Instant,
) -> Result<(), HostError> {
    if let Some(target) = request {
        if layers.pending().is_some() {
            println!(
                "[LAYER] Request {target:?} rejected: busy with {:?}",
                layers.desired
            );
            return Ok(());
        }
        println!("[LAYER] Requested {target:?}");
    }
    let accepted = now();
    let result = (|| {
        let Some(observed) = read_layer_with(layers, &mut read)? else {
            return Ok(());
        };
        let previous = layers.observed;
        let mut step = match request {
            Some(target) => layers.begin(target, observed, accepted),
            None => layers.observe(observed, now()),
        };
        if previous != Some(observed) {
            println!("[LAYER] Fresh observed state: {observed:?}");
        }
        loop {
            match step {
                Step::Send(mutation) => {
                    // The entire planned transition is preflighted before its
                    // first mutation and rechecked before a possible addition.
                    // If time expires during a read/check, only a final read is allowed.
                    if !layers.deadline().is_some_and(|deadline| now() >= deadline) {
                        let advertised = support()?;
                        if !layers.deadline().is_some_and(|deadline| now() >= deadline)
                            && !validate_layer_support(layers, advertised)
                        {
                            return Ok(());
                        }
                    }
                    if layers.deadline().is_some_and(|deadline| now() >= deadline) {
                        let Some(observed) = read_layer_with(layers, &mut read)? else {
                            return Ok(());
                        };
                        step = layers.observe(observed, now());
                        continue;
                    }
                    send(mutation)?;
                    println!("[LAYER] X11 checked {mutation:?}; awaiting fresh WM state");
                    let Some(observed) = read_layer_with(layers, &mut read)? else {
                        return Ok(());
                    };
                    println!("[LAYER] Fresh observed state: {observed:?}");
                    step = layers.observe(observed, now());
                }
                Step::AlreadyMatches => {
                    println!(
                        "[LAYER] Already matches {:?}; no mutation or support proof",
                        layers.desired
                    );
                    return Ok(());
                }
                Step::Confirmed => {
                    println!("[LAYER] Confirmed property state {:?}; visual stacking requires owner observation", layers.observed);
                    return Ok(());
                }
                Step::TimedOut(phase) => {
                    eprintln!("[LAYER] Timeout: target={:?}, last readable={:?}, phase={phase:?}; no retry", layers.desired, layers.observed);
                    return Ok(());
                }
                Step::Busy | Step::Waiting => return Ok(()),
            }
        }
    })();
    if result.is_err() {
        abort_layer(layers, "fatal X11 layer operation failure");
    }
    result
}

fn service_layer(
    conn: &impl Connection,
    atoms: &LayerAtoms,
    root: u32,
    body: u32,
    layers: &mut LayerController,
    request: Option<Layer>,
) -> Result<(), HostError> {
    service_layer_with(
        layers,
        request,
        || atoms.read_body(conn, body),
        || atoms.read_support(conn, root),
        |mutation| atoms.send(conn, root, body, mutation),
        Instant::now,
    )
}

fn layer_property_event(event: &Event, body: u32, atoms: &LayerAtoms) -> bool {
    matches!(event, Event::PropertyNotify(ev) if ev.window == body && ev.atom == atoms.state)
}

fn service_layer_support_event_with(
    layers: &mut LayerController,
    read_support: impl FnOnce() -> Result<PropertyResult<LayerSupport>, HostError>,
    final_observation: impl FnOnce(&mut LayerController) -> Result<(), HostError>,
    mut now: impl FnMut() -> Instant,
) -> Result<(), HostError> {
    let Some(deadline) = layers.deadline() else {
        return Ok(());
    };
    // An expired operation needs only its final body observation, not capabilities.
    if now() >= deadline {
        return final_observation(layers);
    }
    let advertised = read_support()?;
    if now() >= deadline {
        final_observation(layers)
    } else {
        validate_layer_support(layers, advertised);
        Ok(())
    }
}

fn interrupts_layer(
    event: &Event,
    root: u32,
    body: u32,
    atoms: &x11::monitors::LayoutAtoms,
) -> bool {
    relevant_layout_event(event, root, body, atoms)
        || matches!(event, Event::MapNotify(ev) if ev.window == body)
        || matches!(event, Event::UnmapNotify(ev) if ev.window == body)
        || matches!(event, Event::DestroyNotify(ev) if ev.window == body)
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

fn held_buttons(state: x11rb::protocol::xproto::KeyButMask) -> u16 {
    (u16::from(state) >> 8) & 31
}
fn buttons_after_release(state: x11rb::protocol::xproto::KeyButMask, button: u8) -> u16 {
    let buttons = held_buttons(state);
    if (1..=5).contains(&button) {
        buttons & !(1 << (button - 1))
    } else {
        buttons
    }
}
fn pointer_event(event: &Event) -> Option<(u32, u32, Point, bool, bool)> {
    match event {
        Event::ButtonPress(ev) | Event::ButtonRelease(ev) => Some((
            ev.event,
            ev.time,
            Point::new(ev.root_x.into(), ev.root_y.into()),
            ev.same_screen,
            ev.response_type & 0x80 != 0,
        )),
        Event::MotionNotify(ev) => Some((
            ev.event,
            ev.time,
            Point::new(ev.root_x.into(), ev.root_y.into()),
            ev.same_screen,
            ev.response_type & 0x80 != 0,
        )),
        _ => None,
    }
}
fn popup_lifecycle(event: &Event, popup: Option<PopupInputGate>, sequence: u64) -> bool {
    let Some(popup) = popup else {
        return false;
    };
    if sequence < popup.creation_sequence {
        return false;
    }
    match event {
        Event::UnmapNotify(ev) => popup.window == ev.window && ev.response_type & 0x80 == 0,
        Event::DestroyNotify(ev) => popup.window == ev.window && ev.response_type & 0x80 == 0,
        _ => false,
    }
}
/// Release has already been attempted; destruction must still run on failure.
fn cleanup_menu_with<T>(
    popup: &mut Option<T>,
    release: Result<(), HostError>,
    destroy: impl FnOnce(&T) -> Result<(), HostError>,
) -> Result<(), HostError> {
    let cleanup = popup.as_ref().map_or(Ok(()), destroy);
    let result = release.and(cleanup);
    if result.is_ok() {
        *popup = None;
    }
    result
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
    screen: &x11rb::protocol::xproto::Screen,
    duration_secs: Option<u64>,
    atoms: &x11::monitors::LayoutAtoms,
    selected_monitor: u32,
    window: &mut ManagedProbeWindow,
    renderer: &mut Renderer,
    runtime: &mut ProbeRuntime,
    initial_bounds: crate::geometry::ValidOriginBounds,
    initial_area: crate::geometry::Rect,
) -> Result<(), HostError> {
    let root = screen.root;
    let layer_atoms = LayerAtoms::intern(conn)?;
    let mut usable_area = initial_area;
    let mut bounds = initial_bounds;
    let mut buffered_event = None;
    let start = Instant::now();
    let duration_deadline = duration_secs
        .map(|sec| {
            start
                .checked_add(Duration::from_secs(sec))
                .ok_or("Duration exceeds supported monotonic deadline")
        })
        .transpose()?;
    loop {
        let now = Instant::now();
        if duration_deadline.is_some_and(|dl| now >= dl) {
            abort_layer(&mut runtime.layers, "duration expiry");
            runtime.cancel(conn, window, "duration expiry")?;
            return Ok(());
        }
        if runtime
            .layers
            .deadline()
            .is_some_and(|deadline| now >= deadline)
        {
            service_layer(
                conn,
                &layer_atoms,
                root,
                window.window,
                &mut runtime.layers,
                None,
            )?;
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
            if runtime.menu.is_some()
                && !matches!(
                    runtime.interaction.state(),
                    InteractionState::MenuOpen { .. }
                )
            {
                runtime.cancel(conn, window, "movement confirmation cancellation")?;
            }
            if runtime.interaction.active_button().is_none() {
                runtime.safety.clear();
            }
        }
        if runtime.safety.deadline.is_some_and(|dl| now >= dl) {
            let button = runtime
                .interaction
                .active_button()
                .ok_or("Safety timer without an initiating button")?;
            let (sequence, buttons) = crate::x11::pointer::button_state(conn, root)?;
            runtime.safety.observed_buttons(
                button,
                buttons,
                !runtime.interaction.has_left_gesture(),
                sequence,
            );
        }

        let batch = drain_events_bounded(&mut buffered_event, 64, || {
            conn.poll_for_event_with_sequence()
        })?;
        for (event, sequence) in batch {
            // Preserve the original fatal diagnostic even if release/cleanup also fails.
            if let Event::Error(error) = &event {
                return Err(format!("Asynchronous X11 error: {error:?}").into());
            }
            if interrupts_layer(&event, root, window.window, atoms) {
                abort_layer(&mut runtime.layers, "layout or body lifecycle change");
            }
            if popup_lifecycle(
                &event,
                runtime.menu.as_ref().map(|popup| popup.input),
                sequence,
            ) {
                if let Event::DestroyNotify(_) = &event {
                    runtime
                        .menu
                        .as_ref()
                        .unwrap()
                        .window_resource
                        .externally_destroyed();
                }
                runtime.cancel(conn, window, "popup unavailable")?;
                continue;
            }
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
            let body_right = matches!(&event, Event::ButtonPress(ev) if ev.event == window.window && ev.detail == 3);
            if interrupts_pending_movement(&event, root, window.window, atoms)
                && (!body_right || right_cancelled)
            {
                runtime.cancel(conn, window, "host or right-button interruption")?;
            } else if !matches!(event, Event::MotionNotify(_)) {
                runtime.flush_move(conn, window)?;
            }
            if relevant_layout_event(&event, root, window.window, atoms) {
                let layout = query_desktop_layout(conn, root, atoms, Some(selected_monitor))?;
                usable_area = layout.usable_area;
                bounds = checked_probe_bounds(usable_area)?;
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
            if layer_property_event(&event, window.window, &layer_atoms) {
                if matches!(
                    runtime.body,
                    BodyAvailability::Ready | BodyAvailability::Validating
                ) {
                    service_layer(
                        conn,
                        &layer_atoms,
                        root,
                        window.window,
                        &mut runtime.layers,
                        None,
                    )?;
                }
                continue;
            }
            if matches!(&event, Event::PropertyNotify(ev) if ev.window == root && ev.atom == layer_atoms.supported)
            {
                service_layer_support_event_with(
                    &mut runtime.layers,
                    || layer_atoms.read_support(conn, root),
                    |layers| service_layer(conn, &layer_atoms, root, window.window, layers, None),
                    Instant::now,
                )?;
                continue;
            }
            if let Some(popup) = runtime.menu.as_ref() {
                if let Event::Expose(ev) = &event {
                    if ev.window == popup.window && ev.count == 0 {
                        popup.paint(conn, runtime.interaction.state())?;
                        continue;
                    }
                }
                if let Some((event_window, time, root_point, same_screen, synthetic)) =
                    pointer_event(&event)
                {
                    if popup.input.accepts(
                        event_window,
                        runtime.menu_generation,
                        sequence,
                        synthetic,
                    ) {
                        let hit = popup.hit(root_point, same_screen);
                        let previous = runtime.interaction.active_button();
                        let action = match &event {
                            Event::MotionNotify(_) => {
                                runtime.interaction.menu_motion(hit, time);
                                HostAction::None
                            }
                            Event::ButtonPress(ev) => {
                                runtime.interaction.menu_press(
                                    ev.detail,
                                    hit,
                                    time,
                                    held_buttons(ev.state),
                                );
                                HostAction::None
                            }
                            Event::ButtonRelease(ev) => runtime.interaction.menu_release(
                                ev.detail,
                                hit,
                                time,
                                buttons_after_release(ev.state, ev.detail),
                            ),
                            _ => HostAction::None,
                        };
                        runtime.sync_safety(previous);
                        if let HostAction::CloseMenu { time, outcome } = action {
                            let body = window.window;
                            if runtime.complete_menu_with(
                                window,
                                outcome,
                                |pointer| pointer.release_if_held(conn, time).map(|_| ()),
                                |popup| popup.destroy(conn),
                                |layers, target| {
                                    service_layer(
                                        conn,
                                        &layer_atoms,
                                        root,
                                        body,
                                        layers,
                                        Some(target),
                                    )
                                },
                            )? {
                                return Ok(());
                            }
                        }
                    }
                    continue;
                }
            }
            match event {
                Event::MapNotify(ev) if ev.window == window.window => {
                    runtime.cancel(conn, window, "map verification")?;
                    let layout = query_desktop_layout(conn, root, atoms, Some(selected_monitor))?;
                    usable_area = layout.usable_area;
                    bounds = checked_probe_bounds(usable_area)?;
                    if window.verified_visible(conn, layout.current_desktop, atoms.wm_desktop)? {
                        runtime.body = BodyAvailability::Validating;
                        runtime.placement(conn, root, window, &bounds)?;
                        println!("[PLACEMENT] Verified map at {}", window.confirmed_origin());
                        service_layer(
                            conn,
                            &layer_atoms,
                            root,
                            window.window,
                            &mut runtime.layers,
                            None,
                        )?;
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
                    if ev.response_type & 0x80 != 0 {
                        continue;
                    }
                    if matches!(
                        runtime.interaction.state(),
                        InteractionState::RightPressed { .. }
                    ) {
                        let previous = runtime.interaction.active_button();
                        runtime.interaction.opening_chord(ev.time);
                        runtime.sync_safety(previous);
                        continue;
                    }
                    match ev.detail {
                        1 if held_buttons(ev.state) == 0 && !runtime.pointer.is_grabbed() => {
                            let origin = window.query_actual_root_origin(conn, root)?;
                            if let HostAction::AcquireGrab { time } =
                                runtime.interaction.handle_left_press(
                                    Point::new(ev.root_x.into(), ev.root_y.into()),
                                    (ev.event_x, ev.event_y),
                                    ev.time,
                                    origin,
                                )
                            {
                                match runtime
                                    .pointer
                                    .acquire_with(CaptureOwner::BodyLeft, || {
                                        grab_pointer(conn, window.window, time)
                                    })?
                                    .0
                                {
                                    GrabStatus::SUCCESS => {
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
                        3 if !right_cancelled
                            && held_buttons(ev.state) == 0
                            && !runtime.pointer.is_grabbed()
                            && crate::geometry::is_in_interactive_silhouette(
                                ev.event_x, ev.event_y,
                            )
                            && runtime.interaction.handle_right_press(ev.time)
                                == HostAction::TrackOpening =>
                        {
                            runtime.pointer.track_automatic_right()?;
                            runtime.safety.acquired(Instant::now());
                            println!("[INPUT] Right opening gesture pending");
                        }
                        _ => {}
                    }
                }
                Event::ButtonRelease(ev) if ev.event == window.window => {
                    if ev.response_type & 0x80 != 0 {
                        continue;
                    }
                    if matches!(
                        runtime.interaction.state(),
                        InteractionState::RightPressed { .. }
                    ) {
                        let previous = runtime.interaction.active_button();
                        let anchor = Point::new(ev.root_x.into(), ev.root_y.into());
                        let origin = window.query_actual_root_origin(conn, root)?;
                        let local = Point::new(anchor.x - origin.x, anchor.y - origin.y);
                        let on_body = ev.same_screen
                            && i16::try_from(local.x)
                                .ok()
                                .zip(i16::try_from(local.y).ok())
                                .is_some_and(|(x, y)| {
                                    crate::geometry::is_in_interactive_silhouette(x, y)
                                });
                        let action = runtime.interaction.handle_opening_release(
                            ev.detail,
                            ev.time,
                            buttons_after_release(ev.state, ev.detail),
                            anchor,
                            on_body,
                        );
                        runtime.sync_safety(previous);
                        if let HostAction::OpeningFinished { time, anchor } = action {
                            runtime.pointer.automatic_right_finished()?;
                            if let Some(anchor) = anchor {
                                runtime.open_menu(
                                    conn,
                                    screen,
                                    window,
                                    usable_area,
                                    anchor,
                                    time,
                                )?;
                            }
                        }
                        continue;
                    }
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

        if let Some(popup) = runtime.menu.as_mut() {
            popup.repaint_if_changed(conn, runtime.interaction.state())?;
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
            runtime.layers.deadline(),
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
    pub(super) fn test_window() -> ManagedProbeWindow {
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

#[cfg(test)]
mod menu_runtime_tests {
    use super::*;
    use crate::geometry::MenuHit;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use x11rb::protocol::xproto::{
        ButtonReleaseEvent, DestroyNotifyEvent, KeyButMask, MotionNotifyEvent, Property,
        PropertyNotifyEvent, UnmapNotifyEvent,
    };

    fn captured_menu() -> ProbeRuntime {
        let mut runtime = ProbeRuntime::new();
        runtime.body = BodyAvailability::Ready;
        runtime.menu_generation = 2;
        runtime.menu = Some(MenuPopup::test_popup(10, 2));
        runtime
            .pointer
            .acquire_with(
                CaptureOwner::Menu {
                    window: 10,
                    generation: 2,
                },
                || Ok((GrabStatus::SUCCESS, 70000)),
            )
            .unwrap();
        runtime.interaction.menu_acquired(MenuHit::Outside);
        runtime
    }

    #[test]
    fn cancellation_invalidates_menu_then_ungrabs_before_destroy_and_is_idempotent() {
        let mut runtime = captured_menu();
        let mut body = tests::test_window();
        runtime.interaction.menu_press(1, MenuHit::Row(4), 100, 0);
        runtime.safety.acquired(Instant::now());
        runtime.pending_move = Some(Point::new(200, 200));
        let trace = RefCell::new(Vec::new());
        runtime
            .cancel_menu_with(
                &mut body,
                |pointer| {
                    pointer
                        .release_with(|| {
                            trace.borrow_mut().push("ungrab");
                            Ok(())
                        })
                        .map(|_| ())
                },
                |_| {
                    trace.borrow_mut().push("popup");
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(*trace.borrow(), ["ungrab", "popup"]);
        assert!(runtime.interaction.is_idle());
        assert!(runtime.pending_move.is_none());
        assert!(runtime.safety.deadline.is_none());
        assert!(!runtime.pointer.is_grabbed());
        assert!(runtime.menu.is_none());
        assert!(runtime.accepts_input());
        runtime
            .cancel_menu_with(
                &mut body,
                |pointer| {
                    pointer
                        .release_with(|| panic!("duplicate ungrab"))
                        .map(|_| ())
                },
                |_| panic!("duplicate popup destroy"),
            )
            .unwrap();
    }

    #[test]
    fn failed_release_still_destroys_popup_and_cleanup_retry_retains_owner_until_ack() {
        let mut runtime = captured_menu();
        let mut body = tests::test_window();
        let calls = Cell::new(0);
        let error = runtime
            .cancel_menu_with(
                &mut body,
                |pointer| {
                    pointer
                        .release_with(|| Err("release not acknowledged".into()))
                        .map(|_| ())
                },
                |popup| {
                    popup.window_resource.release_with(|| {
                        calls.set(calls.get() + 1);
                        Ok(())
                    })
                },
            )
            .unwrap_err();
        assert_eq!(error.to_string(), "release not acknowledged");
        assert!(runtime.interaction.is_idle());
        assert!(runtime.pointer.is_grabbed());
        assert!(runtime.menu.is_some());
        assert!(!runtime.accepts_input());
        runtime
            .cancel_menu_with(
                &mut body,
                |pointer| pointer.release_with(|| Ok(())).map(|_| ()),
                |popup| {
                    popup
                        .window_resource
                        .release_with(|| panic!("acknowledged destroy repeated"))
                },
            )
            .unwrap();
        assert_eq!(calls.get(), 1);
        assert!(runtime.menu.is_none());
        assert!(!runtime.pointer.is_grabbed());
    }

    #[test]
    fn fatal_error_keeps_priority_over_popup_release_and_destroy_failures() {
        let mut runtime = captured_menu();
        let mut body = tests::test_window();
        let destroyed = Cell::new(false);
        let error = finish_probe_with(Err("original X11 failure".into()), || {
            runtime.cancel_menu_with(
                &mut body,
                |pointer| {
                    pointer
                        .release_with(|| Err("release failure".into()))
                        .map(|_| ())
                },
                |_| {
                    destroyed.set(true);
                    Err("destroy failure".into())
                },
            )
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "original X11 failure");
        assert!(destroyed.get());
        assert!(runtime.pointer.is_grabbed());
    }

    #[test]
    fn pending_opening_cancellation_releases_automatic_grab_and_blocks_late_release() {
        let mut runtime = ProbeRuntime::new();
        let mut body = tests::test_window();
        runtime.interaction.handle_right_press(100);
        runtime.pointer.track_automatic_right().unwrap();
        runtime.safety.acquired(Instant::now());
        let releases = Cell::new(0);
        runtime
            .cancel_menu_with(
                &mut body,
                |pointer| {
                    pointer
                        .release_with(|| {
                            releases.set(releases.get() + 1);
                            Ok(())
                        })
                        .map(|_| ())
                },
                |_| panic!("no popup exists"),
            )
            .unwrap();
        assert_eq!(releases.get(), 1);
        assert_eq!(
            runtime
                .interaction
                .handle_opening_release(3, 101, 0, Point::new(180, 180), true),
            HostAction::None
        );
        assert_eq!(runtime.pointer.owner(), None);
        assert!(runtime.safety.deadline.is_none());
    }

    #[test]
    fn long_menu_hold_rearms_and_missed_release_cancels_without_activation() {
        let mut runtime = captured_menu();
        runtime.interaction.menu_press(1, MenuHit::Row(4), 100, 0);
        runtime.sync_safety(None);
        let now = Instant::now();
        for sequence in 70000..70100 {
            runtime.safety.observed(true, sequence);
            assert!(!runtime.apply_safety(now, None));
            assert_eq!(runtime.safety.deadline, Some(now + POINTER_CHECK_INTERVAL));
        }
        runtime.safety.observed(false, 70100);
        assert!(runtime.apply_safety(now, None));
        let mut body = tests::test_window();
        runtime
            .cancel_menu_with(
                &mut body,
                |pointer| pointer.release_with(|| Ok(())).map(|_| ()),
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(
            runtime.interaction.menu_release(1, MenuHit::Row(4), 110, 0),
            HostAction::None
        );
    }

    #[test]
    fn menu_neutral_has_no_timer_and_chord_invalidates_old_button_observation() {
        let mut runtime = captured_menu();
        assert!(runtime.safety.deadline.is_none());
        runtime.interaction.menu_press(1, MenuHit::Row(4), 100, 0);
        runtime.sync_safety(None);
        runtime.safety.observed(false, 70000);
        runtime.interaction.menu_press(2, MenuHit::Row(4), 101, 1);
        runtime.sync_safety(Some(1));
        assert_eq!(runtime.interaction.active_button(), Some(0));
        assert!(!runtime.apply_safety(Instant::now(), None));
        assert!(runtime.safety.observation.is_none());
        assert!(runtime.safety.deadline.is_some());
        runtime.interaction.menu_release(1, MenuHit::Row(4), 102, 2);
        runtime.interaction.menu_release(2, MenuHit::Row(4), 103, 0);
        runtime.sync_safety(Some(0));
        assert!(runtime.menu.is_some());
        assert!(runtime.pointer.is_grabbed());
        assert!(runtime.safety.deadline.is_none());
    }

    #[test]
    fn queued_menu_completion_precedes_observation_across_bounded_batches() {
        let mut runtime = captured_menu();
        runtime.interaction.menu_press(1, MenuHit::Outside, 100, 0);
        runtime.sync_safety(None);
        runtime.safety.observed(false, 70100);
        let motion = Event::MotionNotify(MotionNotifyEvent {
            response_type: 6,
            detail: Default::default(),
            sequence: 0,
            time: 101,
            root: 1,
            event: 10,
            child: 0,
            root_x: 500,
            root_y: 500,
            event_x: 700,
            event_y: 452,
            state: KeyButMask::BUTTON1,
            same_screen: true,
        });
        let release = Event::ButtonRelease(ButtonReleaseEvent {
            response_type: 5,
            detail: 1,
            sequence: 0,
            time: 102,
            root: 1,
            event: 10,
            child: 0,
            root_x: 500,
            root_y: 500,
            event_x: 700,
            event_y: 452,
            state: KeyButMask::BUTTON1,
            same_screen: true,
        });
        let mut queue = VecDeque::from(vec![(motion, 70099); 130]);
        queue.push_back((release, 70099));
        let mut buffered = None;
        let mut completions = 0;
        while !queue.is_empty() {
            let batch =
                drain_events_bounded(&mut buffered, 64, || Ok::<_, HostError>(queue.pop_front()))
                    .unwrap();
            for (event, sequence) in batch {
                assert!(!runtime.apply_safety(Instant::now(), Some(sequence)));
                if let Event::ButtonRelease(ev) = event {
                    assert_eq!(
                        runtime.interaction.menu_release(
                            ev.detail,
                            MenuHit::Outside,
                            ev.time,
                            buttons_after_release(ev.state, ev.detail)
                        ),
                        HostAction::CloseMenu {
                            time: 102,
                            outcome: MenuOutcome::Dismiss
                        }
                    );
                    runtime.sync_safety(Some(1));
                    completions += 1;
                }
            }
        }
        assert_eq!(completions, 1);
        assert!(!runtime.apply_safety(Instant::now(), None));
    }

    #[test]
    fn layout_and_window_lifecycle_identify_interruptions_before_refresh() {
        let atoms = x11::monitors::LayoutAtoms {
            current_desktop: 20,
            workarea: 21,
            wm_desktop: 22,
        };
        let root = 1;
        let body = 100;
        let events = [
            Event::PropertyNotify(PropertyNotifyEvent {
                response_type: 28,
                sequence: 0,
                window: root,
                atom: 20,
                time: 100,
                state: Property::NEW_VALUE,
            }),
            Event::PropertyNotify(PropertyNotifyEvent {
                response_type: 28,
                sequence: 0,
                window: root,
                atom: 21,
                time: 100,
                state: Property::NEW_VALUE,
            }),
            Event::UnmapNotify(UnmapNotifyEvent {
                response_type: 18,
                sequence: 0,
                event: root,
                window: body,
                from_configure: false,
            }),
            Event::DestroyNotify(DestroyNotifyEvent {
                response_type: 17,
                sequence: 0,
                event: root,
                window: body,
            }),
            Event::UnmapNotify(UnmapNotifyEvent {
                response_type: 18,
                sequence: 0,
                event: root,
                window: 10,
                from_configure: false,
            }),
            Event::DestroyNotify(DestroyNotifyEvent {
                response_type: 17,
                sequence: 0,
                event: root,
                window: 10,
            }),
        ];
        for event in events {
            let mut runtime = captured_menu();
            assert!(
                interrupts_pending_movement(&event, root, body, &atoms)
                    || popup_lifecycle(
                        &event,
                        runtime.menu.as_ref().map(|popup| popup.input),
                        70000
                    )
            );
            let mut window = tests::test_window();
            runtime
                .cancel_menu_with(
                    &mut window,
                    |pointer| pointer.release_with(|| Ok(())).map(|_| ()),
                    |_| Ok(()),
                )
                .unwrap();
            assert!(runtime.menu.is_none());
            assert!(!runtime.pointer.is_grabbed());
            assert!(runtime.interaction.is_idle());
        }
        assert!(!popup_lifecycle(
            &Event::DestroyNotify(DestroyNotifyEvent {
                response_type: 17,
                sequence: 0,
                event: root,
                window: 9
            }),
            Some(MenuPopup::test_popup(10, 2).input),
            70000
        ));
        // Duration expiry uses the same cancellation path, including a held Quit row.
        let mut runtime = captured_menu();
        runtime.interaction.menu_press(1, MenuHit::Row(4), 100, 0);
        runtime.sync_safety(None);
        runtime
            .cancel_menu_with(
                &mut tests::test_window(),
                |pointer| pointer.release_with(|| Ok(())).map(|_| ()),
                |_| Ok(()),
            )
            .unwrap();
        assert!(runtime.menu.is_none());
        assert!(runtime.safety.deadline.is_none());
    }

    #[test]
    fn ordered_observed_chords_preserve_menu_and_automatic_owners_until_all_up() {
        for buttons in [2, 3] {
            let mut runtime = captured_menu();
            runtime.interaction.menu_press(1, MenuHit::Row(4), 100, 0);
            runtime.sync_safety(None);
            runtime.safety.observed_buttons(1, buttons, true, 70100);
            assert!(!runtime.apply_safety(Instant::now(), Some(70099)));
            assert_eq!(runtime.interaction.active_button(), Some(1));
            assert!(!runtime.apply_safety(Instant::now(), Some(70100)));
            assert_eq!(runtime.interaction.active_button(), Some(0));
            assert!(runtime.menu.is_some());
            assert!(runtime.pointer.is_grabbed());
            runtime.safety.observed_buttons(0, 2, true, 70200);
            assert!(!runtime.apply_safety(Instant::now(), None));
            runtime.safety.observed_buttons(0, 0, true, 70300);
            assert!(runtime.apply_safety(Instant::now(), None));
        }
        let mut runtime = ProbeRuntime::new();
        runtime.interaction.handle_right_press(100);
        runtime.pointer.track_automatic_right().unwrap();
        runtime.safety.acquired(Instant::now());
        runtime.safety.observed_buttons(3, 1, true, 1000);
        assert!(!runtime.apply_safety(Instant::now(), None));
        assert_eq!(runtime.pointer.owner(), Some(CaptureOwner::OpeningRight));
        assert_eq!(runtime.interaction.active_button(), Some(0));
        assert_eq!(
            runtime
                .interaction
                .handle_opening_release(1, 110, 0, Point::new(180, 180), true),
            HostAction::OpeningFinished {
                time: 110,
                anchor: None
            }
        );
        runtime.pointer.automatic_right_finished().unwrap();
    }

    #[test]
    fn popup_acquisition_success_and_each_denial_have_single_attempt_and_checked_teardown() {
        for status in [
            GrabStatus::SUCCESS,
            GrabStatus::ALREADY_GRABBED,
            GrabStatus::INVALID_TIME,
            GrabStatus::NOT_VIEWABLE,
            GrabStatus::FROZEN,
        ] {
            let mut runtime = ProbeRuntime::new();
            runtime.body = BodyAvailability::Ready;
            runtime.menu_generation = 2;
            let attempts = Cell::new(0);
            assert_eq!(
                runtime
                    .acquire_menu_with(MenuPopup::test_popup(10, 2), Point::new(-188, 60), || {
                        attempts.set(attempts.get() + 1);
                        Ok((status, 70000))
                    })
                    .unwrap(),
                status
            );
            assert_eq!(attempts.get(), 1);
            if status == GrabStatus::SUCCESS {
                assert!(matches!(
                    runtime.interaction.state(),
                    InteractionState::MenuOpen { .. }
                ));
                assert_eq!(
                    runtime.pointer.owner(),
                    Some(CaptureOwner::Menu {
                        window: 10,
                        generation: 2
                    })
                );
                assert_eq!(
                    runtime.menu.as_ref().unwrap().input.acquisition_sequence,
                    70000
                );
                assert!(runtime.safety.deadline.is_none());
            } else {
                assert!(runtime.interaction.is_idle());
                assert_eq!(runtime.pointer.owner(), None);
                let destroys = Cell::new(0);
                runtime
                    .cancel_menu_with(
                        &mut tests::test_window(),
                        |pointer| {
                            pointer
                                .release_with(|| panic!("denied grab must not ungrab"))
                                .map(|_| ())
                        },
                        |_| {
                            destroys.set(destroys.get() + 1);
                            Ok(())
                        },
                    )
                    .unwrap();
                assert_eq!(destroys.get(), 1);
                assert!(runtime.menu.is_none());
                assert!(runtime.accepts_input());
                assert_eq!(attempts.get(), 1);
            }
        }
    }

    #[test]
    fn acquisition_protocol_failure_retains_resources_for_primary_error_cleanup() {
        let mut runtime = ProbeRuntime::new();
        let result = runtime
            .acquire_menu_with(MenuPopup::test_popup(10, 2), Point::new(-188, 60), || {
                Err("grab reply failure".into())
            })
            .map(|_| ());
        assert!(runtime.menu.is_some());
        assert!(runtime.pointer.is_grabbed());
        assert!(runtime.interaction.is_idle());
        let trace = RefCell::new(Vec::new());
        let error = finish_probe_with(result, || {
            runtime.cancel_menu_with(
                &mut tests::test_window(),
                |pointer| {
                    pointer
                        .release_with(|| {
                            trace.borrow_mut().push("ungrab");
                            Ok(())
                        })
                        .map(|_| ())
                },
                |_| {
                    trace.borrow_mut().push("popup");
                    Ok(())
                },
            )
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "grab reply failure");
        assert_eq!(*trace.borrow(), ["ungrab", "popup"]);
        assert!(runtime.menu.is_none());
    }

    #[test]
    fn queued_previous_popup_destruction_cannot_cancel_reused_window_id() {
        let gate = MenuPopup::test_popup(10, 2).input;
        let destroy = Event::DestroyNotify(DestroyNotifyEvent {
            response_type: 17,
            sequence: 0,
            event: 1,
            window: 10,
        });
        assert!(!popup_lifecycle(
            &destroy,
            Some(gate),
            gate.creation_sequence - 1
        ));
        assert!(popup_lifecycle(
            &destroy,
            Some(gate),
            gate.creation_sequence
        ));
        let fabricated = Event::DestroyNotify(DestroyNotifyEvent {
            response_type: 17 | 0x80,
            sequence: 0,
            event: 1,
            window: 10,
        });
        assert!(!popup_lifecycle(
            &fabricated,
            Some(gate),
            gate.creation_sequence
        ));
    }

    #[test]
    fn popup_resources_block_body_input_until_cleanup_succeeds() {
        let mut runtime = captured_menu();
        assert!(!runtime.accepts_input());
        runtime.interaction.cancel(100);
        assert!(!runtime.accepts_input());
        let mut body = tests::test_window();
        assert!(runtime
            .cancel_menu_with(
                &mut body,
                |pointer| pointer.release_with(|| Ok(())).map(|_| ()),
                |_| Err("destroy unacknowledged".into())
            )
            .is_err());
        assert!(!runtime.accepts_input());
        assert!(runtime.menu.is_some());
    }
}

#[cfg(test)]
mod layer_runtime_tests {
    use super::*;
    use crate::geometry::{MenuHit, ValidOriginBounds};
    use crate::interaction::MenuItem;
    use crate::layer::{Flags, Phase};
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use x11rb::protocol::xproto::{Property, PropertyNotifyEvent};

    const SUPPORT: LayerSupport = LayerSupport {
        state: true,
        above: true,
        below: true,
    };

    fn captured_menu() -> ProbeRuntime {
        let mut runtime = ProbeRuntime::new();
        runtime.body = BodyAvailability::Ready;
        runtime
            .acquire_menu_with(MenuPopup::test_popup(10, 1), Point::new(0, 0), || {
                Ok((GrabStatus::SUCCESS, 100))
            })
            .unwrap();
        runtime
    }

    fn pending_above(layers: &mut LayerController, now: Instant) {
        let reads = Cell::new(0);
        service_layer_with(
            layers,
            Some(Layer::Above),
            || {
                reads.set(reads.get() + 1);
                Ok(Ok(ObservedLayer::Below))
            },
            || Ok(Ok(SUPPORT)),
            |mutation| {
                assert_eq!(
                    mutation,
                    Mutation::Remove(Flags {
                        above: false,
                        below: true
                    })
                );
                Ok(())
            },
            || now,
        )
        .unwrap();
        assert_eq!(reads.get(), 2); // preflight snapshot and fresh read after checked removal
        assert_eq!(layers.pending().unwrap().phase, Phase::AwaitRemoval);
    }

    #[test]
    fn checked_mutations_are_each_followed_by_fresh_read_and_complete_immediately() {
        let now = Instant::now();
        let mut layers = LayerController::default();
        let mut snapshots = VecDeque::from([
            ObservedLayer::Below,
            ObservedLayer::Normal,
            ObservedLayer::Above,
        ]);
        let trace = RefCell::new(Vec::new());
        service_layer_with(
            &mut layers,
            Some(Layer::Above),
            || {
                trace.borrow_mut().push("read");
                Ok(Ok(snapshots.pop_front().unwrap()))
            },
            || {
                trace.borrow_mut().push("support");
                Ok(Ok(SUPPORT))
            },
            |mutation| {
                trace.borrow_mut().push(match mutation {
                    Mutation::Remove(_) => "remove/check",
                    Mutation::AddAbove => "add/check",
                    _ => panic!("wrong mutation"),
                });
                Ok(())
            },
            || now,
        )
        .unwrap();
        assert_eq!(
            *trace.borrow(),
            [
                "read",
                "support",
                "remove/check",
                "read",
                "support",
                "add/check",
                "read"
            ]
        );
        assert_eq!(layers.observed, Some(ObservedLayer::Above));
        assert!(layers.deadline().is_none());
    }

    #[test]
    fn fresh_idempotence_needs_no_support_query_or_send() {
        let mut layers = LayerController::default();
        service_layer_with(
            &mut layers,
            Some(Layer::Normal),
            || Ok(Ok(ObservedLayer::Normal)),
            || panic!("no mutation support proof required"),
            |_| panic!("idempotent request must not send"),
            Instant::now,
        )
        .unwrap();
        assert!(layers.deadline().is_none());
    }

    #[test]
    fn whole_transition_support_is_checked_before_removal_and_again_before_addition() {
        let now = Instant::now();
        for support in [
            Ok(LayerSupport {
                above: false,
                ..SUPPORT
            }),
            Err("malformed support"),
        ] {
            let mut layers = LayerController::default();
            service_layer_with(
                &mut layers,
                Some(Layer::Above),
                || Ok(Ok(ObservedLayer::Below)),
                || Ok(support),
                |_| panic!("must reject before removal"),
                || now,
            )
            .unwrap();
            assert!(layers.pending().is_none());
            assert_eq!(layers.observed, Some(ObservedLayer::Below));
        }
        let mut layers = LayerController::default();
        pending_above(&mut layers, now);
        service_layer_with(
            &mut layers,
            None,
            || Ok(Ok(ObservedLayer::Normal)),
            || {
                Ok(Ok(LayerSupport {
                    above: false,
                    ..SUPPORT
                }))
            },
            |_| panic!("support lost: no addition or rollback"),
            || now,
        )
        .unwrap();
        assert_eq!(layers.observed, Some(ObservedLayer::Normal));
        assert!(layers.pending().is_none());
    }

    #[test]
    fn absent_or_malformed_body_never_confirms_normal_or_removal() {
        let now = Instant::now();
        for target in [Layer::Normal, Layer::Above] {
            for reason in ["mapped body state absent", "malformed state"] {
                let mut layers = LayerController::default();
                layers.begin(target, ObservedLayer::Below, now);
                service_layer_with(
                    &mut layers,
                    None,
                    || Ok(Err(reason)),
                    || panic!("no support query"),
                    |_| panic!("unverifiable removal cannot add"),
                    || now,
                )
                .unwrap();
                assert_eq!(layers.observed, Some(ObservedLayer::Below));
                assert!(layers.deadline().is_none());
            }
        }
    }

    #[test]
    fn later_drag_and_newer_popup_do_not_defer_addition_or_change_ownership() {
        let now = Instant::now();
        for popup in [false, true] {
            let mut runtime = ProbeRuntime::new();
            runtime.body = BodyAvailability::Ready;
            pending_above(&mut runtime.layers, now);
            if popup {
                runtime
                    .acquire_menu_with(MenuPopup::test_popup(10, 2), Point::new(0, 0), || {
                        Ok((GrabStatus::SUCCESS, 200))
                    })
                    .unwrap();
            } else {
                runtime.interaction.handle_left_press(
                    Point::new(80, 80),
                    (80, 80),
                    100,
                    Point::new(0, 0),
                );
                runtime
                    .pointer
                    .acquire_with(CaptureOwner::BodyLeft, || Ok((GrabStatus::SUCCESS, 10)))
                    .unwrap();
                runtime.interaction.on_grab_acquired();
                runtime.interaction.handle_motion(
                    Point::new(100, 100),
                    101,
                    &ValidOriginBounds::new(0, 500, 0, 500),
                );
                assert!(runtime.interaction.is_dragging());
            }
            let state = runtime.interaction.state();
            let owner = runtime.pointer.owner();
            let sends = Cell::new(0);
            let mut snapshots = VecDeque::from([ObservedLayer::Normal, ObservedLayer::Above]);
            service_layer_with(
                &mut runtime.layers,
                None,
                || Ok(Ok(snapshots.pop_front().unwrap())),
                || Ok(Ok(SUPPORT)),
                |mutation| {
                    assert_eq!(mutation, Mutation::AddAbove);
                    sends.set(sends.get() + 1);
                    Ok(())
                },
                || now,
            )
            .unwrap();
            assert_eq!(sends.get(), 1);
            assert_eq!(runtime.interaction.state(), state);
            assert_eq!(runtime.pointer.owner(), owner);
            assert_eq!(runtime.menu.is_some(), popup);
            assert!(runtime.layers.pending().is_none());
        }
    }

    #[test]
    fn layer_selection_cleanup_precedes_dispatch_and_busy_preserves_original_operation() {
        let now = Instant::now();
        for busy in [false, true] {
            let mut runtime = captured_menu();
            if busy {
                pending_above(&mut runtime.layers, now);
            }
            let pending = runtime.layers.pending();
            runtime.interaction.menu_press(1, MenuHit::Row(2), 100, 0);
            let HostAction::CloseMenu { outcome, .. } =
                runtime.interaction.menu_release(1, MenuHit::Row(2), 101, 0)
            else {
                panic!("layer selection missing")
            };
            let trace = RefCell::new(Vec::new());
            runtime
                .complete_menu_with(
                    &mut tests::test_window(),
                    outcome,
                    |pointer| {
                        pointer
                            .release_with(|| {
                                trace.borrow_mut().push("ungrab");
                                Ok(())
                            })
                            .map(|_| ())
                    },
                    |_| {
                        trace.borrow_mut().push("destroy");
                        Ok(())
                    },
                    |layers, target| {
                        assert_eq!(*trace.borrow(), ["ungrab", "destroy"]);
                        trace.borrow_mut().push("dispatch");
                        service_layer_with(
                            layers,
                            Some(target),
                            || {
                                assert!(!busy);
                                Ok(Ok(ObservedLayer::Below))
                            },
                            || panic!("no mutation"),
                            |_| panic!("no mutation"),
                            || now,
                        )
                    },
                )
                .unwrap();
            assert_eq!(*trace.borrow(), ["ungrab", "destroy", "dispatch"]);
            assert!(runtime.menu.is_none());
            assert!(!runtime.pointer.is_grabbed());
            assert!(runtime.interaction.is_idle());
            assert_eq!(runtime.layers.pending(), pending);
            assert_eq!(
                runtime.layers.desired,
                Some(if busy { Layer::Above } else { Layer::Below })
            );
        }
    }

    #[test]
    fn failed_popup_release_or_destruction_blocks_dispatch_and_preserves_error_priority() {
        for release_fails in [false, true] {
            let mut runtime = captured_menu();
            let destroyed = Cell::new(false);
            let result = runtime.complete_menu_with(
                &mut tests::test_window(),
                MenuOutcome::SetLayer(Layer::Above),
                |pointer| {
                    pointer
                        .release_with(|| {
                            if release_fails {
                                Err("release failure".into())
                            } else {
                                Ok(())
                            }
                        })
                        .map(|_| ())
                },
                |_| {
                    destroyed.set(true);
                    Err("destroy failure".into())
                },
                |_, _| panic!("failed cleanup must not dispatch"),
            );
            let error = result.unwrap_err();
            assert_eq!(
                error.to_string(),
                if release_fails {
                    "release failure"
                } else {
                    "destroy failure"
                }
            );
            assert!(destroyed.get());
            assert_eq!(runtime.pointer.is_grabbed(), release_fails);
            assert!(runtime.menu.is_some());
            assert!(runtime.layers.desired.is_none());
        }
    }

    #[test]
    fn protocol_read_support_send_and_check_failures_are_fatal_without_rollback() {
        let now = Instant::now();
        for fail_at in [
            "read",
            "support",
            "send",
            "check",
            "post-read",
            "second-send",
            "second-post-read",
        ] {
            let mut layers = LayerController::default();
            let reads = Cell::new(0);
            let sends = Cell::new(0);
            let result = service_layer_with(
                &mut layers,
                Some(Layer::Above),
                || {
                    let count = reads.get();
                    reads.set(count + 1);
                    if (fail_at == "read" && count == 0)
                        || (fail_at == "post-read" && count == 1)
                        || (fail_at == "second-post-read" && count == 2)
                    {
                        return Err(fail_at.into());
                    }
                    Ok(Ok(if count == 0 {
                        ObservedLayer::Below
                    } else {
                        ObservedLayer::Normal
                    }))
                },
                || {
                    if fail_at == "support" {
                        Err(fail_at.into())
                    } else {
                        Ok(Ok(SUPPORT))
                    }
                },
                |_| {
                    sends.set(sends.get() + 1);
                    if matches!(fail_at, "send" | "check")
                        || fail_at == "second-send" && sends.get() == 2
                    {
                        Err(fail_at.into())
                    } else {
                        Ok(())
                    }
                },
                || now,
            );
            assert_eq!(result.unwrap_err().to_string(), fail_at);
            assert!(layers.deadline().is_none());
            if fail_at == "second-send" {
                assert_eq!(sends.get(), 2);
                assert_eq!(layers.observed, Some(ObservedLayer::Normal));
            }
        }
    }

    #[test]
    fn layer_host_error_keeps_priority_over_later_pointer_and_resource_cleanup() {
        let now = Instant::now();
        let mut runtime = captured_menu();
        pending_above(&mut runtime.layers, now);
        let result = service_layer_with(
            &mut runtime.layers,
            None,
            || Err("original layer read error".into()),
            || panic!("no support"),
            |_| panic!("no send"),
            || now,
        );
        let cleanup_attempted = Cell::new(false);
        let outcome = finish_probe_with(result, || {
            runtime.cancel_menu_with(
                &mut tests::test_window(),
                |pointer| {
                    pointer
                        .release_with(|| Err("ungrab error".into()))
                        .map(|_| ())
                },
                |_| {
                    cleanup_attempted.set(true);
                    Err("destroy error".into())
                },
            )
        });
        assert_eq!(
            outcome.unwrap_err().to_string(),
            "original layer read error"
        );
        assert!(cleanup_attempted.get());
        assert!(runtime.pointer.is_grabbed());
    }

    fn property(window: u32, atom: u32, sequence: u16) -> Event {
        Event::PropertyNotify(PropertyNotifyEvent {
            response_type: 28,
            sequence,
            window,
            atom,
            time: 100,
            state: Property::NEW_VALUE,
        })
    }
    fn atoms() -> LayerAtoms {
        LayerAtoms {
            state: 20,
            supported: 21,
            above: 22,
            below: 23,
        }
    }
    fn layout_atoms() -> x11::monitors::LayoutAtoms {
        x11::monitors::LayoutAtoms {
            workarea: 30,
            current_desktop: 31,
            wm_desktop: 32,
        }
    }

    #[test]
    fn stale_relevant_notifications_reread_truth_unrelated_events_cannot_confirm_or_cancel_input() {
        let now = Instant::now();
        let mut layers = LayerController::default();
        pending_above(&mut layers, now);
        let reads = Cell::new(0);
        for event in [
            property(99, 20, 0),
            property(100, 99, 0),
            property(100, 20, 0),
            property(100, 20, u16::MAX),
        ] {
            assert!(!interrupts_pending_movement(
                &event,
                1,
                100,
                &layout_atoms()
            ));
            assert!(!interrupts_layer(&event, 1, 100, &layout_atoms()));
            if layer_property_event(&event, 100, &atoms()) {
                service_layer_with(
                    &mut layers,
                    None,
                    || {
                        reads.set(reads.get() + 1);
                        Ok(Ok(ObservedLayer::Below))
                    },
                    || panic!("not ready to add"),
                    |_| panic!("stale event cannot confirm removal"),
                    || now,
                )
                .unwrap();
            }
        }
        assert_eq!(reads.get(), 2);
        assert!(layers.pending().is_some());
        service_layer_with(
            &mut layers,
            None,
            || Ok(Ok(ObservedLayer::Above)),
            || panic!("already complete"),
            |_| panic!("already complete"),
            || now,
        )
        .unwrap();
        assert!(layers.pending().is_none());
    }

    #[test]
    fn fixed_deadline_wins_under_event_traffic_and_final_read_can_confirm_or_timeout() {
        let now = Instant::now();
        for final_observed in [
            ObservedLayer::Above,
            ObservedLayer::Normal,
            ObservedLayer::Conflict,
        ] {
            let mut layers = LayerController::default();
            pending_above(&mut layers, now);
            let deadline = layers.deadline().unwrap();
            let mut queue: VecDeque<_> = (0..200).map(|_| property(99, 99, 0)).collect();
            let mut buffered = None;
            let mut reads = 0;
            while !queue.is_empty() {
                if layers.deadline().is_some_and(|time| deadline >= time) {
                    service_layer_with(
                        &mut layers,
                        None,
                        || {
                            reads += 1;
                            Ok(Ok(final_observed))
                        },
                        || panic!("timeout cannot query mutation support"),
                        |_| panic!("timeout cannot add or retry"),
                        || deadline,
                    )
                    .unwrap();
                }
                drain_events_bounded(&mut buffered, 64, || Ok::<_, HostError>(queue.pop_front()))
                    .unwrap();
                assert!(layers.deadline().is_none());
            }
            assert_eq!(reads, 1);
            assert_eq!(layers.observed, Some(final_observed));
            service_layer_with(
                &mut layers,
                None,
                || Ok(Ok(ObservedLayer::Above)),
                || panic!("late property cannot revive operation"),
                |_| panic!("no reconciliation"),
                || deadline,
            )
            .unwrap();
            assert!(layers.pending().is_none());
        }
    }

    #[test]
    fn expired_support_event_skips_capabilities_and_only_observes_final_body_state() {
        let started = Instant::now();
        for final_observed in [ObservedLayer::Above, ObservedLayer::Normal] {
            let mut layers = LayerController::default();
            pending_above(&mut layers, started);
            let handled_at = layers.deadline().unwrap() + Duration::from_millis(1);
            let reads = Cell::new(0);
            service_layer_support_event_with(
                &mut layers,
                || panic!("expired _NET_SUPPORTED event must not read capabilities"),
                |layers| {
                    service_layer_with(
                        layers,
                        None,
                        || {
                            reads.set(reads.get() + 1);
                            Ok(Ok(final_observed))
                        },
                        || panic!("final body observation must not query capabilities"),
                        |_| panic!("final body observation must not send a mutation"),
                        || handled_at,
                    )
                },
                || handled_at,
            )
            .unwrap();
            assert_eq!(reads.get(), 1);
            assert_eq!(layers.observed, Some(final_observed));
            assert_eq!(layers.desired, Some(Layer::Above));
            assert!(layers.pending().is_none());
            assert!(layers.deadline().is_none());
        }
    }

    #[test]
    fn expiry_during_support_query_performs_final_read_without_sending() {
        let started = Instant::now();
        for advertised in [Ok(SUPPORT), Err("support malformed at expiry")] {
            let clock = Cell::new(started);
            let mut layers = LayerController::default();
            let reads = Cell::new(0);
            service_layer_with(
                &mut layers,
                Some(Layer::Above),
                || {
                    reads.set(reads.get() + 1);
                    Ok(Ok(ObservedLayer::Normal))
                },
                || {
                    clock.set(started + crate::layer::CONFIRMATION_TIMEOUT);
                    Ok(advertised)
                },
                |_| panic!("deadline passed while reading support"),
                || clock.get(),
            )
            .unwrap();
            assert_eq!(reads.get(), 2);
            assert!(layers.pending().is_none());
        }
    }

    #[test]
    fn quit_dismiss_duration_and_lifecycle_preserve_cleanup_and_end_only_appropriate_work() {
        use x11rb::protocol::xproto::{DestroyNotifyEvent, MapNotifyEvent, UnmapNotifyEvent};
        let now = Instant::now();
        for item in [MenuItem::Dismiss, MenuItem::Quit] {
            let mut runtime = captured_menu();
            pending_above(&mut runtime.layers, now);
            let quit = runtime
                .complete_menu_with(
                    &mut tests::test_window(),
                    item.outcome(),
                    |pointer| pointer.release_with(|| Ok(())).map(|_| ()),
                    |_| Ok(()),
                    |_, _| panic!("not a layer action"),
                )
                .unwrap();
            assert_eq!(quit, item == MenuItem::Quit);
            assert_eq!(runtime.layers.pending().is_some(), !quit);
            assert!(!runtime.pointer.is_grabbed());
            assert!(runtime.menu.is_none());
        }
        let events = [
            property(1, 30, 0),
            property(1, 31, 0),
            property(100, 32, 0),
            Event::MapNotify(MapNotifyEvent {
                response_type: 19,
                sequence: 0,
                event: 1,
                window: 100,
                override_redirect: false,
            }),
            Event::UnmapNotify(UnmapNotifyEvent {
                response_type: 18,
                sequence: 0,
                event: 1,
                window: 100,
                from_configure: false,
            }),
            Event::DestroyNotify(DestroyNotifyEvent {
                response_type: 17,
                sequence: 0,
                event: 1,
                window: 100,
            }),
        ];
        for event in events {
            assert!(interrupts_layer(&event, 1, 100, &layout_atoms()));
        }
        for reason in [
            "duration expiry",
            "layout or body lifecycle change",
            "shutdown",
        ] {
            let mut runtime = captured_menu();
            pending_above(&mut runtime.layers, now);
            abort_layer(&mut runtime.layers, reason);
            runtime
                .cancel_menu_with(
                    &mut tests::test_window(),
                    |pointer| pointer.release_with(|| Ok(())).map(|_| ()),
                    |_| Ok(()),
                )
                .unwrap();
            assert!(runtime.layers.deadline().is_none());
            assert!(!runtime.pointer.is_grabbed());
            assert!(runtime.menu.is_none());
            assert!(runtime.interaction.is_idle());
        }
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;
    use crate::control::{Command, ExitCode};

    #[test]
    fn default_and_owner_invocations_remain_valid() {
        assert_eq!(
            parse_args(Vec::<String>::new()),
            Ok(CliMode::Owner {
                delay_secs: None,
                duration_secs: None
            })
        );
        assert_eq!(
            parse_args(["--delay", "2", "--duration", "30"]),
            Ok(CliMode::Owner {
                delay_secs: Some(2),
                duration_secs: Some(30)
            })
        );
        assert_eq!(
            parse_args(["--duration", "0", "--delay", "18446744073709551615"]),
            Ok(CliMode::Owner {
                delay_secs: Some(u64::MAX),
                duration_secs: Some(0)
            })
        );
        assert_eq!(
            parse_args(["--delay", "1", "--delay", "2"]),
            Ok(CliMode::Owner {
                delay_secs: Some(2),
                duration_secs: None
            })
        );
        assert_eq!(
            parse_args(["--duration", "5"]),
            Ok(CliMode::Owner {
                delay_secs: None,
                duration_secs: Some(5)
            })
        );
    }
    #[test]
    fn help_and_diagnose_remain_separate_modes() {
        for flag in ["--help", "-h"] {
            assert_eq!(parse_args([flag]), Ok(CliMode::Help));
        }
        assert_eq!(parse_args(["--diagnose"]), Ok(CliMode::Diagnose));
        assert_eq!(
            parse_args(["--delay", "2", "--diagnose", "--duration", "5"]),
            Ok(CliMode::Diagnose)
        );
        assert_eq!(parse_args(["--diagnose", "--help"]), Ok(CliMode::Help));
    }
    #[test]
    fn control_commands_parse_but_runtime_stub_fails_without_host_work() {
        for (flag, command, label) in [
            ("--hide", Command::Hide, "hide:"),
            ("--show", Command::Show, "show:"),
            ("--bring-top", Command::BringTop, "bring-top:"),
        ] {
            assert_eq!(parse_args([flag]), Ok(CliMode::Control(command)));
            let (exit, diagnostic) = unsupported_control(command);
            assert_eq!(exit, ExitCode::OperationFailed);
            assert_eq!(exit as i32, 7);
            assert!(diagnostic.starts_with(label));
            assert!(diagnostic.contains("not implemented in this stage"));
        }
    }
    #[test]
    fn control_conflicts_in_either_order_are_usage_errors() {
        for a in ["--hide", "--show", "--bring-top"] {
            for b in ["--hide", "--show", "--bring-top", "--diagnose"] {
                for args in [[a, b], [b, a]] {
                    assert_eq!(parse_args(args).unwrap_err().exit_code(), ExitCode::Usage);
                }
            }
            for option in ["--delay", "--duration"] {
                for args in [[a, option, "0"], [option, "0", a]] {
                    assert_eq!(parse_args(args).unwrap_err().exit_code() as u32, 2);
                }
            }
        }
    }
    #[test]
    fn invalid_missing_overflowing_and_unknown_arguments_are_meaningful() {
        for flag in ["--delay", "--duration"] {
            assert!(parse_args([flag]).unwrap_err().0.contains("Missing value"));
            for value in ["-1", "1.5", "abc", "18446744073709551616", "--diagnose"] {
                let error = parse_args([flag, value]).unwrap_err();
                assert!(error.0.contains(flag));
                assert!(error.0.contains(value));
                assert_eq!(error.exit_code(), ExitCode::Usage);
            }
        }
        let error = parse_args(["--unknown"]).unwrap_err();
        assert!(error.0.contains("Unknown option: '--unknown'"));
        assert_eq!(error.exit_code() as i32, 2);
    }
}
