mod control;
mod geometry;
mod interaction;
mod layer;
mod x11;

use std::env;
use std::os::unix::io::AsRawFd;
use std::process;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::render::ConnectionExt as RenderExt;
use x11rb::protocol::shape::ConnectionExt as ShapeExt;
use x11rb::protocol::xproto::{GrabStatus, MapNotifyEvent, VisualClass};
use x11rb::protocol::Event;

use crate::geometry::{calculate_centered_origin, compute_valid_origin_bounds, Point, Size};
use crate::interaction::{HostAction, InteractionManager, InteractionState, MenuOutcome};
use crate::layer::{Layer, LayerController, Mutation, ObservedLayer, Step};
use crate::x11::control::{Acquisition, ControlWindow};
use crate::x11::menu::{MenuPopup, PopupInputGate};
use crate::x11::monitors::query_desktop_layout;
use crate::x11::pointer::{grab_pointer, CaptureOwner, PointerCaptureTracker};
use crate::x11::render::{Renderer, WINDOW_HEIGHT, WINDOW_WIDTH};
use crate::x11::state::{LayerAtoms, LayerSupport, PropertyResult, RemoteLayerIssue};
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
            flag @ ("--hide" | "--show" | "--bring-top" | "--bring-here" | "--recover") => {
                let next = match flag {
                    "--hide" => control::Command::Hide,
                    "--show" => control::Command::Show,
                    "--bring-here" => control::Command::BringHere,
                    "--recover" => control::Command::Recover,
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

const HELP: &str = "DesktopRoomie - A00-M03.3: Desktop Probe

Left click toggles color; left drag moves the body.
Right click opens Above/Normal/Below/Dismiss/Quit after release. Outside left click dismisses.

USAGE:
  desktop-probe [OPTIONS]

OPTIONS:
  -h, --help              Show this help message and exit
  --diagnose              Run pure X11 environment diagnostics and exit
  --delay <SECONDS>       Delay in seconds before mapping window (for typing test)
  --duration <SECONDS>    Run for a specified duration in seconds, then exit
  --show       Restore the existing body without activation or workspace/layer changes
  --hide       Minimize the existing body without workspace/layer changes
  --bring-top  Ensure the existing body is not minimized and set Above on its existing workspace
  --bring-here Move the existing body to the caller's current workspace, preserving its effective layer and genuine minimized state; do not activate it or switch the user's workspace
  --recover    Bring the existing body to the caller's frozen current workspace, restore it if minimized, and set Above; confirm placement and input readiness without activation or switching the user's workspace";

fn print_help() {
    println!("{HELP}");
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
            let exit = match x11::control::invoke(command) {
                Ok(outcome) => {
                    eprintln!("{}", outcome.diagnostic(command));
                    outcome.exit_code()
                }
                Err(error) => {
                    eprintln!("{command}: local transport/resource failure — {error}");
                    control::ExitCode::LocalFailure
                }
            };
            process::exit(exit as i32);
        }
        CliMode::Diagnose => (true, None, None),
        CliMode::Owner {
            delay_secs,
            duration_secs,
        } => (false, delay_secs, duration_secs),
    };

    println!("=== DesktopRoomie A00-M03.2: Desktop Probe ===");

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

    if diagnose {
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

        run_diagnostics(&conn, screen, &layout);
        println!("\n[Result]");
        println!("  M01.1: X11 connection established; diagnostics completed.");
        return;
    }

    let exit = run_owner(&conn, screen, screen_num, delay_secs, duration_secs);
    if exit != control::ExitCode::Success {
        process::exit(exit as i32);
    }
    println!("  Probe exited cleanly.");
}

fn run_owner(
    conn: &x11rb::rust_connection::RustConnection,
    screen: &x11rb::protocol::xproto::Screen,
    screen_num: usize,
    delay_secs: Option<u64>,
    duration_secs: Option<u64>,
) -> control::ExitCode {
    let mut owner = match ControlWindow::create(conn, screen, screen_num) {
        Ok(owner) => owner,
        Err(error) => {
            eprintln!("[ERROR] Control endpoint creation failed: {error}");
            return control::ExitCode::LocalFailure;
        }
    };
    let mut body = None;
    let mut renderer = None;
    let mut runtime = ProbeRuntime::new();
    let startup = owner_startup_with(
        &mut owner,
        |owner| owner.acquire(conn),
        |owner| {
            if let Some(delay) = delay_secs {
                println!("[Startup Delay] Waiting {delay} second(s) before creating and mapping probe window...");
                owner.delay(conn, delay)?;
            }
            Ok(())
        },
        |owner| {
            start_body(
                conn,
                screen,
                duration_secs,
                owner,
                &mut body,
                &mut renderer,
                &mut runtime,
            )
        },
    );
    let duplicate = matches!(startup, Ok(Acquisition::Duplicate));
    if duplicate {
        eprintln!(
            "[INSTANCE] Another owner exists on screen {screen_num}; refusing duplicate launch"
        );
    }
    let result = startup.map(|_| ());
    let outcome = finish_probe_with(result, || {
        let closing_popup = runtime.menu.as_ref().map(|popup| popup.window);
        let cleanup = owner_cleanup_with(
            &mut owner,
            |owner| {
                let closing = owner.closing(conn);
                x11::resource::cleanup_all([
                    Box::new(|| closing),
                    Box::new(|| {
                        let remote = owner.finish_remote(conn);
                        let incoming = owner.reply_while_closing(conn, closing_popup);
                        x11::resource::cleanup_all([Box::new(|| remote), Box::new(|| incoming)])
                    }),
                ])
            },
            || {
                abort_layer(&mut runtime.layers, "shutdown");
                runtime.correction = None;
                let cancellation = body
                    .as_mut()
                    .map_or(Ok(()), |body| runtime.cancel(conn, body, "shutdown"));
                x11::resource::cleanup_all([
                    Box::new(|| cancellation),
                    Box::new(|| {
                        renderer
                            .as_ref()
                            .map_or(Ok(()), |renderer| renderer.destroy(conn))
                    }),
                    Box::new(|| body.as_ref().map_or(Ok(()), |body| body.destroy(conn))),
                ])
            },
            |owner| owner.destroy(conn),
            || Ok(conn.flush()?),
        );
        if runtime.pointer.is_grabbed() {
            eprintln!("[ERROR] Pointer release remains unconfirmed; connection teardown is best effort only");
        }
        cleanup
    });
    if outcome.is_err() {
        control::ExitCode::LocalFailure
    } else if duplicate {
        control::ExitCode::DuplicateLaunch
    } else {
        control::ExitCode::Success
    }
}

/// Only a confirmed owner can enter the delay or construct a body.
fn owner_startup_with<T>(
    owner: &mut T,
    acquire: impl FnOnce(&mut T) -> Result<Acquisition, HostError>,
    delay: impl FnOnce(&mut T) -> Result<(), HostError>,
    body: impl FnOnce(&mut T) -> Result<(), HostError>,
) -> Result<Acquisition, HostError> {
    let outcome = acquire(owner)?;
    if outcome == Acquisition::Owned {
        delay(owner)?;
        body(owner)?;
    }
    Ok(outcome)
}

fn owner_cleanup_with<T>(
    owner: &mut T,
    closing: impl FnOnce(&mut T) -> Result<(), HostError>,
    body_cleanup: impl FnOnce() -> Result<(), HostError>,
    control_cleanup: impl FnOnce(&mut T) -> Result<(), HostError>,
    flush: impl FnOnce() -> Result<(), HostError>,
) -> Result<(), HostError> {
    // Evaluate Closing first, then continue ALL teardown obligations despite failures.
    let closing = closing(owner);
    x11::resource::cleanup_all([
        Box::new(|| closing),
        Box::new(body_cleanup),
        Box::new(|| control_cleanup(owner)),
        Box::new(flush),
    ])
}

#[allow(clippy::too_many_arguments)]
fn start_body(
    conn: &x11rb::rust_connection::RustConnection,
    screen: &x11rb::protocol::xproto::Screen,
    duration_secs: Option<u64>,
    owner: &mut ControlWindow,
    body_resource: &mut Option<ManagedProbeWindow>,
    renderer_resource: &mut Option<Renderer>,
    runtime: &mut ProbeRuntime,
) -> Result<(), HostError> {
    let atoms = x11::monitors::LayoutAtoms::subscribe(conn, screen.root)?;
    let layout = query_desktop_layout(conn, screen.root, &atoms, None)?;
    let selected_monitor = layout
        .primary_monitor
        .as_ref()
        .expect("validated monitor")
        .name_atom;
    // M02.5 retains bounded dragging and adds interruption recovery.
    println!("Mission M03.1: managed layers and mouse-only controls...");

    // 1. Discover 32-bit alpha Render visual
    let alpha_vis = find_alpha_visual(conn)?;

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
    let valid_bounds = checked_probe_bounds(layout.usable_area)?;
    let requested_origin = calculate_centered_origin(layout.usable_area, body_size)?;

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

    // Save each successful resource immediately so all later failures reach ordered cleanup.
    let (body, map_sequence) =
        ManagedProbeWindow::create(conn, screen.root, alpha_vis.visual_id, requested_origin)?;
    *body_resource = Some(body);
    runtime.arm_map_input_sync(map_sequence);
    let probe_window = body_resource.as_mut().unwrap();
    owner.publish_body(conn, probe_window.window)?;

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

    *renderer_resource = Some(Renderer::new(
        conn,
        probe_window.window,
        probe_window.colormap,
    )?);
    let renderer = renderer_resource.as_mut().unwrap();
    renderer.paint(conn, probe_window.window)?;

    println!("\n[Running Probe]");
    if let Some(sec) = duration_secs {
        println!("  Probe running for {} seconds (or until closed)...", sec);
    } else {
        println!(
            "  Left-click toggles body color; right-click opens Above/Normal/Below/Dismiss/Quit. Launch with --duration <SECONDS> for automatic termination."
        );
    }

    run_probe(
        conn,
        screen,
        duration_secs,
        &atoms,
        selected_monitor,
        probe_window,
        renderer,
        runtime,
        owner,
        valid_bounds,
        layout.usable_area,
    )
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

/// Independent of body availability: lifecycle/placement events cannot reopen a fence.
#[derive(Default)]
struct InputFence {
    blocked: bool,
    empty_confirmed: bool,
    enabled_after: Option<u64>,
    // Fence-local safety only; no completed request/destination is retained.
    // An explicit command has its own correction budget. Late ordinary events
    // may restore already-safe input, but cannot repeat a Here correction.
    automatic_correction_blocked: bool,
}
impl InputFence {
    fn rejects(&self, event: &Event, body: u32, sequence: u64) -> bool {
        let directed = match event {
            Event::ButtonPress(e) | Event::ButtonRelease(e) => e.event == body,
            Event::MotionNotify(e) => e.event == body,
            Event::EnterNotify(e) | Event::LeaveNotify(e) => e.event == body,
            _ => false,
        };
        directed
            && (self.blocked
                || self
                    .enabled_after
                    .is_some_and(|boundary| sequence < boundary))
    }
}

/// Supplied by fresh retained-monitor/layout and root-geometry validation, never
/// a MapNotify or the desired Ready state.
struct InputPlacement {
    selected_monitor: u32,
    observed_monitor: u32,
    geometry: crate::geometry::Rect,
    bounds: crate::geometry::ValidOriginBounds,
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
    input_fence: InputFence,
    pending_map_input: Option<u64>,
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
            input_fence: InputFence::default(),
            pending_map_input: None,
        }
    }

    /// One allowance per deliberate mapping request, never rearmed by notifications.
    /// A future restoration can arm a new full sequence, but must still use the
    /// readiness-checked visibility restore path while input is fenced.
    fn arm_map_input_sync(&mut self, map_sequence: u64) {
        self.pending_map_input = Some(map_sequence);
    }

    fn synchronize_map_input_with(
        &mut self,
        event: &MapNotifyEvent,
        body: u32,
        sequence: u64,
        restore: impl FnOnce() -> Result<u64, HostError>,
    ) -> Result<(), HostError> {
        if self.input_fence.blocked
            || event.window != body
            || event.event != body
            || event.response_type & 0x80 != 0
            || event.override_redirect
            || !self
                .pending_map_input
                .is_some_and(|after| sequence >= after)
        {
            return Ok(());
        }
        // Consume before sending: a failed check is fatal, not permission to retry
        // on duplicate notifications. Only success establishes the freshness gate.
        self.pending_map_input = None;
        let sequence = restore()?;
        self.input_fence.enabled_after = Some(sequence);
        Ok(())
    }

    /// Show rejects pending local correction before entering this checked fence.
    fn fence_visibility_input(
        &mut self,
        conn: &impl Connection,
        window: &mut ManagedProbeWindow,
    ) -> Result<(), HostError> {
        let body = window.window;
        self.fence_visibility_input_with(
            window,
            || x11::shape::empty_body_input_shape(conn, body),
            |_, _| x11::pointer::ungrab_pointer(conn, x11rb::CURRENT_TIME),
            |popup| popup.destroy(conn),
        )
    }

    #[allow(dead_code)]
    fn fence_visibility_input_with(
        &mut self,
        window: &mut ManagedProbeWindow,
        empty: impl FnOnce() -> Result<u64, HostError>,
        release: impl FnOnce(&Self, &ManagedProbeWindow) -> Result<(), HostError>,
        destroy: impl FnOnce(&MenuPopup) -> Result<(), HostError>,
    ) -> Result<(), HostError> {
        self.input_fence.blocked = true;
        self.input_fence.empty_confirmed = false;
        self.pending_map_input = None; // stale startup MapNotify cannot enable a remap
        let empty = empty();
        self.input_fence.empty_confirmed = empty.is_ok();
        // Mandatory cleanup proceeds even if the native fence failed. Such failure
        // never grants dispatch eligibility or permits logical input to resume.
        self.menu_generation = self.menu_generation.wrapping_add(1);
        self.correction = None;
        if self.body != BodyAvailability::Destroyed {
            self.body = BodyAvailability::Unavailable;
        }
        self.pointer.require_transition_release();
        self.cancel_with(window, |_| Ok(()))?;
        let checked_release = release(self, window);
        let release = self.pointer.release_with(|| checked_release).map(|_| ());
        let cleanup = self.menu.as_ref().map_or(Ok(()), destroy);
        if cleanup.is_ok() {
            self.menu = None;
        }
        let mut primary = None;
        for (step, result) in [
            ("empty input SHAPE", empty.map(|_| ())),
            ("pointer release", release),
            ("popup cleanup", cleanup),
        ] {
            if let Err(error) = result {
                eprintln!("[INPUT FENCE] {step}: {error}");
                if primary.is_none() {
                    primary = Some(error);
                }
            }
        }
        if let Some(error) = primary {
            return Err(error);
        }
        if !self.visibility_dispatch_eligible() {
            return Err("Visibility input obligations remain unresolved".into());
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn visibility_dispatch_eligible(&self) -> bool {
        self.input_fence.blocked
            && self.input_fence.empty_confirmed
            && !self.pointer.is_grabbed()
            && self.menu.is_none()
            && matches!(
                self.interaction.state(),
                InteractionState::Idle | InteractionState::SuppressedUntilRelease { .. }
            )
            && self.pending_move.is_none()
            && self.final_target.is_none()
            && self.confirmation_deadline.is_none()
            && self.correction.is_none()
            && self.safety.deadline.is_none()
            && self.safety.observation.is_none()
            && self.body != BodyAvailability::Destroyed
    }

    #[allow(dead_code)]
    fn restore_visibility_input(
        &mut self,
        conn: &impl Connection,
        window: &ManagedProbeWindow,
        snapshot: &x11::visibility::VisibilitySnapshot,
        placement: &InputPlacement,
    ) -> Result<(), HostError> {
        self.restore_visibility_input_with(window, snapshot.evidence, placement, || {
            x11::shape::restore_body_input_shape(conn, window.window, window.width, window.height)
        })
    }

    #[allow(dead_code)]
    fn restore_visibility_input_with(
        &mut self,
        window: &ManagedProbeWindow,
        evidence: control::VisibilityEvidence,
        placement: &InputPlacement,
        restore: impl FnOnce() -> Result<u64, HostError>,
    ) -> Result<(), HostError> {
        if !self.visibility_dispatch_eligible()
            || self.body != BodyAvailability::Ready
            || control::classify(evidence).visibility != control::Visibility::NotMinimizedHere
            || placement.selected_monitor == 0
            || placement.selected_monitor != placement.observed_monitor
            || placement.geometry.size() != Size::new(WINDOW_WIDTH.into(), WINDOW_HEIGHT.into())
            || placement.geometry.origin() != window.confirmed_origin()
            || !placement.bounds.contains(placement.geometry.origin())
            || !window.in_flight_moves.is_empty()
        {
            return Err("Body input restoration requires fresh local visibility and validated retained-monitor placement with settled obligations".into());
        }
        // A failed check can mean the server applied the shape: retain logical
        // blocking and invalidate empty-fence proof until explicitly fenced again.
        self.input_fence.empty_confirmed = false;
        let sequence = restore()?;
        self.input_fence.enabled_after = Some(sequence);
        self.pending_map_input = None; // checked restoration supplies the post-map silhouette
        self.input_fence.blocked = false;
        self.input_fence.automatic_correction_blocked = false;
        Ok(())
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
        if self.input_fence.blocked || self.pointer.is_grabbed() || self.menu.is_some() {
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
        if self.input_fence.blocked || self.pointer.is_grabbed() || self.menu.is_some() {
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
        !self.input_fence.blocked
            && self.body == BodyAvailability::Ready
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
            if self.input_fence.blocked && self.input_fence.automatic_correction_blocked {
                return Ok(());
            }
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

#[cfg(test)]
#[path = "probe_tests/map_input_tests.rs"]
mod map_input_tests;

#[cfg(test)]
#[path = "probe_tests/visibility_fence_tests.rs"]
mod visibility_fence_tests;

/// Internal service evidence, not a remote OperationResult or a WM acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayerOutcome {
    AlreadyMatches,
    Waiting,
    Confirmed,
    Unsupported,
    Unverifiable,
    TimedOut(layer::Phase),
    Interrupted(layer::Phase),
    Busy,
}

fn layer_read_outcome(
    layers: &mut LayerController,
    issue: RemoteLayerIssue,
    outer_deadline: Option<Instant>,
    now: Instant,
) -> LayerOutcome {
    if issue == RemoteLayerIssue::Waiting {
        if layers
            .deadline()
            .or(outer_deadline)
            .is_some_and(|dl| now >= dl)
        {
            let phase = layers.abort().map_or(layer::Phase::AwaitFinal, |p| p.phase);
            LayerOutcome::TimedOut(phase)
        } else {
            LayerOutcome::Waiting
        }
    } else if issue == RemoteLayerIssue::Unsupported {
        LayerOutcome::Unsupported
    } else {
        LayerOutcome::Unverifiable
    }
}

fn abort_layer(layers: &mut LayerController, reason: &str) -> Option<LayerOutcome> {
    let pending = layers.abort();
    if pending.is_some() {
        eprintln!(
            "[LAYER] Operation ended: {reason}; target={:?}, last readable={:?}, phase={:?}",
            layers.desired,
            layers.observed,
            pending.map(|p| p.phase)
        );
    }
    pending.map(|p| LayerOutcome::Interrupted(p.phase))
}

fn read_layer_with(
    layers: &mut LayerController,
    read: &mut impl FnMut() -> Result<Result<ObservedLayer, RemoteLayerIssue>, HostError>,
) -> Result<Result<ObservedLayer, RemoteLayerIssue>, HostError> {
    match read()? {
        Ok(observed) => Ok(Ok(observed)),
        Err(RemoteLayerIssue::Waiting) => Ok(Err(RemoteLayerIssue::Waiting)),
        Err(reason) => {
            eprintln!(
                "[LAYER] Unverifiable state: {reason:?}; last readable={:?}",
                layers.observed
            );
            abort_layer(layers, "layer qualification failed");
            Ok(Err(reason))
        }
    }
}

fn validate_layer_support(
    layers: &mut LayerController,
    advertised: PropertyResult<LayerSupport>,
) -> Option<LayerOutcome> {
    let pending = layers.pending()?;
    match advertised {
        Ok(support) if support.allows(pending.required) => None,
        support => {
            eprintln!(
                "[LAYER] Required advertised support {:?} unavailable: {support:?}",
                pending.required
            );
            abort_layer(layers, "unsupported or malformed _NET_SUPPORTED");
            Some(if support.is_ok() {
                LayerOutcome::Unsupported
            } else {
                LayerOutcome::Unverifiable
            })
        }
    }
}

/// Bounded orchestration shared by real requests and deterministic host fakes.
/// Reads return protocol errors separately from recoverable property problems.
fn service_layer_with(
    layers: &mut LayerController,
    request: Option<Layer>,
    read: impl FnMut() -> Result<PropertyResult<ObservedLayer>, HostError>,
    support: impl FnMut() -> Result<PropertyResult<LayerSupport>, HostError>,
    send: impl FnMut(Mutation) -> Result<(), HostError>,
    now: impl FnMut() -> Instant,
) -> Result<LayerOutcome, HostError> {
    service_layer_bounded_with(layers, request, None, read, support, send, now)
}

fn service_layer_bounded_with(
    layers: &mut LayerController,
    request: Option<Layer>,
    outer_deadline: Option<Instant>,
    mut read: impl FnMut() -> Result<PropertyResult<ObservedLayer>, HostError>,
    support: impl FnMut() -> Result<PropertyResult<LayerSupport>, HostError>,
    mut send: impl FnMut(Mutation) -> Result<(), HostError>,
    now: impl FnMut() -> Instant,
) -> Result<LayerOutcome, HostError> {
    service_qualified_layer_with(
        layers,
        request,
        outer_deadline,
        false,
        || {
            read().map(|r| {
                r.map_err(|reason| {
                    eprintln!("[LAYER] Property observation failed: {reason}");
                    RemoteLayerIssue::Unverifiable
                })
            })
        },
        support,
        |mutation| send(mutation).map(|()| true),
        now,
    )
}

/// Same G1 bounded adapter, with a narrow transient remote-read result. Local
/// property errors still terminate as Unverifiable through the wrapper above.
#[allow(clippy::too_many_arguments)]
fn service_qualified_layer_with(
    layers: &mut LayerController,
    request: Option<Layer>,
    outer_deadline: Option<Instant>,
    remote: bool,
    mut read: impl FnMut() -> Result<Result<ObservedLayer, RemoteLayerIssue>, HostError>,
    mut support: impl FnMut() -> Result<PropertyResult<LayerSupport>, HostError>,
    mut send: impl FnMut(Mutation) -> Result<bool, HostError>,
    mut now: impl FnMut() -> Instant,
) -> Result<LayerOutcome, HostError> {
    if let Some(target) = request {
        if layers.pending().is_some() {
            println!(
                "[LAYER] Request {target:?} rejected: busy with {:?}",
                layers.desired
            );
            return Ok(LayerOutcome::Busy);
        }
        println!("[LAYER] Requested {target:?}");
    }
    let accepted = now();
    let result = (|| {
        let observed = match read_layer_with(layers, &mut read)? {
            Ok(observed) => observed,
            Err(issue) => return Ok(layer_read_outcome(layers, issue, outer_deadline, now())),
        };
        let previous = layers.observed;
        if remote && remote_layer_interrupted(layers, observed) {
            return Ok(abort_layer(layers, "incompatible remote layer change").unwrap());
        }
        let mut step = match request {
            Some(target) => match outer_deadline {
                Some(deadline) => layers.begin_bounded(target, observed, accepted, deadline),
                None => layers.begin(target, observed, accepted),
            },
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
                        if !layers.deadline().is_some_and(|deadline| now() >= deadline) {
                            if let Some(outcome) = validate_layer_support(layers, advertised) {
                                return Ok(outcome);
                            }
                        }
                    }
                    if layers.deadline().is_some_and(|deadline| now() >= deadline) {
                        let observed = match read_layer_with(layers, &mut read)? {
                            Ok(observed) => observed,
                            Err(issue) => {
                                return Ok(layer_read_outcome(layers, issue, outer_deadline, now()))
                            }
                        };
                        if remote && remote_layer_interrupted(layers, observed) {
                            return Ok(
                                abort_layer(layers, "incompatible remote layer change").unwrap()
                            );
                        }
                        step = layers.observe(observed, now());
                        continue;
                    }
                    if !send(mutation)? {
                        layers.abort();
                        return Ok(LayerOutcome::Unverifiable);
                    }
                    println!("[LAYER] X11 checked {mutation:?}; awaiting fresh WM state");
                    let observed = match read_layer_with(layers, &mut read)? {
                        Ok(observed) => observed,
                        Err(issue) => {
                            return Ok(layer_read_outcome(layers, issue, outer_deadline, now()))
                        }
                    };
                    println!("[LAYER] Fresh observed state: {observed:?}");
                    if remote && remote_layer_interrupted(layers, observed) {
                        return Ok(abort_layer(layers, "incompatible remote layer change").unwrap());
                    }
                    step = layers.observe(observed, now());
                }
                Step::AlreadyMatches => {
                    println!(
                        "[LAYER] Already matches {:?}; no mutation or support proof",
                        layers.desired
                    );
                    return Ok(LayerOutcome::AlreadyMatches);
                }
                Step::Confirmed => {
                    println!("[LAYER] Confirmed property state {:?}; visual stacking requires owner observation", layers.observed);
                    return Ok(LayerOutcome::Confirmed);
                }
                Step::TimedOut(phase) => {
                    eprintln!("[LAYER] Timeout: target={:?}, last readable={:?}, phase={phase:?}; no retry", layers.desired, layers.observed);
                    return Ok(LayerOutcome::TimedOut(phase));
                }
                Step::Busy => return Ok(LayerOutcome::Busy),
                Step::Waiting => return Ok(LayerOutcome::Waiting),
            }
        }
    })();
    if result.is_err() {
        abort_layer(layers, "fatal X11 layer operation failure");
    }
    result
}

fn remote_layer_interrupted(layers: &LayerController, observed: ObservedLayer) -> bool {
    layers
        .pending()
        .is_some_and(|p| p.phase == layer::Phase::AwaitFinal)
        && layers.desired == Some(Layer::Above)
        && observed == ObservedLayer::Below
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
    .map(|_| ())
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

/// Small injected seam for owner progression. The active request, deadline and
/// progress stay in ControlWindow; the native adapter owns no request or queue.
trait VisibilityHost {
    fn now(&self) -> Instant;
    fn layer_atoms(&self) -> LayerAtoms;
    fn observe(&mut self) -> Result<x11::visibility::VisibilitySnapshot, HostError>;
    fn noop_ready(&self) -> bool;
    fn placement_busy(&self) -> bool;
    fn fenced(&self) -> bool;
    fn fence_settled(&self) -> bool;
    fn deferred_placement(&mut self, placement: &ShowPlacement) -> Result<bool, HostError>;
    fn fence(&mut self) -> Result<(), HostError>;
    /// True only after the single WM_CHANGE_STATE dispatch and successful check.
    fn iconify(&mut self, deadline: Instant, dispatched: &mut bool) -> Result<bool, HostError>;
    /// True only after MapWindow dispatch/check; `dispatched` also retains any
    /// earlier user-time mutation, including on expiry or a native error.
    fn restore(&mut self, deadline: Instant, dispatched: &mut bool) -> Result<bool, HostError>;
    /// One checked WM-directed HIDDEN request for Openbox off-workspace state.
    /// True only after dispatch/check; neither ADD nor REMOVE implies success.
    fn hidden(
        &mut self,
        hidden: bool,
        deadline: Instant,
        dispatched: &mut bool,
    ) -> Result<bool, HostError>;
    fn placement(&mut self) -> Result<ShowPlacement, HostError>;
    fn correct(
        &mut self,
        target: Point,
        deadline: Instant,
        dispatched: &mut bool,
    ) -> Result<Option<Instant>, HostError>;
    fn enable(
        &mut self,
        snapshot: &x11::visibility::VisibilitySnapshot,
        placement: &InputPlacement,
        deadline: Option<Instant>,
    ) -> Result<bool, HostError>;
    /// Only the internal Recover composition uses strict Stage-B evidence.
    fn recover_evidence(&mut self, _destination: u32) -> Result<HereEvidence, HostError> {
        Err("Recover requires the strict destination adapter".into())
    }
    fn recover_owned(&mut self) -> Result<bool, HostError> {
        Err("Recover requires fresh selection ownership".into())
    }
    fn recover_geometry(
        &mut self,
        _layout: control::DestinationIdentity,
    ) -> Result<geometry::Rect, HostError> {
        Err("Recover requires actual root geometry".into())
    }
    fn restrict_recover_correction(&mut self) {}
    fn recover_input_ready(&self) -> bool {
        self.noop_ready()
    }
    fn restore_recover(
        &mut self,
        _operation: &mut control::ActiveOperation,
        _deadline: Instant,
        _dispatched: &mut bool,
    ) -> Result<bool, HostError> {
        Err("Recover requires guarded native restoration".into())
    }
}

struct ShowPlacement {
    input: InputPlacement,
    area: crate::geometry::Rect,
    current_desktop: u32,
}

fn show_result(
    operation: &mut control::ActiveOperation,
    status: control::Status,
    reason: control::Reason,
    confirmed: Option<control::ConfirmedVisibility>,
) -> control::OperationResult {
    operation.detail.reason = reason;
    operation.detail.progress.not_minimized_confirmed = confirmed.is_some();
    operation.detail.progress.input_ready =
        confirmed == Some(control::ConfirmedVisibility::NotMinimizedHere);
    operation.detail.progress.placement_deferred =
        confirmed == Some(control::ConfirmedVisibility::NotMinimizedElsewhere);
    control::OperationResult::new(
        control::Command::Show,
        status,
        operation.detail,
        control::Confirmation {
            visibility: confirmed,
            above: false,
        },
    )
    .expect("Show terminal evidence must satisfy the accepted result contract")
}

fn hide_result(
    operation: &mut control::ActiveOperation,
    status: control::Status,
    reason: control::Reason,
) -> control::OperationResult {
    operation.detail.reason = reason;
    control::OperationResult::new(
        control::Command::Hide,
        status,
        operation.detail,
        control::Confirmation {
            visibility: (status == control::Status::Success)
                .then_some(control::ConfirmedVisibility::Minimized),
            above: false,
        },
    )
    .expect("Hide success requires fresh coherent minimization with known workspace")
}

/// Hide always settles input, even for an already minimized body. There is no
/// placement work here: the existing fence invalidates local corrections too.
fn service_hide_with(
    operation: &mut control::ActiveOperation,
    host: &mut impl VisibilityHost,
) -> Result<Option<control::OperationResult>, HostError> {
    use control::{HidePhase, Reason, Status, Visibility};
    operation.hide.observe = false;
    if operation.hide.phase == HidePhase::Fence && host.now() < operation.deadline {
        operation.detail.reason = Reason::ReleaseFailed;
        host.fence()?;
        operation.hide.phase = HidePhase::Fenced;
    }
    let started_expired = host.now() >= operation.deadline;
    let (snapshot, reason) = observe_visibility(operation, host)?;
    if let Some(reason) = reason {
        return Ok(Some(hide_result(operation, Status::Failed, reason)));
    }
    if !started_expired && host.now() >= operation.deadline {
        return service_hide_final(operation, host);
    }
    if visibility_observation_eligible(operation, &snapshot) {
        if operation.hide.phase != HidePhase::Fence && !host.fence_settled() {
            operation.detail.reason = Reason::ReleaseFailed;
            return Err("Hide input fence has unsettled obligations".into());
        }
        if snapshot.candidate.visibility == Visibility::Minimized {
            return service_hide_final(operation, host);
        }
        if operation.hide.phase == HidePhase::Fenced && host.now() < operation.deadline {
            let elsewhere = snapshot.candidate.visibility == Visibility::NotMinimizedElsewhere;
            operation.hide.phase = if elsewhere {
                HidePhase::HiddenAddAttempted
            } else {
                HidePhase::IconifyAttempted
            };
            operation.detail.reason = Reason::X11Failure;
            let requested = if elsewhere {
                host.hidden(
                    true,
                    operation.deadline,
                    &mut operation.detail.progress.host_mutation_dispatched,
                )?
            } else {
                host.iconify(
                    operation.deadline,
                    &mut operation.detail.progress.host_mutation_dispatched,
                )?
            };
            if requested {
                operation.hide.phase = if elsewhere {
                    HidePhase::HiddenAddRequested
                } else {
                    HidePhase::IconifyRequested
                };
            }
            if host.now() >= operation.deadline {
                return service_hide_final(operation, host);
            }
            // Drain buffered WM notifications before waiting. No acknowledgement
            // or continued Iconic/no-HIDDEN state can complete this operation.
            return Ok(None);
        }
    }
    if host.now() >= operation.deadline {
        return Ok(Some(hide_result(
            operation,
            Status::Failed,
            Reason::ConfirmationTimeout,
        )));
    }
    Ok(None)
}

fn service_hide_final(
    operation: &mut control::ActiveOperation,
    host: &mut impl VisibilityHost,
) -> Result<Option<control::OperationResult>, HostError> {
    use control::{HidePhase, Reason, Status, Visibility};
    let (fresh, reason) = observe_visibility(operation, host)?;
    if let Some(reason) = reason {
        return Ok(Some(hide_result(operation, Status::Failed, reason)));
    }
    if visibility_observation_eligible(operation, &fresh)
        && fresh.candidate.visibility == Visibility::Minimized
        && operation.hide.phase != HidePhase::Fence
        && host.fence_settled()
    {
        return Ok(Some(hide_result(operation, Status::Success, Reason::None)));
    }
    // A contradictory final candidate may still settle before the fixed deadline.
    Ok((host.now() >= operation.deadline)
        .then(|| hide_result(operation, Status::Failed, Reason::ConfirmationTimeout)))
}

fn service_visibility_with(
    operation: &mut control::ActiveOperation,
    host: &mut impl VisibilityHost,
) -> Result<Option<control::OperationResult>, HostError> {
    match operation.request.command {
        control::Command::Show => service_show_safely_with(operation, host),
        control::Command::Hide => {
            let outcome = service_hide_with(operation, host)?;
            if outcome
                .as_ref()
                .is_some_and(|r| r.status() == control::Status::Failed)
            {
                // Recovery is independent of the terminal outcome and never retries
                // iconification. Unverifiable/elsewhere bodies retain the fence for
                // later lifecycle evidence, even when no further command arrives.
                if let Err(error) = restore_deferred_input(host) {
                    operation.detail.reason = control::Reason::X11Failure;
                    return Err(error);
                }
            }
            Ok(outcome)
        }
        control::Command::BringTop => unreachable!("Bring Top uses the remote composition service"),
        control::Command::BringHere => {
            Err("Bring Here requires the dedicated Here owner service".into())
        }
        control::Command::Recover => {
            Err("Recover requires the dedicated placement owner service".into())
        }
    }
}

fn recover_top(operation: &control::ActiveOperation) -> bool {
    operation.request.command == control::Command::Recover
        && operation.recover_phase == control::RecoverPhase::Top
}

fn destination_evidence(
    evidence: &HereEvidence,
) -> Result<control::DestinationIdentity, control::Reason> {
    let layout = evidence.layout.as_ref().copied().map_err(|error| {
        eprintln!("[HERE] Destination evidence: {error}");
        here_destination_reason(error)
    });
    let sticky = evidence
        .snapshot
        .observation
        .as_ref()
        .is_some_and(|o| o.desktop == control::Evidence::Readable(u32::MAX));
    if sticky || evidence.snapshot.candidate.reason == control::Reason::Unsupported {
        layout.and(Err(control::Reason::Unsupported))
    } else {
        layout
    }
}

fn read_recover_top(
    operation: &mut control::ActiveOperation,
    host: &mut impl VisibilityHost,
) -> Result<(HereEvidence, control::HereDecision), HostError> {
    if operation.placement.invalidation_reason() == Some(control::Reason::OwnershipLost)
        || !host.recover_owned()?
    {
        reject_recover(operation, control::Reason::OwnershipLost);
        return Ok((
            unavailable_recover_evidence(),
            control::HereDecision::Invalid(control::Reason::OwnershipLost),
        ));
    }
    let evidence = host.recover_evidence(operation.placement.destination_desktop)?;
    if !host.recover_owned()? {
        reject_recover(operation, control::Reason::OwnershipLost);
    }
    let decision =
        operation.observe_recover_top(destination_evidence(&evidence), evidence.observation);
    if decision == control::HereDecision::Destination && operation.show.baseline.is_none() {
        operation.show.baseline = Some(control::VisibilityBaseline {
            desktop: operation.placement.destination_desktop,
            current_desktop: operation.placement.destination_desktop,
            layer: evidence.observation.layer,
        });
    }
    Ok((evidence, decision))
}

/// No native evidence is acquired after proven owner loss. This unavailable
/// snapshot only carries the already-latched lifecycle outcome to the borrower.
fn unavailable_recover_evidence() -> HereEvidence {
    let evidence = control::VisibilityEvidence {
        destroyed: false,
        wm_state: control::WmState::Malformed,
        map_state: control::Evidence::Malformed,
        hidden: control::Evidence::Malformed,
        workspace: control::Workspace::Unknown,
        capability: control::Capability::Unverifiable,
        coherent: false,
    };
    HereEvidence {
        layout: Err(x11::monitors::destination::DestinationError::Changed),
        observation: control::HereObservation {
            membership: None,
            visibility: control::Visibility::Unknown,
            layer: control::LayerObservation::Unknown,
        },
        snapshot: x11::visibility::VisibilitySnapshot {
            observation: None,
            evidence,
            candidate: control::classify(evidence),
            layer: control::LayerObservation::Unknown,
        },
    }
}

fn reject_recover(operation: &mut control::ActiveOperation, reason: control::Reason) {
    operation.placement.invalidate_here(reason);
    operation.detail.reason = reason;
    operation.detail.progress.input_ready = false;
}

/// A guard veto is ordinary evidence failure, never a fabricated HostError.
fn recover_visibility_guard(
    operation: &mut control::ActiveOperation,
    host: &mut impl VisibilityHost,
) -> Result<bool, HostError> {
    let (_, decision) = read_recover_top(operation, host)?;
    if decision != control::HereDecision::Destination {
        let reason = match decision {
            control::HereDecision::Invalid(reason) => reason,
            _ => control::Reason::Unverifiable,
        };
        reject_recover(operation, reason);
        return Ok(false);
    }
    // A minimized origin remains deferred, but restoration must not be used to
    // learn whether an externally resized body still has the fixed canvas.
    let layout = operation
        .placement
        .layout()
        .ok_or("Recover has no admitted destination")?;
    let geometry = host.recover_geometry(layout)?;
    let (_, after) = read_recover_top(operation, host)?;
    if after != control::HereDecision::Destination
        || !operation
            .placement
            .layout()
            .is_some_and(|layout| layout.body_size == geometry.size())
    {
        let reason = operation
            .placement
            .invalidation_reason()
            .unwrap_or(control::Reason::Unverifiable);
        reject_recover(operation, reason);
        return Ok(false);
    }
    if !operation.recover_visibility_mutation_eligible(host.now()) {
        operation.detail.reason = control::Reason::ConfirmationTimeout;
        return Ok(false);
    }
    Ok(true)
}

fn visibility_placement(
    operation: &control::ActiveOperation,
    host: &mut impl VisibilityHost,
) -> Result<ShowPlacement, HostError> {
    if !recover_top(operation) {
        return host.placement();
    }
    let layout = operation
        .placement
        .layout()
        .ok_or("Recover has no admitted destination")?;
    Ok(ShowPlacement {
        area: layout.usable_area,
        current_desktop: layout.current_desktop,
        input: InputPlacement {
            selected_monitor: layout.monitor_name,
            observed_monitor: layout.monitor_name,
            geometry: host.recover_geometry(layout)?,
            bounds: layout.valid_origins,
        },
    })
}

/// Read-only geometry bracket shared by input enable, each layer dispatch and
/// final proof. Calling it at expiry cannot grant mutation authority.
fn recover_readiness(
    operation: &mut control::ActiveOperation,
    host: &mut impl VisibilityHost,
    input_ready: bool,
) -> Result<Option<(x11::visibility::VisibilitySnapshot, InputPlacement)>, HostError> {
    let (_, before) = read_recover_top(operation, host)?;
    if before != control::HereDecision::Destination {
        return Ok(None);
    }
    let placement = visibility_placement(operation, host)?;
    let (fresh, after) = read_recover_top(operation, host)?;
    if after != control::HereDecision::Destination
        || fresh.observation.visibility != control::Visibility::NotMinimizedHere
        || !operation.confirm_recover_top_readiness(
            placement.input.geometry.origin(),
            placement.input.geometry.size(),
            input_ready && host.recover_input_ready(),
        )
    {
        return Ok(None);
    }
    Ok(Some((fresh.snapshot, placement.input)))
}

fn observe_visibility(
    operation: &mut control::ActiveOperation,
    host: &mut impl VisibilityHost,
) -> Result<(x11::visibility::VisibilitySnapshot, Option<control::Reason>), HostError> {
    use control::{Evidence, LayerObservation, Reason, Visibility, VisibilityBaseline};
    if recover_top(operation) {
        let (evidence, decision) = read_recover_top(operation, host)?;
        let reason = match decision {
            control::HereDecision::Invalid(reason) => Some(reason),
            _ => visibility_layer_issue(host, &evidence.snapshot),
        };
        return Ok((evidence.snapshot, reason));
    }
    operation.detail.reason = Reason::X11Failure;
    let snapshot = host.observe()?;
    operation.detail.visibility = snapshot.candidate.visibility;
    operation.detail.workspace = snapshot.evidence.workspace;
    operation.detail.layer = snapshot.layer;
    operation.detail.reason = snapshot.candidate.reason;
    let terminal = match snapshot.candidate.visibility {
        Visibility::Destroyed => Some(Reason::BodyDestroyed),
        Visibility::Withdrawn => Some(Reason::Withdrawn),
        _ if snapshot.candidate.reason == Reason::Unsupported => Some(Reason::Unsupported),
        _ => None,
    };
    if terminal.is_some() {
        return Ok((snapshot, terminal));
    }
    let (baseline, layout_interrupted) = match operation.request.command {
        control::Command::Hide => (
            &mut operation.hide.baseline,
            operation.hide.layout_interrupted,
        ),
        control::Command::Show | control::Command::BringTop => (
            &mut operation.show.baseline,
            operation.show.layout_interrupted,
        ),
        control::Command::BringHere | control::Command::Recover => {
            unreachable!("Placement uses its dedicated owner service")
        }
    };
    if layout_interrupted {
        return Ok((snapshot, Some(Reason::LayoutChanged)));
    }
    if snapshot.evidence.coherent {
        if let Some(observed) = &snapshot.observation {
            if let (Evidence::Readable(desktop), Evidence::Readable(current_desktop)) =
                (observed.desktop, observed.current_desktop)
            {
                if let Some(baseline) = *baseline {
                    if desktop != baseline.desktop || current_desktop != baseline.current_desktop {
                        return Ok((snapshot, Some(Reason::LayoutChanged)));
                    }
                    if operation.request.command != control::Command::BringTop
                        && snapshot.layer != LayerObservation::Unknown
                        && snapshot.layer != baseline.layer
                    {
                        return Ok((snapshot, Some(Reason::LayerChanged)));
                    }
                } else if matches!(
                    snapshot.candidate.visibility,
                    Visibility::Minimized
                        | Visibility::NotMinimizedHere
                        | Visibility::NotMinimizedElsewhere
                ) {
                    if !matches!(
                        snapshot.layer,
                        LayerObservation::Normal
                            | LayerObservation::Above
                            | LayerObservation::Below
                    ) {
                        let reason = if operation.request.command == control::Command::BringTop {
                            visibility_layer_issue(host, &snapshot).unwrap_or(Reason::Unverifiable)
                        } else {
                            Reason::Unverifiable
                        };
                        return Ok((snapshot, Some(reason)));
                    }
                    if operation.request.command == control::Command::BringTop {
                        operation.bring_top.minimized_at_start =
                            snapshot.candidate.visibility == Visibility::Minimized;
                    }
                    *baseline = Some(VisibilityBaseline {
                        desktop,
                        current_desktop,
                        layer: snapshot.layer,
                    });
                }
            }
        }
    }
    if operation.request.command == control::Command::BringTop
        && snapshot.evidence.coherent
        && baseline.is_some()
    {
        if operation.bring_top.minimized_at_start
            && matches!(
                snapshot.candidate.visibility,
                Visibility::NotMinimizedHere | Visibility::NotMinimizedElsewhere
            )
        {
            operation.bring_top.restoration_confirmed = true;
        }
        if let Some(reason) = visibility_layer_issue(host, &snapshot) {
            return Ok((snapshot, Some(reason)));
        }
    }
    Ok((snapshot, None))
}

fn visibility_layer_issue(
    host: &impl VisibilityHost,
    snapshot: &x11::visibility::VisibilitySnapshot,
) -> Option<control::Reason> {
    use control::{Evidence, LayerObservation, Reason};
    let atoms = host.layer_atoms();
    let Some(observation) = snapshot.observation.as_ref() else {
        return Some(Reason::Unverifiable);
    };
    let Evidence::Readable(support) = &observation.supported else {
        return Some(Reason::Unverifiable);
    };
    if ![atoms.state, atoms.above, atoms.below]
        .iter()
        .all(|a| support.contains(a))
    {
        return Some(Reason::Unsupported);
    }
    matches!(
        snapshot.layer,
        LayerObservation::Unknown | LayerObservation::Conflict
    )
    .then_some(Reason::Unverifiable)
}

fn visibility_observation_eligible(
    operation: &control::ActiveOperation,
    snapshot: &x11::visibility::VisibilitySnapshot,
) -> bool {
    if recover_top(operation) {
        return operation.placement.invalidation_reason().is_none()
            && operation.placement.proof.current.destination
            && matches!(
                snapshot.candidate.visibility,
                control::Visibility::Minimized | control::Visibility::NotMinimizedHere
            );
    }
    let baseline = match operation.request.command {
        control::Command::Hide => operation.hide.baseline,
        control::Command::Show | control::Command::BringTop => operation.show.baseline,
        control::Command::BringHere | control::Command::Recover => return false,
    };
    baseline.is_some_and(|b| {
        if operation.request.command == control::Command::BringTop {
            matches!(
                snapshot.layer,
                control::LayerObservation::Normal
                    | control::LayerObservation::Above
                    | control::LayerObservation::Below
            )
        } else {
            snapshot.layer == b.layer
        }
    }) && matches!(
        snapshot.candidate.visibility,
        control::Visibility::Minimized
            | control::Visibility::NotMinimizedHere
            | control::Visibility::NotMinimizedElsewhere
    )
}

fn validate_show_placement(
    operation: &mut control::ActiveOperation,
    placement: &ShowPlacement,
) -> Result<Option<control::Reason>, HostError> {
    let input = &placement.input;
    if recover_top(operation) {
        let valid = operation.placement.layout().is_some_and(|layout| {
            input.selected_monitor == layout.monitor_name
                && input.observed_monitor == layout.monitor_name
                && input.geometry.size() == layout.body_size
                && input.bounds == layout.valid_origins
                && placement.area == layout.usable_area
                && placement.current_desktop == layout.destination_desktop
        });
        return Ok((!valid).then_some(control::Reason::Unverifiable));
    }
    if input.selected_monitor == 0 || input.selected_monitor != input.observed_monitor {
        return Err(
            "Retained monitor disappeared during Show; automatic migration is unsupported".into(),
        );
    }
    if input.geometry.size() != Size::new(WINDOW_WIDTH.into(), WINDOW_HEIGHT.into()) {
        return Err("Body resized during Show placement verification".into());
    }
    if operation
        .show
        .baseline
        .is_none_or(|b| b.current_desktop != placement.current_desktop)
        || operation
            .show
            .area
            .is_some_and(|area| area != placement.area)
    {
        return Ok(Some(control::Reason::LayoutChanged));
    }
    operation.show.area = Some(placement.area);
    Ok(None)
}

/// Each invocation is bounded and returns to the existing event loop. Relevant
/// events only request re-observation; they never confirm a phase or extend time.
#[derive(Debug, Clone, Copy)]
enum VisibilityCompletion {
    Ready(control::ConfirmedVisibility),
    Failed(control::Status, control::Reason),
}

fn visibility_completion(
    operation: &mut control::ActiveOperation,
    status: control::Status,
    reason: control::Reason,
    confirmed: Option<control::ConfirmedVisibility>,
) -> VisibilityCompletion {
    operation.detail.reason = reason;
    match confirmed {
        Some(visibility) if status == control::Status::Success => {
            VisibilityCompletion::Ready(visibility)
        }
        _ => VisibilityCompletion::Failed(status, reason),
    }
}

fn visibility_cutoff(operation: &control::ActiveOperation) -> Instant {
    if operation.request.command == control::Command::BringTop || recover_top(operation) {
        operation
            .bring_top
            .visibility_cutoff
            .min(operation.deadline)
    } else {
        operation.deadline
    }
}

fn service_show_with(
    operation: &mut control::ActiveOperation,
    host: &mut impl VisibilityHost,
) -> Result<Option<control::OperationResult>, HostError> {
    service_visibility_stage_with(operation, host).map(|outcome| {
        outcome.map(|completion| match completion {
            VisibilityCompletion::Ready(visibility) => show_result(
                operation,
                control::Status::Success,
                control::Reason::None,
                Some(visibility),
            ),
            VisibilityCompletion::Failed(status, reason) => {
                show_result(operation, status, reason, None)
            }
        })
    })
}

fn service_visibility_stage_with(
    operation: &mut control::ActiveOperation,
    host: &mut impl VisibilityHost,
) -> Result<Option<VisibilityCompletion>, HostError> {
    use control::{ConfirmedVisibility, Reason, ShowPhase, Stage, Status, Visibility};
    operation.show.observe = false;
    if operation.show.phase == ShowPhase::Observe
        && host.placement_busy()
        && host.now() < visibility_cutoff(operation)
    {
        operation.detail.stage = Stage::Preflight;
        return Ok(Some(visibility_completion(
            operation,
            Status::Busy,
            Reason::Busy,
            None,
        )));
    }
    let started_expired = host.now() >= visibility_cutoff(operation);
    let (mut snapshot, reason) = observe_visibility(operation, host)?;
    if let Some(reason) = reason {
        return Ok(Some(visibility_completion(
            operation,
            Status::Failed,
            reason,
            None,
        )));
    }
    // If synchronous observation crossed expiry, this is the single final fresh
    // read. No subsequent visibility, geometry or enabling mutation is permitted.
    let mut expired = host.now() >= visibility_cutoff(operation);
    if expired && !started_expired {
        return service_visibility_stage_final(operation, host);
    }
    if !visibility_observation_eligible(operation, &snapshot) {
        return Ok(expired.then(|| {
            visibility_completion(operation, Status::Failed, Reason::ConfirmationTimeout, None)
        }));
    }
    if operation.show.phase == ShowPhase::Observe {
        let local_noop =
            snapshot.candidate.visibility == Visibility::NotMinimizedHere && host.noop_ready();
        if !local_noop && !host.fenced() {
            if expired {
                return Ok(Some(visibility_completion(
                    operation,
                    Status::Failed,
                    Reason::ConfirmationTimeout,
                    None,
                )));
            }
            operation.detail.reason = Reason::ReleaseFailed;
            host.fence()?;
            operation.show.phase = ShowPhase::Fenced;
            // Fence/cancellation is not permanent permission to map: classify again.
            let (fresh, reason) = observe_visibility(operation, host)?;
            snapshot = fresh;
            if let Some(reason) = reason {
                return Ok(Some(visibility_completion(
                    operation,
                    Status::Failed,
                    reason,
                    None,
                )));
            }
            expired = host.now() >= visibility_cutoff(operation);
            if expired {
                return service_visibility_stage_final(operation, host);
            }
            if !visibility_observation_eligible(operation, &snapshot) {
                return Ok(expired.then(|| {
                    visibility_completion(
                        operation,
                        Status::Failed,
                        Reason::ConfirmationTimeout,
                        None,
                    )
                }));
            }
        } else if host.fenced() {
            if !host.fence_settled() {
                operation.detail.reason = Reason::ReleaseFailed;
                return Err("Existing visibility input fence has unsettled obligations".into());
            }
            operation.show.phase = ShowPhase::Fenced;
        }
    }
    match snapshot.candidate.visibility {
        Visibility::Minimized => {
            if expired {
                return Ok(Some(visibility_completion(
                    operation,
                    Status::Failed,
                    Reason::ConfirmationTimeout,
                    None,
                )));
            }
            if operation.show.phase == ShowPhase::Fenced {
                if recover_top(operation) && !recover_visibility_guard(operation, host)? {
                    return service_visibility_stage_final(operation, host);
                }
                if recover_top(operation) && operation.detail.visibility != Visibility::Minimized {
                    return service_visibility_stage_final(operation, host);
                }
                operation.detail.reason = Reason::X11Failure;
                // Never retry, including on a checked native failure.
                let elsewhere = snapshot.evidence.workspace == control::Workspace::Other;
                operation.show.phase = if elsewhere {
                    ShowPhase::HiddenRemoveAttempted
                } else {
                    ShowPhase::RestoreAttempted
                };
                let requested = if recover_top(operation) {
                    let mut dispatched = false;
                    let result = host.restore_recover(
                        operation,
                        visibility_cutoff(operation),
                        &mut dispatched,
                    );
                    operation.detail.progress.host_mutation_dispatched |= dispatched;
                    result?
                } else if elsewhere {
                    host.hidden(
                        false,
                        visibility_cutoff(operation),
                        &mut operation.detail.progress.host_mutation_dispatched,
                    )?
                } else {
                    host.restore(
                        visibility_cutoff(operation),
                        &mut operation.detail.progress.host_mutation_dispatched,
                    )?
                };
                if requested {
                    operation.show.phase = if elsewhere {
                        ShowPhase::HiddenRemoveRequested
                    } else {
                        ShowPhase::RestoreRequested
                    };
                }
                if !requested || host.now() >= visibility_cutoff(operation) {
                    return service_visibility_stage_final(operation, host);
                }
                // Checked requests may have buffered WM events. Let the loop drain
                // them before another observation or any socket wait.
                return Ok(None);
            }
            Ok(None)
        }
        Visibility::NotMinimizedElsewhere => {
            let (fresh, reason) = observe_visibility(operation, host)?;
            if let Some(reason) = reason {
                return Ok(Some(visibility_completion(
                    operation,
                    Status::Failed,
                    reason,
                    None,
                )));
            }
            if visibility_observation_eligible(operation, &fresh)
                && fresh.candidate.visibility == Visibility::NotMinimizedElsewhere
                && host.fence_settled()
            {
                return Ok(Some(visibility_completion(
                    operation,
                    Status::Success,
                    Reason::None,
                    Some(ConfirmedVisibility::NotMinimizedElsewhere),
                )));
            }
            Ok((host.now() >= visibility_cutoff(operation)).then(|| {
                visibility_completion(operation, Status::Failed, Reason::ConfirmationTimeout, None)
            }))
        }
        Visibility::NotMinimizedHere => {
            operation.stage = Stage::Geometry;
            operation.detail.stage = Stage::Geometry;
            operation.detail.reason = Reason::X11Failure;
            let placement = visibility_placement(operation, host)?;
            if let Some(reason) = validate_show_placement(operation, &placement)? {
                return Ok(Some(visibility_completion(
                    operation,
                    Status::Failed,
                    reason,
                    None,
                )));
            }
            let now = host.now();
            if now >= visibility_cutoff(operation) {
                return service_visibility_stage_final(operation, host);
            }
            if let Some((target, deadline)) = operation.show.correction {
                if now < deadline {
                    return Ok(None);
                }
                if placement.input.geometry.origin() != target {
                    if recover_top(operation) {
                        return Ok(Some(visibility_completion(
                            operation,
                            Status::Failed,
                            Reason::Unverifiable,
                            None,
                        )));
                    }
                    return Err(
                        "WM refused necessary Show bounds correction; will not retry".into(),
                    );
                }
                operation.show.correction = None;
            }
            if !placement
                .input
                .bounds
                .contains(placement.input.geometry.origin())
            {
                if now >= visibility_cutoff(operation)
                    || now + CONFIRMATION_TIMEOUT > visibility_cutoff(operation)
                {
                    return Ok(Some(visibility_completion(
                        operation,
                        Status::Failed,
                        Reason::ConfirmationTimeout,
                        None,
                    )));
                }
                if operation.show.correction_sent {
                    if recover_top(operation) {
                        return Ok(Some(visibility_completion(
                            operation,
                            Status::Failed,
                            Reason::Unverifiable,
                            None,
                        )));
                    }
                    return Err("Show bounds correction no longer valid; will not retry".into());
                }
                // A Ready no-op never cancels or moves an ongoing gesture.
                if !host.fenced() {
                    return Ok(Some(visibility_completion(
                        operation,
                        Status::Busy,
                        Reason::Busy,
                        None,
                    )));
                }
                let target = placement
                    .input
                    .bounds
                    .clamp(placement.input.geometry.origin());
                if recover_top(operation) {
                    if !recover_visibility_guard(operation, host)? {
                        return service_visibility_stage_final(operation, host);
                    }
                    if !operation.consume_recover_top_correction(target, host.now()) {
                        return Ok(Some(visibility_completion(
                            operation,
                            Status::Failed,
                            Reason::ConfirmationTimeout,
                            None,
                        )));
                    }
                    host.restrict_recover_correction();
                }
                if let Some(confirmation) = host.correct(
                    target,
                    visibility_cutoff(operation),
                    &mut operation.detail.progress.host_mutation_dispatched,
                )? {
                    operation.show.correction_sent = true;
                    if !recover_top(operation) {
                        operation.show.correction = Some((target, confirmation));
                    }
                } else {
                    return service_visibility_stage_final(operation, host);
                }
                if host.now() >= visibility_cutoff(operation) {
                    return service_visibility_stage_final(operation, host);
                }
                return Ok(None);
            }
            // Revalidate visibility/preservation after the synchronous geometry reads.
            let (fresh, reason) = observe_visibility(operation, host)?;
            if let Some(reason) = reason {
                return Ok(Some(visibility_completion(
                    operation,
                    Status::Failed,
                    reason,
                    None,
                )));
            }
            if !visibility_observation_eligible(operation, &fresh)
                || fresh.candidate.visibility != Visibility::NotMinimizedHere
            {
                return Ok((host.now() >= visibility_cutoff(operation)).then(|| {
                    visibility_completion(
                        operation,
                        Status::Failed,
                        Reason::ConfirmationTimeout,
                        None,
                    )
                }));
            }
            if host.fenced() {
                if host.now() >= visibility_cutoff(operation) {
                    return service_visibility_stage_final(operation, host);
                }
                operation.detail.reason = Reason::X11Failure;
                let verified = visibility_placement(operation, host)?;
                if let Some(reason) = validate_show_placement(operation, &verified)? {
                    return Ok(Some(visibility_completion(
                        operation,
                        Status::Failed,
                        reason,
                        None,
                    )));
                }
                if verified.input.geometry != placement.input.geometry {
                    return Ok(Some(visibility_completion(
                        operation,
                        Status::Failed,
                        Reason::LayoutChanged,
                        None,
                    )));
                }
                operation.show.phase = ShowPhase::EnablingInput;
                let enabled = if recover_top(operation) {
                    let Some((snapshot, input)) = recover_readiness(operation, host, false)? else {
                        reject_recover(operation, Reason::Unverifiable);
                        return service_visibility_stage_final(operation, host);
                    };
                    if input.geometry != verified.input.geometry
                        || !host.fence_settled()
                        || !operation.recover_visibility_mutation_eligible(host.now())
                    {
                        return service_visibility_stage_final(operation, host);
                    }
                    let enabled =
                        host.enable(&snapshot, &input, Some(visibility_cutoff(operation)))?;
                    if operation.placement.correction().is_some() {
                        host.restrict_recover_correction();
                    }
                    enabled
                } else {
                    host.enable(&fresh, &verified.input, Some(visibility_cutoff(operation)))?
                };
                if !enabled {
                    operation.show.phase = ShowPhase::Fenced;
                    return service_visibility_stage_final(operation, host);
                }
                operation.show.phase = ShowPhase::InputRestored;
            } else if !host.noop_ready() {
                return Ok(Some(visibility_completion(
                    operation,
                    Status::Busy,
                    Reason::Busy,
                    None,
                )));
            }
            service_visibility_stage_final(operation, host)
        }
        _ => unreachable!("eligible classification checked above"),
    }
}

/// Final read-only observation, also used at expiry. Success here requires input
/// that was already safely enabled; expiry never creates new readiness.
fn service_visibility_stage_final(
    operation: &mut control::ActiveOperation,
    host: &mut impl VisibilityHost,
) -> Result<Option<VisibilityCompletion>, HostError> {
    use control::{ConfirmedVisibility, Reason, Status, Visibility};
    if recover_top(operation) {
        let ready = recover_readiness(operation, host, true)?.is_some()
            && host.recover_input_ready()
            && operation.show.correction.is_none();
        return Ok(Some(if ready {
            visibility_completion(
                operation,
                Status::Success,
                Reason::None,
                Some(ConfirmedVisibility::NotMinimizedHere),
            )
        } else {
            let reason = operation.placement.invalidation_reason().unwrap_or(
                if host.now() >= visibility_cutoff(operation) {
                    Reason::ConfirmationTimeout
                } else {
                    Reason::Unverifiable
                },
            );
            visibility_completion(operation, Status::Failed, reason, None)
        }));
    }
    let (snapshot, reason) = observe_visibility(operation, host)?;
    if let Some(reason) = reason {
        return Ok(Some(visibility_completion(
            operation,
            Status::Failed,
            reason,
            None,
        )));
    }
    if visibility_observation_eligible(operation, &snapshot) {
        if snapshot.candidate.visibility == Visibility::NotMinimizedElsewhere
            && host.fence_settled()
        {
            return Ok(Some(visibility_completion(
                operation,
                Status::Success,
                Reason::None,
                Some(ConfirmedVisibility::NotMinimizedElsewhere),
            )));
        }
        if snapshot.candidate.visibility == Visibility::NotMinimizedHere && host.noop_ready() {
            operation.detail.reason = Reason::X11Failure;
            let placement = visibility_placement(operation, host)?;
            if let Some(reason) = validate_show_placement(operation, &placement)? {
                return Ok(Some(visibility_completion(
                    operation,
                    Status::Failed,
                    reason,
                    None,
                )));
            }
            if placement
                .input
                .bounds
                .contains(placement.input.geometry.origin())
                && operation.show.correction.is_none()
            {
                let (fresh, reason) = observe_visibility(operation, host)?;
                if let Some(reason) = reason {
                    return Ok(Some(visibility_completion(
                        operation,
                        Status::Failed,
                        reason,
                        None,
                    )));
                }
                if visibility_observation_eligible(operation, &fresh)
                    && fresh.candidate.visibility == Visibility::NotMinimizedHere
                {
                    return Ok(Some(visibility_completion(
                        operation,
                        Status::Success,
                        Reason::None,
                        Some(ConfirmedVisibility::NotMinimizedHere),
                    )));
                }
            }
        }
    }
    Ok(Some(visibility_completion(
        operation,
        Status::Failed,
        if host.now() >= visibility_cutoff(operation) {
            Reason::ConfirmationTimeout
        } else {
            Reason::Unverifiable
        },
        None,
    )))
}

/// Layer servicing is a host operation; it does not own a second request or
/// controller. Fakes use the same bounded adapter as the native implementation.
trait BringTopHost: VisibilityHost {
    fn qualify_layer(
        &self,
        snapshot: &x11::visibility::VisibilitySnapshot,
        baseline: control::VisibilityBaseline,
    ) -> Result<ObservedLayer, RemoteLayerIssue>;
    fn service_above(
        &mut self,
        operation: &mut control::ActiveOperation,
    ) -> Result<(LayerOutcome, Option<RemoteLayerIssue>), HostError>;
    fn abort_above(&mut self);
    fn layer_deadline(&self) -> Option<Instant>;
}

fn bring_top_failure_status(operation: &control::ActiveOperation) -> control::Status {
    if operation.bring_top.minimized_at_start && operation.bring_top.restoration_confirmed {
        control::Status::Partial
    } else {
        control::Status::Failed
    }
}

/// Current visibility is independent of historical restoration and of input
/// readiness. Even a Partial after destruction must clear current proof bits.
fn bring_top_confirmation(
    snapshot: &x11::visibility::VisibilitySnapshot,
) -> Option<control::ConfirmedVisibility> {
    use control::{ConfirmedVisibility as C, Visibility as V};
    if !snapshot.evidence.coherent {
        return None;
    }
    match snapshot.candidate.visibility {
        V::Minimized => Some(C::Minimized),
        V::NotMinimizedHere => Some(C::NotMinimizedHere),
        V::NotMinimizedElsewhere => Some(C::NotMinimizedElsewhere),
        _ => None,
    }
}

fn bring_top_workspace_preserved(
    operation: &control::ActiveOperation,
    snapshot: &x11::visibility::VisibilitySnapshot,
) -> bool {
    use control::Evidence;
    snapshot.evidence.coherent
        && operation.show.baseline.is_some_and(|baseline| {
            snapshot.observation.as_ref().is_some_and(|observed| {
                observed.desktop == Evidence::Readable(baseline.desktop)
                    && observed.current_desktop == Evidence::Readable(baseline.current_desktop)
            })
        })
}

fn compose_bring_top_result(
    operation: &mut control::ActiveOperation,
    reason: control::Reason,
    confirmed: control::Confirmation,
    ready: bool,
    allow_success: bool,
) -> control::OperationResult {
    use control::{ConfirmedVisibility as C, Reason, Status};
    let p = &mut operation.detail.progress;
    p.not_minimized_confirmed = matches!(
        confirmed.visibility,
        Some(C::NotMinimizedHere | C::NotMinimizedElsewhere)
    );
    p.above_confirmed = confirmed.above;
    p.placement_deferred = confirmed.visibility == Some(C::NotMinimizedElsewhere) && ready;
    p.input_ready = confirmed.visibility == Some(C::NotMinimizedHere) && ready;
    let success = allow_success && ready && p.not_minimized_confirmed && confirmed.above;
    let status = if success {
        Status::Success
    } else {
        bring_top_failure_status(operation)
    };
    operation.detail.reason = if success { Reason::None } else { reason };
    control::OperationResult::bring_top(status, operation.detail, confirmed, &operation.bring_top)
        .expect("Bring Top result requires genuine restoration proof and fresh current evidence")
}

fn finish_bring_top_with(
    operation: &mut control::ActiveOperation,
    host: &mut impl BringTopHost,
    reason: control::Reason,
    allow_success: bool,
) -> Result<control::OperationResult, HostError> {
    finish_bring_top_observed_with(operation, host, reason, allow_success).map(|(result, _)| result)
}

fn finish_bring_top_observed_with(
    operation: &mut control::ActiveOperation,
    host: &mut impl BringTopHost,
    reason: control::Reason,
    allow_success: bool,
) -> Result<(control::OperationResult, Option<RemoteLayerIssue>), HostError> {
    use control::{ConfirmedVisibility as C, Reason};
    if recover_top(operation) {
        let ready = recover_readiness(operation, host, true)?.is_some()
            && host.recover_input_ready()
            && operation.show.correction.is_none();
        let success = allow_success
            && ready
            && operation.detail.progress.above_confirmed
            && operation.placement.invalidation_reason().is_none()
            && !operation.show.layout_interrupted;
        if success {
            operation.stage = control::Stage::Layer;
            return Ok((
                operation
                    .recover_result(Reason::None, control::Stage::Layer)
                    .expect("Recover success requires the E1 fresh proof validator"),
                None,
            ));
        }
        let reason =
            operation
                .placement
                .invalidation_reason()
                .unwrap_or(if reason == Reason::None {
                    Reason::Unverifiable
                } else {
                    reason
                });
        return Ok((incomplete_recover(operation, reason), None));
    }
    let (mut snapshot, issue) = observe_visibility(operation, host)?;
    let mut reason = issue.unwrap_or(reason);
    let mut visibility = bring_top_confirmation(&snapshot);
    let mut ready = visibility == Some(C::NotMinimizedElsewhere)
        && host.fence_settled()
        && bring_top_workspace_preserved(operation, &snapshot);
    if visibility == Some(C::NotMinimizedHere)
        && host.noop_ready()
        && operation.show.correction.is_none()
    {
        operation.detail.reason = Reason::X11Failure;
        let placement = visibility_placement(operation, host)?;
        if let Some(issue) = validate_show_placement(operation, &placement)? {
            reason = issue;
        } else if placement
            .input
            .bounds
            .contains(placement.input.geometry.origin())
        {
            let (fresh, issue) = observe_visibility(operation, host)?;
            snapshot = fresh;
            visibility = bring_top_confirmation(&snapshot);
            if let Some(issue) = issue {
                reason = issue;
            }
            ready = visibility == Some(C::NotMinimizedHere) && host.noop_ready();
        }
    }
    let layer = operation
        .show
        .baseline
        .map(|baseline| host.qualify_layer(&snapshot, baseline));
    let above = layer == Some(Ok(ObservedLayer::Above));
    if reason == Reason::None && !(above && ready) {
        reason = match layer {
            Some(Err(issue)) => remote_layer_reason(issue),
            Some(Ok(ObservedLayer::Normal | ObservedLayer::Below)) => Reason::LayerChanged,
            _ => Reason::Unverifiable,
        };
    }
    // Workspace/layout failure cannot be converted into Success by Above alone.
    let eligible_success = allow_success
        && !operation.show.layout_interrupted
        && !matches!(
            reason,
            Reason::LayoutChanged
                | Reason::LayerChanged
                | Reason::OwnershipLost
                | Reason::Closing
                | Reason::X11Failure
        );
    Ok((
        compose_bring_top_result(
            operation,
            reason,
            control::Confirmation { visibility, above },
            ready,
            eligible_success,
        ),
        layer.and_then(Result::err),
    ))
}

fn service_bring_top_final_with(
    operation: &mut control::ActiveOperation,
    host: &mut impl BringTopHost,
    reason: control::Reason,
    allow_success: bool,
) -> Result<Option<control::OperationResult>, HostError> {
    let (result, issue) = finish_bring_top_observed_with(operation, host, reason, allow_success)?;
    if operation.bring_top.layer_finished
        && issue == Some(RemoteLayerIssue::Waiting)
        && host.now() < remote_operation_deadline(operation)
    {
        // Fresh final evidence is transitional. Wait for an event or the fixed
        // deadline; never restart the completed controller or send another request.
        Ok(None)
    } else {
        Ok(Some(result))
    }
}

fn remote_layer_reason(issue: RemoteLayerIssue) -> control::Reason {
    use control::{Reason, Visibility};
    match issue {
        RemoteLayerIssue::Waiting => Reason::ConfirmationTimeout,
        RemoteLayerIssue::Unsupported => Reason::Unsupported,
        RemoteLayerIssue::Unverifiable => Reason::Unverifiable,
        RemoteLayerIssue::WorkspaceChanged => Reason::LayoutChanged,
        RemoteLayerIssue::Unavailable(Visibility::Destroyed) => Reason::BodyDestroyed,
        RemoteLayerIssue::Unavailable(Visibility::Withdrawn) => Reason::Withdrawn,
        RemoteLayerIssue::Unavailable(_) => Reason::Unverifiable,
    }
}

fn service_bring_top_with(
    operation: &mut control::ActiveOperation,
    host: &mut impl BringTopHost,
) -> Result<Option<control::OperationResult>, HostError> {
    use control::{Reason, Stage, Status};
    if operation.stage != Stage::Layer {
        let outcome = service_visibility_stage_with(operation, host);
        let failed = matches!(
            outcome,
            Err(_) | Ok(Some(VisibilityCompletion::Failed(_, _)))
        );
        if failed
            && !recover_top(operation)
            && matches!(
                operation.show.phase,
                control::ShowPhase::EnablingInput | control::ShowPhase::InputRestored
            )
        {
            let cleanup = host.fence();
            if let Err(error) = &cleanup {
                eprintln!("[BRING TOP] Input safety cleanup: {error}");
            }
            // Retain original native failure priority, exactly as standalone Show.
            if let Err(original) = outcome {
                return Err(original);
            }
            cleanup?;
        }
        match outcome? {
            None => return Ok(None),
            Some(VisibilityCompletion::Failed(Status::Busy, reason)) => {
                if recover_top(operation) {
                    return Ok(Some(incomplete_recover(operation, Reason::Unverifiable)));
                }
                if operation.detail.progress.host_mutation_dispatched
                    || operation.show.phase != control::ShowPhase::Observe
                    || operation.bring_top.restoration_confirmed
                {
                    return finish_bring_top_with(operation, host, reason, false).map(Some);
                }
                // Busy is reserved for admission/readiness before operation work.
                operation.detail.reason = reason;
                return Ok(Some(
                    control::OperationResult::bring_top(
                        Status::Busy,
                        operation.detail,
                        control::Confirmation::default(),
                        &operation.bring_top,
                    )
                    .unwrap(),
                ));
            }
            Some(VisibilityCompletion::Failed(_, reason)) => {
                return finish_bring_top_with(operation, host, reason, false).map(Some)
            }
            Some(VisibilityCompletion::Ready(_)) => {
                if host.now() >= visibility_cutoff(operation) {
                    // A final read may confirm the whole already-achieved
                    // contract, but late readiness cannot start layer mutation.
                    return finish_bring_top_with(
                        operation,
                        host,
                        Reason::ConfirmationTimeout,
                        true,
                    )
                    .map(Some);
                }
                if recover_top(operation) {
                    if !operation.admit_recover_layer(host.now()) {
                        return finish_bring_top_with(
                            operation,
                            host,
                            Reason::ConfirmationTimeout,
                            true,
                        )
                        .map(Some);
                    }
                } else {
                    operation.stage = Stage::Layer;
                    operation.detail.stage = Stage::Layer;
                    operation.bring_top.layer_admitted = Some(host.now());
                    operation.bring_top.layer_observe = true;
                }
            }
        }
    }
    operation.bring_top.layer_observe = false;
    if operation.show.layout_interrupted {
        host.abort_above();
        return finish_bring_top_with(operation, host, Reason::LayoutChanged, false).map(Some);
    }
    if operation.bring_top.layer_finished {
        let reason = if host.now() >= remote_operation_deadline(operation) {
            Reason::ConfirmationTimeout
        } else {
            Reason::None
        };
        return service_bring_top_final_with(operation, host, reason, true);
    }
    operation.detail.reason = Reason::X11Failure;
    let (outcome, issue) = host.service_above(operation)?;
    let reason = issue.map(remote_layer_reason).unwrap_or(match outcome {
        LayerOutcome::AlreadyMatches | LayerOutcome::Confirmed => Reason::None,
        LayerOutcome::Waiting | LayerOutcome::TimedOut(_) => Reason::ConfirmationTimeout,
        LayerOutcome::Unsupported => Reason::Unsupported,
        LayerOutcome::Unverifiable => Reason::Unverifiable,
        LayerOutcome::Interrupted(_) => Reason::LayerChanged,
        LayerOutcome::Busy => Reason::Busy,
    });
    if outcome == LayerOutcome::Waiting {
        return Ok(None);
    }
    operation.bring_top.layer_finished = matches!(
        outcome,
        LayerOutcome::AlreadyMatches | LayerOutcome::Confirmed
    );
    host.abort_above();
    service_bring_top_final_with(
        operation,
        host,
        reason,
        matches!(
            outcome,
            LayerOutcome::AlreadyMatches | LayerOutcome::Confirmed | LayerOutcome::TimedOut(_)
        ),
    )
}

fn service_remote_with(
    operation: &mut control::ActiveOperation,
    host: &mut impl BringTopHost,
) -> Result<Option<control::OperationResult>, HostError> {
    if operation.request.command == control::Command::BringHere {
        return Err("Bring Here requires the dedicated Here owner service".into());
    }
    if operation.request.command == control::Command::Recover {
        return Err("Recover requires the dedicated placement owner service".into());
    }
    if operation.request.command == control::Command::BringTop {
        service_bring_top_with(operation, host)
    } else {
        service_visibility_with(operation, host)
    }
}

/// Retains marked dispatch even when the native check fails. A transitional
/// read cannot destroy a pending G1 controller; its original deadline still wins.
#[allow(clippy::too_many_arguments)]
fn service_remote_layer_with(
    layers: &mut LayerController,
    operation: &mut control::ActiveOperation,
    mut read: impl FnMut(
        &mut control::ActiveOperation,
    ) -> Result<Result<ObservedLayer, RemoteLayerIssue>, HostError>,
    support: impl FnMut() -> Result<PropertyResult<LayerSupport>, HostError>,
    mut send: impl FnMut(Mutation, &mut control::ActiveOperation, &mut bool) -> Result<bool, HostError>,
    now: impl FnMut() -> Instant,
) -> Result<(LayerOutcome, Option<RemoteLayerIssue>), HostError> {
    let admitted = operation
        .bring_top
        .layer_admitted
        .expect("Layer admission is fixed at visibility readiness");
    let outer = (admitted + layer::CONFIRMATION_TIMEOUT).min(operation.deadline);
    let issue = std::cell::Cell::new(None);
    if layers.pending().is_none() && operation.bring_top.layer_attempted {
        return Ok((LayerOutcome::Interrupted(layer::Phase::AwaitFinal), None));
    }
    let request = layers.pending().is_none().then_some(Layer::Above);
    let operation = std::cell::RefCell::new(operation);
    let result = service_qualified_layer_with(
        layers,
        request,
        Some(outer),
        true,
        || {
            let result = read(&mut operation.borrow_mut())?;
            issue.set(result.as_ref().err().copied());
            Ok(result)
        },
        support,
        |mutation| {
            let mut operation = operation.borrow_mut();
            operation.bring_top.layer_attempted = true;
            let mut dispatched = false;
            let result = send(mutation, &mut operation, &mut dispatched);
            operation.bring_top.layer_dispatched |= dispatched;
            operation.detail.progress.host_mutation_dispatched |= dispatched;
            operation.bring_top.layer_checked |= matches!(result, Ok(true));
            result
        },
        now,
    );
    result.map(|outcome| (outcome, issue.get()))
}

struct NativeVisibility<'a, C> {
    conn: &'a C,
    root: u32,
    atoms: &'a x11::visibility::VisibilityAtoms,
    layout_atoms: &'a x11::monitors::LayoutAtoms,
    selected_monitor: u32,
    window: &'a mut ManagedProbeWindow,
    runtime: &'a mut ProbeRuntime,
    bounds: &'a mut crate::geometry::ValidOriginBounds,
    area: &'a mut crate::geometry::Rect,
    recover_owner: Option<(u32, u32)>, // borrowed selection / admitted control XID
}

/// Strict D2.1 evidence. Ordinary invalid evidence is deliberately separate
/// from the outer, fatal native acquisition error.
struct HereEvidence {
    layout: Result<control::DestinationIdentity, x11::monitors::destination::DestinationError>,
    observation: control::HereObservation,
    snapshot: x11::visibility::VisibilitySnapshot,
}

/// Here owner service with no Show, layer, activation, resize, or current-desktop operation.
trait BringHereHost {
    fn now(&self) -> Instant;
    fn owned(&mut self) -> Result<bool, HostError>;
    fn live(&self) -> bool;
    fn evidence(&mut self, destination: u32) -> Result<HereEvidence, HostError>;
    fn placement_busy(&self) -> bool;
    fn input_active(&self) -> bool;
    fn visible_ready(&self) -> bool;
    fn fence_settled(&self) -> bool;
    fn fence(&mut self) -> Result<(), HostError>;
    fn abort_recovery_layer(&mut self) {}
    fn desktop(
        &mut self,
        destination: u32,
        deadline: Instant,
        dispatched: &mut bool,
    ) -> Result<bool, HostError>;
    fn geometry(&mut self) -> Result<geometry::Rect, HostError>;
    fn restrict_automatic_correction(&mut self);
    fn correct(
        &mut self,
        target: Point,
        deadline: Instant,
        dispatched: &mut bool,
    ) -> Result<Option<Instant>, HostError>;
    fn enable(
        &mut self,
        evidence: &HereEvidence,
        geometry: geometry::Rect,
        deadline: Instant,
    ) -> Result<bool, HostError>;
}

fn here_destination_reason(
    error: &x11::monitors::destination::DestinationError,
) -> control::Reason {
    use x11::monitors::destination::DestinationError;
    match error {
        DestinationError::Changed => control::Reason::LayoutChanged,
        DestinationError::UnsupportedViewport => control::Reason::Unsupported,
        DestinationError::Unverifiable(_) => control::Reason::Unverifiable,
    }
}

/// Function outcome only; the existing ActiveOperation remains the sole owner.
#[derive(Debug, Clone, Copy)]
enum HereCompletion {
    Ready(control::Confirmation),
    Incomplete(control::Reason),
    Busy,
}

/// Borrowed access to the same body's existing Show/layer mechanisms.
trait RecoverHost: BringHereHost {
    type Top: BringTopHost;
    fn top(&mut self) -> &mut Self::Top;
}

fn incomplete_recover(
    operation: &mut control::ActiveOperation,
    reason: control::Reason,
) -> control::OperationResult {
    operation.placement.invalidate_here(reason);
    let dispatched = operation.detail.progress.host_mutation_dispatched;
    operation.detail = control::preflight_detail(reason, operation.stage);
    operation.detail.progress.host_mutation_dispatched = dispatched;
    operation
        .recover_result(reason, operation.stage)
        .expect("Incomplete Recover retains only independently confirmed history")
}

/// Internal composition uses the same D4.1 safety/history policy as Here.
/// Both production admission guards remain closed through E3.
fn service_recover_with(
    operation: &mut control::ActiveOperation,
    host: &mut impl RecoverHost,
) -> Result<Option<control::OperationResult>, HostError> {
    let outcome = step_recover_with(operation, host);
    finalize_bring_here_with(operation, host, outcome)
}

fn step_recover_with(
    operation: &mut control::ActiveOperation,
    host: &mut impl RecoverHost,
) -> Result<Option<control::OperationResult>, HostError> {
    if operation.request.command != control::Command::Recover {
        return Err("Internal Recover requires its original request identity".into());
    }
    if operation.recover_phase == control::RecoverPhase::Here {
        match step_bring_here_with(operation, host)? {
            None => return Ok(None),
            Some(HereCompletion::Ready(confirmed)) => {
                if !host.owned()? {
                    return Ok(Some(incomplete_recover(
                        operation,
                        control::Reason::OwnershipLost,
                    )));
                }
                if !operation.handoff_recover_top(host.now(), operation.detail, confirmed) {
                    return Ok(Some(incomplete_recover(
                        operation,
                        control::Reason::ConfirmationTimeout,
                    )));
                }
                if operation.placement.correction().is_some() {
                    host.restrict_automatic_correction();
                }
            }
            Some(HereCompletion::Incomplete(reason)) => {
                return Ok(Some(incomplete_recover(operation, reason)));
            }
            Some(HereCompletion::Busy) => {
                return Ok(Some(
                    control::OperationResult::placement(
                        control::Command::Recover,
                        control::Status::Busy,
                        control::preflight_detail(control::Reason::Busy, control::Stage::Preflight),
                        control::Confirmation::default(),
                        &operation.placement,
                    )
                    .expect("Unstarted Recover cannot take existing placement ownership"),
                ));
            }
        }
    }
    // The handoff is immediately actionable; no new notification is needed.
    service_bring_top_with(operation, host.top())
}

fn here_cutoff(operation: &control::ActiveOperation) -> Instant {
    operation.here_cutoff().unwrap_or(operation.deadline)
}

fn stop_here(operation: &mut control::ActiveOperation, reason: control::Reason) -> HereCompletion {
    operation.stage = control::Stage::Shutdown;
    incomplete_here(operation, reason)
}

fn standalone_here_result(
    operation: &mut control::ActiveOperation,
    completion: HereCompletion,
) -> control::OperationResult {
    match completion {
        HereCompletion::Ready(confirmed) => operation
            .placement
            .here_result(operation.detail, confirmed)
            .expect("Here Success requires complete fresh placement/input proof"),
        HereCompletion::Incomplete(reason) => {
            operation.detail.reason = reason;
            operation
                .placement
                .here_result(operation.detail, control::Confirmation::default())
                .expect("Incomplete Here clears current claims and retains genuine history")
        }
        HereCompletion::Busy => control::OperationResult::placement(
            control::Command::BringHere,
            control::Status::Busy,
            control::preflight_detail(control::Reason::Busy, control::Stage::Preflight),
            control::Confirmation::default(),
            &operation.placement,
        )
        .expect("Unstarted Here does not steal existing placement ownership"),
    }
}

fn read_here(
    operation: &mut control::ActiveOperation,
    host: &mut impl BringHereHost,
) -> Result<(HereEvidence, control::HereDecision), HostError> {
    let evidence = host.evidence(operation.placement.destination_desktop)?;
    let layout = destination_evidence(&evidence);
    let decision = if operation.request.command == control::Command::Recover {
        let stage = operation.stage;
        let decision = operation.observe_recover_here(layout, evidence.observation);
        if stage == control::Stage::Geometry {
            operation.stage = stage;
            operation.detail.stage = stage;
        }
        decision
    } else {
        operation
            .placement
            .observe_here(layout, evidence.observation)
    };
    Ok((evidence, decision))
}

/// Ownership queries are synchronous too. Recover revalidates the complete
/// identity after them, immediately before consuming each mutation allowance.
fn guard_recover_here(
    operation: &mut control::ActiveOperation,
    host: &mut impl BringHereHost,
    expected: control::HereDecision,
    geometry: Option<geometry::Rect>,
) -> Result<Option<control::Reason>, HostError> {
    if operation.request.command != control::Command::Recover {
        return Ok(None);
    }
    let (_, decision) = read_here(operation, host)?;
    if !host.owned()? {
        operation
            .placement
            .invalidate_here(control::Reason::OwnershipLost);
        return Ok(Some(control::Reason::OwnershipLost));
    }
    if decision != expected {
        return Ok(Some(match decision {
            control::HereDecision::Invalid(reason) => reason,
            _ => control::Reason::Unverifiable,
        }));
    }
    if let Some(actual) = geometry {
        if !operation
            .placement
            .confirm_here_geometry(actual.origin(), actual.size())
        {
            return Ok(Some(control::Reason::Unverifiable));
        }
    }
    Ok(None)
}

fn incomplete_here(
    operation: &mut control::ActiveOperation,
    reason: control::Reason,
) -> HereCompletion {
    operation.placement.invalidate_here(reason);
    let dispatched = operation.detail.progress.host_mutation_dispatched;
    operation.detail = control::preflight_detail(reason, operation.stage);
    operation.detail.progress.host_mutation_dispatched = dispatched;
    HereCompletion::Incomplete(reason)
}

fn complete_here(
    operation: &mut control::ActiveOperation,
    evidence: &HereEvidence,
    input_ready: bool,
) -> HereCompletion {
    use control::{ConfirmedVisibility, GeometryProof, LayerObservation, Visibility};
    let minimized = evidence.observation.visibility == Visibility::Minimized;
    let above = !minimized && evidence.observation.layer == LayerObservation::Above;
    operation.detail = control::Detail {
        visibility: evidence.observation.visibility,
        workspace: control::Workspace::Current,
        layer: evidence.observation.layer,
        progress: control::Progress {
            host_mutation_dispatched: operation.detail.progress.host_mutation_dispatched,
            not_minimized_confirmed: !minimized,
            above_confirmed: above,
            placement_deferred: operation.placement.proof.current.geometry
                == GeometryProof::Deferred,
            input_ready,
        },
        placement: operation.placement.proof,
        ..control::preflight_detail(control::Reason::None, operation.stage)
    };
    HereCompletion::Ready(control::Confirmation {
        visibility: Some(if minimized {
            ConfirmedVisibility::Minimized
        } else {
            ConfirmedVisibility::NotMinimizedHere
        }),
        above,
    })
}

/// Read-only final visible proof brackets actual geometry with fresh strict
/// evidence. Neither checked ConfigureWindow nor SHAPE acknowledgment supplies
/// this proof. Used by safe no-op, restored input, and expiry finalization.
fn verify_here_visible(
    operation: &mut control::ActiveOperation,
    host: &mut impl BringHereHost,
) -> Result<HereCompletion, HostError> {
    use control::{HereDecision, Reason, Visibility};
    let (_, before) = read_here(operation, host)?;
    if let HereDecision::Invalid(reason) = before {
        return Ok(incomplete_here(operation, reason));
    }
    if before != HereDecision::Destination || !host.visible_ready() {
        return Ok(incomplete_here(operation, Reason::Unverifiable));
    }
    let actual = host.geometry()?;
    let (fresh, after) = read_here(operation, host)?;
    if let HereDecision::Invalid(reason) = after {
        return Ok(incomplete_here(operation, reason));
    }
    if after != HereDecision::Destination
        || fresh.observation.visibility != Visibility::NotMinimizedHere
        || !host.visible_ready()
        || !operation
            .placement
            .confirm_here_geometry(actual.origin(), actual.size())
    {
        return Ok(incomplete_here(operation, Reason::Unverifiable));
    }
    if !host.owned()? {
        return Ok(stop_here(operation, Reason::OwnershipLost));
    }
    if operation.request.command == control::Command::Recover {
        let (fresh, decision) = read_here(operation, host)?;
        if let HereDecision::Invalid(reason) = decision {
            return Ok(incomplete_here(operation, reason));
        }
        if decision != HereDecision::Destination
            || !host.visible_ready()
            || !operation
                .placement
                .confirm_here_geometry(actual.origin(), actual.size())
        {
            return Ok(incomplete_here(operation, Reason::Unverifiable));
        }
        return Ok(complete_here(operation, &fresh, true));
    }
    Ok(complete_here(operation, &fresh, true))
}

/// No loops/polling: immediate safety phases are bounded, then observation is
/// driven by notifications or the earliest fixed deadline.
fn step_bring_here_with(
    operation: &mut control::ActiveOperation,
    host: &mut impl BringHereHost,
) -> Result<Option<HereCompletion>, HostError> {
    use control::{HereDecision as D, HerePhase as P, Reason as R, Stage, Visibility as V};
    if !host.owned()? {
        return Ok(Some(stop_here(operation, R::OwnershipLost)));
    }
    if !host.live() {
        return Ok(Some(stop_here(operation, R::BodyDestroyed)));
    }
    if operation.placement.here_phase() == P::Observe && host.placement_busy() {
        operation.stage = Stage::Preflight;
        return Ok(Some(HereCompletion::Busy));
    }
    let (mut evidence, mut decision) = read_here(operation, host)?;
    if let D::Invalid(reason) = decision {
        return Ok(Some(incomplete_here(operation, reason)));
    }
    operation.stage = match operation.placement.here_phase() {
        P::Geometry | P::EnablingInput | P::InputRestored => Stage::Geometry,
        _ => Stage::Placement,
    };
    if decision == D::Pending
        && operation
            .placement
            .correction()
            .is_some_and(|(_, dl)| host.now() >= dl)
    {
        return Ok(Some(incomplete_here(operation, R::ConfirmationTimeout)));
    }
    // At phase expiry there is only this bounded final read (plus geometry for
    // already enabled input). No mutation authority is renewed by a late read.
    if host.now() >= here_cutoff(operation) {
        if decision == D::Destination {
            if evidence.observation.visibility == V::Minimized
                && host.fence_settled()
                && operation.placement.defer_here_geometry()
            {
                if !host.owned()? {
                    return Ok(Some(stop_here(operation, R::OwnershipLost)));
                }
                if operation.request.command == control::Command::Recover {
                    let (fresh, decision) = read_here(operation, host)?;
                    if let D::Invalid(reason) = decision {
                        return Ok(Some(incomplete_here(operation, reason)));
                    }
                    if decision != D::Destination
                        || !host.fence_settled()
                        || !operation.placement.defer_here_geometry()
                    {
                        return Ok(Some(incomplete_here(operation, R::Unverifiable)));
                    }
                    return Ok(Some(complete_here(operation, &fresh, false)));
                }
                return Ok(Some(complete_here(operation, &evidence, false)));
            }
            if host.visible_ready() {
                return verify_here_visible(operation, host).map(Some);
            }
        }
        return Ok(Some(incomplete_here(operation, R::ConfirmationTimeout)));
    }
    if operation.placement.here_phase() == P::Observe {
        if decision == D::Destination
            && evidence.observation.visibility == V::NotMinimizedHere
            && host.visible_ready()
        {
            let actual = host.geometry()?;
            if operation
                .placement
                .confirm_here_geometry(actual.origin(), actual.size())
            {
                return verify_here_visible(operation, host).map(Some);
            }
        }
        if host.now() >= here_cutoff(operation) {
            return Ok(Some(incomplete_here(operation, R::ConfirmationTimeout)));
        }
        if !host.owned()? {
            return Ok(Some(stop_here(operation, R::OwnershipLost)));
        }
        if host.now() >= here_cutoff(operation) {
            return Ok(Some(incomplete_here(operation, R::ConfirmationTimeout)));
        }
        if !host.fence_settled() {
            operation.detail.reason = R::ReleaseFailed;
            host.fence()?;
        }
        if !host.fence_settled() {
            return Ok(Some(incomplete_here(operation, R::ReleaseFailed)));
        }
        assert!(operation.placement.here_fenced());
        (evidence, decision) = read_here(operation, host)?;
        if let D::Invalid(reason) = decision {
            return Ok(Some(incomplete_here(operation, reason)));
        }
        if operation.placement.baseline.unwrap().desktop != operation.placement.destination_desktop
        {
            // The fresh pre-send round must still prove the original body,
            // including preservation. Transitional evidence grants no send.
            if decision != D::Original {
                if decision == D::Pending && host.now() < here_cutoff(operation) {
                    return Ok(None);
                }
                return Ok(Some(incomplete_here(operation, R::LayoutChanged)));
            }
        }
    }
    if operation.placement.here_phase() == P::Fenced
        && operation.placement.baseline.unwrap().desktop != operation.placement.destination_desktop
    {
        if decision == D::Pending && host.now() < here_cutoff(operation) {
            return Ok(None);
        }
        if !host.owned()? {
            return Ok(Some(stop_here(operation, R::OwnershipLost)));
        }
        if let Some(reason) = guard_recover_here(operation, host, D::Original, None)? {
            return Ok(Some(incomplete_here(operation, reason)));
        }
        if !host.fence_settled() || !operation.consume_here_desktop_attempt(host.now()) {
            return Ok(Some(incomplete_here(operation, R::ConfirmationTimeout)));
        }
        let mut dispatched = false;
        let transport = host.desktop(
            operation.placement.destination_desktop,
            here_cutoff(operation),
            &mut dispatched,
        );
        operation.detail.progress.host_mutation_dispatched |= dispatched;
        operation
            .placement
            .record_here_desktop_transport(dispatched, matches!(transport, Ok(true)));
        return match transport? {
            true => Ok(None), // drain WM evidence before observing again
            false => Ok(Some(incomplete_here(operation, R::ConfirmationTimeout))),
        };
    }
    if decision != D::Destination {
        return Ok(None);
    }
    if operation.placement.baseline.unwrap().minimized {
        // No geometry query, correction, map, hidden mutation, or enable here.
        let (fresh, final_decision) = read_here(operation, host)?;
        if let D::Invalid(reason) = final_decision {
            return Ok(Some(incomplete_here(operation, reason)));
        }
        if final_decision == D::Destination
            && host.fence_settled()
            && operation.placement.defer_here_geometry()
        {
            if !host.owned()? {
                return Ok(Some(stop_here(operation, R::OwnershipLost)));
            }
            if operation.request.command == control::Command::Recover {
                let (fresh, decision) = read_here(operation, host)?;
                if let D::Invalid(reason) = decision {
                    return Ok(Some(incomplete_here(operation, reason)));
                }
                if decision != D::Destination
                    || !host.fence_settled()
                    || !operation.placement.defer_here_geometry()
                {
                    return Ok(Some(incomplete_here(operation, R::Unverifiable)));
                }
                return Ok(Some(complete_here(operation, &fresh, false)));
            }
            return Ok(Some(complete_here(operation, &fresh, false)));
        }
        return Ok(None);
    }
    if evidence.observation.visibility != V::NotMinimizedHere {
        return Ok(None);
    }
    if matches!(
        operation.placement.here_phase(),
        P::Fenced | P::AwaitDestination
    ) {
        assert!(operation.placement.begin_here_geometry());
    }
    operation.stage = Stage::Geometry;
    let actual = host.geometry()?;
    // Discard geometry permission unless the complete fresh evidence bracket
    // still agrees. This also clears old current proof while retaining history.
    (evidence, decision) = read_here(operation, host)?;
    if let D::Invalid(reason) = decision {
        return Ok(Some(incomplete_here(operation, reason)));
    }
    if decision != D::Destination || evidence.observation.visibility != V::NotMinimizedHere {
        if operation
            .placement
            .correction()
            .is_some_and(|(_, dl)| host.now() >= dl)
        {
            return Ok(Some(incomplete_here(operation, R::ConfirmationTimeout)));
        }
        return Ok(None);
    }
    let layout = operation.placement.layout().unwrap();
    if actual.size() != layout.body_size {
        return Ok(Some(incomplete_here(operation, R::Unverifiable)));
    }
    if !operation
        .placement
        .confirm_here_geometry(actual.origin(), actual.size())
    {
        if let Some((_, deadline)) = operation.placement.correction() {
            return if host.now() >= deadline {
                Ok(Some(incomplete_here(operation, R::Unverifiable)))
            } else {
                Ok(None)
            };
        }
        if !host.owned()? {
            return Ok(Some(stop_here(operation, R::OwnershipLost)));
        }
        if let Some(reason) = guard_recover_here(operation, host, D::Destination, None)? {
            return Ok(Some(incomplete_here(operation, reason)));
        }
        let target = layout.valid_origins.clamp(actual.origin());
        if !host.fence_settled() || !operation.consume_here_correction(target, host.now()) {
            return Ok(Some(incomplete_here(operation, R::ConfirmationTimeout)));
        }
        // Consume runtime's automatic budget before even a failing native call.
        host.restrict_automatic_correction();
        let mut dispatched = false;
        let correction = host.correct(target, here_cutoff(operation), &mut dispatched);
        operation.detail.progress.host_mutation_dispatched |= dispatched;
        return match correction? {
            Some(_) => Ok(None), // D1's earlier immutable deadline governs
            None => Ok(Some(incomplete_here(operation, R::ConfirmationTimeout))),
        };
    }
    if !host.owned()? {
        return Ok(Some(stop_here(operation, R::OwnershipLost)));
    }
    if let Some(reason) = guard_recover_here(operation, host, D::Destination, Some(actual))? {
        return Ok(Some(incomplete_here(operation, reason)));
    }
    if !host.fence_settled() || !operation.consume_here_input_enable(host.now()) {
        return Ok(Some(incomplete_here(operation, R::ConfirmationTimeout)));
    }
    // Attempt consumed before checked SHAPE. The wrapper re-fences even when a
    // failing check may already have enabled native input.
    if !host.enable(&evidence, actual, here_cutoff(operation))? {
        return Ok(Some(incomplete_here(operation, R::ConfirmationTimeout)));
    }
    assert!(operation.here_input_restored());
    verify_here_visible(operation, host).map(Some)
}

/// Preserve the exact primary native error through final observation/cleanup.
/// Committing fatal teardown precedes the bounded history-only read: no later
/// snapshot can reconstruct Success for an owner that is already terminating.
fn service_bring_here_with(
    operation: &mut control::ActiveOperation,
    host: &mut impl BringHereHost,
) -> Result<Option<control::OperationResult>, HostError> {
    let outcome = step_bring_here_with(operation, host)
        .map(|outcome| outcome.map(|completion| standalone_here_result(operation, completion)));
    finalize_bring_here_with(operation, host, outcome)
}

/// Shared D3 safety/history policy for host-method and external native failures.
/// An externally supplied error enters before invalidation/cleanup, exactly as
/// an error raised by the Here step. There is no second fatal policy.
fn finalize_bring_here_with(
    operation: &mut control::ActiveOperation,
    host: &mut impl BringHereHost,
    mut outcome: Result<Option<control::OperationResult>, HostError>,
) -> Result<Option<control::OperationResult>, HostError> {
    if outcome.is_err() {
        operation
            .here_shutdown(control::Reason::X11Failure)
            .unwrap();
    }
    let failed = match &outcome {
        Err(_) => true,
        Ok(Some(result)) => !matches!(
            result.status(),
            control::Status::Success | control::Status::Busy
        ),
        Ok(None) => false,
    };
    let recover = operation.request.command == control::Command::Recover;
    let ownership_lost = recover
        && operation.placement.invalidation_reason() == Some(control::Reason::OwnershipLost);
    if failed && recover {
        host.abort_recovery_layer();
    }
    if failed
        && !ownership_lost
        && host.live()
        && operation.placement.baseline.is_some()
        && (host.input_active()
            || if recover {
                !host.fence_settled()
            } else {
                matches!(
                    operation.placement.here_phase(),
                    control::HerePhase::EnablingInput | control::HerePhase::InputRestored
                )
            })
    {
        if operation.placement.correction().is_some() {
            host.restrict_automatic_correction();
        }
        let cleanup = host
            .owned()
            .and_then(|owned| if owned { host.fence() } else { Ok(()) });
        if let Err(error) = cleanup {
            eprintln!("[HERE] Input safety cleanup: {error}");
            if outcome.is_ok() {
                outcome = Err(error);
            }
        }
    }
    if let Err(original) = outcome {
        operation
            .placement
            .invalidate_here(control::Reason::X11Failure);
        if (operation.detail.progress.host_mutation_dispatched
            || (recover && operation.placement.baseline.is_some()))
            && !ownership_lost
            && host.live()
            && host.owned().unwrap_or(false)
        {
            let history = if recover_top(operation) {
                host.evidence(operation.placement.destination_desktop)
                    .map(|evidence| {
                        operation.observe_recover_top(
                            destination_evidence(&evidence),
                            evidence.observation,
                        );
                    })
            } else {
                read_here(operation, host).map(|_| ())
            };
            if let Err(error) = history {
                eprintln!("[PLACEMENT] Final history observation after native failure: {error}");
            }
        }
        operation
            .here_shutdown(control::Reason::X11Failure)
            .unwrap();
        return Err(original);
    }
    outcome
}

/// Internal owner completion seam: caller disappearance does not cancel work;
/// terminal send failure clears active exactly once and never undoes progress.
fn service_here_owner_with(
    active: &mut Option<control::ActiveOperation>,
    host: &mut impl BringHereHost,
    reply: impl FnOnce(control::OperationResult) -> Result<(), HostError>,
) -> Result<(), HostError> {
    service_placement_owner_with(active, host, reply, service_bring_here_with)
}

fn service_recover_owner_with(
    active: &mut Option<control::ActiveOperation>,
    host: &mut impl RecoverHost,
    reply: impl FnOnce(control::OperationResult) -> Result<(), HostError>,
) -> Result<(), HostError> {
    service_placement_owner_with(active, host, reply, service_recover_with)
}

/// Dispatch the borrowed public placement slot without entering legacy visibility.
fn service_admitted_placement_with(
    active: &mut Option<control::ActiveOperation>,
    host: &mut impl RecoverHost,
    reply: impl FnMut(control::OperationResult) -> Result<(), HostError>,
) -> Result<(), HostError> {
    if active
        .as_ref()
        .is_some_and(|op| op.request.command == control::Command::Recover)
    {
        service_recover_owner_with(active, host, reply)
    } else {
        service_here_owner_with(active, host, reply)
    }
}

fn service_placement_owner_with<H: BringHereHost>(
    active: &mut Option<control::ActiveOperation>,
    host: &mut H,
    reply: impl FnOnce(control::OperationResult) -> Result<(), HostError>,
    service: impl FnOnce(
        &mut control::ActiveOperation,
        &mut H,
    ) -> Result<Option<control::OperationResult>, HostError>,
) -> Result<(), HostError> {
    let Some(operation) = active.as_mut() else {
        return Ok(());
    };
    if !remote_service_due(operation, host.now()) {
        return Ok(());
    }
    let outcome = service(operation, host);
    complete_here_owner_with(active, outcome, reply)
}

/// External fatal teardown bypasses scheduling, never the D3 finalizer. Leave
/// legacy commands and an already-cleared slot untouched, including the error.
fn fail_here_owner_with(
    active: &mut Option<control::ActiveOperation>,
    host: &mut impl BringHereHost,
    reply: impl FnOnce(control::OperationResult) -> Result<(), HostError>,
    original: HostError,
) -> Result<(), HostError> {
    let Some(operation) = active.as_mut().filter(|operation| {
        matches!(
            operation.request.command,
            control::Command::BringHere | control::Command::Recover
        )
    }) else {
        return Err(original);
    };
    if operation.request.command == control::Command::Recover
        && original.is::<x11::control::OwnershipLost>()
    {
        host.abort_recovery_layer();
        let result = operation
            .recover_shutdown(control::Reason::OwnershipLost)
            .unwrap();
        // Proven transport loss is the same ordinary lifecycle outcome as the
        // scheduler's mismatch. Reuse terminal clearing, without native cleanup
        // or a history acquisition against the replacement selection owner.
        let _ = complete_here_owner_with(active, Ok(Some(result)), reply);
        return Err(original);
    }
    let outcome = finalize_bring_here_with(operation, host, Err(original));
    complete_here_owner_with(active, outcome, reply)
}

/// Both service and external failure paths attempt one reply, clear the same
/// slot, and preserve the original native error over a terminal-send failure.
fn complete_here_owner_with(
    active: &mut Option<control::ActiveOperation>,
    outcome: Result<Option<control::OperationResult>, HostError>,
    reply: impl FnOnce(control::OperationResult) -> Result<(), HostError>,
) -> Result<(), HostError> {
    let operation = active.as_mut().expect("Here owner checked the active slot");
    let (result, primary) = match outcome {
        Ok(None) => return Ok(()),
        Ok(Some(result)) => {
            // Losing the selection ends the owner run after its one reply;
            // ordinary local services must not resume against a replacement.
            let primary: Option<HostError> =
                (result.detail().reason == control::Reason::OwnershipLost).then(|| {
                    if operation.request.command == control::Command::Recover {
                        "Control selection ownership lost during Recover".into()
                    } else {
                        "Control selection ownership lost during Bring Here".into()
                    }
                });
            (result, primary)
        }
        Err(error) => (fatal_placement_result(operation), Some(error)),
    };
    *active = None;
    let sent = reply(result);
    match primary {
        Some(error) => {
            if let Err(reply) = sent {
                eprintln!("[HERE] Terminal reply after failure: {reply}");
            }
            Err(error)
        }
        None => sent,
    }
}

/// Fatal native failure has priority over an earlier ordinary invalidation.
/// Preserve the pure record's failure latch; adapt only the terminal diagnostic.
fn fatal_placement_result(operation: &mut control::ActiveOperation) -> control::OperationResult {
    let shutdown = operation
        .here_shutdown(control::Reason::X11Failure)
        .unwrap();
    if operation.request.command != control::Command::Recover {
        return shutdown;
    }
    let mut detail = shutdown.detail();
    detail.reason = control::Reason::X11Failure;
    operation.detail = detail;
    control::OperationResult::placement(
        control::Command::Recover,
        shutdown.status(),
        detail,
        control::Confirmation::default(),
        &operation.placement,
    )
    .expect("Fatal Recover retains history with no current proof")
}

/// Borrows the existing runtime/resources; owns no request or controller.
struct NativeHere<'a, 'b, C> {
    visibility: &'a mut NativeVisibility<'b, C>,
    selection: u32,
    control_xid: u32,
}

impl<C: Connection> BringHereHost for NativeHere<'_, '_, C> {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn owned(&mut self) -> Result<bool, HostError> {
        use x11rb::protocol::xproto::ConnectionExt;
        Ok(self
            .visibility
            .conn
            .get_selection_owner(self.selection)?
            .reply()?
            .owner
            == self.control_xid)
    }
    fn live(&self) -> bool {
        self.visibility.runtime.body != BodyAvailability::Destroyed
    }
    fn evidence(&mut self, destination: u32) -> Result<HereEvidence, HostError> {
        self.visibility.here_observe(destination)
    }
    fn placement_busy(&self) -> bool {
        self.visibility.runtime.correction.is_some()
    }
    fn input_active(&self) -> bool {
        !self.visibility.runtime.input_fence.blocked
    }
    fn visible_ready(&self) -> bool {
        here_runtime_ready(self.visibility.runtime, self.visibility.window)
    }
    fn fence_settled(&self) -> bool {
        self.visibility.runtime.visibility_dispatch_eligible()
    }
    fn fence(&mut self) -> Result<(), HostError> {
        self.visibility.fence()
    }
    fn abort_recovery_layer(&mut self) {
        self.visibility.abort_above();
    }
    fn desktop(
        &mut self,
        destination: u32,
        deadline: Instant,
        dispatched: &mut bool,
    ) -> Result<bool, HostError> {
        let h = &mut self.visibility;
        request_here_desktop(
            h.conn,
            h.runtime,
            h.root,
            h.window.window,
            h.atoms.desktop,
            destination,
            deadline,
            dispatched,
        )
    }
    fn geometry(&mut self) -> Result<geometry::Rect, HostError> {
        self.visibility
            .window
            .query_actual_root_geometry(self.visibility.conn, self.visibility.root)
    }
    fn restrict_automatic_correction(&mut self) {
        self.visibility
            .runtime
            .input_fence
            .automatic_correction_blocked = true;
    }
    fn correct(
        &mut self,
        target: Point,
        deadline: Instant,
        dispatched: &mut bool,
    ) -> Result<Option<Instant>, HostError> {
        self.visibility.here_correct(target, deadline, dispatched)
    }
    fn enable(
        &mut self,
        evidence: &HereEvidence,
        actual: geometry::Rect,
        deadline: Instant,
    ) -> Result<bool, HostError> {
        let layout = evidence
            .layout
            .as_ref()
            .expect("Fresh accepted Here layout gates enable");
        let h = &mut self.visibility;
        *h.bounds = layout.valid_origins;
        *h.area = layout.usable_area;
        let input = InputPlacement {
            selected_monitor: h.selected_monitor,
            observed_monitor: layout.monitor_name,
            geometry: actual,
            bounds: layout.valid_origins,
        };
        h.enable(&evidence.snapshot, &input, Some(deadline))
    }
}

impl<'b, C: Connection> RecoverHost for NativeHere<'_, 'b, C> {
    type Top = NativeVisibility<'b, C>;
    fn top(&mut self) -> &mut Self::Top {
        self.visibility.recover_owner = Some((self.selection, self.control_xid));
        self.visibility
    }
}

fn here_runtime_ready(runtime: &ProbeRuntime, window: &ManagedProbeWindow) -> bool {
    runtime.body == BodyAvailability::Ready
        && !runtime.input_fence.blocked
        && runtime.input_fence.enabled_after.is_some()
        && runtime.pending_map_input.is_none()
        && !runtime.pointer.is_grabbed()
        && runtime.menu.is_none()
        && matches!(
            runtime.interaction.state(),
            InteractionState::Idle | InteractionState::SuppressedUntilRelease { .. }
        )
        && runtime.pending_move.is_none()
        && runtime.final_target.is_none()
        && runtime.confirmation_deadline.is_none()
        && runtime.correction.is_none()
        && runtime.safety.deadline.is_none()
        && runtime.safety.observation.is_none()
        && window.in_flight_moves.is_empty()
}

fn here_observation(snapshot: &x11::visibility::VisibilitySnapshot) -> control::HereObservation {
    use control::{Capability, Evidence};
    let membership = snapshot.observation.as_ref().and_then(|observation| {
        if snapshot.evidence.destroyed
            || !snapshot.evidence.coherent
            || snapshot.evidence.capability != Capability::Supported
        {
            return None;
        }
        match (observation.desktop, observation.current_desktop) {
            (Evidence::Readable(body), Evidence::Readable(current))
                if body != u32::MAX && current != u32::MAX =>
            {
                Some((body, current))
            }
            _ => None,
        }
    });
    // Membership is independent of visibility readiness and layout validity.
    // D1 checks range/current against the strict layout before baseline/readiness;
    // independently trustworthy membership can retain history after layout loss.
    control::HereObservation {
        membership,
        visibility: snapshot.candidate.visibility,
        layer: snapshot.layer,
    }
}

fn here_evidence_with(
    mut layout: impl FnMut() -> Result<
        Result<control::DestinationIdentity, x11::monitors::destination::DestinationError>,
        HostError,
    >,
    observe: impl FnOnce() -> Result<x11::visibility::VisibilitySnapshot, HostError>,
) -> Result<HereEvidence, HostError> {
    use x11::monitors::destination::DestinationError;
    let before = layout()?;
    let snapshot = observe()?;
    let after = layout()?;
    let layout = match (before, after) {
        (Ok(before), Ok(after)) if before == after => Ok(after),
        (Ok(_), Ok(_)) => Err(DestinationError::Changed),
        (Err(error), _) | (_, Err(error)) => Err(error),
    };
    Ok(HereEvidence {
        layout,
        observation: here_observation(&snapshot),
        snapshot,
    })
}

impl<C: Connection> NativeVisibility<'_, C> {
    fn here_observe(&mut self, frozen_destination: u32) -> Result<HereEvidence, HostError> {
        use x11::monitors::destination::query_destination_layout_preserving_native;
        let (conn, root, atoms, selected) = (
            self.conn,
            self.root,
            self.layout_atoms,
            self.selected_monitor,
        );
        here_evidence_with(
            || {
                let layout = query_destination_layout_preserving_native(
                    conn,
                    root,
                    atoms,
                    selected,
                    frozen_destination,
                    Size::new(WINDOW_WIDTH.into(), WINDOW_HEIGHT.into()),
                )?;
                Ok(layout.and_then(here_destination_identity))
            },
            || self.observe(),
        )
    }

    fn here_correct(
        &mut self,
        target: Point,
        deadline: Instant,
        dispatched: &mut bool,
    ) -> Result<Option<Instant>, HostError> {
        here_correct_with(
            self.runtime,
            target,
            deadline,
            dispatched,
            |target, deadline, mark| {
                self.window
                    .correct_position(self.conn, target, deadline, mark)
            },
        )
    }
}

fn here_destination_identity(
    layout: x11::monitors::destination::DestinationLayout,
) -> Result<control::DestinationIdentity, x11::monitors::destination::DestinationError> {
    // Stage B proves capacity; signed native coordinate limits remain evidence
    // validation, never an outer native acquisition failure.
    checked_probe_bounds(layout.usable_area).map_err(|error| {
        x11::monitors::destination::DestinationError::Unverifiable(error.to_string())
    })?;
    Ok((&layout).into())
}

fn here_correct_with(
    runtime: &mut ProbeRuntime,
    target: Point,
    deadline: Instant,
    dispatched: &mut bool,
    correct: impl FnOnce(Point, Instant, &mut bool) -> Result<Option<Instant>, HostError>,
) -> Result<Option<Instant>, HostError> {
    if !runtime.visibility_dispatch_eligible() {
        return Err("Bring Here input fence is not settled".into());
    }
    runtime.body = BodyAvailability::Validating;
    correct(target, deadline, dispatched)
}

/// Explicit, already-admitted envelope inputs. The consuming Stage-C primitive
/// alone handles expiry/possible dispatch/checking, with no WM confirmation.
#[allow(clippy::too_many_arguments)]
fn request_here_desktop(
    conn: &impl Connection,
    runtime: &ProbeRuntime,
    root: u32,
    body: u32,
    desktop_atom: u32,
    frozen_destination: u32,
    deadline: Instant,
    dispatched: &mut bool,
) -> Result<bool, HostError> {
    request_here_desktop_with(
        runtime,
        root,
        body,
        desktop_atom,
        frozen_destination,
        deadline,
        dispatched,
        |root, body, atom, target, deadline, dispatched| {
            x11::desktop::DesktopRequest::new(root, body, atom, target)?
                .send_before(conn, deadline, dispatched)
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn request_here_desktop_with(
    runtime: &ProbeRuntime,
    root: u32,
    body: u32,
    desktop_atom: u32,
    frozen_destination: u32,
    deadline: Instant,
    dispatched: &mut bool,
    send: impl FnOnce(u32, u32, u32, u32, Instant, &mut bool) -> Result<bool, HostError>,
) -> Result<bool, HostError> {
    if !runtime.visibility_dispatch_eligible() {
        return Err("Bring Here input fence is not settled".into());
    }
    if frozen_destination == u32::MAX {
        return Err("All-desktops is not an ordinary relocation destination".into());
    }
    send(
        root,
        body,
        desktop_atom,
        frozen_destination,
        deadline,
        dispatched,
    )
}

#[cfg(test)]
#[path = "probe_tests/here_adapter_tests.rs"]
mod here_adapter_tests;

impl<C: Connection> VisibilityHost for NativeVisibility<'_, C> {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn layer_atoms(&self) -> LayerAtoms {
        LayerAtoms {
            supported: self.atoms.supported,
            state: self.atoms.state,
            above: self.atoms.above,
            below: self.atoms.below,
        }
    }
    fn observe(&mut self) -> Result<x11::visibility::VisibilitySnapshot, HostError> {
        let snapshot = self
            .atoms
            .observe(self.conn, self.root, self.window.window)?;
        if snapshot.evidence.destroyed {
            self.window.window_resource.externally_destroyed();
            self.runtime.unavailable(true);
        }
        Ok(snapshot)
    }
    fn noop_ready(&self) -> bool {
        self.runtime.body == BodyAvailability::Ready
            && !self.runtime.input_fence.blocked
            && self.runtime.correction.is_none()
    }
    fn placement_busy(&self) -> bool {
        self.runtime.correction.is_some()
    }
    fn fenced(&self) -> bool {
        self.runtime.input_fence.blocked
    }
    fn fence_settled(&self) -> bool {
        self.runtime.visibility_dispatch_eligible()
    }
    fn deferred_placement(&mut self, placement: &ShowPlacement) -> Result<bool, HostError> {
        if self.runtime.correction.is_some() {
            return Ok(false);
        }
        self.runtime.body = BodyAvailability::Validating;
        self.runtime.placement_with(
            self.window,
            placement.input.geometry,
            &placement.input.bounds,
            |window, target| window.configure_position(self.conn, target),
        )?;
        Ok(self.runtime.body == BodyAvailability::Ready)
    }
    fn fence(&mut self) -> Result<(), HostError> {
        self.runtime.fence_visibility_input(self.conn, self.window)
    }
    fn iconify(&mut self, deadline: Instant, dispatched: &mut bool) -> Result<bool, HostError> {
        if !self.runtime.visibility_dispatch_eligible() {
            return Err("Hide input fence is not settled".into());
        }
        self.atoms.request_iconify_before(
            self.conn,
            self.root,
            self.window.window,
            deadline,
            dispatched,
        )
    }
    fn restore(&mut self, deadline: Instant, dispatched: &mut bool) -> Result<bool, HostError> {
        if !self.runtime.visibility_dispatch_eligible() {
            return Err("Show input fence is not settled".into());
        }
        self.atoms
            .request_restore_before(self.conn, self.window.window, deadline, dispatched)
    }
    fn hidden(
        &mut self,
        hidden: bool,
        deadline: Instant,
        dispatched: &mut bool,
    ) -> Result<bool, HostError> {
        if !self.runtime.visibility_dispatch_eligible() {
            return Err("Off-workspace visibility input fence is not settled".into());
        }
        self.atoms.request_hidden_before(
            self.conn,
            self.root,
            self.window.window,
            hidden,
            deadline,
            dispatched,
        )
    }
    fn placement(&mut self) -> Result<ShowPlacement, HostError> {
        let layout = x11::monitors::query_control_layout(
            self.conn,
            self.root,
            self.layout_atoms,
            self.selected_monitor,
        )?;
        let bounds = checked_probe_bounds(layout.usable_area)?;
        let geometry = self
            .window
            .query_actual_root_geometry(self.conn, self.root)?;
        *self.bounds = bounds;
        *self.area = layout.usable_area;
        Ok(ShowPlacement {
            area: layout.usable_area,
            current_desktop: layout.current_desktop,
            input: InputPlacement {
                selected_monitor: self.selected_monitor,
                observed_monitor: layout
                    .primary_monitor
                    .as_ref()
                    .ok_or("Retained monitor unavailable")?
                    .name_atom,
                geometry,
                bounds,
            },
        })
    }
    fn correct(
        &mut self,
        target: Point,
        deadline: Instant,
        dispatched: &mut bool,
    ) -> Result<Option<Instant>, HostError> {
        self.runtime.body = BodyAvailability::Validating;
        self.window
            .correct_show_position(self.conn, target, deadline, dispatched)
    }
    fn enable(
        &mut self,
        snapshot: &x11::visibility::VisibilitySnapshot,
        placement: &InputPlacement,
        deadline: Option<Instant>,
    ) -> Result<bool, HostError> {
        if deadline.is_some_and(|dl| Instant::now() >= dl) {
            return Ok(false);
        }
        self.window.confirmed_origin = placement.geometry.origin();
        self.window.cancel_in_flight_moves();
        self.runtime.body = BodyAvailability::Ready;
        self.runtime
            .restore_visibility_input(self.conn, self.window, snapshot, placement)?;
        Ok(true)
    }
    fn recover_owned(&mut self) -> Result<bool, HostError> {
        use x11rb::protocol::xproto::ConnectionExt;
        let (selection, control_xid) = self
            .recover_owner
            .ok_or("Recover Top requires its admitted owner identity")?;
        Ok(self.conn.get_selection_owner(selection)?.reply()?.owner == control_xid)
    }
    fn recover_evidence(&mut self, destination: u32) -> Result<HereEvidence, HostError> {
        self.here_observe(destination)
    }
    fn recover_geometry(
        &mut self,
        layout: control::DestinationIdentity,
    ) -> Result<geometry::Rect, HostError> {
        // Match ordinary placement's borrowed resource updates, using only the
        // already admitted strict identity rather than a new legacy layout.
        *self.bounds = layout.valid_origins;
        *self.area = layout.usable_area;
        self.window.query_actual_root_geometry(self.conn, self.root)
    }
    fn recover_input_ready(&self) -> bool {
        here_runtime_ready(self.runtime, self.window)
    }
    fn restrict_recover_correction(&mut self) {
        self.runtime.input_fence.automatic_correction_blocked = true;
    }
    fn restore_recover(
        &mut self,
        operation: &mut control::ActiveOperation,
        deadline: Instant,
        dispatched: &mut bool,
    ) -> Result<bool, HostError> {
        if !self.runtime.visibility_dispatch_eligible() {
            return Err("Recover restoration requires a settled input fence".into());
        }
        let (atoms, conn, body) = (self.atoms, self.conn, self.window.window);
        atoms.request_restore_guarded_before(conn, body, deadline, dispatched, || {
            Ok(recover_visibility_guard(operation, self)?
                && operation.detail.visibility == control::Visibility::Minimized)
        })
    }
}

fn recover_layer_dispatch_eligible(
    operation: &mut control::ActiveOperation,
    host: &mut impl VisibilityHost,
) -> Result<bool, HostError> {
    if recover_readiness(operation, host, true)?.is_none() || !host.recover_input_ready() {
        let reason = operation
            .placement
            .invalidation_reason()
            .unwrap_or(control::Reason::Unverifiable);
        reject_recover(operation, reason);
        return Ok(false);
    }
    if !operation.recover_layer_mutation_eligible(host.now()) {
        reject_recover(operation, control::Reason::ConfirmationTimeout);
        return Ok(false);
    }
    Ok(true)
}

impl<C: Connection> BringTopHost for NativeVisibility<'_, C> {
    fn qualify_layer(
        &self,
        snapshot: &x11::visibility::VisibilitySnapshot,
        baseline: control::VisibilityBaseline,
    ) -> Result<ObservedLayer, RemoteLayerIssue> {
        self.layer_atoms().qualify_remote(snapshot, baseline)
    }
    fn service_above(
        &mut self,
        operation: &mut control::ActiveOperation,
    ) -> Result<(LayerOutcome, Option<RemoteLayerIssue>), HostError> {
        let atoms = self.layer_atoms();
        let baseline = operation
            .show
            .baseline
            .expect("visibility readiness establishes baseline");
        let mut layers = std::mem::take(&mut self.runtime.layers);
        let host = std::cell::RefCell::new(&mut *self);
        let result = service_remote_layer_with(
            &mut layers,
            operation,
            |operation| {
                let mut h = host.borrow_mut();
                if recover_top(operation) {
                    let (evidence, decision) = read_recover_top(operation, &mut **h)?;
                    return Ok(match decision {
                        control::HereDecision::Destination => {
                            atoms.qualify_remote(&evidence.snapshot, baseline)
                        }
                        control::HereDecision::Pending => Err(RemoteLayerIssue::Waiting),
                        _ => Err(RemoteLayerIssue::WorkspaceChanged),
                    });
                }
                atoms.read_remote(h.atoms, h.conn, h.root, h.window.window, baseline)
            },
            || {
                let h = host.borrow();
                atoms.read_support(h.conn, h.root)
            },
            |mutation, operation, dispatched| {
                let mut h = host.borrow_mut();
                if recover_top(operation) && !recover_layer_dispatch_eligible(operation, &mut **h)?
                {
                    return Ok(false);
                }
                atoms.send_marked(h.conn, h.root, h.window.window, mutation, dispatched)?;
                Ok(true)
            },
            Instant::now,
        );
        self.runtime.layers = layers;
        result
    }
    fn abort_above(&mut self) {
        abort_layer(&mut self.runtime.layers, "remote operation ended");
    }
    fn layer_deadline(&self) -> Option<Instant> {
        self.runtime.layers.deadline()
    }
}

fn visibility_event(
    event: &Event,
    root: u32,
    body: u32,
    visibility: &x11::visibility::VisibilityAtoms,
    layout: &x11::monitors::LayoutAtoms,
) -> bool {
    relevant_layout_event(event, root, body, layout)
        || matches!(event, Event::MapNotify(e) if e.window == body)
        || matches!(event, Event::UnmapNotify(e) if e.window == body)
        || matches!(event, Event::DestroyNotify(e) if e.window == body)
        || matches!(event, Event::ConfigureNotify(e) if e.window == body)
        || matches!(event, Event::PropertyNotify(e) if
            (e.window == body && [visibility.state, visibility.wm_state, visibility.desktop].contains(&e.atom))
            || (e.window == root && [visibility.current_desktop, visibility.supported].contains(&e.atom)))
}

fn visibility_layout_interruption(
    event: &Event,
    root: u32,
    layout: &x11::monitors::LayoutAtoms,
) -> bool {
    matches!(
        event,
        Event::RandrNotify(_) | Event::RandrScreenChangeNotify(_)
    ) || matches!(event, Event::ConfigureNotify(e) if e.window == root)
        || matches!(event, Event::PropertyNotify(e) if e.window == root && e.atom == layout.workarea)
}

fn here_evidence_event(
    event: &Event,
    root: u32,
    body: u32,
    visibility: &x11::visibility::VisibilityAtoms,
    layout: &x11::monitors::LayoutAtoms,
) -> bool {
    visibility_event(event, root, body, visibility, layout)
        || matches!(event, Event::PropertyNotify(e) if e.window == root
            && [layout.number_of_desktops, layout.desktop_viewport].contains(&e.atom))
}

fn remote_owns_placement(operation: &control::ActiveOperation) -> bool {
    operation.request.command == control::Command::BringHere
        || (operation.request.command == control::Command::Recover && !remote_owns_layer(operation))
}

/// Consume before every legacy map/layout/geometry consumer. Notification
/// payloads never establish proof, including expected relocation UnmapNotify.
fn route_here_evidence_event(
    operation: Option<&mut control::ActiveOperation>,
    event: &Event,
    root: u32,
    body: u32,
    visibility: &x11::visibility::VisibilityAtoms,
    layout: &x11::monitors::LayoutAtoms,
) -> bool {
    if let Some(operation) = operation.filter(|op| remote_owns_placement(op) || recover_top(op)) {
        if here_evidence_event(event, root, body, visibility, layout) {
            mark_remote_observation(operation, false);
            return remote_consumes_visibility_event(operation, event, body);
        }
    }
    false
}

fn remote_deadline(owner: &ControlWindow) -> Option<Instant> {
    owner.active().map(remote_operation_deadline)
}

fn remote_operation_deadline(operation: &control::ActiveOperation) -> Instant {
    if remote_owns_placement(operation) && !recover_top(operation) {
        let cutoff = here_cutoff(operation);
        return operation
            .placement
            .correction()
            .map_or(cutoff, |(_, dl)| dl.min(cutoff));
    }
    if remote_owns_layer(operation) {
        return operation
            .bring_top
            .layer_admitted
            .map_or(operation.deadline, |admitted| {
                (admitted + layer::CONFIRMATION_TIMEOUT).min(operation.deadline)
            });
    }
    let cutoff = visibility_cutoff(operation);
    if operation.request.command == control::Command::Hide {
        return cutoff;
    }
    operation
        .show
        .correction
        .map_or(cutoff, |(_, dl)| dl.min(cutoff))
}

fn remote_owns_layer(operation: &control::ActiveOperation) -> bool {
    (operation.request.command == control::Command::BringTop || recover_top(operation))
        && operation.stage == control::Stage::Layer
}

/// G Layer owns evidence notifications, while body ConfigureNotify remains on
/// the ordinary movement reconciliation path.
fn remote_consumes_visibility_event(
    operation: &control::ActiveOperation,
    event: &Event,
    body: u32,
) -> bool {
    !remote_owns_layer(operation) || !matches!(event, Event::ConfigureNotify(e) if e.window == body)
}

fn local_layer_service_allowed(operation: Option<&control::ActiveOperation>) -> bool {
    !operation.is_some_and(|op| remote_owns_layer(op) || remote_owns_placement(op))
}

fn remote_service_due(operation: &control::ActiveOperation, now: Instant) -> bool {
    let observe = match operation.request.command {
        control::Command::Show => operation.show.observe,
        control::Command::Hide => operation.hide.observe,
        control::Command::BringTop if remote_owns_layer(operation) => {
            operation.bring_top.layer_observe
        }
        control::Command::BringTop => operation.show.observe,
        control::Command::BringHere => operation.placement.observation_due(),
        control::Command::Recover if !recover_top(operation) => {
            operation.placement.observation_due()
        }
        control::Command::Recover if remote_owns_layer(operation) => {
            operation.bring_top.layer_observe
        }
        control::Command::Recover => operation.show.observe,
    };
    observe || now >= remote_operation_deadline(operation)
}

fn mark_remote_observation(operation: &mut control::ActiveOperation, layout_interrupted: bool) {
    match operation.request.command {
        control::Command::Show | control::Command::BringTop => {
            operation.show.observe = true;
            operation.show.layout_interrupted |= layout_interrupted;
            if remote_owns_layer(operation) {
                operation.bring_top.layer_observe = true;
            }
        }
        control::Command::Hide => {
            operation.hide.observe = true;
            operation.hide.layout_interrupted |= layout_interrupted;
        }
        control::Command::BringHere => operation.placement.request_here_observation(),
        control::Command::Recover => {
            // Notifications invalidate readiness, never the immutable identity,
            // history, correction allowance or phase deadlines. Fresh strict
            // reads distinguish equal layout evidence from confirmed drift.
            operation.placement.request_here_observation();
            let dispatched = operation.detail.progress.host_mutation_dispatched;
            operation.detail = control::preflight_detail(control::Reason::None, operation.stage);
            operation.detail.progress.host_mutation_dispatched = dispatched;
            operation.detail.placement = operation.placement.proof;
            if recover_top(operation) {
                operation.show.observe = true;
                if remote_owns_layer(operation) {
                    operation.bring_top.layer_observe = true;
                }
            }
        }
    }
}

/// Both routing sites preserve event order and give consumed requests a visibility
/// progression boundary before the next event or socket wait.
fn route_owner_request_with<O, H>(
    owner: &mut O,
    host: &mut H,
    route: impl FnOnce(&mut O, &mut H) -> Result<bool, HostError>,
    service: impl FnOnce(&mut O, &mut H) -> Result<(), HostError>,
) -> Result<bool, HostError> {
    let consumed = route(owner, host)?;
    if consumed {
        service(owner, host)?;
    }
    Ok(consumed)
}

fn route_owner_request<C: Connection>(
    owner: &mut ControlWindow,
    event: &Event,
    host: &mut NativeVisibility<'_, C>,
) -> Result<bool, HostError> {
    route_owner_request_with(
        owner,
        host,
        |owner, host| {
            owner.route_request(
                host.conn,
                event,
                host.runtime.menu.as_ref().map(|popup| popup.window),
                host.runtime.layers.pending().is_some(),
            )
        },
        service_owner_operation,
    )
}

/// A failed enabling check may still have changed native SHAPE. Re-fence once
/// for safety and settle possible automatic capture; never retry enabling/map.
fn service_show_safely_with(
    operation: &mut control::ActiveOperation,
    host: &mut impl VisibilityHost,
) -> Result<Option<control::OperationResult>, HostError> {
    let outcome = service_show_with(operation, host);
    let failed = match &outcome {
        Err(_) => true,
        Ok(Some(result)) => result.status() != control::Status::Success,
        Ok(None) => false,
    };
    if failed
        && matches!(
            operation.show.phase,
            control::ShowPhase::EnablingInput | control::ShowPhase::InputRestored
        )
    {
        let cleanup = host.fence();
        if let Err(error) = &cleanup {
            eprintln!("[SHOW] Input safety cleanup: {error}");
        }
        match (outcome, cleanup) {
            (Err(original), _) => Err(original),
            (Ok(_), Err(cleanup)) => Err(cleanup),
            (Ok(outcome), Ok(())) => Ok(outcome),
        }
    } else {
        outcome
    }
}

fn service_owner_operation<C: Connection>(
    owner: &mut ControlWindow,
    host: &mut NativeVisibility<'_, C>,
) -> Result<(), HostError> {
    let Some(operation) = owner.active_mut() else {
        return Ok(());
    };
    if !remote_service_due(operation, host.now())
        && !host
            .layer_deadline()
            .is_some_and(|dl| remote_owns_layer(operation) && host.now() >= dl)
    {
        return Ok(());
    }
    // Placement borrows the one active slot before the legacy ownership check.
    // Both phases validate ownership through the shared D4.1 failure policy.
    if owner.service_here_with(host.conn, |active, selection, control_xid, reply| {
        let mut here = NativeHere {
            visibility: host,
            selection,
            control_xid,
        };
        service_admitted_placement_with(active, &mut here, reply)
    })? {
        return Ok(());
    }
    owner.verify_active_ownership(host.conn)?; // unchanged legacy check
    let operation = owner
        .active_mut()
        .expect("active remote operation checked above");
    let outcome = service_remote_with(operation, host);
    match outcome {
        Ok(Some(result)) => owner.complete(host.conn, result),
        Ok(None) => Ok(()),
        Err(error) => {
            let reason = if operation.detail.reason == control::Reason::ReleaseFailed
                && (!host.runtime.input_fence.empty_confirmed || !host.runtime.pointer.is_grabbed())
            {
                control::Reason::X11Failure
            } else {
                operation.detail.reason
            };
            let result = match operation.request.command {
                control::Command::BringHere | control::Command::Recover => {
                    unreachable!("Placement uses the shared owner finalizer")
                }
                control::Command::Hide => hide_result(operation, control::Status::Failed, reason),
                control::Command::Show => {
                    show_result(operation, control::Status::Failed, reason, None)
                }
                control::Command::BringTop => {
                    host.abort_above();
                    match finish_bring_top_with(operation, host, reason, false) {
                        Ok(result) => result,
                        Err(_) => {
                            operation.detail.visibility = control::Visibility::Unknown;
                            operation.detail.layer = control::LayerObservation::Unknown;
                            operation.detail.workspace = control::Workspace::Unknown;
                            compose_bring_top_result(
                                operation,
                                reason,
                                control::Confirmation::default(),
                                false,
                                false,
                            )
                        }
                    }
                }
            };
            if let Err(reply) = owner.complete(host.conn, result) {
                eprintln!("[CONTROL] Terminal reply after failure: {reply}");
            }
            Err(error) // original owner-local failure has priority
        }
    }
}

/// Finalize still-active placement before global cleanup can classify it as
/// Closing. Legacy commands and cleared slots pass the exact error through.
fn fail_owner_operation<C: Connection>(
    owner: &mut ControlWindow,
    host: &mut NativeVisibility<'_, C>,
    original: HostError,
) -> HostError {
    let mut original = Some(original);
    match owner.service_here_with(host.conn, |active, selection, control_xid, reply| {
        let mut here = NativeHere {
            visibility: host,
            selection,
            control_xid,
        };
        fail_here_owner_with(
            active,
            &mut here,
            reply,
            original.take().expect("one Here fatal dispatch"),
        )
    }) {
        Err(original) => original,
        Ok(false) => original.expect("non-Here fatal error was not consumed"),
        Ok(true) => unreachable!("fatal Here finalization returns the original error"),
    }
}

/// Independent of any completed visibility command: later local lifecycle evidence validates
/// placement before restoring fenced input. No remote result is resurrected.
fn restore_deferred_input(host: &mut impl VisibilityHost) -> Result<(), HostError> {
    if !host.fence_settled() {
        return Ok(());
    }
    let snapshot = host.observe()?;
    if snapshot.candidate.visibility != control::Visibility::NotMinimizedHere {
        return Ok(());
    }
    let placement = host.placement()?;
    if snapshot
        .observation
        .as_ref()
        .is_none_or(|o| o.current_desktop != control::Evidence::Readable(placement.current_desktop))
    {
        return Ok(());
    }
    if !host.deferred_placement(&placement)? {
        return Ok(());
    }
    let fresh = host.observe()?;
    let verified = host.placement()?;
    if fresh.candidate.visibility == control::Visibility::NotMinimizedHere
        && fresh.observation == snapshot.observation
        && verified.area == placement.area
        && verified.current_desktop == placement.current_desktop
        && verified.input.geometry == placement.input.geometry
    {
        if let Err(original) = host.enable(&fresh, &verified.input, None) {
            // A failed SHAPE check may have enabled native input. Re-fence and
            // release potential automatic capture, preserving the primary error.
            if let Err(cleanup) = host.fence() {
                eprintln!("[INPUT] Deferred restoration safety cleanup: {cleanup}");
            }
            return Err(original);
        }
    }
    Ok(())
}

fn local_layer_after_menu_with(
    remote_busy: bool,
    target: Layer,
    dispatch: impl FnOnce() -> Result<(), HostError>,
) -> Result<(), HostError> {
    if remote_busy {
        eprintln!("[LAYER] Request {target:?} rejected: remote operation is Busy");
        Ok(())
    } else {
        dispatch()
    }
}

fn wm_delete_event(
    event: &x11rb::protocol::xproto::ClientMessageEvent,
    window: &ManagedProbeWindow,
) -> bool {
    event.window == window.window
        && event.type_ == window.wm_protocols
        && event.format == 32
        && event.data.as_data32()[0] == window.wm_delete_window
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
    owner: &mut ControlWindow,
    initial_bounds: crate::geometry::ValidOriginBounds,
    initial_area: crate::geometry::Rect,
) -> Result<(), HostError> {
    let root = screen.root;
    let layer_atoms = LayerAtoms::intern(conn)?;
    let visibility_atoms = x11::visibility::VisibilityAtoms::intern(conn)?;
    let mut usable_area = initial_area;
    let mut bounds = initial_bounds;
    let mut buffered_event = None;
    let duration_deadline = probe_duration_deadline(Instant::now(), duration_secs)?;
    // Every native failure, including all Event::Error sites and poll/read/
    // dispatch failures, leaves through this boundary before global cleanup.
    // Intentional lifecycle exits use the existing ProbeTermination separately.
    let mut outcome = (|| loop {
        let now = Instant::now();
        if duration_deadline.is_some_and(|dl| now >= dl) {
            // Ordered owner cleanup publishes Closing before layer/input teardown.
            return Ok(Some(ProbeTermination::DurationExpired));
        }
        if let Some((Event::Error(error), _)) = &buffered_event {
            return Err(format!("Asynchronous X11 error: {error:?}").into());
        }
        service_owner_operation(
            owner,
            &mut NativeVisibility {
                recover_owner: None,
                conn,
                root,
                atoms: &visibility_atoms,
                layout_atoms: atoms,
                selected_monitor,
                window,
                runtime,
                bounds: &mut bounds,
                area: &mut usable_area,
            },
        )?;
        if runtime.body == BodyAvailability::Destroyed {
            return Ok(None);
        }
        if local_layer_service_allowed(owner.active())
            && runtime
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
        // Here owns geometry and cancellation until completion. Its correction
        // lives under ActiveOperation, never the ordinary movement controller.
        if !owner.active().is_some_and(remote_owns_placement) {
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
        }

        if owner.active().is_none()
            && runtime.input_fence.blocked
            && runtime.body == BodyAvailability::Ready
        {
            restore_deferred_input(&mut NativeVisibility {
                recover_owner: None,
                conn,
                root,
                atoms: &visibility_atoms,
                layout_atoms: atoms,
                selected_monitor,
                window,
                runtime,
                bounds: &mut bounds,
                area: &mut usable_area,
            })?;
        }
        publish_ready_if(runtime.body, || owner.publish_ready(conn))?;
        let batch = drain_events_bounded(&mut buffered_event, 64, || {
            conn.poll_for_event_with_sequence()
        })?;
        for (event, sequence) in batch {
            if owner.handle_event(conn, &event)? {
                return Ok(Some(ProbeTermination::ControlEndpointLost));
            }
            if route_owner_request(
                owner,
                &event,
                &mut NativeVisibility {
                    recover_owner: None,
                    conn,
                    root,
                    atoms: &visibility_atoms,
                    layout_atoms: atoms,
                    selected_monitor,
                    window,
                    runtime,
                    bounds: &mut bounds,
                    area: &mut usable_area,
                },
            )? {
                if runtime.body == BodyAvailability::Destroyed {
                    return Ok(None);
                }
                continue;
            }
            if let Event::Error(error) = &event {
                return Err(format!("Asynchronous X11 error: {error:?}").into());
            }
            if remote_deadline(owner).is_some_and(|deadline| Instant::now() >= deadline) {
                service_owner_operation(
                    owner,
                    &mut NativeVisibility {
                        recover_owner: None,
                        conn,
                        root,
                        atoms: &visibility_atoms,
                        layout_atoms: atoms,
                        selected_monitor,
                        window,
                        runtime,
                        bounds: &mut bounds,
                        area: &mut usable_area,
                    },
                )?;
                if runtime.body == BodyAvailability::Destroyed {
                    return Ok(None);
                }
            }
            if route_here_evidence_event(
                owner.active_mut(),
                &event,
                root,
                window.window,
                &visibility_atoms,
                atoms,
            ) {
                if matches!(&event, Event::DestroyNotify(e) if e.window == window.window) {
                    window.window_resource.externally_destroyed();
                    runtime.unavailable(true);
                }
                if owner
                    .active()
                    .is_some_and(|op| op.request.command == control::Command::Recover)
                {
                    service_owner_operation(
                        owner,
                        &mut NativeVisibility {
                            recover_owner: None,
                            conn,
                            root,
                            atoms: &visibility_atoms,
                            layout_atoms: atoms,
                            selected_monitor,
                            window,
                            runtime,
                            bounds: &mut bounds,
                            area: &mut usable_area,
                        },
                    )?;
                    if runtime.body == BodyAvailability::Destroyed {
                        return Ok(None);
                    }
                }
                continue;
            }
            if visibility_event(&event, root, window.window, &visibility_atoms, atoms) {
                if let Some(operation) = owner.active_mut() {
                    mark_remote_observation(
                        operation,
                        visibility_layout_interruption(&event, root, atoms),
                    );
                    if remote_owns_layer(operation) {
                        if remote_consumes_visibility_event(operation, &event, window.window) {
                            service_owner_operation(
                                owner,
                                &mut NativeVisibility {
                                    recover_owner: None,
                                    conn,
                                    root,
                                    atoms: &visibility_atoms,
                                    layout_atoms: atoms,
                                    selected_monitor,
                                    window,
                                    runtime,
                                    bounds: &mut bounds,
                                    area: &mut usable_area,
                                },
                            )?;
                            if runtime.body == BodyAvailability::Destroyed {
                                return Ok(None);
                            }
                            if matches!(&event, Event::UnmapNotify(e) if e.window == window.window)
                            {
                                // A delayed unmap is evidence, not unconditional
                                // cancellation. Real unavailability still settles input.
                                if !window.is_viewable(conn)? {
                                    runtime.cancel(
                                        conn,
                                        window,
                                        "body unavailable during remote layer operation",
                                    )?;
                                    runtime.body = BodyAvailability::Unavailable;
                                    runtime.correction = None;
                                }
                                continue;
                            }
                            if !relevant_layout_event(&event, root, window.window, atoms) {
                                continue;
                            }
                            // Ordinary layout safety still runs below.
                        }
                    } else {
                        if matches!(&event, Event::UnmapNotify(e) if e.window == window.window) {
                            runtime.cancel(
                                conn,
                                window,
                                "body unavailable during visibility operation",
                            )?;
                            runtime.unavailable(false);
                        }
                        // E/F owns expected visibility and geometry notifications.
                        continue;
                    }
                }
                if owner.active().is_none() && runtime.input_fence.blocked {
                    restore_deferred_input(&mut NativeVisibility {
                        recover_owner: None,
                        conn,
                        root,
                        atoms: &visibility_atoms,
                        layout_atoms: atoms,
                        selected_monitor,
                        window,
                        runtime,
                        bounds: &mut bounds,
                        area: &mut usable_area,
                    })?;
                    if runtime.body == BodyAvailability::Destroyed {
                        return Ok(None);
                    }
                    continue;
                }
            }
            if runtime.input_fence.rejects(&event, window.window, sequence) {
                continue;
            }
            // Preserve the original fatal diagnostic even if release/cleanup also fails.
            if let Event::Error(error) = &event {
                return Err(format!("Asynchronous X11 error: {error:?}").into());
            }
            if local_layer_service_allowed(owner.active())
                && interrupts_layer(&event, root, window.window, atoms)
            {
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
                if local_layer_service_allowed(owner.active())
                    && matches!(
                        runtime.body,
                        BodyAvailability::Ready | BodyAvailability::Validating
                    )
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
                continue;
            }
            if matches!(&event, Event::PropertyNotify(ev) if ev.window == root && ev.atom == layer_atoms.supported)
            {
                if !local_layer_service_allowed(owner.active()) {
                    continue;
                }
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
                                    local_layer_after_menu_with(
                                        owner.active().is_some(),
                                        target,
                                        || {
                                            service_layer(
                                                conn,
                                                &layer_atoms,
                                                root,
                                                body,
                                                layers,
                                                Some(target),
                                            )
                                        },
                                    )
                                },
                            )? {
                                return Ok(None);
                            }
                        }
                    }
                    continue;
                }
            }
            match event {
                Event::MapNotify(ev) if ev.window == window.window => {
                    if owner.active().is_some_and(remote_owns_layer) {
                        continue;
                    }
                    runtime.cancel(conn, window, "map verification")?;
                    let layout = query_desktop_layout(conn, root, atoms, Some(selected_monitor))?;
                    usable_area = layout.usable_area;
                    bounds = checked_probe_bounds(usable_area)?;
                    if window.verified_visible(conn, layout.current_desktop, atoms.wm_desktop)? {
                        // Openbox may not propagate the pre-map input shape to its
                        // managed frame. Reassert the identical SK::INPUT silhouette
                        // once after a genuine map, before existing placement checks.
                        runtime.synchronize_map_input_with(&ev, window.window, sequence, || {
                            x11::shape::restore_body_input_shape(
                                conn,
                                window.window,
                                window.width,
                                window.height,
                            )
                        })?;
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
                    return Ok(None);
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
                Event::ClientMessage(ev) if wm_delete_event(&ev, window) => {
                    return Ok(Some(ProbeTermination::WmClose));
                }
                Event::Error(error) => {
                    return Err(format!("Asynchronous X11 error: {error:?}").into())
                }
                _ => {} // LeaveNotify never cancels a stationary hold.
            }
        }

        // Notifications in this batch can make visibility work actionable without leaving
        // another event to wake the socket. Settle that work before any wait.
        service_owner_operation(
            owner,
            &mut NativeVisibility {
                recover_owner: None,
                conn,
                root,
                atoms: &visibility_atoms,
                layout_atoms: atoms,
                selected_monitor,
                window,
                runtime,
                bounds: &mut bounds,
                area: &mut usable_area,
            },
        )?;
        if runtime.body == BodyAvailability::Destroyed {
            return Ok(None);
        }
        publish_ready_if(runtime.body, || owner.publish_ready(conn))?;
        if let Some(popup) = runtime.menu.as_mut() {
            popup.repaint_if_changed(conn, runtime.interaction.state())?;
        }
        // Look one event ahead before the batch-end flush. An interruption at the
        // batch boundary must discard the coalesced move just like one within a batch.
        if let Some((event, sequence)) = conn.poll_for_event_with_sequence()? {
            if owner.handle_event(conn, &event)? {
                return Ok(Some(ProbeTermination::ControlEndpointLost));
            }
            if route_owner_request(
                owner,
                &event,
                &mut NativeVisibility {
                    recover_owner: None,
                    conn,
                    root,
                    atoms: &visibility_atoms,
                    layout_atoms: atoms,
                    selected_monitor,
                    window,
                    runtime,
                    bounds: &mut bounds,
                    area: &mut usable_area,
                },
            )? {
                if runtime.body == BodyAvailability::Destroyed {
                    return Ok(None);
                }
                continue;
            }
            if runtime.input_fence.rejects(&event, window.window, sequence) {
                continue;
            }
            if runtime.apply_safety(Instant::now(), Some(sequence)) {
                runtime.cancel(
                    conn,
                    window,
                    "QueryPointer confirmed initiating button released",
                )?;
            }
            let interrupted = owner.is_lifecycle_event(&event)
                || interrupts_pending_movement(&event, root, window.window, atoms);
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
        let timeout = probe_deadline(runtime, duration_deadline)
            .into_iter()
            .chain(remote_deadline(owner))
            .min();
        wait_x11(conn, timeout)?;
    })();
    // Settle an internal Recover while its borrowed host still exists. A failed
    // shutdown ownership lookup/publication must enter the same fatal boundary,
    // rather than letting later global cleanup emit a misleading Closing reply.
    if outcome.is_ok()
        && owner
            .active()
            .is_some_and(|op| op.request.command == control::Command::Recover)
    {
        outcome = owner
            .closing(conn)
            .and_then(|()| owner.finish_remote(conn))
            .and(outcome);
    }
    finish_probe_run_with(outcome, |original| {
        fail_owner_operation(
            owner,
            &mut NativeVisibility {
                recover_owner: None,
                conn,
                root,
                atoms: &visibility_atoms,
                layout_atoms: atoms,
                selected_monitor,
                window,
                runtime,
                bounds: &mut bounds,
                area: &mut usable_area,
            },
            original,
        )
    })
}

/// Intentional termination retains its existing classification. Only a fatal
/// loop error invokes placement-aware teardown; every error object is moved intact.
fn finish_probe_run_with(
    outcome: Result<Option<ProbeTermination>, HostError>,
    fatal: impl FnOnce(HostError) -> HostError,
) -> Result<(), HostError> {
    match outcome {
        Ok(Some(reason)) => probe_termination_result(reason),
        Ok(None) => Ok(()),
        Err(original) => Err(fatal(original)),
    }
}

/// Exit classification shared by both control-event routing sites and normal completion.
#[derive(Clone, Copy)]
enum ProbeTermination {
    DurationExpired,
    WmClose,
    ControlEndpointLost,
}

fn probe_termination_result(reason: ProbeTermination) -> Result<(), HostError> {
    match reason {
        ProbeTermination::DurationExpired | ProbeTermination::WmClose => Ok(()),
        ProbeTermination::ControlEndpointLost => {
            println!("[LIFECYCLE] Control endpoint lost; shutting down without reacquisition");
            Err("Control endpoint lost during running probe".into())
        }
    }
}

fn probe_duration_deadline(
    start: Instant,
    seconds: Option<u64>,
) -> Result<Option<Instant>, HostError> {
    seconds
        .map(|sec| {
            start
                .checked_add(Duration::from_secs(sec))
                .ok_or_else(|| "Duration exceeds supported monotonic deadline".into())
        })
        .transpose()
}

fn probe_deadline(runtime: &ProbeRuntime, duration: Option<Instant>) -> Option<Instant> {
    [
        duration,
        runtime.confirmation_deadline,
        runtime.correction.map(|(_, dl)| dl),
        runtime.safety.deadline,
        runtime.layers.deadline(),
    ]
    .into_iter()
    .flatten()
    .min()
}

fn publish_ready_if(
    body: BodyAvailability,
    publish: impl FnOnce() -> Result<(), HostError>,
) -> Result<(), HostError> {
    if body == BodyAvailability::Ready {
        publish()?;
    }
    Ok(())
}

fn wait_x11(
    conn: &x11rb::rust_connection::RustConnection,
    deadline: Option<Instant>,
) -> Result<(), HostError> {
    conn.flush()?;
    let timeout_ms = deadline.map_or(-1, |deadline| {
        // Round up: a sub-millisecond remaining interval should not spin on poll(0).
        deadline
            .saturating_duration_since(Instant::now())
            .as_nanos()
            .div_ceil(1_000_000)
            .min(i32::MAX as u128) as i32
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
        return Err(format!("X11 socket unavailable (poll revents=0x{:x})", pfd.revents).into());
    }
    Ok(())
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
#[path = "probe_tests/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "probe_tests/menu_runtime_tests.rs"]
mod menu_runtime_tests;

#[cfg(test)]
#[path = "probe_tests/layer_runtime_tests.rs"]
mod layer_runtime_tests;

#[cfg(test)]
#[path = "probe_tests/cli_tests.rs"]
mod cli_tests;

#[cfg(test)]
#[path = "probe_tests/ownership_integration_tests.rs"]
mod ownership_integration_tests;

#[cfg(test)]
#[path = "probe_tests/show_tests.rs"]
mod show_tests;

#[cfg(test)]
mod here_service_tests {
    use super::*;
    use control::{
        ActiveOperation, Command, HerePhase, LayerObservation as L, MapState, Reason, Stage,
        Status, WmState,
    };
    use geometry::Rect;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use x11::monitors::destination::DestinationError;

    #[derive(Debug, PartialEq, Eq)]
    struct NativeFault(&'static str, usize);
    impl std::fmt::Display for NativeFault {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "native {} #{}", self.0, self.1)
        }
    }
    impl std::error::Error for NativeFault {}

    #[derive(Clone, Copy)]
    struct Reading {
        desktop: u32,
        layer: L,
        minimized: bool,
        transitional: bool,
    }
    impl Reading {
        fn snapshot(self) -> x11::visibility::VisibilitySnapshot {
            here_adapter_tests::snapshot(
                if self.minimized || self.desktop != 2 || self.transitional {
                    WmState::Iconic
                } else {
                    WmState::Normal
                },
                if self.minimized || self.desktop != 2 {
                    MapState::Unmapped
                } else {
                    MapState::Viewable
                },
                self.minimized,
                self.desktop,
                self.layer,
            )
        }
    }
    fn layout() -> control::DestinationIdentity {
        let area = Rect::new(0, 0, 800, 600);
        control::DestinationIdentity {
            desktop_count: 5,
            destination_desktop: 2,
            current_desktop: 2,
            monitor_name: 20,
            monitor_bounds: area,
            target_workarea: Some(area),
            viewport: control::DestinationViewport::AllZero,
            usable_area: area,
            body_size: Size::new(160, 160),
            valid_origins: checked_probe_bounds(area).unwrap(),
        }
    }
    struct Fake {
        now: Instant,
        runtime: ProbeRuntime,
        window: ManagedProbeWindow,
        current: Reading,
        reads: VecDeque<Reading>,
        layout: Result<control::DestinationIdentity, DestinationError>,
        layouts: VecDeque<Result<control::DestinationIdentity, DestinationError>>,
        actual: Rect,
        geometries: VecDeque<Rect>,
        log: Vec<&'static str>,
        faults: Vec<(&'static str, usize)>,
        changes: VecDeque<(&'static str, Reading)>,
        advances: VecDeque<(&'static str, Duration)>,
        owned: bool,
        refuse_correction: bool,
        send_before_cookie: bool,
        terminal: Option<control::Visibility>,
        lose_at: Option<(&'static str, usize)>,
    }
    impl Fake {
        fn new(remote: bool, minimized: bool, layer: L) -> Self {
            let mut runtime = visibility_fence_tests::runtime(0);
            runtime.input_fence.enabled_after = Some(10);
            Self {
                now: Instant::now(),
                runtime,
                window: tests::test_window(),
                current: Reading {
                    desktop: if remote { 0 } else { 2 },
                    minimized,
                    layer,
                    transitional: false,
                },
                reads: VecDeque::new(),
                layout: Ok(layout()),
                layouts: VecDeque::new(),
                actual: Rect::new(100, 100, 160, 160),
                geometries: VecDeque::new(),
                log: vec![],
                faults: vec![],
                changes: VecDeque::new(),
                advances: VecDeque::new(),
                owned: true,
                refuse_correction: false,
                send_before_cookie: false,
                terminal: None,
                lose_at: None,
            }
        }
        pub(super) fn operation(&self) -> ActiveOperation {
            let mut active = None;
            assert_eq!(
                control::admit(
                    &mut active,
                    control::Request {
                        reply_xid: 9,
                        request_id: 99,
                        epoch: 11,
                        command: Command::BringHere,
                    },
                    2,
                    control::Lifecycle::Ready,
                    false,
                    self.now
                ),
                control::Admission::Admitted
            );
            active.unwrap()
        }
        fn count(&self, step: &str) -> usize {
            self.log.iter().filter(|&&s| s == step).count()
        }
        fn step(&mut self, step: &'static str) -> Result<(), HostError> {
            self.log.push(step);
            if self.changes.front().is_some_and(|(s, _)| *s == step) {
                self.current = self.changes.pop_front().unwrap().1;
            }
            if self.advances.front().is_some_and(|(s, _)| *s == step) {
                self.now += self.advances.pop_front().unwrap().1;
            }
            let count = self.count(step);
            if self.lose_at == Some((step, count)) {
                self.owned = false;
            }
            if self.faults.contains(&(step, count)) {
                return Err(Box::new(NativeFault(step, count)));
            }
            Ok(())
        }
        fn read_layout(
            &mut self,
        ) -> Result<Result<control::DestinationIdentity, DestinationError>, HostError> {
            self.step("layout")?;
            Ok(self
                .layouts
                .pop_front()
                .unwrap_or_else(|| self.layout.clone()))
        }
        fn target(&mut self) {
            self.current.desktop = 2;
            self.current.transitional = false;
        }
        fn run(&mut self, op: &mut ActiveOperation) -> Option<control::OperationResult> {
            service_bring_here_with(op, self).unwrap()
        }
        fn arrived(&mut self, op: &mut ActiveOperation) -> control::OperationResult {
            self.target();
            mark_remote_observation(op, false);
            self.run(op).unwrap()
        }
        fn assert_no_mutation(&self) {
            for step in ["empty", "release", "popup", "desktop", "correct", "shape"] {
                assert_eq!(self.count(step), 0, "{step}");
            }
        }
    }
    impl BringHereHost for Fake {
        fn now(&self) -> Instant {
            self.now
        }
        fn owned(&mut self) -> Result<bool, HostError> {
            self.step("owner")?;
            Ok(self.owned)
        }
        fn live(&self) -> bool {
            self.runtime.body != BodyAvailability::Destroyed
        }
        fn evidence(&mut self, _: u32) -> Result<HereEvidence, HostError> {
            let cell = RefCell::new(&mut *self);
            here_evidence_with(
                || cell.borrow_mut().read_layout(),
                || {
                    let mut h = cell.borrow_mut();
                    h.step("observe")?;
                    let reading = h.reads.pop_front().unwrap_or(h.current);
                    let mut snapshot = reading.snapshot();
                    if let Some(terminal) = h.terminal {
                        if terminal == control::Visibility::Destroyed {
                            snapshot.evidence.destroyed = true;
                            h.window.window_resource.externally_destroyed();
                            h.runtime.unavailable(true);
                        } else {
                            snapshot.evidence.wm_state = WmState::Withdrawn;
                            snapshot.observation.as_mut().unwrap().wm_state = WmState::Withdrawn;
                        }
                        snapshot.candidate = control::classify(snapshot.evidence);
                    }
                    Ok(snapshot)
                },
            )
        }
        fn placement_busy(&self) -> bool {
            self.runtime.correction.is_some()
        }
        fn input_active(&self) -> bool {
            !self.runtime.input_fence.blocked
        }
        fn visible_ready(&self) -> bool {
            here_runtime_ready(&self.runtime, &self.window)
        }
        fn fence_settled(&self) -> bool {
            self.runtime.visibility_dispatch_eligible()
        }
        fn fence(&mut self) -> Result<(), HostError> {
            self.log.push("fence");
            let empty = self.step("empty");
            let release = self.step("release");
            let popup = self.step("popup");
            self.runtime.fence_visibility_input_with(
                &mut self.window,
                || empty.map(|()| 100),
                |_, _| release,
                |_| popup,
            )
        }
        fn desktop(
            &mut self,
            target: u32,
            deadline: Instant,
            dispatched: &mut bool,
        ) -> Result<bool, HostError> {
            assert_eq!(target, 2);
            assert!(self.fence_settled());
            if self.now >= deadline {
                return Ok(false);
            }
            if !self.send_before_cookie {
                *dispatched = true;
            }
            self.step("desktop")?;
            Ok(true)
        }
        fn geometry(&mut self) -> Result<Rect, HostError> {
            self.step("geometry")?;
            Ok(self.geometries.pop_front().unwrap_or(self.actual))
        }
        fn restrict_automatic_correction(&mut self) {
            self.runtime.input_fence.automatic_correction_blocked = true;
            self.log.push("restrict");
        }
        fn correct(
            &mut self,
            target: Point,
            deadline: Instant,
            dispatched: &mut bool,
        ) -> Result<Option<Instant>, HostError> {
            assert!(self.runtime.input_fence.automatic_correction_blocked);
            assert!(self.fence_settled());
            if self.now + CONFIRMATION_TIMEOUT > deadline {
                return Ok(None);
            }
            *dispatched = true;
            self.step("correct")?;
            if !self.refuse_correction {
                self.actual = Rect::new(target.x, target.y, 160, 160);
            }
            self.runtime.body = BodyAvailability::Validating;
            Ok(Some(self.now + CONFIRMATION_TIMEOUT))
        }
        fn enable(
            &mut self,
            evidence: &HereEvidence,
            actual: Rect,
            deadline: Instant,
        ) -> Result<bool, HostError> {
            assert!(self.now < deadline);
            self.step("enable")?;
            let shape = self.step("shape");
            self.window.confirmed_origin = actual.origin();
            self.window.cancel_in_flight_moves();
            self.runtime.body = BodyAvailability::Ready;
            self.runtime.restore_visibility_input_with(
                &self.window,
                evidence.snapshot.evidence,
                &InputPlacement {
                    selected_monitor: 20,
                    observed_monitor: evidence.layout.as_ref().unwrap().monitor_name,
                    geometry: actual,
                    bounds: evidence.layout.as_ref().unwrap().valid_origins,
                },
                || shape.map(|()| 1000),
            )?;
            Ok(true)
        }
    }
    fn check_result(result: control::OperationResult, status: Status, history: bool) {
        assert_eq!(result.status(), status);
        assert_eq!(
            result.detail().placement.history.relocation_confirmed,
            history
        );
        assert!(!result.detail().placement.history.restoration_confirmed);
        let reply = control::TerminalResponse {
            status,
            detail: result.detail(),
            request_id: 99,
            control_xid: 10,
        };
        assert_eq!(
            control::TerminalResponse::decode(&reply.encode().unwrap()).unwrap(),
            reply
        );
        if status != Status::Success {
            assert_eq!(
                result.detail().placement.current,
                control::PlacementConfirmation::default()
            );
        }
    }

    #[test]
    fn safe_local_noop_is_fresh_and_has_zero_mutations_for_all_layers() {
        for layer in [L::Normal, L::Above, L::Below] {
            let mut h = Fake::new(false, false, layer);
            let mut op = h.operation();
            let result = h.run(&mut op).unwrap();
            check_result(result, Status::Success, false);
            h.assert_no_mutation();
            assert!(h.count("layout") >= 6);
            assert!(h.count("geometry") >= 2);
            assert_eq!(
                result.detail().placement.current.geometry,
                control::GeometryProof::Verified
            );
        }
    }
    #[test]
    fn local_unsafe_fence_cancels_every_gesture_popup_and_release_debt_neutrally() {
        for case in 0..6 {
            let mut h = Fake::new(false, false, L::Below);
            h.runtime = visibility_fence_tests::runtime(case);
            let mut op = h.operation();
            let result = h.run(&mut op).unwrap();
            check_result(result, Status::Success, false);
            assert_eq!(h.count("fence"), 1);
            assert_eq!(h.count("desktop"), 0);
            assert_eq!(h.count("correct"), 0);
            assert_eq!(h.count("shape"), 1);
            assert!(!h.runtime.pointer.is_grabbed());
            assert!(h.runtime.menu.is_none());
            assert!(h.runtime.pending_move.is_none());
            assert!(h.runtime.final_target.is_none());
        }
    }
    #[test]
    fn stronger_noop_checks_all_runtime_obligations() {
        for case in 0..11 {
            let mut h = Fake::new(false, false, L::Normal);
            match case {
                0 => h.runtime.pending_map_input = Some(1),
                1 => h.runtime.pending_move = Some(Point::new(120, 120)),
                2 => h.runtime.final_target = Some(Point::new(120, 120)),
                3 => h.runtime.confirmation_deadline = Some(h.now),
                4 => h.runtime.safety.deadline = Some(h.now),
                5 => h.runtime.body = BodyAvailability::Validating,
                6 => h.runtime.pointer.require_transition_release(),
                7 => h.runtime.input_fence.enabled_after = None,
                8 => h.window.in_flight_moves.push_back(Point::new(120, 120)),
                9 => h.runtime.menu = Some(MenuPopup::test_popup(200, 1)),
                _ => h.runtime.input_fence.blocked = true,
            }
            let mut op = h.operation();
            check_result(h.run(&mut op).unwrap(), Status::Success, false);
            assert_eq!(h.count("fence"), 1, "case {case}");
        }
    }
    #[test]
    fn existing_ordinary_correction_is_busy_and_not_stolen() {
        let mut h = Fake::new(false, false, L::Normal);
        let mut op = h.operation();
        h.runtime.correction = Some((Point::new(0, 0), h.now));
        assert_eq!(h.run(&mut op).unwrap().status(), Status::Busy);
        h.assert_no_mutation();
        assert!(h.runtime.correction.is_some());
        assert_eq!(h.count("layout"), 0);
    }
    #[test]
    fn local_correction_clamps_live_origin_once_and_cannot_authorize_partial() {
        let mut h = Fake::new(false, false, L::Normal);
        h.actual = Rect::new(900, -30, 160, 160);
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        assert_eq!(op.placement.correction().unwrap().0, Point::new(640, 0));
        check_result(h.run(&mut op).unwrap(), Status::Success, false);
        assert_eq!(h.count("correct"), 1);
        assert_eq!(h.count("desktop"), 0);
        assert_eq!(h.count("shape"), 1);
    }
    #[test]
    fn local_and_remote_minimized_preserve_each_layer_and_defer_without_geometry() {
        for remote in [false, true] {
            for layer in [L::Normal, L::Above, L::Below] {
                let mut h = Fake::new(remote, true, layer);
                h.actual = Rect::new(9999, 9999, 160, 160);
                let mut op = h.operation();
                let result = if remote {
                    assert!(h.run(&mut op).is_none());
                    h.arrived(&mut op)
                } else {
                    h.run(&mut op).unwrap()
                };
                check_result(result, Status::Success, remote);
                let d = result.detail();
                assert!(d.progress.placement_deferred);
                assert!(!d.progress.input_ready);
                assert!(!d.progress.above_confirmed);
                assert_eq!(d.layer, layer);
                assert_eq!(
                    d.placement.current.geometry,
                    control::GeometryProof::Deferred
                );
                for step in ["geometry", "correct", "shape"] {
                    assert_eq!(h.count(step), 0);
                }
                assert!(h.fence_settled());
                assert!(!h.runtime.input_fence.automatic_correction_blocked);
            }
        }
    }
    #[test]
    fn remote_visible_each_layer_including_iconic_unmapped_without_hidden() {
        for layer in [L::Normal, L::Above, L::Below] {
            let mut h = Fake::new(true, false, layer);
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            assert!(!op.placement.baseline.unwrap().minimized);
            assert!(!op.placement.proof.history.relocation_confirmed);
            assert_eq!(h.count("desktop"), 1);
            let log = &h.log;
            let send = log.iter().position(|s| *s == "desktop").unwrap();
            let release = log.iter().position(|s| *s == "release").unwrap();
            assert!(send > release);
            assert!(log[release..send].contains(&"layout"));
            check_result(h.arrived(&mut op), Status::Success, true);
            assert_eq!(h.count("shape"), 1);
            assert_eq!(h.count("desktop"), 1);
            assert_eq!(h.count("correct"), 0);
        }
    }
    #[test]
    fn initial_transitional_or_invalid_evidence_never_establishes_mutation_authority() {
        for variant in 0..5 {
            let mut h = Fake::new(true, false, L::Normal);
            match variant {
                0 => {
                    h.current.desktop = 2;
                    h.current.transitional = true;
                }
                1 => h.current.layer = L::Unknown,
                2 => h.layout = Err(DestinationError::UnsupportedViewport),
                3 => h.layout = Err(DestinationError::Changed),
                _ => h.layout = Err(DestinationError::Unverifiable("bad monitor".into())),
            }
            let mut op = h.operation();
            check_result(h.run(&mut op).unwrap(), Status::Failed, false);
            h.assert_no_mutation();
        }
    }
    #[test]
    fn every_fence_failure_stops_send_but_attempts_release_and_popup_cleanup() {
        for step in ["empty", "release", "popup"] {
            let mut h = Fake::new(true, false, L::Normal);
            h.runtime = visibility_fence_tests::runtime(3);
            h.faults.push((step, 1));
            let mut op = h.operation();
            let error = service_bring_here_with(&mut op, &mut h).unwrap_err();
            assert_eq!(
                error.downcast_ref::<NativeFault>(),
                Some(&NativeFault(step, 1))
            );
            assert_eq!(h.count("release"), 1);
            assert_eq!(h.count("popup"), 1);
            assert_eq!(h.count("desktop"), 0);
            assert!(h.runtime.input_fence.blocked);
            assert_eq!(op.detail.stage, Stage::Shutdown);
        }
    }
    #[test]
    fn failed_enable_gets_exactly_one_refence_and_original_error_priority() {
        for remote in [false, true] {
            for cleanup_fails in [false, true] {
                let mut h = Fake::new(remote, false, L::Normal);
                h.runtime.input_fence.enabled_after = None;
                let mut op = h.operation();
                if remote {
                    assert!(h.run(&mut op).is_none());
                    h.target();
                }
                h.faults.push(("shape", 1));
                if cleanup_fails {
                    h.faults.push(("empty", 2));
                }
                let error = service_bring_here_with(&mut op, &mut h).unwrap_err();
                assert_eq!(
                    error.downcast_ref::<NativeFault>(),
                    Some(&NativeFault("shape", 1))
                );
                assert_eq!(h.count("fence"), 2);
                assert_eq!(h.count("shape"), 1);
                assert!(h.runtime.input_fence.blocked);
                assert_eq!(op.detail.reason, Reason::X11Failure);
                check_result(
                    op.placement
                        .here_result(op.detail, control::Confirmation::default())
                        .unwrap(),
                    if remote {
                        Status::Partial
                    } else {
                        Status::Failed
                    },
                    remote,
                );
            }
        }
    }
    #[test]
    fn final_loss_after_enable_refences_once_and_never_reenables() {
        for remote in [false, true] {
            let mut h = Fake::new(remote, false, L::Normal);
            h.runtime.input_fence.enabled_after = None;
            let mut op = h.operation();
            if remote {
                assert!(h.run(&mut op).is_none());
                h.target();
            }
            let mut changed = h.current;
            changed.layer = L::Below;
            h.changes.push_back(("shape", changed));
            let result = h.run(&mut op).unwrap();
            check_result(
                result,
                if remote {
                    Status::Partial
                } else {
                    Status::Failed
                },
                remote,
            );
            assert_eq!(result.detail().reason, Reason::LayerChanged);
            assert_eq!(h.count("shape"), 1);
            assert_eq!(h.count("fence"), 2);
            assert!(h.runtime.input_fence.blocked);
        }
    }
    #[test]
    fn lost_final_noop_safety_also_fences_without_creating_relocation_history() {
        let mut h = Fake::new(false, false, L::Normal);
        let mut op = h.operation();
        h.geometries = VecDeque::from([h.actual, Rect::new(999, 999, 160, 160)]);
        check_result(h.run(&mut op).unwrap(), Status::Failed, false);
        assert_eq!(h.count("fence"), 1);
        assert_eq!(h.count("desktop"), 0);
        assert_eq!(h.count("shape"), 0);
    }
    #[test]
    fn uncertain_send_can_only_gain_history_from_bounded_fresh_membership() {
        for before_cookie in [false, true] {
            for reaches_target in [false, true] {
                let mut h = Fake::new(true, false, L::Normal);
                h.send_before_cookie = before_cookie;
                h.faults.push(("desktop", 1));
                let mut changed = h.current;
                changed.desktop = 2;
                if reaches_target {
                    h.changes.push_back(("desktop", changed));
                }
                let mut op = h.operation();
                let err = service_bring_here_with(&mut op, &mut h).unwrap_err();
                assert_eq!(
                    err.downcast_ref::<NativeFault>(),
                    Some(&NativeFault("desktop", 1))
                );
                let history = !before_cookie && reaches_target;
                check_result(
                    op.placement
                        .here_result(op.detail, control::Confirmation::default())
                        .unwrap(),
                    if history {
                        Status::Partial
                    } else {
                        Status::Failed
                    },
                    history,
                );
                assert_eq!(h.count("desktop"), 1);
                assert_eq!(h.count("correct"), 0);
                assert_eq!(h.count("shape"), 0);
            }
        }
    }
    #[test]
    fn outer_layout_native_failure_before_send_after_possible_send_and_after_history() {
        for phase in 0..3 {
            for final_fails in [false, true] {
                let mut h = Fake::new(true, false, L::Normal);
                let mut op = h.operation();
                if phase > 0 {
                    assert!(h.run(&mut op).is_none());
                }
                if phase == 2 {
                    h.target();
                    h.current.transitional = true;
                    assert!(h.run(&mut op).is_none());
                    assert!(op.placement.proof.history.relocation_confirmed);
                }
                let fault_at = h.count("layout") + 1;
                h.faults.push(("layout", fault_at));
                if final_fails {
                    h.faults.push(("layout", fault_at + 1));
                } else if phase > 0 {
                    h.target();
                }
                let err = service_bring_here_with(&mut op, &mut h).unwrap_err();
                assert_eq!(
                    err.downcast_ref::<NativeFault>(),
                    Some(&NativeFault("layout", fault_at))
                );
                let history = phase == 2 || (phase == 1 && !final_fails);
                let result = op
                    .placement
                    .here_result(op.detail, control::Confirmation::default())
                    .unwrap();
                check_result(
                    result,
                    if history {
                        Status::Partial
                    } else {
                        Status::Failed
                    },
                    history,
                );
                assert_eq!(result.detail().stage, Stage::Shutdown);
                assert_eq!(result.detail().reason, Reason::X11Failure);
                assert_eq!(h.count("desktop"), usize::from(phase > 0));
                assert_eq!(h.count("shape"), 0);
                if phase == 0 {
                    h.assert_no_mutation();
                    assert_eq!(h.count("layout"), 1);
                }
            }
        }
    }
    #[test]
    fn inner_destination_errors_are_ordinary_results_at_all_corresponding_phases() {
        for phase in 0..3 {
            for error in [
                DestinationError::Changed,
                DestinationError::UnsupportedViewport,
                DestinationError::Unverifiable("invalid viewport/monitor".into()),
            ] {
                let mut h = Fake::new(true, false, L::Normal);
                let mut op = h.operation();
                if phase > 0 {
                    assert!(h.run(&mut op).is_none());
                }
                if phase == 2 {
                    h.target();
                    h.current.transitional = true;
                    assert!(h.run(&mut op).is_none());
                }
                h.layout = Err(error.clone());
                let result = h.run(&mut op).unwrap();
                check_result(
                    result,
                    if phase == 2 {
                        Status::Partial
                    } else {
                        Status::Failed
                    },
                    phase == 2,
                );
                assert_eq!(result.detail().reason, here_destination_reason(&error));
                assert_ne!(result.detail().stage, Stage::Shutdown);
                assert_eq!(h.count("desktop"), usize::from(phase > 0));
            }
        }
    }
    #[test]
    fn native_primary_survives_final_inner_error_and_terminal_reply_error() {
        let mut h = Fake::new(true, false, L::Normal);
        let mut active = Some(h.operation());
        h.faults.push(("desktop", 1));
        h.target();
        h.current.desktop = 0;
        h.layouts = VecDeque::from([
            Ok(layout()),
            Ok(layout()),
            Ok(layout()),
            Ok(layout()),
            Err(DestinationError::Changed),
            Err(DestinationError::Changed),
        ]);
        let mut target = h.current;
        target.desktop = 2;
        h.changes.push_back(("desktop", target));
        let mut replies = vec![];
        let err = service_here_owner_with(&mut active, &mut h, |r| {
            replies.push(r);
            Err("reply failed".into())
        })
        .unwrap_err();
        assert_eq!(
            err.downcast_ref::<NativeFault>(),
            Some(&NativeFault("desktop", 1))
        );
        assert!(active.is_none());
        assert_eq!(replies.len(), 1);
        check_result(replies[0], Status::Partial, true);
        assert_eq!(replies[0].detail().reason, Reason::X11Failure);
    }
    #[test]
    fn layout_and_preservation_drift_never_retarget_or_mutate_layer() {
        for phase in 0..3 {
            for drift in 0..10 {
                let mut h = Fake::new(true, false, L::Above);
                let mut op = h.operation();
                if phase > 0 {
                    assert!(h.run(&mut op).is_none());
                }
                if phase == 2 {
                    h.target();
                    h.current.transitional = true;
                    assert!(h.run(&mut op).is_none());
                    h.current.transitional = false;
                }
                let mut changed = layout();
                match drift {
                    0 => changed.desktop_count += 1,
                    1 => changed.current_desktop = 3,
                    2 => changed.target_workarea = None,
                    3 => changed.viewport = control::DestinationViewport::Absent,
                    4 => changed.monitor_name += 1,
                    5 => changed.monitor_bounds = Rect::new(1, 0, 800, 600),
                    6 => {
                        h.current.layer = L::Below;
                    }
                    7 => {
                        h.current.minimized = true;
                    }
                    8 => {
                        h.current.desktop = 3;
                    }
                    _ => {
                        h.current.desktop = 0;
                    }
                }
                if phase == 0 && drift < 6 {
                    // changes are detected against the admitted round, not a fabricated baseline
                    h.layouts =
                        VecDeque::from([Ok(layout()), Ok(layout()), Ok(changed), Ok(changed)]);
                } else if drift < 6 {
                    h.layout = Ok(changed);
                }
                if phase == 0 && drift == 9 {
                    continue;
                }
                if phase == 0 && drift >= 6 {
                    h.reads.push_back(Reading {
                        desktop: 0,
                        layer: L::Above,
                        minimized: false,
                        transitional: false,
                    });
                }
                if phase == 1 && drift == 9 {
                    h.now = op.deadline;
                }
                let result = h
                    .run(&mut op)
                    .unwrap_or_else(|| panic!("phase {phase}, drift {drift}"));
                check_result(
                    result,
                    if phase == 2 {
                        Status::Partial
                    } else {
                        Status::Failed
                    },
                    phase == 2,
                );
                assert_eq!(h.count("desktop"), usize::from(phase > 0));
                assert_eq!(h.count("correct"), 0);
                assert_eq!(h.count("shape"), 0);
            }
        }
    }
    #[test]
    fn target_membership_survives_same_read_invalid_layout_or_layer_as_history_only() {
        for drift in 0..3 {
            let mut h = Fake::new(true, false, L::Normal);
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            h.target();
            match drift {
                0 => h.layout = Err(DestinationError::Changed),
                1 => h.current.layer = L::Above,
                _ => h.current.minimized = true,
            }
            check_result(h.run(&mut op).unwrap(), Status::Partial, true);
        }
    }
    #[test]
    fn correction_failure_or_mismatch_never_sends_twice_or_claims_geometry() {
        for remote in [false, true] {
            for native_error in [false, true] {
                let mut h = Fake::new(remote, false, L::Normal);
                h.actual = Rect::new(-30, 999, 160, 160);
                h.refuse_correction = true;
                let mut op = h.operation();
                if remote {
                    assert!(h.run(&mut op).is_none());
                    h.target();
                }
                if native_error {
                    h.faults.push(("correct", 1));
                }
                if native_error {
                    assert!(service_bring_here_with(&mut op, &mut h).is_err());
                } else {
                    assert!(h.run(&mut op).is_none());
                    assert!(!remote_service_due(&op, h.now) || op.placement.observation_due());
                    assert!(h.run(&mut op).is_none());
                    assert!(!remote_service_due(&op, h.now));
                    h.now = op.placement.correction().unwrap().1;
                    check_result(
                        h.run(&mut op).unwrap(),
                        if remote {
                            Status::Partial
                        } else {
                            Status::Failed
                        },
                        remote,
                    );
                }
                assert_eq!(h.count("correct"), 1);
                assert_eq!(h.count("shape"), 0);
                assert!(h.runtime.input_fence.automatic_correction_blocked);
            }
        }
    }
    #[test]
    fn fixed_size_and_full_correction_interval_are_required() {
        for invalid_size in [false, true] {
            let mut h = Fake::new(true, false, L::Normal);
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            h.target();
            h.actual = Rect::new(999, 999, if invalid_size { 159 } else { 160 }, 160);
            if !invalid_size {
                h.now = op.deadline - Duration::from_millis(149);
            }
            let result = h.run(&mut op).unwrap();
            check_result(result, Status::Partial, true);
            assert_eq!(h.count("correct"), 0);
            assert_eq!(h.count("shape"), 0);
        }
    }
    #[test]
    fn crossing_each_read_boundary_never_starts_an_expired_mutation() {
        for step in ["owner", "layout", "observe", "geometry", "empty"] {
            let mut h = Fake::new(false, false, L::Normal);
            h.runtime.input_fence.enabled_after = None;
            let mut op = h.operation();
            h.advances.push_back((step, op.deadline - h.now));
            let result = h.run(&mut op).unwrap();
            check_result(result, Status::Failed, false);
            assert_eq!(h.count("desktop"), 0);
            assert_eq!(h.count("correct"), 0);
            assert_eq!(h.count("shape"), 0);
        }
    }
    #[test]
    fn deadline_final_read_distinguishes_complete_minimized_and_incomplete_without_notifications() {
        for minimized in [false, true] {
            for target in [false, true] {
                let mut h = Fake::new(true, minimized, L::Above);
                let mut op = h.operation();
                assert!(h.run(&mut op).is_none());
                h.now = op.deadline;
                if target {
                    h.target();
                }
                let result = h.run(&mut op).unwrap();
                check_result(
                    result,
                    if target && minimized {
                        Status::Success
                    } else if target {
                        Status::Partial
                    } else {
                        Status::Failed
                    },
                    target,
                );
                assert_eq!(h.count("desktop"), 1);
                assert_eq!(h.count("shape"), 0);
                assert_eq!(h.count("correct"), 0);
            }
        }
    }
    #[test]
    fn ownership_loss_and_body_destruction_stop_all_mutation_and_keep_history() {
        for history in [false, true] {
            for destroyed in [false, true] {
                let mut h = Fake::new(true, false, L::Normal);
                let mut op = h.operation();
                assert!(h.run(&mut op).is_none());
                if history {
                    h.target();
                    h.current.transitional = true;
                    assert!(h.run(&mut op).is_none());
                }
                if destroyed {
                    h.window.window_resource.externally_destroyed();
                    h.runtime.unavailable(true);
                } else {
                    h.owned = false;
                }
                let before = h.log.len();
                let result = h.run(&mut op).unwrap();
                check_result(
                    result,
                    if history {
                        Status::Partial
                    } else {
                        Status::Failed
                    },
                    history,
                );
                assert_eq!(
                    result.detail().reason,
                    if destroyed {
                        Reason::BodyDestroyed
                    } else {
                        Reason::OwnershipLost
                    }
                );
                assert!(!h.log[before..]
                    .iter()
                    .any(|s| ["empty", "desktop", "correct", "shape"].contains(s)));
            }
        }
    }
    #[test]
    fn caller_disappearance_and_failed_terminal_send_clear_once_without_undoing_host_progress() {
        for fail_reply in [false, true] {
            let mut h = Fake::new(true, false, L::Below);
            let mut active = Some(h.operation());
            service_here_owner_with(&mut active, &mut h, |_| panic!("pending")).unwrap();
            h.target();
            mark_remote_observation(active.as_mut().unwrap(), false);
            let mut replies = vec![];
            let done = service_here_owner_with(&mut active, &mut h, |r| {
                replies.push(r);
                if fail_reply {
                    Err("caller gone".into())
                } else {
                    Ok(())
                }
            });
            assert_eq!(done.is_err(), fail_reply);
            assert!(active.is_none());
            assert_eq!(replies.len(), 1);
            check_result(replies[0], Status::Success, true);
            assert!(h.visible_ready());
            service_here_owner_with(&mut active, &mut h, |_| panic!("second reply")).unwrap();
            assert_eq!(h.count("desktop"), 1);
            assert_eq!(h.count("shape"), 1);
        }
    }
    #[test]
    fn restricted_fence_survives_completion_and_every_ordinary_placement_path() {
        let mut h = Fake::new(false, false, L::Normal);
        h.actual = Rect::new(999, 999, 160, 160);
        h.refuse_correction = true;
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.now = op.placement.correction().unwrap().1;
        check_result(h.run(&mut op).unwrap(), Status::Failed, false);
        for _ in 0..4 {
            h.runtime.body = BodyAvailability::Validating;
            h.runtime
                .placement_with(&mut h.window, h.actual, &layout().valid_origins, |_, _| {
                    panic!("automatic second correction")
                })
                .unwrap();
            assert_ne!(h.runtime.body, BodyAvailability::Ready);
            assert!(h.runtime.correction.is_none());
            assert!(h.runtime.input_fence.automatic_correction_blocked);
        }
        h.actual = Rect::new(100, 100, 160, 160);
        h.runtime
            .placement_with(
                &mut h.window,
                h.actual,
                &layout().valid_origins,
                |_, _| panic!(),
            )
            .unwrap();
        assert_eq!(h.runtime.body, BodyAvailability::Ready);
        let evidence = h.evidence(2).unwrap();
        assert!(h
            .enable(&evidence, h.actual, h.now + Duration::from_secs(1))
            .unwrap());
        assert!(!h.runtime.input_fence.automatic_correction_blocked);
    }
    #[test]
    fn restricted_fence_does_not_take_a_new_here_commands_correction_authority() {
        let mut h = Fake::new(false, false, L::Normal);
        h.fence().unwrap();
        h.restrict_automatic_correction();
        let mut op = h.operation();
        h.actual = Rect::new(999, 999, 160, 160);
        assert!(h.run(&mut op).is_none());
        assert_eq!(h.count("correct"), 1);
        check_result(h.run(&mut op).unwrap(), Status::Success, false);
    }
    #[test]
    fn minimized_deferral_leaves_ordinary_future_correction_available() {
        let mut h = Fake::new(false, true, L::Normal);
        let mut op = h.operation();
        check_result(h.run(&mut op).unwrap(), Status::Success, false);
        h.current.minimized = false;
        h.runtime.body = BodyAvailability::Validating;
        let mut requests = 0;
        h.runtime
            .placement_with(
                &mut h.window,
                Rect::new(999, 999, 160, 160),
                &layout().valid_origins,
                |_, _| {
                    requests += 1;
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(requests, 1);
    }
    fn atoms() -> (x11::visibility::VisibilityAtoms, x11::monitors::LayoutAtoms) {
        (
            x11::visibility::VisibilityAtoms {
                change_state: 11,
                state: 12,
                hidden: 13,
                above: 14,
                below: 15,
                desktop: 16,
                current_desktop: 17,
                supported: 18,
                wm_state: 19,
                user_time: 20,
            },
            x11::monitors::LayoutAtoms {
                current_desktop: 17,
                workarea: 21,
                wm_desktop: 16,
                number_of_desktops: 22,
                desktop_viewport: 23,
            },
        )
    }
    fn property(window: u32, atom: u32) -> Event {
        Event::PropertyNotify(x11rb::protocol::xproto::PropertyNotifyEvent {
            window,
            atom,
            ..Default::default()
        })
    }
    fn event(kind: usize) -> Event {
        use x11rb::protocol::xproto::*;
        match kind {
            0 => property(100, 16),
            1 => property(100, 12),
            2 => property(100, 19),
            3 => Event::MapNotify(MapNotifyEvent {
                window: 100,
                event: 100,
                ..Default::default()
            }),
            4 => Event::UnmapNotify(UnmapNotifyEvent {
                window: 100,
                event: 100,
                ..Default::default()
            }),
            5 => Event::ConfigureNotify(ConfigureNotifyEvent {
                window: 100,
                x: 999,
                y: 999,
                width: 160,
                height: 160,
                ..Default::default()
            }),
            6 => property(1, 17),
            7 => property(1, 22),
            8 => property(1, 21),
            9 => property(1, 23),
            10 => property(1, 18),
            11 => Event::ConfigureNotify(ConfigureNotifyEvent {
                window: 1,
                ..Default::default()
            }),
            12 => Event::DestroyNotify(DestroyNotifyEvent {
                window: 100,
                event: 100,
                ..Default::default()
            }),
            _ => Event::RandrScreenChangeNotify(Default::default()),
        }
    }
    #[test]
    fn every_here_evidence_event_is_consumed_before_legacy_placement_and_map_paths() {
        let (visibility, layout) = atoms();
        let mut h = Fake::new(true, false, L::Normal);
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        let history = op.placement.proof.history;
        for kind in 0..14 {
            let e = event(kind);
            assert!(route_here_evidence_event(
                Some(&mut op),
                &e,
                1,
                100,
                &visibility,
                &layout
            ));
            assert!(op.placement.observation_due());
            assert_eq!(op.placement.proof.history, history);
        }
        assert!(!route_here_evidence_event(
            Some(&mut op),
            &property(999, 22),
            1,
            100,
            &visibility,
            &layout
        ));
        assert!(!route_here_evidence_event(
            None,
            &event(0),
            1,
            100,
            &visibility,
            &layout
        ));
        assert!(!local_layer_service_allowed(Some(&op)));
    }
    #[test]
    fn desktop_map_state_event_permutations_duplicates_delays_converge_only_through_reads() {
        let (visibility, layout) = atoms();
        for order in [
            [0, 3, 2],
            [0, 2, 3],
            [3, 0, 2],
            [3, 2, 0],
            [2, 0, 3],
            [2, 3, 0],
        ] {
            let mut h = Fake::new(true, false, L::Below);
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            for &kind in &order {
                assert!(route_here_evidence_event(
                    Some(&mut op),
                    &event(kind),
                    1,
                    100,
                    &visibility,
                    &layout
                ));
                assert!(h.run(&mut op).is_none()); // notifications alone cannot prove membership
                assert!(!remote_service_due(&op, h.now));
            }
            h.target();
            h.current.transitional = true;
            route_here_evidence_event(Some(&mut op), &event(4), 1, 100, &visibility, &layout);
            assert!(h.run(&mut op).is_none());
            assert!(op.placement.proof.history.relocation_confirmed);
            assert_eq!(h.count("shape"), 0);
            assert_eq!(h.count("geometry"), 0);
            h.current.transitional = false;
            route_here_evidence_event(
                Some(&mut op),
                &event(order[2]),
                1,
                100,
                &visibility,
                &layout,
            );
            check_result(h.run(&mut op).unwrap(), Status::Success, true);
            let mut active = None;
            for &kind in &order {
                assert!(!route_here_evidence_event(
                    active.as_mut(),
                    &event(kind),
                    1,
                    100,
                    &visibility,
                    &layout
                ));
            }
            assert_eq!(h.count("desktop"), 1);
            assert_eq!(h.count("correct"), 0);
            assert_eq!(h.count("shape"), 1);
            // A late map has no new silhouette synchronization allowance.
            let Event::MapNotify(map) = event(3) else {
                unreachable!()
            };
            h.runtime
                .synchronize_map_input_with(&map, 100, 9999, || {
                    panic!("late map must not reassert shape")
                })
                .unwrap();
        }
    }
    fn owner_tick(
        active: &mut Option<ActiveOperation>,
        h: &mut Fake,
        replies: &mut Vec<control::OperationResult>,
    ) {
        service_here_owner_with(active, h, |r| {
            replies.push(r);
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn sustained_traffic_over_64_events_cannot_starve_total_or_correction_deadline() {
        for correction in [false, true] {
            let (visibility, layout) = atoms();
            let mut h = Fake::new(!correction, false, L::Normal);
            if correction {
                h.actual = Rect::new(999, 999, 160, 160);
                h.refuse_correction = true;
            }
            let mut active = Some(h.operation());
            let immutable = active.unwrap().deadline;
            let mut replies = vec![];
            owner_tick(&mut active, &mut h, &mut replies);
            owner_tick(&mut active, &mut h, &mut replies); // consumes correction's immediate observation
            let expected = remote_operation_deadline(active.as_ref().unwrap());
            assert_eq!(
                expected,
                if correction {
                    h.now + CONFIRMATION_TIMEOUT
                } else {
                    immutable
                }
            );
            let mut queue = VecDeque::from(vec![property(999, 99); 5000]);
            let mut processed = 0;
            while active.is_some() {
                let batch: Vec<_> =
                    drain_events_bounded(&mut None, 64, || Ok::<_, HostError>(queue.pop_front()))
                        .unwrap();
                for e in batch {
                    h.now += Duration::from_millis(1);
                    processed += 1;
                    if active
                        .as_ref()
                        .is_some_and(|op| h.now >= remote_operation_deadline(op))
                    {
                        owner_tick(&mut active, &mut h, &mut replies);
                        if active.is_none() {
                            assert_eq!(h.now, expected);
                        }
                    }
                    route_here_evidence_event(active.as_mut(), &e, 1, 100, &visibility, &layout);
                }
                owner_tick(&mut active, &mut h, &mut replies);
                if queue.is_empty() {
                    // no notifications: wait advances to the fixed deadline, no sleep
                    if let Some(op) = active {
                        h.now = remote_operation_deadline(&op);
                    }
                    owner_tick(&mut active, &mut h, &mut replies);
                }
            }
            assert!(processed > 64);
            assert_eq!(replies.len(), 1);
            check_result(replies[0], Status::Failed, false);
            assert!(h.now >= expected);
            assert_eq!(h.count("correct"), usize::from(correction));
            assert_eq!(h.count("desktop"), usize::from(!correction));
            assert!(
                h.count("observe") < 12,
                "irrelevant traffic must not cause polling"
            );
        }
    }
    #[test]
    fn correction_deadline_also_finishes_transitional_readiness_without_spinning() {
        let mut h = Fake::new(false, false, L::Normal);
        h.actual = Rect::new(999, 999, 160, 160);
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.current.transitional = true;
        h.now = op.placement.correction().unwrap().1;
        check_result(h.run(&mut op).unwrap(), Status::Failed, false);
        assert_eq!(h.count("correct"), 1);
        assert_eq!(h.count("shape"), 0);
    }
    #[test]
    fn both_consumed_request_sites_and_end_of_batch_progress_before_wait() {
        for site in ["batch", "lookahead"] {
            let mut h = Fake::new(true, false, L::Normal);
            let mut active = None;
            route_owner_request_with(
                &mut active,
                &mut h,
                |active, h| {
                    h.log.push(site);
                    *active = Some(h.operation());
                    Ok(true)
                },
                |active, h| service_here_owner_with(active, h, |_| panic!("pending")),
            )
            .unwrap();
            assert_eq!(h.count("desktop"), 1);
            assert!(active.is_some());
            let reads = h.count("observe");
            let mut replies = vec![];
            owner_tick(&mut active, &mut h, &mut replies); // one immediate post-send observation
            assert!(!remote_service_due(active.as_ref().unwrap(), h.now));
            let settled = h.count("observe");
            owner_tick(&mut active, &mut h, &mut replies);
            assert_eq!(h.count("observe"), settled);
            assert_eq!(settled, reads + 1);
            h.target();
            mark_remote_observation(active.as_mut().unwrap(), false);
            owner_tick(&mut active, &mut h, &mut replies);
            assert!(active.is_none());
            assert_eq!(replies.len(), 1);
        }
    }
    #[test]
    fn sticky_is_unsupported_and_cannot_be_converted_to_an_ordinary_desktop() {
        let mut h = Fake::new(false, false, L::Normal);
        h.current.desktop = u32::MAX;
        let mut op = h.operation();
        let result = h.run(&mut op).unwrap();
        assert_eq!(result.detail().reason, Reason::Unsupported);
        h.assert_no_mutation();
    }
    #[test]
    fn final_native_error_after_enable_has_one_safety_refence_and_never_success() {
        let mut h = Fake::new(true, false, L::Normal);
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.target();
        // arrival read, geometry bracket, then the final proof's first layout read
        let at = h.count("layout") + 5;
        h.faults.push(("layout", at));
        h.faults.push(("empty", 2));
        let err = service_bring_here_with(&mut op, &mut h).unwrap_err();
        assert_eq!(
            err.downcast_ref::<NativeFault>(),
            Some(&NativeFault("layout", at))
        );
        assert_eq!(h.count("shape"), 1);
        assert_eq!(h.count("fence"), 2);
        assert!(h.runtime.input_fence.blocked);
        assert_eq!(op.detail.stage, Stage::Shutdown);
        check_result(
            op.placement
                .here_result(op.detail, control::Confirmation::default())
                .unwrap(),
            Status::Partial,
            true,
        );
    }
    pub(super) fn lifecycle_snapshots() -> Vec<(&'static str, ActiveOperation)> {
        let mut result = vec![];
        let mut h = Fake::new(true, false, L::Normal);
        let op = h.operation();
        result.push(("observe", op));
        let mut fenced = op;
        h.reads = VecDeque::from([
            h.current,
            Reading {
                transitional: true,
                desktop: 2,
                ..h.current
            },
        ]);
        // Use independent unreadable layer evidence after the fence; baseline remains original.
        h.reads[1] = Reading {
            layer: L::Unknown,
            ..h.current
        };
        assert!(h.run(&mut fenced).is_none());
        assert_eq!(fenced.placement.here_phase(), HerePhase::Fenced);
        result.push(("fenced", fenced));
        let mut h = Fake::new(true, false, L::Normal);
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        result.push(("await", op));
        h.target();
        h.current.transitional = true;
        assert!(h.run(&mut op).is_none());
        result.push(("membership", op));
        h.current.transitional = false;
        h.actual = Rect::new(999, 999, 160, 160);
        assert!(h.run(&mut op).is_none());
        result.push(("correction", op));
        h.faults.push(("shape", 1));
        assert!(service_bring_here_with(&mut op, &mut h).is_err());
        result.push(("enable-failed", op));
        let mut h = Fake::new(true, false, L::Normal);
        let mut op = h.operation();
        h.faults.push(("desktop", 1));
        assert!(service_bring_here_with(&mut op, &mut h).is_err());
        result.push(("desktop-attempted", op));
        let mut h = Fake::new(true, false, L::Normal);
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.target();
        let mut changed = h.current;
        changed.layer = L::Below;
        h.changes.push_back(("shape", changed));
        h.run(&mut op).unwrap();
        result.push(("input-restored", op));
        result
    }
    #[test]
    fn final_ownership_loss_cannot_report_success_in_visible_or_minimized_branches() {
        for (remote, minimized, lost_at) in [(false, false, 6), (true, false, 12), (true, true, 8)]
        {
            let mut h = Fake::new(remote, minimized, L::Above);
            let mut op = h.operation();
            if remote {
                assert!(h.run(&mut op).is_none());
                h.target();
            }
            h.lose_at = Some(("layout", lost_at));
            let result = h.run(&mut op).unwrap();
            check_result(
                result,
                if remote {
                    Status::Partial
                } else {
                    Status::Failed
                },
                remote,
            );
            assert_eq!(result.detail().reason, Reason::OwnershipLost);
            assert_eq!(result.detail().stage, Stage::Shutdown);
            assert_eq!(h.count("fence"), usize::from(remote));
        }
    }
    #[test]
    fn ownership_loss_ends_owner_service_after_one_reply_even_if_reply_fails() {
        for fail_reply in [false, true] {
            let mut h = Fake::new(true, false, L::Normal);
            let mut active = Some(h.operation());
            service_here_owner_with(&mut active, &mut h, |_| panic!("pending")).unwrap();
            h.owned = false;
            mark_remote_observation(active.as_mut().unwrap(), false);
            let mut replies = vec![];
            let error = service_here_owner_with(&mut active, &mut h, |result| {
                replies.push(result);
                if fail_reply {
                    Err("reply lost".into())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Control selection ownership lost during Bring Here"
            );
            assert!(active.is_none());
            assert_eq!(replies.len(), 1);
            assert_eq!(replies[0].detail().reason, Reason::OwnershipLost);
            assert_eq!(h.count("desktop"), 1);
            assert_eq!(h.count("fence"), 1);
            assert_eq!(h.count("shape"), 0);
        }
    }
    #[test]
    fn withdrawn_and_destroyed_observations_clear_current_proof_and_keep_real_history() {
        for history in [false, true] {
            for terminal in [
                control::Visibility::Withdrawn,
                control::Visibility::Destroyed,
            ] {
                let mut h = Fake::new(true, false, L::Normal);
                let mut op = h.operation();
                if history {
                    assert!(h.run(&mut op).is_none());
                    h.target();
                    h.current.transitional = true;
                    assert!(h.run(&mut op).is_none());
                }
                h.terminal = Some(terminal);
                let mutations = (h.count("desktop"), h.count("correct"), h.count("shape"));
                let result = h.run(&mut op).unwrap();
                check_result(
                    result,
                    if history {
                        Status::Partial
                    } else {
                        Status::Failed
                    },
                    history,
                );
                assert_eq!(
                    result.detail().reason,
                    if terminal == control::Visibility::Destroyed {
                        Reason::BodyDestroyed
                    } else {
                        Reason::Withdrawn
                    }
                );
                assert_eq!(
                    (h.count("desktop"), h.count("correct"), h.count("shape")),
                    mutations
                );
                if terminal == control::Visibility::Destroyed {
                    assert!(!h.live());
                }
            }
        }
    }
    #[test]
    fn original_native_error_survives_cleanup_final_native_read_and_reply_all_failing() {
        let mut h = Fake::new(true, false, L::Normal);
        let mut active = Some(h.operation());
        service_here_owner_with(&mut active, &mut h, |_| panic!("pending")).unwrap();
        h.target();
        h.faults = vec![("shape", 1), ("empty", 2), ("layout", 9)];
        let mut replies = vec![];
        let error = service_here_owner_with(&mut active, &mut h, |result| {
            replies.push(result);
            Err("reply also failed".into())
        })
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<NativeFault>(),
            Some(&NativeFault("shape", 1))
        );
        assert_eq!(h.count("fence"), 2);
        assert_eq!(h.count("shape"), 1);
        assert_eq!(h.count("layout"), 9);
        assert!(active.is_none());
        assert_eq!(replies.len(), 1);
        check_result(replies[0], Status::Partial, true);
        assert_eq!(replies[0].detail().reason, Reason::X11Failure);
    }
    #[test]
    fn all_service_phase_snapshots_have_truthful_shutdown_and_monotonic_history() {
        for (_, mut op) in lifecycle_snapshots() {
            let history = op.placement.proof.history.relocation_confirmed;
            let result = op.here_shutdown(Reason::Closing).unwrap();
            check_result(
                result,
                if history {
                    Status::Partial
                } else {
                    Status::Failed
                },
                history,
            );
            assert_eq!(result.detail().stage, Stage::Shutdown);
            assert_eq!(result.detail().reason, Reason::Closing);
        }
    }
    use x11::control::transport_tests::HereOwner;

    pub(crate) fn public_roundtrip_service(
        active: &mut Option<ActiveOperation>,
        remote: bool,
        minimized: bool,
        layer: L,
        reply: &mut dyn FnMut(control::OperationResult) -> Result<(), HostError>,
    ) -> Result<(), HostError> {
        let mut host = Fake::new(remote, minimized, layer);
        host.window.window = 88;
        assert_eq!(active.as_ref().unwrap().placement.destination_desktop, 2);
        service_here_owner_with(active, &mut host, &mut *reply)?;
        if remote {
            host.target();
            mark_remote_observation(active.as_mut().unwrap(), false);
            service_here_owner_with(active, &mut host, &mut *reply)?;
        }
        assert!(active.is_none());
        assert_eq!(host.count("desktop"), usize::from(remote));
        assert_eq!(host.window.window, 88);
        Ok(())
    }

    fn public_tick(owner: &mut HereOwner, host: &mut Fake) -> Result<bool, HostError> {
        // Same scoped transport callback and D3 finalizer as production; only
        // the borrowed NativeHere adapter is replaced with its injected host.
        owner.service(|active, selection, control_xid, reply| {
            assert_eq!((selection, control_xid), (50, 10));
            service_here_owner_with(active, host, reply)
        })
    }
    fn public_owner(host: &mut Fake) -> HereOwner {
        // The fixture publishes 88 as its existing body; no create/map API is
        // available in the Here service. Track that exact resource throughout.
        host.window.window = 88;
        HereOwner::new(host.now, 2)
    }
    fn public_terminal(
        owner: &HereOwner,
        status: Status,
        history: bool,
    ) -> control::TerminalResponse {
        assert!(owner.active().is_none());
        let replies = owner.replies();
        assert_eq!(replies.len(), 1);
        let reply = replies[0];
        assert_eq!(reply.request_id, owner.original_id());
        assert_eq!(reply.status, status);
        assert_eq!(reply.detail.placement.history.relocation_confirmed, history);
        assert!(!reply.detail.placement.history.restoration_confirmed);
        let caller = x11::control::CallerOutcome::Terminal(reply);
        let before = caller.exit_code();
        let text = caller.diagnostic(Command::BringHere);
        assert!(text.starts_with("bring-here:"));
        assert_eq!(before, status.exit_code());
        assert_eq!(
            caller.exit_code(),
            before,
            "diagnostics cannot reclassify results"
        );
        reply
    }
    #[test]
    fn public_admission_dispatch_and_completion_preserve_each_layer_and_minimization() {
        for remote in [false, true] {
            for minimized in [false, true] {
                for layer in [L::Normal, L::Above, L::Below] {
                    let mut host = Fake::new(remote, minimized, layer);
                    let mut owner = public_owner(&mut host);
                    assert!(public_tick(&mut owner, &mut host).unwrap());
                    if remote {
                        assert!(owner.active().is_some());
                        assert_eq!(host.count("desktop"), 1);
                        let deadline = owner.active().unwrap().deadline;
                        // D3 schedules one fresh observation after checked dispatch.
                        assert!(public_tick(&mut owner, &mut host).unwrap());
                        let calls = host.log.len();
                        assert!(public_tick(&mut owner, &mut host).unwrap());
                        assert_eq!(host.log.len(), calls);
                        assert_eq!(owner.active().unwrap().deadline, deadline);
                        let send = host.log.iter().position(|s| *s == "desktop").unwrap();
                        let release = host.log.iter().position(|s| *s == "release").unwrap();
                        assert!(release < send);
                        assert!(host.log[release..send].contains(&"layout"));
                        host.target();
                        mark_remote_observation(owner.active_mut().unwrap(), false);
                        assert!(public_tick(&mut owner, &mut host).unwrap());
                    }
                    let reply = public_terminal(&owner, Status::Success, remote);
                    assert_eq!(reply.detail.layer, layer);
                    assert_eq!(reply.detail.progress.placement_deferred, minimized);
                    assert_eq!(reply.detail.progress.input_ready, !minimized);
                    let diagnostic =
                        x11::control::CallerOutcome::Terminal(reply).diagnostic(Command::BringHere);
                    assert!(diagnostic.contains("layer"));
                    assert!(diagnostic.contains("preserved"));
                    if minimized {
                        assert!(diagnostic.contains("remaining genuinely minimized"));
                        assert!(diagnostic.contains("placement/input deferred"));
                        assert!(!diagnostic.contains("restored"));
                        assert_eq!(host.count("shape"), 0);
                        assert_eq!(host.count("geometry"), 0);
                    } else {
                        assert!(diagnostic.contains("placement verified, input ready"));
                        assert_eq!(diagnostic.contains("relocated"), remote);
                        assert_eq!(host.count("shape"), usize::from(remote));
                    }
                    assert_eq!(host.count("desktop"), usize::from(remote));
                    assert_eq!(host.count("correct"), 0);
                    if !remote && !minimized {
                        host.assert_no_mutation();
                    }
                    assert_eq!(host.window.window, 88);
                    assert!(host.runtime.layers.pending().is_none());
                    let log = host.log.clone();
                    assert!(!public_tick(&mut owner, &mut host).unwrap());
                    assert_eq!(host.log, log);
                    assert_eq!(owner.replies().len(), 1);
                }
            }
        }
    }
    #[test]
    fn public_local_repair_uses_d3_correction_without_relocation_or_false_moved_diagnostic() {
        let mut host = Fake::new(false, false, L::Below);
        host.actual = Rect::new(900, -30, 160, 160);
        let mut owner = public_owner(&mut host);
        assert!(public_tick(&mut owner, &mut host).unwrap());
        assert_eq!(host.count("correct"), 1);
        mark_remote_observation(owner.active_mut().unwrap(), false);
        assert!(public_tick(&mut owner, &mut host).unwrap());
        let reply = public_terminal(&owner, Status::Success, false);
        let text = x11::control::CallerOutcome::Terminal(reply).diagnostic(Command::BringHere);
        assert!(!text.contains("relocated"));
        assert!(!text.contains("moved"));
        assert!(text.contains("confirmed on caller's frozen workspace"));
        assert_eq!(host.actual.origin(), Point::new(640, 0));
        assert_eq!(host.count("desktop"), 0);
        assert_eq!(host.count("correct"), 1);
        assert_eq!(host.count("shape"), 1);
    }
    #[test]
    fn public_existing_placement_is_busy_without_body_calls() {
        let mut host = Fake::new(true, false, L::Normal);
        host.runtime.correction = Some((Point::new(0, 0), host.now));
        let mut owner = public_owner(&mut host);
        assert!(public_tick(&mut owner, &mut host).unwrap());
        public_terminal(&owner, Status::Busy, false);
        assert_eq!(host.count("layout"), 0);
        host.assert_no_mutation();
        assert!(host.runtime.correction.is_some());
    }
    #[test]
    fn public_out_of_range_destination_never_falls_back_to_zero() {
        let mut host = Fake::new(true, false, L::Normal);
        let mut owner = HereOwner::new(host.now, 9);
        assert_eq!(owner.active().unwrap().placement.destination_desktop, 9);
        public_tick(&mut owner, &mut host).unwrap();
        let reply = public_terminal(&owner, Status::Failed, false);
        assert_eq!(reply.detail.reason, Reason::LayoutChanged);
        host.assert_no_mutation();
    }
    #[test]
    fn public_duplicate_busy_and_changed_marker_cannot_restart_or_retarget_here() {
        let mut host = Fake::new(true, false, L::Normal);
        let mut owner = public_owner(&mut host);
        public_tick(&mut owner, &mut host).unwrap();
        public_tick(&mut owner, &mut host).unwrap(); // D3 post-dispatch observation
        let original = *owner.active().unwrap();
        owner.advance_peer(Duration::from_millis(800));
        owner.request(owner.original_id(), 2, false);
        assert_eq!(*owner.active().unwrap(), original);
        assert!(owner.replies().is_empty());
        let log = host.log.clone();
        public_tick(&mut owner, &mut host).unwrap();
        assert_eq!(host.log, log);
        owner.request(owner.original_id(), 3, false);
        owner.request(owner.original_id() + 1, 2, false);
        assert_eq!(*owner.active().unwrap(), original);
        assert_eq!(owner.replies().len(), 2);
        assert!(owner.replies().iter().all(|r| r.status == Status::Busy));
        host.target();
        mark_remote_observation(owner.active_mut().unwrap(), false);
        public_tick(&mut owner, &mut host).unwrap();
        assert!(owner.active().is_none());
        assert_eq!(host.count("desktop"), 1);
        assert_eq!(owner.replies().len(), 3);
        assert_eq!(owner.replies()[2].status, Status::Success);
    }
    #[test]
    fn public_frozen_target_and_current_drift_invalidate_without_chasing() {
        for phase in 0..3 {
            let after_history = phase == 2;
            let mut host = Fake::new(true, false, L::Above);
            let mut owner = public_owner(&mut host);
            if phase > 0 {
                public_tick(&mut owner, &mut host).unwrap();
            }
            if after_history {
                host.target();
                host.current.transitional = true;
                mark_remote_observation(owner.active_mut().unwrap(), false);
                public_tick(&mut owner, &mut host).unwrap();
                assert!(
                    owner
                        .active()
                        .unwrap()
                        .placement
                        .proof
                        .history
                        .relocation_confirmed
                );
            }
            host.layout = Err(DestinationError::Changed);
            if let Some(op) = owner.active_mut() {
                mark_remote_observation(op, false);
            }
            public_tick(&mut owner, &mut host).unwrap();
            let reply = public_terminal(
                &owner,
                if after_history {
                    Status::Partial
                } else {
                    Status::Failed
                },
                after_history,
            );
            assert_eq!(reply.detail.reason, Reason::LayoutChanged);
            let text = x11::control::CallerOutcome::Terminal(reply).diagnostic(Command::BringHere);
            assert!(text.contains("workspace/layout changed"));
            assert_eq!(host.count("desktop"), usize::from(phase > 0));
            assert_eq!(host.count("correct"), 0);
            assert_eq!(host.count("shape"), 0);
        }
    }
    #[test]
    fn public_native_failure_uses_d3_history_and_preserves_original_error_over_reply_failure() {
        for history in [false, true] {
            let mut host = Fake::new(true, false, L::Normal);
            let mut owner = public_owner(&mut host);
            let fault = if history {
                public_tick(&mut owner, &mut host).unwrap();
                host.target();
                mark_remote_observation(owner.active_mut().unwrap(), false);
                ("shape", 1)
            } else {
                ("desktop", 1)
            };
            host.faults.push(fault);
            owner.fail_reply(true);
            let error = public_tick(&mut owner, &mut host).unwrap_err();
            assert_eq!(
                error.downcast_ref::<NativeFault>(),
                Some(&NativeFault(fault.0, fault.1))
            );
            let reply = public_terminal(
                &owner,
                if history {
                    Status::Partial
                } else {
                    Status::Failed
                },
                history,
            );
            assert_eq!(reply.detail.reason, Reason::X11Failure);
            let text = x11::control::CallerOutcome::Terminal(reply).diagnostic(Command::BringHere);
            assert!(text.contains("X11 failure"));
            if history {
                assert!(text.contains("actual relocation was confirmed earlier"));
            } else {
                assert!(text.contains("no accepted relocation history"));
                assert!(text.contains("later WM movement remains possible"));
            }
            assert_eq!(host.count("desktop"), 1);
            assert!(!public_tick(&mut owner, &mut host).unwrap());
        }
    }
    #[test]
    fn public_owner_loss_keeps_history_and_never_mutates_after_loss() {
        for history in [false, true] {
            let mut host = Fake::new(true, false, L::Below);
            let mut owner = public_owner(&mut host);
            if history {
                public_tick(&mut owner, &mut host).unwrap();
                host.target();
                host.current.transitional = true;
                mark_remote_observation(owner.active_mut().unwrap(), false);
                public_tick(&mut owner, &mut host).unwrap();
            }
            host.owned = false;
            mark_remote_observation(owner.active_mut().unwrap(), false);
            let mutations = (
                host.count("desktop"),
                host.count("correct"),
                host.count("shape"),
                host.count("fence"),
            );
            assert!(public_tick(&mut owner, &mut host).is_err());
            let reply = public_terminal(
                &owner,
                if history {
                    Status::Partial
                } else {
                    Status::Failed
                },
                history,
            );
            assert_eq!(reply.detail.reason, Reason::OwnershipLost);
            assert_eq!(
                mutations,
                (
                    host.count("desktop"),
                    host.count("correct"),
                    host.count("shape"),
                    host.count("fence")
                )
            );
        }
    }
    #[test]
    fn public_caller_disappearance_reply_failure_and_shutdown_clear_once() {
        for fatal in [false, true] {
            let mut host = Fake::new(true, true, L::Above);
            let mut owner = public_owner(&mut host);
            public_tick(&mut owner, &mut host).unwrap();
            owner.fail_reply(fatal);
            host.target();
            mark_remote_observation(owner.active_mut().unwrap(), false);
            assert_eq!(public_tick(&mut owner, &mut host).is_err(), fatal);
            public_terminal(&owner, Status::Success, true);
            assert_eq!(host.count("desktop"), 1);
            assert!(!public_tick(&mut owner, &mut host).unwrap());
        }
        for history in [false, true] {
            for owned in [false, true] {
                let mut host = Fake::new(true, false, L::Normal);
                let mut owner = public_owner(&mut host);
                public_tick(&mut owner, &mut host).unwrap();
                if history {
                    host.target();
                    host.current.transitional = true;
                    mark_remote_observation(owner.active_mut().unwrap(), false);
                    public_tick(&mut owner, &mut host).unwrap();
                }
                owner.shutdown(owned);
                let reply = public_terminal(
                    &owner,
                    if history {
                        Status::Partial
                    } else {
                        Status::Failed
                    },
                    history,
                );
                assert_eq!(reply.detail.stage, Stage::Shutdown);
                assert_eq!(
                    reply.detail.reason,
                    if owned {
                        Reason::Closing
                    } else {
                        Reason::OwnershipLost
                    }
                );
                owner.shutdown(owned);
                assert_eq!(owner.replies().len(), 1);
                assert_eq!(host.count("desktop"), 1);
            }
        }
    }
    #[test]
    fn public_here_partial_and_failed_diagnostics_explain_reason_without_exit_changes() {
        for history in [false, true] {
            for (reason, expected) in [
                (Reason::LayoutChanged, "workspace/layout changed"),
                (Reason::LayerChanged, "effective layer changed"),
                (Reason::ConfirmationTimeout, "confirmation timed out"),
                (Reason::X11Failure, "X11 failure"),
                (Reason::OwnershipLost, "owner selection lost"),
                (Reason::Closing, "owner shutdown"),
            ] {
                let mut host = Fake::new(true, false, L::Normal);
                let mut op = host.operation();
                host.run(&mut op);
                if history {
                    host.target();
                    host.current.transitional = true;
                    host.run(&mut op);
                }
                let result = op.here_shutdown(reason).unwrap();
                let reply = control::TerminalResponse {
                    status: result.status(),
                    detail: result.detail(),
                    request_id: 99,
                    control_xid: 10,
                };
                let outcome = x11::control::CallerOutcome::Terminal(reply);
                let text = outcome.diagnostic(Command::BringHere);
                assert!(text.contains(expected));
                assert!(text.contains(if history {
                    "actual relocation was confirmed earlier"
                } else {
                    "no accepted relocation history"
                }));
                assert!(!text.contains("rollback"));
                assert_eq!(
                    outcome.exit_code(),
                    if history {
                        control::ExitCode::Partial
                    } else {
                        control::ExitCode::OperationFailed
                    }
                );
            }
        }
    }
    fn external_public_tick(
        owner: &mut HereOwner,
        host: &mut Fake,
        original: HostError,
    ) -> HostError {
        // The production loop boundary and ControlWindow dispatch are real;
        // only NativeHere's borrowed X11 operations use the existing fake.
        finish_probe_run_with(Err(original), |original| {
            let mut original = Some(original);
            match owner.service(|active, _, _, reply| {
                fail_here_owner_with(active, host, reply, original.take().unwrap())
            }) {
                Err(original) => original,
                Ok(false) => original.unwrap(),
                Ok(true) => panic!("fatal finalization must return its original error"),
            }
        })
        .unwrap_err()
    }
    fn error_address(error: &HostError) -> *const () {
        &**error as *const dyn std::error::Error as *const ()
    }
    fn assert_fatal_terminal(owner: &mut HereOwner, history: bool) {
        let reply = public_terminal(
            owner,
            if history {
                Status::Partial
            } else {
                Status::Failed
            },
            history,
        );
        assert_eq!(reply.detail.reason, Reason::X11Failure);
        assert_eq!(reply.detail.stage, Stage::Shutdown);
        assert_eq!(
            reply.detail.placement.current,
            control::PlacementConfirmation::default()
        );
        assert!(!reply.detail.progress.input_ready);
        // Actual global cleanup uses finish_remote; neither Closing nor a
        // later selection-loss classification can emit a second response.
        owner.shutdown(true);
        owner.shutdown(false);
        assert!(owner.active().is_none());
        assert_eq!(owner.replies().len(), 1);
    }
    #[test]
    fn production_first_ownership_query_failure_is_failed_x11_and_clears_before_cleanup() {
        let mut host = Fake::new(true, false, L::Normal);
        let mut owner = public_owner(&mut host);
        host.faults.push(("owner", 1));
        let error = public_tick(&mut owner, &mut host).unwrap_err();
        assert_eq!(
            error.downcast_ref::<NativeFault>(),
            Some(&NativeFault("owner", 1))
        );
        assert_fatal_terminal(&mut owner, false);
        host.assert_no_mutation();
        assert_eq!(host.log, ["owner"]);
    }
    #[test]
    fn production_ownership_query_failure_after_dispatch_can_observe_history_once() {
        let mut host = Fake::new(true, false, L::Below);
        let mut owner = public_owner(&mut host);
        public_tick(&mut owner, &mut host).unwrap();
        assert!(
            !owner
                .active()
                .unwrap()
                .placement
                .proof
                .history
                .relocation_confirmed
        );
        let query = host.count("owner") + 1;
        let reads = host.count("layout");
        host.target();
        host.faults.push(("owner", query));
        let error = public_tick(&mut owner, &mut host).unwrap_err();
        assert_eq!(
            error.downcast_ref::<NativeFault>(),
            Some(&NativeFault("owner", query))
        );
        assert_fatal_terminal(&mut owner, true);
        assert_eq!(
            host.count("layout"),
            reads + 2,
            "one bounded Here evidence round"
        );
        assert_eq!(host.count("desktop"), 1);
        assert_eq!(host.count("correct"), 0);
        assert_eq!(host.count("shape"), 0);
    }
    #[test]
    fn production_first_ownership_mismatch_remains_ownership_lost_without_mutation() {
        let mut host = Fake::new(true, false, L::Normal);
        let mut owner = public_owner(&mut host);
        host.owned = false;
        assert!(public_tick(&mut owner, &mut host).is_err());
        let reply = public_terminal(&owner, Status::Failed, false);
        assert_eq!(reply.detail.reason, Reason::OwnershipLost);
        host.assert_no_mutation();
        assert_eq!(host.log, ["owner"]);
        owner.shutdown(false);
        assert_eq!(owner.replies().len(), 1);
    }
    #[test]
    fn external_async_before_mutation_preserves_original_error_and_replies_failed_once() {
        let mut host = Fake::new(true, false, L::Above);
        let mut owner = public_owner(&mut host);
        let original: HostError = Box::new(NativeFault("async before service", 1));
        let address = error_address(&original);
        let error = external_public_tick(&mut owner, &mut host, original);
        assert_eq!(error_address(&error), address);
        assert_eq!(
            error.downcast_ref::<NativeFault>(),
            Some(&NativeFault("async before service", 1))
        );
        assert_fatal_terminal(&mut owner, false);
        host.assert_no_mutation();
        assert!(
            host.log.is_empty(),
            "no baseline or dispatch permits no final evidence read"
        );
    }
    #[test]
    fn external_async_after_dispatch_uses_one_final_history_round_without_retry() {
        for (target, prior_history) in [(false, false), (true, false), (true, true)] {
            let mut host = Fake::new(true, false, L::Normal);
            let mut owner = public_owner(&mut host);
            public_tick(&mut owner, &mut host).unwrap();
            if target {
                host.target();
            }
            if prior_history {
                host.current.transitional = true;
                mark_remote_observation(owner.active_mut().unwrap(), false);
                public_tick(&mut owner, &mut host).unwrap();
                assert!(
                    owner
                        .active()
                        .unwrap()
                        .placement
                        .proof
                        .history
                        .relocation_confirmed
                );
            }
            let reads = host.count("layout");
            let original: HostError = Box::new(NativeFault("async after dispatch", 1));
            let address = error_address(&original);
            let error = external_public_tick(&mut owner, &mut host, original);
            assert_eq!(error_address(&error), address);
            assert_fatal_terminal(&mut owner, target);
            assert!(owner.replies()[0].detail.progress.host_mutation_dispatched);
            assert_eq!(host.count("layout"), reads + 2);
            assert_eq!(host.count("desktop"), 1);
            assert_eq!(host.count("correct"), 0);
            assert_eq!(host.count("shape"), 0);
        }
    }
    fn public_input_enable_boundary(owner: &mut HereOwner, host: &mut Fake, enabled: bool) {
        public_tick(owner, host).unwrap();
        host.target();
        let operation = owner.active_mut().unwrap();
        let (evidence, decision) = read_here(operation, host).unwrap();
        assert_eq!(decision, control::HereDecision::Destination);
        assert!(operation.placement.begin_here_geometry());
        assert!(operation
            .placement
            .confirm_here_geometry(host.actual.origin(), host.actual.size()));
        assert!(operation.consume_here_input_enable(host.now));
        if enabled {
            assert!(host
                .enable(&evidence, host.actual, operation.deadline)
                .unwrap());
            assert!(operation.here_input_restored());
        }
    }
    #[test]
    fn external_async_during_or_after_input_enable_refences_once_and_never_enables_again() {
        for enabled in [false, true] {
            let mut host = Fake::new(true, false, L::Above);
            let mut owner = public_owner(&mut host);
            public_input_enable_boundary(&mut owner, &mut host, enabled);
            let fences = host.count("fence");
            let original: HostError = Box::new(NativeFault("async enable boundary", 1));
            let address = error_address(&original);
            let error = external_public_tick(&mut owner, &mut host, original);
            assert_eq!(error_address(&error), address);
            assert_fatal_terminal(&mut owner, true);
            assert_eq!(host.count("fence"), fences + 1);
            assert_eq!(host.count("shape"), usize::from(enabled));
            assert_eq!(host.count("enable"), usize::from(enabled));
            assert_eq!(host.count("desktop"), 1);
            assert!(host.fence_settled());
            assert!(!host.input_active());
        }
    }
    #[test]
    fn external_error_wins_over_safety_final_read_and_reply_failures_together() {
        let mut host = Fake::new(true, false, L::Below);
        let mut owner = public_owner(&mut host);
        public_input_enable_boundary(&mut owner, &mut host, true);
        let empty = host.count("empty");
        let reads = host.count("layout");
        host.faults = vec![("empty", empty + 1), ("layout", reads + 1)];
        owner.fail_reply(true);
        let original: HostError = Box::new(NativeFault("original external error", 99));
        let address = error_address(&original);
        let error = external_public_tick(&mut owner, &mut host, original);
        assert_eq!(error_address(&error), address);
        assert_eq!(
            error.downcast_ref::<NativeFault>(),
            Some(&NativeFault("original external error", 99))
        );
        assert_fatal_terminal(&mut owner, true);
        assert_eq!(host.count("empty"), empty + 1);
        assert_eq!(host.count("release"), 2);
        assert_eq!(host.count("popup"), 2);
        assert_eq!(
            host.count("layout"),
            reads + 1,
            "failed read is never retried"
        );
        assert_eq!(host.count("desktop"), 1);
        assert_eq!(host.count("shape"), 1);
    }
    #[test]
    fn loop_boundary_keeps_clean_shutdown_and_proven_control_loss_out_of_fatal_policy() {
        for termination in [
            Some(ProbeTermination::DurationExpired),
            Some(ProbeTermination::WmClose),
            Some(ProbeTermination::ControlEndpointLost),
            None,
        ] {
            let mut host = Fake::new(true, false, L::Normal);
            let mut owner = public_owner(&mut host);
            public_tick(&mut owner, &mut host).unwrap();
            let result = finish_probe_run_with(Ok(termination), |_| {
                panic!("intentional lifecycle exit is not a native failure")
            });
            let lost = matches!(termination, Some(ProbeTermination::ControlEndpointLost));
            assert_eq!(result.is_err(), lost);
            owner.shutdown(!lost);
            let reply = public_terminal(&owner, Status::Failed, false);
            assert_eq!(
                reply.detail.reason,
                if lost {
                    Reason::OwnershipLost
                } else {
                    Reason::Closing
                }
            );
            assert_eq!(host.count("desktop"), 1);
        }
    }
    #[test]
    fn external_fatal_policy_leaves_legacy_commands_untouched() {
        for command in [Command::Hide, Command::Show, Command::BringTop] {
            let mut host = Fake::new(true, false, L::Normal);
            let mut active = None;
            let destination = if command == Command::Recover {
                2
            } else {
                control::NO_DESTINATION
            };
            control::admit(
                &mut active,
                control::Request {
                    command,
                    request_id: 99,
                    reply_xid: 88,
                    epoch: 7,
                },
                destination,
                control::Lifecycle::Ready,
                false,
                host.now,
            );
            let original_active = active;
            let original: HostError = Box::new(NativeFault("legacy external error", 1));
            let address = error_address(&original);
            let error = finish_probe_run_with(Err(original), |error| {
                fail_here_owner_with(
                    &mut active,
                    &mut host,
                    |_| panic!("no Here reply for another command"),
                    error,
                )
                .unwrap_err()
            })
            .unwrap_err();
            assert_eq!(error_address(&error), address);
            assert_eq!(active, original_active);
            assert!(host.log.is_empty());
            host.assert_no_mutation();
        }
    }
    #[test]
    fn every_production_async_error_return_is_inside_the_shared_fatal_loop_boundary() {
        let source = include_str!("main.rs");
        let probe = source
            .split("fn run_probe(")
            .nth(1)
            .unwrap()
            .split("fn finish_probe_run_with(")
            .next()
            .unwrap();
        let loop_start = probe.find("let mut outcome = (|| loop {").unwrap();
        let loop_end = probe.rfind("    })();").unwrap();
        let sites: Vec<_> = probe.match_indices("Event::Error(error)").collect();
        assert_eq!(
            sites.len(),
            4,
            "buffered, batch, later routing, final match"
        );
        for (site, _) in sites {
            assert!(site > loop_start && site < loop_end);
            assert!(probe[site..].starts_with("Event::Error(error)"));
        }
        assert_eq!(
            probe[loop_start..loop_end]
                .matches("return Err(format!(\"Asynchronous X11 error:")
                .count(),
            4
        );
        assert!(probe[loop_end..].contains("finish_probe_run_with(outcome"));
        assert!(probe[loop_end..].contains("fail_owner_operation("));
        assert!(!probe[loop_start..loop_end].contains("return probe_termination_result("));
        let service = source
            .split("fn service_owner_operation<")
            .nth(1)
            .unwrap()
            .split("fn fail_owner_operation<")
            .next()
            .unwrap();
        assert!(
            service.find("owner.service_here_with").unwrap()
                < service.find("owner.verify_active_ownership").unwrap()
        );
        assert_eq!(service.matches("owner.verify_active_ownership").count(), 1);
    }

    pub(crate) mod recover_tests {
        use super::*;
        use control::{RecoverPhase, Visibility};
        use std::cell::{Cell, RefCell};

        struct Host {
            here: Fake,
            layer_changes: bool,
            mutations: Vec<Mutation>,
            fail_before_cookie: Option<&'static str>,
        }
        impl Host {
            fn new(remote: bool, minimized: bool, layer: L) -> Self {
                Self {
                    here: Fake::new(remote, minimized, layer),
                    layer_changes: true,
                    mutations: vec![],
                    fail_before_cookie: None,
                }
            }
            fn operation(&self) -> ActiveOperation {
                let mut active = None;
                assert_eq!(
                    control::admit(
                        &mut active,
                        control::Request {
                            command: Command::Recover,
                            request_id: 99,
                            reply_xid: 9,
                            epoch: 11
                        },
                        2,
                        control::Lifecycle::Ready,
                        false,
                        self.here.now,
                    ),
                    control::Admission::Admitted
                );
                active.unwrap()
            }
            fn run(&mut self, op: &mut ActiveOperation) -> Option<control::OperationResult> {
                service_recover_with(op, self).unwrap()
            }
            fn restored(&mut self) {
                self.here.current.minimized = false;
                self.here.current.transitional = false;
            }
            fn atoms() -> LayerAtoms {
                LayerAtoms {
                    state: 12,
                    supported: 18,
                    above: 14,
                    below: 15,
                }
            }
        }
        impl BringHereHost for Host {
            fn now(&self) -> Instant {
                self.here.now
            }
            fn owned(&mut self) -> Result<bool, HostError> {
                self.here.owned()
            }
            fn live(&self) -> bool {
                self.here.live()
            }
            fn evidence(&mut self, destination: u32) -> Result<HereEvidence, HostError> {
                self.here.evidence(destination)
            }
            fn placement_busy(&self) -> bool {
                self.here.placement_busy()
            }
            fn input_active(&self) -> bool {
                self.here.input_active()
            }
            fn visible_ready(&self) -> bool {
                self.here.visible_ready()
            }
            fn fence_settled(&self) -> bool {
                self.here.fence_settled()
            }
            fn fence(&mut self) -> Result<(), HostError> {
                self.here.fence()
            }
            fn abort_recovery_layer(&mut self) {
                self.here.runtime.layers.abort();
            }
            fn desktop(
                &mut self,
                destination: u32,
                deadline: Instant,
                dispatched: &mut bool,
            ) -> Result<bool, HostError> {
                self.here.desktop(destination, deadline, dispatched)
            }
            fn geometry(&mut self) -> Result<Rect, HostError> {
                self.here.geometry()
            }
            fn restrict_automatic_correction(&mut self) {
                self.here.restrict_automatic_correction();
            }
            fn correct(
                &mut self,
                target: Point,
                deadline: Instant,
                dispatched: &mut bool,
            ) -> Result<Option<Instant>, HostError> {
                self.here.correct(target, deadline, dispatched)
            }
            fn enable(
                &mut self,
                evidence: &HereEvidence,
                geometry: Rect,
                deadline: Instant,
            ) -> Result<bool, HostError> {
                self.here.enable(evidence, geometry, deadline)
            }
        }
        impl RecoverHost for Host {
            type Top = Self;
            fn top(&mut self) -> &mut Self {
                self
            }
        }
        impl VisibilityHost for Host {
            fn now(&self) -> Instant {
                self.here.now
            }
            fn layer_atoms(&self) -> LayerAtoms {
                Self::atoms()
            }
            fn observe(&mut self) -> Result<x11::visibility::VisibilitySnapshot, HostError> {
                self.here.step("top_observe")?;
                Ok(self.here.current.snapshot())
            }
            fn noop_ready(&self) -> bool {
                self.here.visible_ready()
            }
            fn placement_busy(&self) -> bool {
                self.here.placement_busy()
            }
            fn fenced(&self) -> bool {
                self.here.runtime.input_fence.blocked
            }
            fn fence_settled(&self) -> bool {
                self.here.fence_settled()
            }
            fn deferred_placement(&mut self, _: &ShowPlacement) -> Result<bool, HostError> {
                panic!("Recover never defers local readiness")
            }
            fn fence(&mut self) -> Result<(), HostError> {
                self.here.fence()
            }
            fn iconify(&mut self, _: Instant, _: &mut bool) -> Result<bool, HostError> {
                panic!("Recover never hides")
            }
            fn restore(&mut self, _: Instant, _: &mut bool) -> Result<bool, HostError> {
                panic!("Recover requires guarded restore")
            }
            fn hidden(&mut self, _: bool, _: Instant, _: &mut bool) -> Result<bool, HostError> {
                panic!("Recover never accepts off-workspace Top")
            }
            fn placement(&mut self) -> Result<ShowPlacement, HostError> {
                panic!("Recover must not use legacy placement")
            }
            fn correct(
                &mut self,
                target: Point,
                deadline: Instant,
                dispatched: &mut bool,
            ) -> Result<Option<Instant>, HostError> {
                self.here.correct(target, deadline, dispatched)
            }
            fn enable(
                &mut self,
                snapshot: &x11::visibility::VisibilitySnapshot,
                input: &InputPlacement,
                deadline: Option<Instant>,
            ) -> Result<bool, HostError> {
                self.here.step("top_enable")?;
                if deadline.is_some_and(|dl| self.here.now >= dl) {
                    return Ok(false);
                }
                let shape = self.here.step("shape");
                self.here.window.confirmed_origin = input.geometry.origin();
                self.here.window.cancel_in_flight_moves();
                self.here.runtime.body = BodyAvailability::Ready;
                self.here.runtime.restore_visibility_input_with(
                    &self.here.window,
                    snapshot.evidence,
                    input,
                    || shape.map(|()| 2000),
                )?;
                Ok(true)
            }
            fn recover_owned(&mut self) -> Result<bool, HostError> {
                self.here.owned()
            }
            fn recover_evidence(&mut self, destination: u32) -> Result<HereEvidence, HostError> {
                self.here.evidence(destination)
            }
            fn recover_geometry(
                &mut self,
                _: control::DestinationIdentity,
            ) -> Result<Rect, HostError> {
                self.here.geometry()
            }
            fn restrict_recover_correction(&mut self) {
                self.here.restrict_automatic_correction();
            }
            fn restore_recover(
                &mut self,
                op: &mut ActiveOperation,
                deadline: Instant,
                dispatched: &mut bool,
            ) -> Result<bool, HostError> {
                let clock = Cell::new(self.here.now);
                let host = RefCell::new(&mut *self);
                let atoms = x11::visibility::VisibilityAtoms {
                    wm_state: 10,
                    change_state: 11,
                    state: 12,
                    hidden: 13,
                    above: 14,
                    below: 15,
                    desktop: 16,
                    current_desktop: 17,
                    supported: 18,
                    user_time: 19,
                };
                atoms.restore_guarded_before_with(
                    1,
                    deadline,
                    dispatched,
                    || clock.get(),
                    |request, sent| {
                        let mut h = host.borrow_mut();
                        let name = match request {
                            x11::visibility::VisibilityRequest::UserTime { .. } => "zero",
                            x11::visibility::VisibilityRequest::Map { .. } => "map",
                            _ => panic!("Unexpected restoration mutation"),
                        };
                        if h.fail_before_cookie != Some(name) {
                            *sent = true;
                        }
                        let result = h.here.step(name);
                        clock.set(h.here.now);
                        result
                    },
                    Some(|| {
                        let mut h = host.borrow_mut();
                        h.here.step("restore_guard")?;
                        let allowed = recover_visibility_guard(op, &mut **h)?
                            && op.detail.visibility == Visibility::Minimized;
                        clock.set(h.here.now);
                        Ok(allowed)
                    }),
                )
            }
        }
        impl BringTopHost for Host {
            fn qualify_layer(
                &self,
                snapshot: &x11::visibility::VisibilitySnapshot,
                baseline: control::VisibilityBaseline,
            ) -> Result<ObservedLayer, RemoteLayerIssue> {
                Self::atoms().qualify_remote(snapshot, baseline)
            }
            fn service_above(
                &mut self,
                op: &mut ActiveOperation,
            ) -> Result<(LayerOutcome, Option<RemoteLayerIssue>), HostError> {
                let baseline = op.show.baseline.unwrap();
                let mut layers = std::mem::take(&mut self.here.runtime.layers);
                let host = RefCell::new(&mut *self);
                let result = service_remote_layer_with(
                    &mut layers,
                    op,
                    |op| {
                        let mut h = host.borrow_mut();
                        h.here.step("layer_read")?;
                        let (evidence, decision) = read_recover_top(op, &mut **h)?;
                        Ok(match decision {
                            control::HereDecision::Destination => {
                                Self::atoms().qualify_remote(&evidence.snapshot, baseline)
                            }
                            control::HereDecision::Pending => Err(RemoteLayerIssue::Waiting),
                            _ => Err(RemoteLayerIssue::WorkspaceChanged),
                        })
                    },
                    || {
                        host.borrow_mut().here.step("support")?;
                        Ok(Ok(LayerSupport {
                            state: true,
                            above: true,
                            below: true,
                        }))
                    },
                    |mutation, op, dispatched| {
                        let mut h = host.borrow_mut();
                        h.here.step("layer_guard")?;
                        if !recover_layer_dispatch_eligible(op, &mut **h)? {
                            return Ok(false);
                        }
                        let step = match mutation {
                            Mutation::Remove(_) => "remove",
                            _ => "above",
                        };
                        if h.fail_before_cookie != Some(step) {
                            *dispatched = true;
                        }
                        h.here.step(match mutation {
                            Mutation::Remove(_) => "remove",
                            Mutation::AddAbove => "above",
                            _ => panic!("Above only"),
                        })?;
                        h.mutations.push(mutation);
                        if h.layer_changes {
                            h.here.current.layer = match mutation {
                                Mutation::Remove(_) => L::Normal,
                                _ => L::Above,
                            };
                        }
                        Ok(true)
                    },
                    || host.borrow().here.now,
                );
                self.here.runtime.layers = layers;
                result
            }
            fn abort_above(&mut self) {
                self.here.runtime.layers.abort();
            }
            fn layer_deadline(&self) -> Option<Instant> {
                self.here.runtime.layers.deadline()
            }
        }

        fn owner(h: &Host) -> HereOwner {
            HereOwner::recover(h.here.now, 2)
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(crate) enum PublicFault {
            None,
            Layout,
            AfterRelocation,
            AfterRestoration,
            AfterBoth,
            OwnerLost,
            Native,
            NativeDispatch,
        }

        /// Borrow the actually admitted public slot into the production dispatcher
        /// and scheduler. Only WM observations and native resources are injected.
        pub(crate) fn public_roundtrip_service(
            active: &mut Option<ActiveOperation>,
            remote: bool,
            minimized: bool,
            layer: L,
            fault: PublicFault,
            reply: &mut dyn FnMut(control::OperationResult) -> Result<(), HostError>,
        ) -> Result<(), HostError> {
            let mut h = Host::new(remote, minimized, layer);
            h.here.window.window = 88;
            let original = *active.as_ref().unwrap();
            h.here.now = original.deadline - Command::Recover.owner_budget();
            assert_eq!(original.request.command, Command::Recover);
            assert_eq!(original.placement.destination_desktop, 2);
            assert_eq!(original.stage, Stage::Placement);
            match fault {
                PublicFault::Layout => h.here.layout = Err(DestinationError::Changed),
                PublicFault::OwnerLost => h.here.owned = false,
                PublicFault::Native => h.here.faults.push(("layout", 1)),
                PublicFault::NativeDispatch => h.here.faults.push(("desktop", 1)),
                PublicFault::AfterRestoration | PublicFault::AfterBoth => h.layer_changes = false,
                _ => {}
            }
            let mut injected = false;
            for _ in 0..16 {
                let Some(op) = active.as_mut() else {
                    assert_eq!(h.here.window.window, 88);
                    assert!(h.here.count("desktop") <= 1);
                    assert!(h.here.count("map") <= 1);
                    assert!(h.here.count("correct") <= 1);
                    return Ok(());
                };
                assert_eq!(op.request, original.request);
                assert_eq!(op.deadline, original.deadline);
                let history = op.placement.proof.history;
                let fail_now = match fault {
                    PublicFault::AfterRelocation => history.relocation_confirmed,
                    PublicFault::AfterRestoration => history.restoration_confirmed,
                    PublicFault::AfterBoth => {
                        history.relocation_confirmed && history.restoration_confirmed
                    }
                    _ => false,
                };
                if fail_now && !injected {
                    h.here.layout = Err(DestinationError::Changed);
                    mark_remote_observation(op, false);
                    injected = true;
                }
                if !remote_service_due(op, h.here.now) {
                    // A real WM notification supplies the next fresh observation.
                    if op.recover_phase == RecoverPhase::Here && h.here.count("desktop") > 0 {
                        h.here.target();
                    } else if h.here.count("map") > 0 && fault != PublicFault::AfterRelocation {
                        h.restored();
                    }
                    mark_remote_observation(op, false);
                }
                let outcome = service_admitted_placement_with(active, &mut h, &mut *reply);
                if let Err(error) = outcome {
                    // Fatal owner errors still send their one correlated terminal
                    // result before propagating to global cleanup.
                    assert!(active.is_none());
                    if !matches!(
                        fault,
                        PublicFault::Native | PublicFault::NativeDispatch | PublicFault::OwnerLost
                    ) {
                        return Err(error);
                    }
                }
            }
            panic!("public Recover did not terminate: {fault:?}");
        }

        fn tick(owner: &mut HereOwner, h: &mut Host) -> Result<(), HostError> {
            owner.service(|active, _, _, reply| service_recover_owner_with(active, h, reply))?;
            Ok(())
        }

        fn wake(owner: &mut HereOwner) {
            mark_remote_observation(owner.active_mut().unwrap(), false);
        }

        fn finish_owner(owner: &mut HereOwner, h: &mut Host) -> Result<(), HostError> {
            for _ in 0..8 {
                if owner.active().is_none() {
                    return Ok(());
                }
                tick(owner, h)?;
                if let Some(op) = owner.active_mut() {
                    if let Some((_, deadline)) = op.show.correction.or_else(|| {
                        (op.recover_phase == RecoverPhase::Here)
                            .then(|| op.placement.correction())
                            .flatten()
                    }) {
                        h.here.now = h.here.now.max(deadline);
                    }
                    if op.recover_phase == RecoverPhase::Here && h.here.count("desktop") > 0 {
                        h.here.target();
                    } else if h.here.count("map") > 0 {
                        h.restored();
                    }
                    mark_remote_observation(op, false);
                }
            }
            panic!("Recover owner failed to terminate");
        }

        fn assert_terminal(
            owner: &mut HereOwner,
            h: &Host,
            reason: Reason,
        ) -> control::TerminalResponse {
            assert!(owner.active().is_none());
            let replies = owner.replies();
            assert_eq!(replies.len(), 1);
            let reply = replies[0];
            assert_eq!(reply.detail.reason, reason);
            if reply.status != Status::Success {
                assert_eq!(
                    reply.detail.placement.current,
                    control::PlacementConfirmation::default()
                );
                assert!(!reply.detail.progress.input_ready);
                assert!(!reply.detail.progress.above_confirmed);
                assert!(!reply.detail.progress.not_minimized_confirmed);
                let history = reply.detail.placement.history;
                assert_eq!(
                    reply.status,
                    if history.relocation_confirmed || history.restoration_confirmed {
                        Status::Partial
                    } else {
                        Status::Failed
                    }
                );
            }
            assert!(h.here.runtime.layers.pending().is_none());
            owner.shutdown(true);
            owner.shutdown(false);
            assert_eq!(
                owner.replies(),
                replies,
                "global cleanup cannot reply twice"
            );
            reply
        }

        #[test]
        fn actual_owner_handoff_progresses_before_sleep_and_then_waits_without_polling() {
            let mut h = Host::new(true, true, L::Below);
            let mut owner = owner(&h);
            let total = owner.active().unwrap().deadline;
            tick(&mut owner, &mut h).unwrap();
            tick(&mut owner, &mut h).unwrap(); // settle possible-dispatch observation
            assert!(!remote_service_due(owner.active().unwrap(), h.here.now));
            let log = h.here.log.clone();
            for _ in 0..8 {
                tick(&mut owner, &mut h).unwrap();
            }
            assert_eq!(h.here.log, log);
            h.here.target();
            wake(&mut owner);
            tick(&mut owner, &mut h).unwrap();
            let op = owner.active().unwrap();
            assert_eq!(op.recover_phase, RecoverPhase::Top);
            assert_eq!(
                h.here.count("map"),
                1,
                "handoff starts Top in the same service turn"
            );
            assert_eq!(op.deadline, total);
            assert_eq!(
                remote_operation_deadline(op),
                op.bring_top.visibility_cutoff
            );
            assert!(!remote_service_due(op, h.here.now));
            let log = h.here.log.clone();
            tick(&mut owner, &mut h).unwrap();
            assert_eq!(h.here.log, log);
            h.restored();
            wake(&mut owner);
            finish_owner(&mut owner, &mut h).unwrap();
            let reply = assert_terminal(&mut owner, &h, Reason::None);
            assert_eq!(reply.status, Status::Success);
            assert_eq!(
                reply.detail.placement.history,
                control::RecoveryHistory {
                    relocation_confirmed: true,
                    restoration_confirmed: true,
                }
            );
            assert!(local_layer_service_allowed(owner.active()));
        }

        #[test]
        fn actual_scheduler_honors_each_phase_and_correction_boundary_exactly() {
            for top in [false, true] {
                let mut h = Host::new(false, top, L::Normal);
                h.here.refuse_correction = true;
                let mut owner = owner(&h);
                if top {
                    tick(&mut owner, &mut h).unwrap();
                    h.restored();
                    wake(&mut owner);
                }
                h.here.actual.x = -100;
                tick(&mut owner, &mut h).unwrap();
                tick(&mut owner, &mut h).unwrap();
                let op = owner.active().unwrap();
                let cutoff = if top {
                    op.bring_top.visibility_cutoff
                } else {
                    here_cutoff(op)
                };
                let correction = op.placement.correction().unwrap().1;
                assert!(correction < cutoff);
                assert_eq!(remote_operation_deadline(op), correction);
                assert!(!remote_service_due(
                    op,
                    correction - Duration::from_nanos(1)
                ));
                assert!(remote_service_due(op, correction));
                h.here.now = correction;
                tick(&mut owner, &mut h).unwrap();
                assert_terminal(&mut owner, &h, Reason::Unverifiable);
                assert_eq!(h.here.count("correct"), 1);
            }
            for phase in 0..3 {
                let mut h = Host::new(phase == 0, phase == 1, L::Normal);
                h.layer_changes = false;
                let mut owner = owner(&h);
                tick(&mut owner, &mut h).unwrap();
                tick(&mut owner, &mut h).unwrap();
                let op = owner.active().unwrap();
                let cutoff = match phase {
                    0 => op.deadline - Duration::from_millis(3500),
                    1 => op.bring_top.visibility_cutoff,
                    _ => op.recover_layer_cutoff().unwrap(),
                };
                assert_eq!(remote_operation_deadline(op), cutoff);
                assert!(!remote_service_due(op, cutoff - Duration::from_nanos(1)));
                assert!(remote_service_due(op, cutoff));
                h.here.now = cutoff;
                tick(&mut owner, &mut h).unwrap();
                assert_terminal(&mut owner, &h, Reason::ConfirmationTimeout);
            }
        }

        #[test]
        fn strict_event_families_are_phase_owned_and_layer_configure_reconciles_movement() {
            let (visibility, layout_atoms) = atoms();
            for phase in 0..3 {
                let mut h = Host::new(phase == 0, phase == 1, L::Normal);
                h.layer_changes = false;
                let mut owner = owner(&h);
                tick(&mut owner, &mut h).unwrap();
                tick(&mut owner, &mut h).unwrap();
                let original = *owner.active().unwrap();
                assert!(!remote_service_due(&original, h.here.now));
                assert!(!local_layer_service_allowed(Some(&original)));
                assert_eq!(remote_owns_placement(&original), phase != 2);
                assert_eq!(remote_owns_layer(&original), phase == 2);
                for kind in 0..15 {
                    let mut op = original;
                    let e = if kind == 14 {
                        Event::RandrNotify(x11rb::protocol::randr::NotifyEvent {
                            response_type: 0,
                            sub_code: x11rb::protocol::randr::Notify::RESOURCE_CHANGE,
                            sequence: 0,
                            u: x11rb::protocol::randr::ResourceChange {
                                timestamp: 0,
                                window: 1,
                            }
                            .into(),
                        })
                    } else {
                        event(kind)
                    };
                    let consumed = route_here_evidence_event(
                        Some(&mut op),
                        &e,
                        1,
                        100,
                        &visibility,
                        &layout_atoms,
                    );
                    assert_eq!(
                        consumed,
                        !(phase == 2 && kind == 5),
                        "phase={phase} kind={kind}"
                    );
                    assert!(remote_service_due(&op, h.here.now));
                    assert_eq!(op.deadline, original.deadline);
                    assert_eq!(
                        op.bring_top.layer_admitted,
                        original.bring_top.layer_admitted
                    );
                    assert_eq!(op.placement.proof.history, original.placement.proof.history);
                    assert_eq!(
                        op.placement.proof.current.geometry,
                        control::GeometryProof::Unknown
                    );
                    assert!(!op.detail.progress.input_ready);
                    assert!(
                        !op.show.layout_interrupted,
                        "equal strict layout must be re-read, not rejected"
                    );
                }
                for e in [property(999, 22), property(1, 999)] {
                    let mut op = original;
                    assert!(!route_here_evidence_event(
                        Some(&mut op),
                        &e,
                        1,
                        100,
                        &visibility,
                        &layout_atoms
                    ));
                    assert!(!remote_service_due(&op, h.here.now));
                }
            }
        }

        #[test]
        fn owner_native_fault_matrix_keeps_original_error_one_read_one_reply_and_no_retry() {
            for corrected in [false, true] {
                let mut reference = Host::new(!corrected, true, L::Below);
                if corrected {
                    reference.here.actual.x = -100;
                }
                let mut reference_owner = owner(&reference);
                finish_owner(&mut reference_owner, &mut reference).unwrap();
                let mut counts = std::collections::HashMap::new();
                for &step in reference
                    .here
                    .log
                    .iter()
                    .filter(|&&s| !["fence", "restrict", "popup"].contains(&s))
                {
                    let nth = counts.entry(step).or_insert(0);
                    *nth += 1;
                    // Includes ownership at/after handoff, user-time/map, every
                    // strict read, correction, SHAPE, removal/addition and final proof.
                    let mut h = Host::new(!corrected, true, L::Below);
                    if corrected {
                        h.here.actual.x = -100;
                    }
                    h.here.faults.push((step, *nth));
                    let mut owner = owner(&h);
                    let error = finish_owner(&mut owner, &mut h)
                        .err()
                        .unwrap_or_else(|| panic!("fault not reached: {step}/{nth}"));
                    assert_eq!(
                        error.downcast_ref::<NativeFault>(),
                        Some(&NativeFault(step, *nth))
                    );
                    let index = h
                        .here
                        .log
                        .iter()
                        .enumerate()
                        .filter(|(_, s)| **s == step)
                        .nth(*nth - 1)
                        .unwrap()
                        .0;
                    let after = &h.here.log[index + 1..];
                    assert!(
                        !after.iter().any(|s| [
                            "desktop", "zero", "map", "correct", "shape", "remove", "above"
                        ]
                        .contains(s)),
                        "retry after {step}/{nth}: {after:?}"
                    );
                    assert!(
                        after.iter().filter(|&&s| s == "observe").count() <= 1,
                        "one history round"
                    );
                    assert!(after.iter().filter(|&&s| s == "layout").count() <= 2);
                    assert!(
                        after.iter().filter(|&&s| s == "fence").count() <= 1,
                        "one safety cleanup"
                    );
                    let reply = assert_terminal(&mut owner, &h, Reason::X11Failure);
                    assert_eq!(reply.detail.stage, Stage::Shutdown);
                }
            }
        }

        #[test]
        fn every_owner_query_mismatch_stops_mutation_and_is_distinct_from_native_failure() {
            let mut reference = Host::new(true, true, L::Below);
            let mut reference_owner = owner(&reference);
            finish_owner(&mut reference_owner, &mut reference).unwrap();
            for nth in 1..=reference.here.count("owner") {
                let mut h = Host::new(true, true, L::Below);
                h.here.lose_at = Some(("owner", nth));
                let mut owner = owner(&h);
                assert!(finish_owner(&mut owner, &mut h).is_err());
                assert_terminal(&mut owner, &h, Reason::OwnershipLost);
                let index = h
                    .here
                    .log
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| **s == "owner")
                    .nth(nth - 1)
                    .unwrap()
                    .0;
                assert!(
                    !h.here.log[index + 1..].iter().any(|s| [
                        "empty", "desktop", "zero", "map", "correct", "shape", "remove", "above"
                    ]
                    .contains(s)),
                    "mutation after ownership loss #{nth}"
                );
            }
        }

        /// Real owner turns prepare boundaries; no public request can seed them.
        fn checkpoint(phase: usize) -> (Host, HereOwner) {
            let mut h = Host::new(true, phase != 2, L::Below);
            h.layer_changes = false;
            let mut owner = owner(&h);
            if phase == 0 {
                return (h, owner);
            }
            tick(&mut owner, &mut h).unwrap();
            if phase == 1 {
                return (h, owner);
            }
            h.here.target();
            if phase == 2 {
                h.here.current.transitional = true;
            }
            wake(&mut owner);
            tick(&mut owner, &mut h).unwrap();
            if phase <= 3 {
                return (h, owner);
            }
            h.restored();
            if phase == 4 {
                h.here.actual.x = -100;
            }
            wake(&mut owner);
            tick(&mut owner, &mut h).unwrap();
            if phase <= 5 {
                return (h, owner);
            }
            h.here.current.layer = L::Normal;
            wake(&mut owner);
            tick(&mut owner, &mut h).unwrap();
            if phase == 7 {
                h.here.current.layer = L::Above;
            }
            (h, owner)
        }

        fn external(owner: &mut HereOwner, h: &mut Host, original: HostError) -> HostError {
            finish_probe_run_with(Err(original), |original| {
                let mut original = Some(original);
                match owner.service(|active, _, _, reply| {
                    fail_here_owner_with(active, h, reply, original.take().unwrap())
                }) {
                    Err(original) => original,
                    Ok(false) => original.unwrap(),
                    Ok(true) => panic!("fatal owner must return its original error"),
                }
            })
            .unwrap_err()
        }

        #[test]
        fn external_fatal_matrix_preserves_original_error_history_and_one_bounded_read() {
            for phase in 0..8 {
                for boundary in ["async", "buffered error", "batch read", "poll", "flush"] {
                    for fail_reply in [false, true] {
                        let (mut h, mut owner) = checkpoint(phase);
                        let prior = h.here.log.len();
                        owner.fail_reply(fail_reply);
                        let original: HostError = Box::new(NativeFault(boundary, phase));
                        let address = error_address(&original);
                        let error = external(&mut owner, &mut h, original);
                        assert_eq!(error_address(&error), address);
                        let after = &h.here.log[prior..];
                        assert!(!after.iter().any(|s| [
                            "desktop", "zero", "map", "correct", "shape", "remove", "above"
                        ]
                        .contains(s)));
                        assert_eq!(
                            after.iter().filter(|&&s| s == "observe").count(),
                            usize::from(phase != 0)
                        );
                        assert!(after.iter().filter(|&&s| s == "fence").count() <= 1);
                        let reply = assert_terminal(&mut owner, &h, Reason::X11Failure);
                        assert_eq!(reply.detail.stage, Stage::Shutdown);
                        assert_eq!(
                            reply.detail.placement.history.relocation_confirmed,
                            phase >= 2
                        );
                        assert_eq!(
                            reply.detail.placement.history.restoration_confirmed,
                            phase >= 4
                        );
                    }
                }
            }
        }

        #[test]
        fn fatal_history_uses_here_preservation_and_top_restoration_interpretations() {
            // Here can independently confirm unexpected restoration, but never hand off.
            let (mut h, mut owner) = checkpoint(1);
            h.restored();
            let original: HostError = Box::new(NativeFault("Here restoration", 1));
            external(&mut owner, &mut h, original);
            let reply = assert_terminal(&mut owner, &h, Reason::X11Failure);
            assert!(!reply.detail.placement.history.relocation_confirmed);
            assert!(reply.detail.placement.history.restoration_confirmed);
            assert_eq!(h.here.count("zero"), 0);
            assert_eq!(h.here.count("above"), 0);

            // Possibly dispatched relocation can first be confirmed on the fatal read.
            let (mut h, mut owner) = checkpoint(1);
            h.here.target();
            external(
                &mut owner,
                &mut h,
                Box::new(NativeFault("Here relocation", 1)),
            );
            assert!(
                assert_terminal(&mut owner, &h, Reason::X11Failure)
                    .detail
                    .placement
                    .history
                    .relocation_confirmed
            );

            // Top's single read can first establish restoration after a map.
            let (mut h, mut owner) = checkpoint(3);
            h.restored();
            external(
                &mut owner,
                &mut h,
                Box::new(NativeFault("Top restoration", 1)),
            );
            let reply = assert_terminal(&mut owner, &h, Reason::X11Failure);
            assert!(reply.detail.placement.history.restoration_confirmed);
            assert_eq!(h.here.count("map"), 1);
            assert_eq!(h.here.count("shape"), 0);
        }

        #[test]
        fn fatal_cleanup_read_reply_failure_combinations_keep_original_and_do_not_retry() {
            for cleanup_fails in [false, true] {
                for history_fails in [false, true] {
                    for reply_fails in [false, true] {
                        let (mut h, mut owner) = checkpoint(5);
                        let fences = h.here.count("fence");
                        if cleanup_fails {
                            h.here.faults.push(("empty", h.here.count("empty") + 1));
                        }
                        if history_fails {
                            h.here.faults.push(("layout", h.here.count("layout") + 1));
                        }
                        owner.fail_reply(reply_fails);
                        let original: HostError = Box::new(NativeFault("primary", 7));
                        let address = error_address(&original);
                        let error = external(&mut owner, &mut h, original);
                        assert_eq!(error_address(&error), address);
                        assert_eq!(h.here.count("fence"), fences + 1);
                        assert_terminal(&mut owner, &h, Reason::X11Failure);
                    }
                }
            }
        }

        #[test]
        fn fatal_history_is_skipped_without_live_body_or_trustworthy_ownership() {
            for destroyed in [false, true] {
                let (mut h, mut owner) = checkpoint(5);
                if destroyed {
                    h.here.runtime.unavailable(true);
                } else {
                    h.here.owned = false;
                }
                let reads = h.here.count("observe");
                external(&mut owner, &mut h, Box::new(NativeFault("outer", 1)));
                assert_eq!(h.here.count("observe"), reads);
                assert_terminal(&mut owner, &h, Reason::X11Failure);
            }
        }

        #[test]
        fn native_failure_after_ordinary_invalidation_has_x11_diagnostic_priority() {
            let (mut h, mut owner) = checkpoint(5);
            owner
                .active_mut()
                .unwrap()
                .placement
                .invalidate_here(Reason::LayoutChanged);
            external(
                &mut owner,
                &mut h,
                Box::new(NativeFault("outer after drift", 1)),
            );
            assert_terminal(&mut owner, &h, Reason::X11Failure);
        }

        #[test]
        fn orderly_termination_preserves_recover_history_and_never_enters_fatal_policy() {
            for phase in 0..8 {
                for termination in [
                    Some(ProbeTermination::WmClose),
                    Some(ProbeTermination::DurationExpired),
                    Some(ProbeTermination::ControlEndpointLost),
                    None,
                ] {
                    let (h, mut owner) = checkpoint(phase);
                    let history = owner.active().unwrap().placement.proof.history;
                    let lost = matches!(termination, Some(ProbeTermination::ControlEndpointLost));
                    assert_eq!(
                        finish_probe_run_with(Ok(termination), |_| panic!(
                            "orderly shutdown is not a native failure"
                        ))
                        .is_err(),
                        lost
                    );
                    owner.shutdown(!lost);
                    let reply = owner.replies()[0];
                    assert_eq!(reply.detail.placement.history, history);
                    assert_eq!(
                        reply.detail.reason,
                        if lost {
                            Reason::OwnershipLost
                        } else {
                            Reason::Closing
                        }
                    );
                    assert_eq!(
                        reply.detail.placement.current,
                        control::PlacementConfirmation::default()
                    );
                    assert!(!reply.detail.progress.input_ready);
                    assert!(owner.active().is_none());
                    owner.shutdown(true);
                    assert_eq!(owner.replies().len(), 1);
                    assert_eq!(h.here.count("desktop"), usize::from(phase > 0));
                }
            }
        }

        #[test]
        fn body_destruction_is_nonfatal_and_does_not_become_x11_failure() {
            for phase in 0..8 {
                let (mut h, mut owner) = checkpoint(phase);
                h.here.runtime.unavailable(true);
                h.here.terminal = Some(Visibility::Destroyed);
                wake(&mut owner);
                tick(&mut owner, &mut h).unwrap();
                assert_terminal(&mut owner, &h, Reason::BodyDestroyed);
            }
        }

        #[test]
        fn incoming_transport_owner_loss_and_query_error_use_the_same_owner_terminal_policy() {
            for phase in [1, 3, 5, 6] {
                for during_validation in [false, true] {
                    for query_error in [false, true] {
                        let (mut h, mut owner) = checkpoint(phase);
                        let prior = h.here.log.len();
                        let original =
                            owner.request_ownership_failure(during_validation, query_error);
                        let address = error_address(&original);
                        let error = external(&mut owner, &mut h, original);
                        assert_eq!(error_address(&error), address);
                        assert_terminal(
                            &mut owner,
                            &h,
                            if query_error {
                                Reason::X11Failure
                            } else {
                                Reason::OwnershipLost
                            },
                        );
                        if !query_error {
                            assert_eq!(h.here.log.len(), prior);
                        }
                    }
                }
            }
        }

        #[test]
        fn expected_layer_transitions_and_configure_require_fresh_final_geometry() {
            let (mut h, mut owner) = checkpoint(5);
            let (visibility, layout_atoms) = atoms();
            let deadline = owner.active().unwrap().deadline;
            h.here.current.layer = L::Normal;
            assert!(route_here_evidence_event(
                owner.active_mut(),
                &event(1),
                1,
                100,
                &visibility,
                &layout_atoms
            ));
            tick(&mut owner, &mut h).unwrap();
            assert_eq!(h.here.count("above"), 1);
            assert_eq!(owner.active().unwrap().deadline, deadline);
            let op = owner.active_mut().unwrap();
            op.placement.proof.current.geometry = control::GeometryProof::Verified;
            op.detail.progress.input_ready = true;
            assert!(
                !route_here_evidence_event(Some(op), &event(5), 1, 100, &visibility, &layout_atoms),
                "Configure stays on ordinary reconciliation path"
            );
            assert_eq!(
                op.placement.proof.current.geometry,
                control::GeometryProof::Unknown
            );
            assert!(!op.detail.progress.input_ready);
            h.here.current.layer = L::Above;
            h.here.actual.x = -100;
            tick(&mut owner, &mut h).unwrap();
            assert_terminal(&mut owner, &h, Reason::Unverifiable);
            assert_eq!(
                h.here.count("correct"),
                0,
                "ordinary movement cannot grant a Recover correction"
            );
        }

        #[test]
        fn recover_phase_deadlines_survive_sustained_bounded_batches_without_polling() {
            let (visibility, layout_atoms) = atoms();
            for phase in [1, 3, 5] {
                let (mut h, mut owner) = checkpoint(phase);
                tick(&mut owner, &mut h).unwrap();
                let deadline = remote_operation_deadline(owner.active().unwrap());
                let reads = h.here.count("observe");
                let mut queue = VecDeque::from(vec![property(999, 99); 4000]);
                let mut buffered = Some(property(999, 99));
                let mut processed = 0;
                while owner.active().is_some() {
                    let batch = drain_events_bounded(&mut buffered, 64, || {
                        Ok::<_, HostError>(queue.pop_front())
                    })
                    .unwrap();
                    assert!(batch.len() <= 64);
                    for e in batch {
                        h.here.now += Duration::from_millis(1);
                        processed += 1;
                        if owner
                            .active()
                            .is_some_and(|op| h.here.now >= remote_operation_deadline(op))
                        {
                            tick(&mut owner, &mut h).unwrap();
                            assert_eq!(h.here.now, deadline);
                        }
                        assert!(!route_here_evidence_event(
                            owner.active_mut(),
                            &e,
                            1,
                            100,
                            &visibility,
                            &layout_atoms
                        ));
                    }
                    tick(&mut owner, &mut h).unwrap();
                }
                assert!(processed > 64);
                assert!(
                    h.here.count("observe") <= reads + 3,
                    "unrelated events do not poll"
                );
                assert_terminal(&mut owner, &h, Reason::ConfirmationTimeout);
            }
        }

        #[test]
        fn both_consumed_request_boundaries_immediately_service_internal_recover() {
            for _site in ["batch", "lookahead"] {
                let mut h = Host::new(true, true, L::Below);
                let mut active = None;
                route_owner_request_with(
                    &mut active,
                    &mut h,
                    |active, h| {
                        *active = Some(h.operation());
                        Ok(true)
                    },
                    |active, h| service_recover_owner_with(active, h, |_| panic!("pending")),
                )
                .unwrap();
                assert_eq!(h.here.count("desktop"), 1);
                let deadline = active.as_ref().unwrap().deadline;
                h.here.target();
                mark_remote_observation(active.as_mut().unwrap(), false);
                service_recover_owner_with(&mut active, &mut h, |_| panic!("Top still pending"))
                    .unwrap();
                assert_eq!(h.here.count("map"), 1);
                assert_eq!(active.as_ref().unwrap().deadline, deadline);
                assert!(!remote_service_due(active.as_ref().unwrap(), h.here.now));
            }
        }

        #[test]
        fn completed_recover_reply_failure_clears_the_slot_without_undoing_success() {
            for history in [false, true] {
                let mut h = Host::new(history, history, L::Below);
                let mut owner = owner(&h);
                owner.fail_reply(true);
                assert!(finish_owner(&mut owner, &mut h).is_err());
                let reply = assert_terminal(&mut owner, &h, Reason::None);
                assert_eq!(reply.status, Status::Success);
                assert!(h.here.visible_ready());
                assert_eq!(h.here.current.layer, L::Above);
                assert_eq!(
                    reply.detail.placement.history,
                    control::RecoveryHistory {
                        relocation_confirmed: history,
                        restoration_confirmed: history,
                    }
                );
                tick(&mut owner, &mut h).unwrap();
                assert_eq!(owner.replies().len(), 1);
            }
        }

        #[test]
        fn production_owner_wait_boundaries_remain_reachable_as_specified() {
            let source = include_str!("main.rs");
            let probe = source
                .split("fn run_probe(")
                .nth(1)
                .unwrap()
                .split("fn finish_probe_run_with(")
                .next()
                .unwrap();
            assert_eq!(probe.matches("route_owner_request(").count(), 2);
            assert!(probe.contains("drain_events_bounded(&mut buffered_event, 64"));
            assert!(probe.contains(".chain(remote_deadline(owner))"));
            assert!(probe.contains(".closing(conn)"));
            assert!(probe.contains("owner.finish_remote(conn)"));
            assert!(
                probe.rfind("service_owner_operation(").unwrap()
                    < probe.rfind("wait_x11(conn, timeout)").unwrap()
            );
        }

        fn assert_result(
            op: &ActiveOperation,
            result: control::OperationResult,
            status: Status,
            relocation: bool,
            restoration: bool,
        ) {
            assert_eq!(
                op.recover_result(result.detail().reason, result.detail().stage),
                Ok(result)
            );
            assert_eq!(result.status(), status);
            assert_eq!(
                result.detail().placement.history,
                control::RecoveryHistory {
                    relocation_confirmed: relocation,
                    restoration_confirmed: restoration,
                }
            );
            assert_eq!(
                op.request,
                control::Request {
                    command: Command::Recover,
                    request_id: 99,
                    reply_xid: 9,
                    epoch: 11
                }
            );
            assert_eq!(op.placement.destination_desktop, 2);
            if status == Status::Success {
                assert_eq!(op.recover_phase, RecoverPhase::Top);
                assert_eq!(result.detail().stage, Stage::Layer);
                assert!(result.detail().progress.input_ready);
                assert_eq!(result.detail().visibility, Visibility::NotMinimizedHere);
            }
        }

        fn drive(
            h: &mut Host,
            op: &mut ActiveOperation,
        ) -> Result<control::OperationResult, HostError> {
            for _ in 0..4 {
                if let Some(result) = service_recover_with(op, h)? {
                    return Ok(result);
                }
                if op.recover_phase == RecoverPhase::Here && h.here.count("desktop") > 0 {
                    h.here.target();
                } else if op.recover_phase == RecoverPhase::Top && h.here.count("map") > 0 {
                    h.restored();
                }
            }
            panic!("scripted WM evidence did not settle");
        }

        #[test]
        fn invalid_here_never_enters_top_and_safe_above_noop_has_fresh_top_proof() {
            let mut h = Host::new(false, false, L::Above);
            let mut op = h.operation();
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Success, false, false);
            h.here.assert_no_mutation();
            assert!(
                h.here.count("geometry") >= 3,
                "Here geometry cannot substitute for Top proof"
            );
            assert!(!op.handoff_recover_top(
                h.here.now,
                op.detail,
                control::Confirmation::default()
            ));
            assert_eq!(op.recover_phase, RecoverPhase::Top);
            let mut h = Host::new(false, false, L::Below);
            h.here.current.transitional = true;
            let mut op = h.operation();
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Failed, false, false);
            assert_eq!(op.recover_phase, RecoverPhase::Here);
            assert_eq!(h.here.count("map"), 0);
            assert!(h.mutations.is_empty());
        }

        #[test]
        fn history_partitions_survive_layout_failure_without_current_readiness() {
            for remote in [false, true] {
                for restored in [false, true] {
                    let mut h = Host::new(remote, true, L::Below);
                    let mut op = h.operation();
                    assert!(h.run(&mut op).is_none());
                    if remote {
                        h.here.target();
                        assert!(h.run(&mut op).is_none());
                    }
                    assert_eq!(op.recover_phase, RecoverPhase::Top);
                    if restored {
                        h.restored();
                    }
                    h.here.layout = Err(DestinationError::Changed);
                    let result = h.run(&mut op).unwrap();
                    assert_result(
                        &op,
                        result,
                        if remote || restored {
                            Status::Partial
                        } else {
                            Status::Failed
                        },
                        remote,
                        restored,
                    );
                    assert_eq!(result.detail().reason, Reason::LayoutChanged);
                    assert!(!result.detail().progress.input_ready);
                    assert_eq!(
                        result.detail().placement.current,
                        control::PlacementConfirmation::default()
                    );
                    assert!(h.mutations.is_empty());
                }
            }
        }

        #[test]
        fn every_strict_read_veto_latches_before_any_later_recovery_mutation() {
            let mut reference = Host::new(true, true, L::Below);
            let mut op = reference.operation();
            assert_eq!(
                drive(&mut reference, &mut op).unwrap().status(),
                Status::Success
            );
            let reads = reference.here.count("layout");
            assert!(reads > 20);
            for index in 1..=reads {
                let mut h = Host::new(true, true, L::Below);
                h.here
                    .layouts
                    .extend(std::iter::repeat_n(Ok(layout()), index - 1));
                h.here.layouts.push_back(Err(DestinationError::Changed));
                let mut op = h.operation();
                let result = drive(&mut h, &mut op).unwrap();
                assert_ne!(result.status(), Status::Success, "layout read {index}");
                let invalid = h
                    .here
                    .log
                    .iter()
                    .enumerate()
                    .filter(|(_, step)| **step == "layout")
                    .nth(index - 1)
                    .unwrap()
                    .0;
                for step in &h.here.log[invalid + 1..] {
                    assert!(
                        !["desktop", "zero", "map", "correct", "shape", "remove", "above"]
                            .contains(step),
                        "mutation {step} after invalid layout read {index}"
                    );
                }
            }
        }

        #[test]
        fn top_rejects_each_frozen_identity_component_and_invalid_destination_policy() {
            for case in 0..12 {
                let mut h = Host::new(false, true, L::Below);
                let mut op = h.operation();
                assert!(h.run(&mut op).is_none());
                h.restored();
                let mut changed = layout();
                match case {
                    0 => changed.desktop_count += 1,
                    1 => changed.destination_desktop = 3,
                    2 => changed.current_desktop = 3,
                    3 => changed.target_workarea = Some(Rect::new(0, 0, 700, 600)),
                    4 => changed.viewport = control::DestinationViewport::Absent,
                    5 => changed.monitor_name += 1,
                    6 => changed.monitor_bounds.x += 1,
                    7 => changed.usable_area.width -= 1,
                    8 => changed.body_size = Size::new(161, 160),
                    9 => changed.valid_origins = geometry::ValidOriginBounds::new(1, 500, 0, 500),
                    10 => {}
                    _ => {}
                }
                h.here.layout = match case {
                    10 => Err(DestinationError::UnsupportedViewport),
                    11 => Err(DestinationError::Unverifiable(
                        "invalid count or workarea".into(),
                    )),
                    _ => Ok(changed),
                };
                let result = h.run(&mut op).unwrap();
                assert_result(&op, result, Status::Partial, false, true);
                assert_eq!(
                    result.detail().reason,
                    match case {
                        10 => Reason::Unsupported,
                        11 => Reason::Unverifiable,
                        _ => Reason::LayoutChanged,
                    }
                );
                assert_eq!(h.here.count("shape"), 0);
                assert!(h.mutations.is_empty());
            }
        }

        #[test]
        fn guard_after_user_time_veto_retains_dispatch_and_never_maps() {
            let mut reference = Host::new(false, true, L::Normal);
            let mut op = reference.operation();
            assert!(reference.run(&mut op).is_none());
            let guard = reference
                .here
                .log
                .iter()
                .position(|&step| step == "restore_guard")
                .unwrap();
            let before = reference.here.log[..guard]
                .iter()
                .filter(|&&step| step == "layout")
                .count();
            let mut h = Host::new(false, true, L::Normal);
            h.here
                .layouts
                .extend(std::iter::repeat_n(Ok(layout()), before));
            h.here.layouts.push_back(Err(DestinationError::Changed));
            let mut op = h.operation();
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Failed, false, false);
            assert!(result.detail().progress.host_mutation_dispatched);
            assert_eq!(h.here.count("zero"), 1);
            assert_eq!(h.here.count("map"), 0);
            assert_eq!(h.here.count("restore_guard"), 1);
        }

        #[test]
        fn layer_addition_has_its_own_guard_after_successful_below_removal() {
            let mut reference = Host::new(false, false, L::Below);
            let mut op = reference.operation();
            assert_eq!(reference.run(&mut op).unwrap().status(), Status::Success);
            let addition = reference
                .here
                .log
                .iter()
                .rposition(|&step| step == "layer_guard")
                .unwrap();
            let before = reference.here.log[..addition]
                .iter()
                .filter(|&&step| step == "layout")
                .count();
            let mut h = Host::new(false, false, L::Below);
            h.here
                .layouts
                .extend(std::iter::repeat_n(Ok(layout()), before));
            h.here.layouts.push_back(Err(DestinationError::Changed));
            let mut op = h.operation();
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Failed, false, false);
            assert_eq!(h.here.count("remove"), 1);
            assert_eq!(h.here.count("above"), 0);
            assert_eq!(result.detail().reason, Reason::LayoutChanged);
        }

        #[test]
        fn here_cutoff_is_exact_and_handoff_visibility_budget_uses_actual_time() {
            for remote in [false, true] {
                let mut h = Host::new(remote, true, L::Normal);
                let mut op = h.operation();
                let start = h.here.now;
                if remote {
                    assert!(h.run(&mut op).is_none());
                    h.here.target();
                }
                h.here.now = start + Duration::from_millis(3500);
                let result = h.run(&mut op).unwrap();
                assert_result(
                    &op,
                    result,
                    if remote {
                        Status::Partial
                    } else {
                        Status::Failed
                    },
                    remote,
                    false,
                );
                assert_eq!(op.recover_phase, RecoverPhase::Here);
                assert_eq!(h.here.count("zero"), 0);
            }
            let mut h = Host::new(true, true, L::Normal);
            let mut op = h.operation();
            let start = h.here.now;
            assert!(h.run(&mut op).is_none());
            h.here.target();
            h.here.now = start + Duration::from_millis(3400);
            assert!(h.run(&mut op).is_none());
            assert_eq!(
                op.bring_top.visibility_cutoff,
                start + Duration::from_millis(5900)
            );
            assert_eq!(op.deadline, start + Duration::from_millis(7000));
            h.restored();
            h.here.now = op.bring_top.visibility_cutoff;
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Partial, true, true);
            assert_eq!(h.here.count("shape"), 0);
            assert!(h.mutations.is_empty());
        }

        #[test]
        fn synchronous_restore_and_guard_reads_cannot_spend_past_visibility_cutoff() {
            for step in ["zero", "restore_guard"] {
                let mut h = Host::new(false, true, L::Normal);
                h.here
                    .advances
                    .push_back((step, Duration::from_millis(2500)));
                let mut op = h.operation();
                let result = h.run(&mut op).unwrap();
                assert_result(&op, result, Status::Failed, false, false);
                assert_eq!(h.here.count("zero"), 1);
                assert_eq!(h.here.count("map"), 0);
                assert!(h.mutations.is_empty());
            }
        }

        #[test]
        fn layer_deadline_and_final_read_never_authorize_late_addition() {
            let mut h = Host::new(false, false, L::Below);
            h.layer_changes = false;
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            let admitted = op.bring_top.layer_admitted.unwrap();
            assert_eq!(
                op.recover_layer_cutoff(),
                Some(admitted + Duration::from_millis(1000))
            );
            assert_eq!(h.here.count("remove"), 1);
            h.here.current.layer = L::Normal;
            h.here.now = op.recover_layer_cutoff().unwrap();
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Failed, false, false);
            assert_eq!(h.here.count("above"), 0);
            assert_eq!(h.here.count("remove"), 1);
        }

        #[test]
        fn minimized_here_leaves_one_top_correction_with_actual_confirmation() {
            let mut h = Host::new(false, true, L::Normal);
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            assert!(op.placement.correction().is_none());
            h.restored();
            h.here.actual.x = -100;
            assert!(h.run(&mut op).is_none());
            let consumed = op.placement.correction().unwrap();
            assert_eq!(h.here.count("correct"), 1);
            assert_eq!(h.here.count("shape"), 0);
            h.here.now = consumed.1;
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Success, false, true);
            assert_eq!(op.placement.correction(), Some(consumed));
            assert_eq!(h.here.count("correct"), 1);
            assert!(h.here.runtime.input_fence.automatic_correction_blocked);
        }

        #[test]
        fn consumed_here_correction_blocks_top_correction_and_cannot_be_replenished() {
            let mut h = Host::new(false, false, L::Normal);
            h.here.actual.x = -100;
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            let consumed = op.placement.correction().unwrap();
            h.here.now = consumed.1;
            let actual = h.here.actual;
            h.here
                .geometries
                .extend([actual, actual, Rect::new(-100, 100, 160, 160)]);
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Failed, false, false);
            assert_eq!(op.recover_phase, RecoverPhase::Top);
            assert_eq!(op.placement.correction(), Some(consumed));
            assert_eq!(h.here.count("correct"), 1);
            assert!(h.here.runtime.input_fence.automatic_correction_blocked);
            assert!(h.mutations.is_empty());
        }

        #[test]
        fn top_correction_needs_full_interval_and_refused_correction_never_retries() {
            for enough_time in [false, true] {
                let mut h = Host::new(false, true, L::Normal);
                let mut op = h.operation();
                assert!(h.run(&mut op).is_none());
                h.restored();
                h.here.actual.x = -100;
                h.here.refuse_correction = true;
                if !enough_time {
                    h.here.now = op.bring_top.visibility_cutoff - Duration::from_millis(149);
                }
                let mut result = h.run(&mut op);
                if enough_time {
                    assert!(result.is_none());
                    h.here.now = op.placement.correction().unwrap().1;
                    result = h.run(&mut op);
                }
                assert_result(&op, result.unwrap(), Status::Partial, false, true);
                assert_eq!(h.here.count("correct"), usize::from(enough_time));
                assert_eq!(h.here.count("shape"), 0);
                assert!(h.mutations.is_empty());
            }
        }

        #[test]
        fn strict_drift_at_here_and_top_correction_boundaries_prevents_dispatch() {
            for top in [false, true] {
                let mut reference = Host::new(false, top, L::Normal);
                let mut op = reference.operation();
                if top {
                    assert!(reference.run(&mut op).is_none());
                    reference.restored();
                }
                let prior = reference.here.count("layout");
                reference.here.actual.x = -100;
                assert!(reference.run(&mut op).is_none());
                assert_eq!(reference.here.count("correct"), 1);
                let last = reference.here.count("layout");
                for index in prior + 1..=last {
                    let mut h = Host::new(false, top, L::Normal);
                    let mut op = h.operation();
                    if top {
                        assert!(h.run(&mut op).is_none());
                        h.restored();
                    }
                    h.here.actual.x = -100;
                    h.here
                        .layouts
                        .extend(std::iter::repeat_n(Ok(layout()), index - prior - 1));
                    h.here.layouts.push_back(Err(DestinationError::Changed));
                    let result = h.run(&mut op).unwrap();
                    assert_result(
                        &op,
                        result,
                        if top { Status::Partial } else { Status::Failed },
                        false,
                        top,
                    );
                    assert_eq!(h.here.count("correct"), 0, "Top={top} layout read={index}");
                    assert_eq!(h.here.count("shape"), 0);
                    assert!(h.mutations.is_empty());
                }
            }
        }

        #[test]
        fn minimized_body_size_is_checked_before_any_restoration_mutation() {
            let mut h = Host::new(false, true, L::Normal);
            h.here.actual.width = 161;
            let mut op = h.operation();
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Failed, false, false);
            assert_eq!(result.detail().reason, Reason::Unverifiable);
            assert_eq!(h.here.count("zero"), 0);
            assert_eq!(h.here.count("map"), 0);
            assert!(h.mutations.is_empty());
        }

        #[test]
        fn wrong_actual_body_size_and_off_workspace_top_cannot_enable_or_raise() {
            for elsewhere in [false, true] {
                let mut h = Host::new(false, true, L::Below);
                let mut op = h.operation();
                assert!(h.run(&mut op).is_none());
                h.restored();
                if elsewhere {
                    h.here.current.desktop = 3;
                } else {
                    h.here.actual.width = 161;
                }
                let result = h.run(&mut op).unwrap();
                assert_result(&op, result, Status::Partial, false, true);
                assert_eq!(h.here.count("shape"), 0);
                assert!(h.mutations.is_empty());
            }
        }

        #[test]
        fn fence_pointer_and_popup_obligations_precede_restore_and_input_enable() {
            let mut h = Host::new(false, true, L::Below);
            h.here.runtime = visibility_fence_tests::runtime(4);
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            let zero = h.here.log.iter().position(|&step| step == "zero").unwrap();
            for step in ["empty", "release", "popup"] {
                assert!(h.here.log.iter().position(|&item| item == step).unwrap() < zero);
            }
            assert!(h.here.runtime.visibility_dispatch_eligible());
            h.restored();
            assert_eq!(h.run(&mut op).unwrap().status(), Status::Success);
            assert!(here_runtime_ready(&h.here.runtime, &h.here.window));
        }

        #[test]
        fn original_native_errors_survive_each_internal_boundary_without_retry() {
            let mut reference = Host::new(true, true, L::Below);
            let mut op = reference.operation();
            drive(&mut reference, &mut op).unwrap();
            for step in [
                "desktop", "zero", "map", "layout", "geometry", "shape", "remove", "above",
            ] {
                for before_cookie in [false, true] {
                    let mut h = Host::new(true, true, L::Below);
                    h.here.send_before_cookie = before_cookie && step == "desktop";
                    h.fail_before_cookie = before_cookie.then_some(step);
                    h.here.faults.push((step, 1));
                    let mut op = h.operation();
                    let result = drive(&mut h, &mut op);
                    assert_eq!(
                        result.unwrap_err().downcast_ref::<NativeFault>(),
                        Some(&NativeFault(step, 1))
                    );
                    assert_eq!(h.here.count(step), 1);
                    assert_eq!(
                        h.here.count("map"),
                        usize::from(!["desktop", "zero", "layout", "geometry"].contains(&step))
                    );
                    assert_eq!(op.request.command, Command::Recover);
                }
            }
        }

        #[test]
        fn uncertain_correction_is_consumed_and_cleanup_cannot_replace_original_error() {
            let mut h = Host::new(false, true, L::Normal);
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            h.restored();
            h.here.actual.x = -100;
            h.here.faults.push(("correct", 1));
            let error = service_recover_with(&mut op, &mut h).unwrap_err();
            assert_eq!(
                error.downcast_ref::<NativeFault>(),
                Some(&NativeFault("correct", 1))
            );
            let consumed = op.placement.correction();
            assert!(consumed.is_some());
            h.here.now = consumed.unwrap().1;
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Partial, false, true);
            assert_eq!(h.here.count("correct"), 1);
            assert_eq!(op.placement.correction(), consumed);

            let mut h = Host::new(false, true, L::Normal);
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            h.restored();
            h.here
                .faults
                .extend([("shape", 1), ("empty", 2), ("empty", 3)]);
            let error = service_recover_with(&mut op, &mut h).unwrap_err();
            assert_eq!(
                error.downcast_ref::<NativeFault>(),
                Some(&NativeFault("shape", 1))
            );
            assert!(h.here.runtime.input_fence.blocked);
            assert_eq!(h.here.count("shape"), 1);
            assert!(h.mutations.is_empty());
        }

        #[test]
        fn pointer_release_or_popup_failure_prevents_handoff_and_restore() {
            for step in ["release", "popup"] {
                let mut h = Host::new(false, true, L::Below);
                h.here.runtime = visibility_fence_tests::runtime(4);
                h.here.faults.push((step, 1));
                let mut op = h.operation();
                let error = service_recover_with(&mut op, &mut h).unwrap_err();
                assert_eq!(
                    error.downcast_ref::<NativeFault>(),
                    Some(&NativeFault(step, 1))
                );
                assert_eq!(op.recover_phase, RecoverPhase::Here);
                assert_eq!(h.here.count("zero"), 0);
                assert_eq!(h.here.count("map"), 0);
                assert!(h.mutations.is_empty());
            }
        }

        #[test]
        fn delayed_geometry_and_layer_support_reads_cannot_authorize_mutation() {
            let mut h = Host::new(false, true, L::Normal);
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            h.restored();
            h.here
                .advances
                .push_back(("geometry", Duration::from_millis(2500)));
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Partial, false, true);
            assert_eq!(h.here.count("shape"), 0);
            assert!(h.mutations.is_empty());

            let mut h = Host::new(false, false, L::Normal);
            h.here
                .advances
                .push_back(("support", Duration::from_millis(1000)));
            let mut op = h.operation();
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Failed, false, false);
            assert!(h.mutations.is_empty());
        }

        #[test]
        fn final_read_can_confirm_already_completed_layer_but_not_checked_transport() {
            let mut h = Host::new(false, false, L::Normal);
            h.layer_changes = false;
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            assert_eq!(h.here.count("above"), 1);
            h.here.current.layer = L::Above;
            h.here.now = op.recover_layer_cutoff().unwrap();
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Success, false, false);
            assert_eq!(h.here.count("above"), 1);

            let mut h = Host::new(false, true, L::Above);
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            assert_eq!(h.here.count("map"), 1);
            h.here.now = op.bring_top.visibility_cutoff;
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Failed, false, false);
            assert_eq!(h.here.count("map"), 1);
            assert_eq!(h.here.count("shape"), 0);
        }

        #[test]
        fn composition_local_remote_visible_minimized_all_layers_retains_one_identity() {
            for remote in [false, true] {
                for minimized in [false, true] {
                    for layer in [L::Normal, L::Above, L::Below] {
                        let mut h = Host::new(remote, minimized, layer);
                        let mut op = h.operation();
                        let deadline = op.deadline;
                        let initial = h.here.now;
                        let mut result = h.run(&mut op);
                        if remote {
                            assert!(result.is_none());
                            assert_eq!(op.recover_phase, RecoverPhase::Here);
                            assert_eq!(h.here.count("map"), 0);
                            assert!(h.mutations.is_empty());
                            h.here.target();
                            result = h.run(&mut op);
                        }
                        if minimized {
                            assert!(result.is_none());
                            assert_eq!(op.recover_phase, RecoverPhase::Top);
                            assert_eq!(
                                h.here.count("map"),
                                1,
                                "handoff must immediately start Show"
                            );
                            assert!(h.mutations.is_empty());
                            assert!(!op.placement.proof.history.restoration_confirmed);
                            h.restored();
                            result = h.run(&mut op);
                        }
                        assert_result(&op, result.unwrap(), Status::Success, remote, minimized);
                        assert_eq!(op.deadline, deadline);
                        assert_eq!(
                            op.bring_top.visibility_cutoff,
                            initial + Duration::from_millis(2500)
                        );
                        assert_eq!(op.placement.layout(), Some(layout()));
                        assert_eq!(op.placement.baseline.unwrap().layer, layer);
                        assert_eq!(h.here.count("desktop"), usize::from(remote));
                        assert_eq!(h.here.count("zero"), usize::from(minimized));
                        assert_eq!(h.here.count("map"), usize::from(minimized));
                        assert_eq!(h.here.count("above"), usize::from(layer != L::Above));
                        assert_eq!(h.here.count("remove"), usize::from(layer == L::Below));
                        assert_eq!(h.here.count("correct"), 0);
                        assert!(!h.here.runtime.input_fence.blocked);
                    }
                }
            }
        }
    }
}
