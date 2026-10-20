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

fn print_help() {
    println!("DesktopRoomie - A00-M03.2: Desktop Probe");
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
    println!(
        "  --show       Restore the existing body without activation or workspace/layer changes\n  --hide       Minimize the existing body without workspace/layer changes\n  --bring-top  Ensure the existing body is not minimized and set Above on its existing workspace"
    );
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

/// Supplied by a fresh retained-monitor/layout and root-geometry validation, never
/// a MapNotify or the desired Ready state. No destination-workspace layout is used.
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
mod map_input_tests {
    use super::*;

    const BODY: u32 = 100;
    const MAP: u64 = 2 * 65536 + 100;
    const SHAPE: u64 = MAP + 20;

    fn mapped() -> MapNotifyEvent {
        MapNotifyEvent {
            response_type: x11rb::protocol::xproto::MAP_NOTIFY_EVENT,
            sequence: MAP as u16,
            event: BODY,
            window: BODY,
            override_redirect: false,
        }
    }

    #[test]
    fn one_map_synchronizes_once_and_retains_full_shape_sequence_without_readiness() {
        let mut runtime = ProbeRuntime::new();
        runtime.arm_map_input_sync(MAP);
        runtime
            .synchronize_map_input_with(&mapped(), BODY, MAP, || Ok(SHAPE))
            .unwrap();
        assert_eq!(runtime.input_fence.enabled_after, Some(SHAPE));
        assert_eq!(runtime.body, BodyAvailability::AwaitingMap);
        assert!(!runtime.accepts_input());
        for sequence in MAP..MAP + 1000 {
            runtime
                .synchronize_map_input_with(&mapped(), BODY, sequence, || panic!("duplicate"))
                .unwrap();
        }
        assert_eq!(runtime.input_fence.enabled_after, Some(SHAPE));
    }

    #[test]
    fn unrelated_synthetic_and_stale_maps_do_not_consume_sync_allowance() {
        let mut unrelated = mapped();
        unrelated.window += 1;
        let mut parent = mapped();
        parent.event += 1;
        let mut synthetic = mapped();
        synthetic.response_type |= 0x80;
        let mut unmanaged = mapped();
        unmanaged.override_redirect = true;
        for (event, sequence) in [
            (unrelated, MAP),
            (parent, MAP),
            (synthetic, MAP),
            (unmanaged, MAP),
            (mapped(), MAP - 1),
            (mapped(), MAP - 65536),
            (mapped(), MAP - 2 * 65536),
        ] {
            let mut runtime = ProbeRuntime::new();
            runtime.arm_map_input_sync(MAP);
            runtime
                .synchronize_map_input_with(&event, BODY, sequence, || panic!("invalid map"))
                .unwrap();
            assert_eq!(runtime.pending_map_input, Some(MAP));
            assert_eq!(runtime.input_fence.enabled_after, None);
            runtime
                .synchronize_map_input_with(&mapped(), BODY, MAP, || Ok(SHAPE))
                .unwrap();
            assert_eq!(runtime.input_fence.enabled_after, Some(SHAPE));
        }
    }

    #[test]
    fn failed_map_shape_check_is_fatal_unsynchronized_and_not_retried() {
        let mut runtime = ProbeRuntime::new();
        runtime.arm_map_input_sync(MAP);
        let error = runtime
            .synchronize_map_input_with(&mapped(), BODY, MAP, || Err("SHAPE check failed".into()))
            .unwrap_err();
        assert_eq!(error.to_string(), "SHAPE check failed");
        assert_eq!(runtime.input_fence.enabled_after, None);
        assert_eq!(runtime.pending_map_input, None);
        assert!(!runtime.accepts_input());
        runtime
            .synchronize_map_input_with(&mapped(), BODY, MAP + 1, || panic!("blind retry"))
            .unwrap();
    }

    #[test]
    fn future_remap_needs_explicit_new_request_boundary_and_rejects_old_maps() {
        let mut runtime = ProbeRuntime::new();
        runtime
            .synchronize_map_input_with(&mapped(), BODY, MAP, || panic!("not armed"))
            .unwrap();
        runtime.arm_map_input_sync(MAP);
        runtime
            .synchronize_map_input_with(&mapped(), BODY, MAP, || Ok(SHAPE))
            .unwrap();
        runtime.unavailable(false);
        runtime
            .synchronize_map_input_with(&mapped(), BODY, SHAPE, || panic!("not rearmed"))
            .unwrap();
        let remap = MAP + 65536;
        runtime.arm_map_input_sync(remap);
        runtime
            .synchronize_map_input_with(&mapped(), BODY, MAP, || panic!("previous mapping"))
            .unwrap();
        assert_eq!(runtime.pending_map_input, Some(remap));
        assert_eq!(runtime.input_fence.enabled_after, Some(SHAPE));
        runtime
            .synchronize_map_input_with(&mapped(), BODY, remap, || Ok(remap + 20))
            .unwrap();
        assert_eq!(runtime.input_fence.enabled_after, Some(remap + 20));
        assert_eq!(runtime.body, BodyAvailability::Unavailable);
        runtime
            .synchronize_map_input_with(&mapped(), BODY, remap + 21, || panic!("duplicate remap"))
            .unwrap();
    }

    #[test]
    fn post_map_sync_cannot_bypass_visibility_fence_or_placement_safety() {
        for availability in [
            BodyAvailability::AwaitingMap,
            BodyAvailability::Validating,
            BodyAvailability::Ready,
        ] {
            let mut runtime = ProbeRuntime::new();
            let mut window = tests::test_window();
            runtime.arm_map_input_sync(MAP);
            runtime
                .fence_visibility_input_with(&mut window, || Ok(MAP), |_, _| Ok(()), |_| Ok(()))
                .unwrap();
            runtime.body = availability;
            runtime
                .synchronize_map_input_with(&mapped(), BODY, MAP, || panic!("input fenced"))
                .unwrap();
            assert!(runtime.input_fence.blocked);
            assert!(runtime.input_fence.empty_confirmed);
            assert_eq!(runtime.input_fence.enabled_after, None);
            assert!(!runtime.accepts_input());
        }
    }
}

#[cfg(test)]
mod visibility_fence_tests {
    use super::*;
    use crate::control::{Capability, Evidence, MapState, VisibilityEvidence, WmState, Workspace};
    use crate::geometry::MenuHit;
    use std::cell::RefCell;
    use x11rb::protocol::xproto::{ButtonPressEvent, EnterNotifyEvent, MotionNotifyEvent};

    fn evidence() -> VisibilityEvidence {
        VisibilityEvidence {
            destroyed: false,
            wm_state: WmState::Normal,
            map_state: Evidence::Readable(MapState::Viewable),
            hidden: Evidence::Readable(false),
            workspace: Workspace::Current,
            capability: Capability::Supported,
            coherent: true,
        }
    }
    fn placement() -> InputPlacement {
        InputPlacement {
            selected_monitor: 20,
            observed_monitor: 20,
            geometry: crate::geometry::Rect::new(100, 100, 160, 160),
            bounds: crate::geometry::ValidOriginBounds::new(0, 500, 0, 500),
        }
    }
    pub(super) fn runtime(case: u8) -> ProbeRuntime {
        let mut r = ProbeRuntime::new();
        r.body = BodyAvailability::Ready;
        match case {
            1 => {
                r.interaction.handle_left_press(
                    Point::new(180, 180),
                    (80, 80),
                    100,
                    Point::new(100, 100),
                );
                r.interaction.on_grab_acquired();
                r.interaction
                    .handle_motion(Point::new(220, 220), 101, &placement().bounds);
                assert!(r.interaction.is_dragging());
                r.pointer.set_grabbed(true);
            }
            2 => {
                r.interaction.handle_right_press(100);
                r.pointer.track_automatic_right().unwrap();
            }
            3 | 4 => {
                r.menu_generation = 5;
                r.menu = Some(MenuPopup::test_popup(200, 5));
                r.pointer
                    .acquire_with(
                        CaptureOwner::Menu {
                            window: 200,
                            generation: 5,
                        },
                        || Ok((GrabStatus::SUCCESS, 60000)),
                    )
                    .unwrap();
                r.interaction.menu_acquired(MenuHit::Outside);
                if case == 4 {
                    r.interaction.menu_press(1, MenuHit::Row(0), 100, 0);
                }
            }
            _ => {} // Idle / queued server automatic capture with no tracked owner.
        }
        r
    }
    fn assert_invalidated(r: &ProbeRuntime, window: &ManagedProbeWindow) {
        assert!(matches!(
            r.interaction.state(),
            InteractionState::Idle | InteractionState::SuppressedUntilRelease { .. }
        ));
        assert!(r.pending_move.is_none());
        assert!(r.final_target.is_none());
        assert!(r.confirmation_deadline.is_none());
        assert!(r.correction.is_none());
        assert!(r.safety.deadline.is_none());
        assert!(r.safety.observation.is_none());
        assert!(window.in_flight_moves.is_empty());
        assert_eq!(window.requested_origin(), window.confirmed_origin());
        assert!(!r.accepts_input());
    }
    fn fence(r: &mut ProbeRuntime, w: &mut ManagedProbeWindow) {
        r.fence_visibility_input_with(
            w,
            || Ok(70000),
            |r, w| {
                assert_invalidated(r, w);
                Ok(())
            },
            |_| Ok(()),
        )
        .unwrap();
    }
    #[test]
    fn every_gesture_is_invalidated_between_empty_shape_and_ungrab_before_popup_cleanup() {
        for case in 0..=5 {
            let mut r = runtime(case);
            let mut w = tests::test_window();
            r.pending_move = Some(Point::new(220, 220));
            r.final_target = r.pending_move;
            r.confirmation_deadline = Some(Instant::now());
            r.correction = Some((Point::new(220, 220), Instant::now()));
            r.safety.acquired(Instant::now());
            r.safety.observed(false, 60000);
            w.in_flight_moves.push_back(Point::new(220, 220));
            let generation = r.menu_generation;
            let trace = RefCell::new(vec![]);
            r.fence_visibility_input_with(
                &mut w,
                || {
                    trace.borrow_mut().push("empty");
                    Ok(70000)
                },
                |r, w| {
                    assert_invalidated(r, w);
                    assert!(r.pointer.is_grabbed()); // Even idle/queued automatic capture.
                    assert_eq!(r.menu_generation, generation + 1);
                    assert_eq!(*trace.borrow(), ["empty"]);
                    assert!(!r.visibility_dispatch_eligible());
                    trace.borrow_mut().push("ungrab");
                    Ok(())
                },
                |_| {
                    trace.borrow_mut().push("popup");
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(
                *trace.borrow(),
                if case == 3 || case == 4 {
                    vec!["empty", "ungrab", "popup"]
                } else {
                    vec!["empty", "ungrab"]
                }
            );
            assert!(r.visibility_dispatch_eligible());
            // Fake future dispatch only; no native request helper is connected.
            if r.visibility_dispatch_eligible() {
                trace.borrow_mut().push("fake dispatch");
            }
            assert_eq!(trace.borrow().last(), Some(&"fake dispatch"));
            assert_invalidated(&r, &w);
        }
    }
    #[test]
    fn all_failure_combinations_attempt_cleanup_preserve_primary_and_block_dispatch() {
        for failures in 1..8 {
            let mut r = runtime(4);
            let mut w = tests::test_window();
            let trace = RefCell::new(vec![]);
            let error = r
                .fence_visibility_input_with(
                    &mut w,
                    || {
                        trace.borrow_mut().push("empty");
                        if failures & 1 != 0 {
                            Err("shape".into())
                        } else {
                            Ok(70000)
                        }
                    },
                    |r, w| {
                        assert_invalidated(r, w);
                        trace.borrow_mut().push("ungrab");
                        if failures & 2 != 0 {
                            Err("release".into())
                        } else {
                            Ok(())
                        }
                    },
                    |popup| {
                        trace.borrow_mut().push("popup");
                        popup.window_resource.release_with(|| {
                            if failures & 4 != 0 {
                                Err("destroy".into())
                            } else {
                                Ok(())
                            }
                        })
                    },
                )
                .unwrap_err();
            assert_eq!(*trace.borrow(), ["empty", "ungrab", "popup"]);
            assert_eq!(
                error.to_string(),
                if failures & 1 != 0 {
                    "shape"
                } else if failures & 2 != 0 {
                    "release"
                } else {
                    "destroy"
                }
            );
            assert_eq!(r.pointer.is_grabbed(), failures & 2 != 0);
            assert_eq!(r.menu.is_some(), failures & 4 != 0);
            assert!(!r.visibility_dispatch_eligible());
            assert!(!r.accepts_input());
            r.body = BodyAvailability::Ready; // A later lifecycle event cannot bypass debt.
            assert!(r
                .restore_visibility_input_with(&w, evidence(), &placement(), || panic!(
                    "unresolved fence"
                ))
                .is_err());
        }
    }
    #[test]
    fn repeated_fencing_can_settle_release_but_cannot_retry_uncertain_popup_destruction() {
        let mut r = runtime(4);
        let mut w = tests::test_window();
        assert!(r
            .fence_visibility_input_with(
                &mut w,
                || Ok(1),
                |_, _| Err("release".into()),
                |p| p.window_resource.release_with(|| Err("destroy".into()))
            )
            .is_err());
        assert!(r
            .fence_visibility_input_with(
                &mut w,
                || Ok(2),
                |_, _| Ok(()),
                |p| p
                    .window_resource
                    .release_with(|| panic!("uncertain destroy must not be resent"))
            )
            .is_err());
        assert!(!r.pointer.is_grabbed());
        assert!(r.menu.is_some());
        assert!(!r.visibility_dispatch_eligible());
        r.menu
            .as_ref()
            .unwrap()
            .window_resource
            .externally_destroyed();
        r.fence_visibility_input_with(
            &mut w,
            || Ok(3),
            |_, _| Ok(()),
            |p| {
                p.window_resource
                    .release_with(|| panic!("already destroyed"))
            },
        )
        .unwrap();
        assert!(r.visibility_dispatch_eligible());
        fence(&mut r, &mut w);
        assert!(r.visibility_dispatch_eligible());
    }
    #[test]
    fn completed_menu_outcome_cannot_dispatch_after_transition_invalidation() {
        let mut r = runtime(4);
        let mut w = tests::test_window();
        let HostAction::CloseMenu { outcome, .. } =
            r.interaction.menu_release(1, MenuHit::Row(0), 101, 0)
        else {
            panic!("expected selection")
        };
        fence(&mut r, &mut w);
        assert!(r
            .complete_menu_with(
                &mut w,
                outcome,
                |_| Ok(()),
                |_| panic!("no popup"),
                |_, _| panic!("stale action")
            )
            .is_err());
    }
    #[test]
    fn enabling_requires_fresh_local_managed_visibility_placement_and_every_settled_obligation() {
        for case in 0..17 {
            let mut r = runtime(0);
            let mut w = tests::test_window();
            fence(&mut r, &mut w);
            r.body = BodyAvailability::Ready;
            let mut e = evidence();
            let mut p = placement();
            match case {
                0 => e.workspace = Workspace::Other,
                1 => e.wm_state = WmState::Iconic,
                2 => e.hidden = Evidence::Readable(true),
                3 => e.map_state = Evidence::Readable(MapState::Unviewable),
                4 => e.destroyed = true,
                5 => e.wm_state = WmState::Withdrawn,
                6 => e.coherent = false,
                7 => e.capability = Capability::Unsupported,
                8 => p.observed_monitor = 21,
                9 => p.geometry.width = 159,
                10 => p.geometry.x = 101,
                11 => p.bounds = crate::geometry::ValidOriginBounds::new(0, 50, 0, 50),
                12 => r.correction = Some((Point::new(100, 100), Instant::now())),
                13 => r.pointer.require_transition_release(),
                14 => r.menu = Some(MenuPopup::test_popup(200, 9)),
                15 => r.body = BodyAvailability::Validating,
                16 => r.input_fence.empty_confirmed = false,
                _ => unreachable!(),
            }
            assert!(r
                .restore_visibility_input_with(&w, e, &p, || panic!("not ready: {case}"))
                .is_err());
            assert!(!r.accepts_input());
        }
    }
    #[test]
    fn failed_enable_retains_blocking_and_requires_new_native_empty_fence() {
        let mut r = runtime(0);
        let mut w = tests::test_window();
        fence(&mut r, &mut w);
        r.body = BodyAvailability::Ready;
        assert!(r
            .restore_visibility_input_with(&w, evidence(), &placement(), || Err(
                "enable check".into()
            ))
            .is_err());
        assert!(!r.visibility_dispatch_eligible());
        assert!(!r.accepts_input());
        assert!(r
            .restore_visibility_input_with(&w, evidence(), &placement(), || panic!(
                "no blind retry"
            ))
            .is_err());
        fence(&mut r, &mut w);
        r.body = BodyAvailability::Ready;
        r.restore_visibility_input_with(&w, evidence(), &placement(), || Ok(140000))
            .unwrap();
        assert!(r.accepts_input());
        assert_eq!(r.input_fence.enabled_after, Some(140000));
    }
    fn pointer_events() -> Vec<Event> {
        let button = ButtonPressEvent {
            event: 100,
            detail: 1,
            time: 200,
            event_x: 80,
            event_y: 80,
            root_x: 180,
            root_y: 180,
            same_screen: true,
            ..Default::default()
        };
        vec![
            Event::ButtonPress(button),
            Event::ButtonRelease(button),
            Event::MotionNotify(MotionNotifyEvent {
                event: 100,
                ..Default::default()
            }),
            Event::EnterNotify(EnterNotifyEvent {
                event: 100,
                ..Default::default()
            }),
            Event::LeaveNotify(EnterNotifyEvent {
                event: 100,
                ..Default::default()
            }),
        ]
    }
    #[test]
    fn full_sequence_gate_rejects_stale_presses_releases_motion_and_crossings_across_wrap() {
        let mut r = runtime(0);
        let mut w = tests::test_window();
        for event in pointer_events() {
            assert!(!r.input_fence.rejects(&event, 100, 1));
        }
        fence(&mut r, &mut w);
        for event in pointer_events() {
            assert!(r.input_fence.rejects(&event, 100, 90000));
        }
        r.body = BodyAvailability::Ready;
        let boundary = 2 * 65536 + 100;
        r.restore_visibility_input_with(&w, evidence(), &placement(), || Ok(boundary))
            .unwrap();
        for event in pointer_events() {
            for sequence in [100, 65536 + 100, boundary - 1] {
                assert!(r.input_fence.rejects(&event, 100, sequence));
            }
            assert!(!r.input_fence.rejects(&event, 100, boundary));
            assert!(!r.input_fence.rejects(&event, 100, boundary + 1));
            assert!(!r.input_fence.rejects(&event, 999, 1));
        }
        // A new legitimate press reaches the existing gesture/capture path.
        let event = pointer_events().remove(0);
        if !r.input_fence.rejects(&event, 100, boundary) && r.accepts_input() {
            assert!(matches!(
                r.interaction.handle_left_press(
                    Point::new(180, 180),
                    (80, 80),
                    200,
                    Point::new(100, 100)
                ),
                HostAction::AcquireGrab { .. }
            ));
        } else {
            panic!("fresh input lost");
        }
    }
    #[test]
    fn stale_buffered_input_spanning_bounded_batches_cannot_recreate_actions() {
        let mut r = runtime(1);
        let mut w = tests::test_window();
        fence(&mut r, &mut w);
        r.body = BodyAvailability::Ready;
        r.restore_visibility_input_with(&w, evidence(), &placement(), || Ok(140000))
            .unwrap();
        let mut queue = std::collections::VecDeque::new();
        for _ in 0..40 {
            for event in pointer_events() {
                queue.push_back((event, 140000 - 65536));
            }
        }
        let mut buffered = None;
        let mut seen = 0;
        while !queue.is_empty() {
            let batch =
                drain_events_bounded(&mut buffered, 64, || Ok::<_, HostError>(queue.pop_front()))
                    .unwrap();
            assert!(batch.len() <= 64);
            for (event, sequence) in batch {
                assert!(r.input_fence.rejects(&event, 100, sequence));
                seen += 1;
            }
        }
        assert_eq!(seen, 200);
        assert!(!r.interaction.has_left_gesture());
        assert!(r.pending_move.is_none());
        assert!(!r.pointer.is_grabbed());
    }
}

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
    send: impl FnMut(Mutation) -> Result<(), HostError>,
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
        send,
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
    mut send: impl FnMut(Mutation) -> Result<(), HostError>,
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
                    send(mutation)?;
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
        control::Command::BringHere | control::Command::Recover => {
            Ok(Some(unsupported_placement(operation)))
        }
    }
}

fn observe_visibility(
    operation: &mut control::ActiveOperation,
    host: &mut impl VisibilityHost,
) -> Result<(x11::visibility::VisibilitySnapshot, Option<control::Reason>), HostError> {
    use control::{Evidence, LayerObservation, Reason, Visibility, VisibilityBaseline};
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
            unreachable!("Stage A has no placement observation")
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
    if operation.request.command == control::Command::BringTop {
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
                operation.detail.reason = Reason::X11Failure;
                // Never retry, including on a checked native failure.
                let elsewhere = snapshot.evidence.workspace == control::Workspace::Other;
                operation.show.phase = if elsewhere {
                    ShowPhase::HiddenRemoveAttempted
                } else {
                    ShowPhase::RestoreAttempted
                };
                let requested = if elsewhere {
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
            let placement = host.placement()?;
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
                if let Some(confirmation) = host.correct(
                    target,
                    visibility_cutoff(operation),
                    &mut operation.detail.progress.host_mutation_dispatched,
                )? {
                    operation.show.correction_sent = true;
                    operation.show.correction = Some((target, confirmation));
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
                let verified = host.placement()?;
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
                if !host.enable(&fresh, &verified.input, Some(visibility_cutoff(operation)))? {
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
            let placement = host.placement()?;
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
        let placement = host.placement()?;
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
                operation.stage = Stage::Layer;
                operation.detail.stage = Stage::Layer;
                operation.bring_top.layer_admitted = Some(host.now());
                operation.bring_top.layer_observe = true;
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
    if matches!(
        operation.request.command,
        control::Command::BringHere | control::Command::Recover
    ) {
        return Ok(Some(unsupported_placement(operation)));
    }
    if operation.request.command == control::Command::BringTop {
        service_bring_top_with(operation, host)
    } else {
        service_visibility_with(operation, host)
    }
}

/// Defense at the service boundary too: no host method is called for Stage A.
fn unsupported_placement(operation: &control::ActiveOperation) -> control::OperationResult {
    control::OperationResult::placement(
        operation.request.command,
        control::Status::Failed,
        control::preflight_detail(control::Reason::Unsupported, control::Stage::Preflight),
        control::Confirmation::default(),
        &operation.placement,
    )
    .expect("Stage A placement commands are unsupported without progress")
}

/// Retains marked dispatch even when the native check fails. A transitional
/// read cannot destroy a pending G1 controller; its original deadline still wins.
#[allow(clippy::too_many_arguments)]
fn service_remote_layer_with(
    layers: &mut LayerController,
    operation: &mut control::ActiveOperation,
    mut read: impl FnMut() -> Result<Result<ObservedLayer, RemoteLayerIssue>, HostError>,
    support: impl FnMut() -> Result<PropertyResult<LayerSupport>, HostError>,
    mut send: impl FnMut(Mutation, &mut bool) -> Result<(), HostError>,
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
    let result = service_qualified_layer_with(
        layers,
        request,
        Some(outer),
        true,
        || {
            let result = read()?;
            issue.set(result.as_ref().err().copied());
            Ok(result)
        },
        support,
        |mutation| {
            operation.bring_top.layer_attempted = true;
            let mut dispatched = false;
            let result = send(mutation, &mut dispatched);
            operation.bring_top.layer_dispatched |= dispatched;
            operation.detail.progress.host_mutation_dispatched |= dispatched;
            operation.bring_top.layer_checked |= result.is_ok();
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
}

/// D2 evidence only. D3 will own progression and final evidence/geometry rounds;
/// neither production placement guard can currently invoke these adapters.
#[allow(dead_code)]
struct HereEvidence {
    layout: Result<control::DestinationIdentity, x11::monitors::destination::DestinationError>,
    observation: control::HereObservation,
    snapshot: x11::visibility::VisibilitySnapshot,
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
    #[allow(dead_code)] // D2 only; both production guards remain closed until D4.
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

    #[allow(dead_code)] // D2 only; D1/D3 own one-shot authority, never this adapter.
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
#[allow(dead_code, clippy::too_many_arguments)] // Guarded D2 native seam until D3/D4.
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
mod here_adapter_tests {
    use super::*;
    use control::{
        Capability, Evidence, LayerObservation, MapState, Visibility, WmState, Workspace,
    };
    use std::cell::RefCell;
    use x11::monitors::destination::{
        DestinationError, DestinationLayout, RetainedMonitor, ViewportEvidence,
    };
    use x11::visibility::{Observation, VisibilitySnapshot};

    fn destination_layout() -> DestinationLayout {
        let area = geometry::Rect::new(-1920, -100, 1800, 1000);
        DestinationLayout {
            desktop_count: 5,
            destination_desktop: 2,
            current_desktop: 2,
            monitor: RetainedMonitor {
                name_atom: 0xf000_1234,
                bounds: area,
            },
            target_workarea: Some(area),
            viewport: ViewportEvidence::AllZero,
            usable_area: area,
            body_size: Size::new(160, 160),
            valid_origins: checked_probe_bounds(area).unwrap(),
        }
    }

    fn layout() -> control::DestinationIdentity {
        (&destination_layout()).into()
    }

    fn snapshot(
        wm_state: WmState,
        map: MapState,
        hidden: bool,
        body: u32,
        layer: LayerObservation,
    ) -> VisibilitySnapshot {
        let evidence = control::VisibilityEvidence {
            destroyed: false,
            wm_state,
            map_state: Evidence::Readable(map),
            hidden: Evidence::Readable(hidden),
            workspace: if body == u32::MAX {
                Workspace::AllDesktops
            } else if body == 2 {
                Workspace::Current
            } else {
                Workspace::Other
            },
            capability: Capability::Supported,
            coherent: true,
        };
        let mut state = vec![];
        if hidden {
            state.push(13);
        }
        match layer {
            LayerObservation::Above => state.push(14),
            LayerObservation::Below => state.push(15),
            _ => {}
        }
        VisibilitySnapshot {
            candidate: control::classify(evidence),
            layer,
            observation: Some(Observation {
                wm_state,
                map_state: evidence.map_state,
                state: Evidence::Readable(state),
                desktop: Evidence::Readable(body),
                current_desktop: Evidence::Readable(2),
                supported: Evidence::Readable(vec![12, 13, 14, 15, 16, 17]),
            }),
            evidence,
        }
    }

    fn local() -> VisibilitySnapshot {
        snapshot(
            WmState::Normal,
            MapState::Viewable,
            false,
            2,
            LayerObservation::Normal,
        )
    }

    fn settled_runtime() -> ProbeRuntime {
        let mut runtime = ProbeRuntime::new();
        runtime.body = BodyAvailability::Ready;
        runtime
            .fence_visibility_input_with(
                &mut tests::test_window(),
                || Ok(1),
                |_, _| Ok(()),
                |_| Ok(()),
            )
            .unwrap();
        assert!(runtime.visibility_dispatch_eligible());
        runtime
    }

    #[test]
    fn here_observation_keeps_ordinary_membership_separate_from_transitional_readiness() {
        for (wm, map, hidden, body, expected) in [
            (
                WmState::Normal,
                MapState::Viewable,
                false,
                2,
                Visibility::NotMinimizedHere,
            ),
            (
                WmState::Iconic,
                MapState::Unmapped,
                false,
                3,
                Visibility::NotMinimizedElsewhere,
            ),
            (
                WmState::Normal,
                MapState::Unmapped,
                false,
                2,
                Visibility::Transitional,
            ),
            (
                WmState::Normal,
                MapState::Viewable,
                true,
                2,
                Visibility::Transitional,
            ),
        ] {
            let s = snapshot(wm, map, hidden, body, LayerObservation::Below);
            let observation = here_observation(&s);
            assert_eq!(observation.membership, Some((body, 2)));
            assert_eq!(observation.visibility, expected);
            assert_eq!(observation.layer, LayerObservation::Below);
            if expected == Visibility::Transitional {
                let mut pure = control::PlacementProgress::new(2);
                assert_eq!(
                    pure.observe_here(Ok(layout()), observation),
                    control::HereDecision::Invalid(control::Reason::Unverifiable)
                );
                assert_eq!(
                    pure.baseline, None,
                    "transitional evidence is not preflight readiness"
                );
            }
        }
    }

    #[test]
    fn minimized_snapshot_keeps_each_preserved_layer_without_remote_layer_qualification() {
        for body in [2, 3] {
            for layer in [
                LayerObservation::Normal,
                LayerObservation::Above,
                LayerObservation::Below,
            ] {
                let s = snapshot(WmState::Iconic, MapState::Unmapped, true, body, layer);
                let observation = here_observation(&s);
                assert_eq!(observation.membership, Some((body, 2)));
                assert_eq!(observation.visibility, Visibility::Minimized);
                assert_eq!(observation.layer, layer);
                let mut pure = control::PlacementProgress::new(2);
                assert!(matches!(
                    pure.observe_here(Ok(layout()), observation),
                    control::HereDecision::Original | control::HereDecision::Destination
                ));
                assert_eq!(pure.baseline.unwrap().layer, layer);
                assert!(pure.baseline.unwrap().minimized);
            }
        }
    }

    #[test]
    fn sticky_missing_malformed_or_incoherent_membership_never_becomes_ordinary() {
        for case in 0..11 {
            let mut s = local();
            match case {
                0 => s.observation.as_mut().unwrap().desktop = Evidence::Readable(u32::MAX),
                1 => s.observation.as_mut().unwrap().desktop = Evidence::Missing,
                2 => s.observation.as_mut().unwrap().desktop = Evidence::Malformed,
                3 => s.observation.as_mut().unwrap().current_desktop = Evidence::Readable(u32::MAX),
                4 => s.observation.as_mut().unwrap().current_desktop = Evidence::Missing,
                5 => s.observation.as_mut().unwrap().current_desktop = Evidence::Malformed,
                6 => s.evidence.coherent = false,
                7 => s.evidence.capability = Capability::Unsupported,
                8 => s.evidence.capability = Capability::Unverifiable,
                9 => s.evidence.destroyed = true,
                _ => s.observation = None,
            }
            assert_eq!(here_observation(&s).membership, None, "case {case}");
        }
    }

    #[test]
    fn missing_state_never_invents_normal_or_visibility_but_membership_is_independent() {
        let mut s = local();
        s.observation.as_mut().unwrap().state = Evidence::Missing;
        s.evidence.hidden = Evidence::Missing;
        s.candidate = control::classify(s.evidence);
        s.layer = LayerObservation::Unknown; // Accepted observer's missing-state result.
        assert_eq!(
            here_observation(&s),
            control::HereObservation {
                membership: Some((2, 2)),
                visibility: Visibility::Unknown,
                layer: LayerObservation::Unknown,
            }
        );
    }

    #[test]
    fn here_round_brackets_fresh_visibility_with_two_layout_reads_without_retry() {
        let trace = RefCell::new(vec![]);
        let evidence = here_evidence_with(
            || {
                trace.borrow_mut().push("layout");
                Ok(Ok(layout()))
            },
            || {
                trace.borrow_mut().push("visibility");
                Ok(local())
            },
        )
        .unwrap();
        assert_eq!(*trace.borrow(), ["layout", "visibility", "layout"]);
        assert_eq!(evidence.layout, Ok(layout()));
        assert_eq!(evidence.observation.membership, Some((2, 2)));
        assert!(evidence.snapshot.evidence.coherent);
        assert_eq!(evidence.snapshot.layer, LayerObservation::Normal);
    }

    #[test]
    fn layout_drift_or_failure_withholds_layout_without_erasing_independent_membership() {
        for case in 0..4 {
            let mut reads = 0;
            let evidence = here_evidence_with(
                || {
                    reads += 1;
                    if (case == 0 && reads == 1) || (case == 1 && reads == 2) {
                        return Ok(Err(DestinationError::Unverifiable("bad layout".into())));
                    }
                    let mut l = layout();
                    if reads == 2 && case == 2 {
                        l.target_workarea = None;
                    }
                    if reads == 2 && case == 3 {
                        l.viewport = control::DestinationViewport::Absent;
                    }
                    Ok(Ok(l))
                },
                || Ok(local()),
            )
            .unwrap();
            assert_eq!(reads, 2);
            if case < 2 {
                assert_eq!(
                    evidence.layout,
                    Err(DestinationError::Unverifiable("bad layout".into()))
                );
            } else {
                assert_eq!(evidence.layout, Err(DestinationError::Changed));
            }
            assert_eq!(evidence.observation.membership, Some((2, 2)));
        }
    }

    #[derive(Debug)]
    struct NativeEvidenceFailure(u32);

    impl std::fmt::Display for NativeEvidenceFailure {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "injected native evidence failure {}", self.0)
        }
    }

    impl std::error::Error for NativeEvidenceFailure {}

    #[test]
    fn native_failure_at_each_here_read_preserves_original_error_and_stops_immediately() {
        for (failed_step, invalid_first) in
            [(0, false), (1, false), (2, false), (1, true), (2, true)]
        {
            let original = Box::new(NativeEvidenceFailure(17));
            let original_address = std::ptr::from_ref(original.as_ref());
            let error = RefCell::new(Some(original));
            let trace = RefCell::new(vec![]);
            let result = here_evidence_with(
                || {
                    let step = trace.borrow().len();
                    trace.borrow_mut().push("layout");
                    if step == failed_step {
                        Err(error.borrow_mut().take().unwrap())
                    } else if step == 0 && invalid_first {
                        Ok(Err(DestinationError::Unverifiable("bad layout".into())))
                    } else {
                        Ok(Ok(layout()))
                    }
                },
                || {
                    let step = trace.borrow().len();
                    trace.borrow_mut().push("visibility");
                    if step == failed_step {
                        Err(error.borrow_mut().take().unwrap())
                    } else {
                        Ok(local())
                    }
                },
            );
            let native = result.err().expect("Native failure must escape the round");
            assert!(std::ptr::eq(
                native.downcast_ref::<NativeEvidenceFailure>().unwrap(),
                original_address
            ));
            assert_eq!(
                *trace.borrow(),
                ["layout", "visibility", "layout"][..=failed_step]
            );
        }
    }

    #[test]
    fn here_native_fit_keeps_the_existing_signed_wire_limits() {
        for origin in [i32::from(i16::MIN), i32::from(i16::MAX)] {
            let area = geometry::Rect::new(origin, origin, 160, 160);
            assert!(checked_probe_bounds(area)
                .unwrap()
                .contains(Point::new(origin, origin)));
        }
        assert!(
            checked_probe_bounds(geometry::Rect::new(i32::from(i16::MAX), 0, 161, 160)).is_err()
        );
        assert!(
            checked_probe_bounds(geometry::Rect::new(i32::from(i16::MIN) - 1, 0, 160, 160))
                .is_err()
        );
    }

    #[test]
    fn here_native_fit_failure_remains_inner_evidence_with_independent_membership() {
        let mut bad = destination_layout();
        bad.usable_area = geometry::Rect::new(i32::from(i16::MAX), 0, 161, 160);
        let mut reads = 0;
        let evidence = here_evidence_with(
            || {
                reads += 1;
                Ok(here_destination_identity(bad.clone()))
            },
            || Ok(local()),
        )
        .unwrap();
        assert_eq!(reads, 2);
        assert!(matches!(
            evidence.layout,
            Err(DestinationError::Unverifiable(_))
        ));
        assert_eq!(evidence.observation.membership, Some((2, 2)));
        assert_eq!(
            here_destination_identity(destination_layout()),
            Ok(layout())
        );
    }

    #[test]
    fn here_transport_refuses_every_unsettled_fence_before_calling_native() {
        let now = Instant::now();
        for case in 0..13 {
            let mut runtime = settled_runtime();
            match case {
                0 => runtime.input_fence.blocked = false,
                1 => runtime.input_fence.empty_confirmed = false,
                2 => runtime.pointer.require_transition_release(),
                3 => runtime.pointer.set_grabbed(true),
                4 => runtime.pending_move = Some(Point::new(1, 1)),
                5 => runtime.final_target = Some(Point::new(1, 1)),
                6 => runtime.confirmation_deadline = Some(now),
                7 => runtime.safety.deadline = Some(now),
                8 => runtime.body = BodyAvailability::Destroyed,
                9 => runtime.correction = Some((Point::new(1, 1), now)),
                10 => runtime.menu = Some(MenuPopup::test_popup(200, 1)),
                11 => runtime.safety.observation = Some((1, ButtonObservation::Held, 1)),
                _ => {
                    runtime.interaction = InteractionManager::new();
                    runtime.interaction.handle_left_press(
                        Point::new(180, 180),
                        (80, 80),
                        1000,
                        Point::new(100, 100),
                    );
                    runtime.interaction.on_grab_acquired();
                }
            }
            assert!(!runtime.visibility_dispatch_eligible(), "case {case}");
            let mut dispatched = false;
            assert!(request_here_desktop_with(
                &runtime,
                11,
                22,
                33,
                2,
                now,
                &mut dispatched,
                |_, _, _, _, _, _| panic!("unsettled fence must not dispatch")
            )
            .is_err());
            assert!(!dispatched);
        }
        let mut dispatched = true;
        assert!(request_here_desktop_with(
            &settled_runtime(),
            11,
            22,
            33,
            u32::MAX,
            now,
            &mut dispatched,
            |_, _, _, _, _, _| panic!("sticky target must not dispatch")
        )
        .is_err());
        assert!(
            dispatched,
            "refusal cannot clear previous possible dispatch"
        );
    }

    #[test]
    fn here_transport_passes_exact_admitted_envelope_deadline_and_dispatch_reference_once() {
        let now = Instant::now();
        let deadline = now + Duration::from_millis(3500);
        let runtime = settled_runtime();
        // Stage-C outcomes injected at its consuming send_before boundary.
        for outcome in ["send failure", "check failure", "expired", "checked"] {
            for prior_dispatch in [false, true] {
                let mut dispatched = prior_dispatch;
                let mut calls = 0;
                let result = request_here_desktop_with(
                    &runtime,
                    0x1234_5678,
                    0x2345_6789,
                    0x3456_789a,
                    4,
                    deadline,
                    &mut dispatched,
                    |root, body, atom, target, dl, mark| {
                        calls += 1;
                        assert_eq!(
                            (root, body, atom, target, dl),
                            (0x1234_5678, 0x2345_6789, 0x3456_789a, 4, deadline)
                        );
                        assert_eq!(*mark, prior_dispatch);
                        match outcome {
                            "send failure" => Err("send failure".into()),
                            "expired" => Ok(false),
                            _ => {
                                *mark = true;
                                if outcome == "check failure" {
                                    Err("check failure".into())
                                } else {
                                    Ok(true)
                                }
                            }
                        }
                    },
                );
                assert_eq!(calls, 1);
                assert_eq!(
                    dispatched,
                    prior_dispatch || matches!(outcome, "check failure" | "checked")
                );
                match outcome {
                    "expired" => assert!(!result.unwrap()),
                    "checked" => assert!(result.unwrap()),
                    error => assert_eq!(result.unwrap_err().to_string(), error),
                }
            }
        }
    }

    #[test]
    fn checked_transport_is_not_relocation_but_fresh_transitional_membership_can_be() {
        let now = Instant::now();
        let mut active = None;
        control::admit(
            &mut active,
            control::Request {
                command: control::Command::BringHere,
                request_id: 99,
                reply_xid: 88,
                epoch: 7,
            },
            2,
            control::Lifecycle::Ready,
            false,
            now,
        );
        let mut op = active.unwrap();
        let original = here_observation(&snapshot(
            WmState::Iconic,
            MapState::Unmapped,
            false,
            3,
            LayerObservation::Normal,
        ));
        assert_eq!(
            op.placement.observe_here(Ok(layout()), original),
            control::HereDecision::Original
        );
        assert!(op.placement.here_fenced());
        assert_eq!(
            op.placement.observe_here(Ok(layout()), original),
            control::HereDecision::Original
        );
        assert!(op.consume_here_desktop_attempt(now));
        let mut dispatched = false;
        let checked = request_here_desktop_with(
            &settled_runtime(),
            11,
            22,
            33,
            2,
            op.deadline,
            &mut dispatched,
            |_, _, _, _, _, mark| {
                *mark = true;
                Ok(true)
            },
        )
        .unwrap();
        assert!(op
            .placement
            .record_here_desktop_transport(dispatched, checked));
        assert!(!op.placement.proof.history.relocation_confirmed);
        assert!(!op.placement.proof.current.destination);
        let transitional = here_observation(&snapshot(
            WmState::Normal,
            MapState::Unmapped,
            false,
            2,
            LayerObservation::Normal,
        ));
        assert_eq!(
            op.placement.observe_here(Ok(layout()), transitional),
            control::HereDecision::Pending
        );
        assert!(op.placement.proof.history.relocation_confirmed);
        assert!(!op.placement.begin_here_geometry());
        assert!(!op.consume_here_input_enable(now));
        assert_eq!(
            op.placement.proof.current.geometry,
            control::GeometryProof::Unknown
        );
        assert!(!op.placement.proof.history.restoration_confirmed);
    }

    #[test]
    fn here_correction_adapter_passes_exact_target_deadline_and_uncertain_dispatch_once() {
        let now = Instant::now();
        let deadline = now + Duration::from_millis(3500);
        let target = Point::new(-1900, -80);
        for outcome in 0..3 {
            let mut runtime = settled_runtime();
            let mut dispatched = false;
            let mut calls = 0;
            let result = here_correct_with(
                &mut runtime,
                target,
                deadline,
                &mut dispatched,
                |pt, dl, mark| {
                    calls += 1;
                    assert_eq!((pt, dl), (target, deadline));
                    if outcome == 0 {
                        return Ok(None);
                    }
                    *mark = true;
                    if outcome == 1 {
                        Err("checked failure".into())
                    } else {
                        Ok(Some(now + CONFIRMATION_TIMEOUT))
                    }
                },
            );
            assert_eq!(calls, 1);
            assert_eq!(dispatched, outcome != 0);
            assert_eq!(runtime.body, BodyAvailability::Validating);
            match outcome {
                0 => assert_eq!(result.unwrap(), None),
                1 => assert_eq!(result.unwrap_err().to_string(), "checked failure"),
                _ => assert_eq!(result.unwrap(), Some(now + CONFIRMATION_TIMEOUT)),
            }
        }
        let mut runtime = ProbeRuntime::new();
        assert!(here_correct_with(
            &mut runtime,
            target,
            deadline,
            &mut false,
            |_, _, _| panic!("unsettled fence")
        )
        .is_err());
        assert_eq!(runtime.body, BodyAvailability::AwaitingMap);
    }
}

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
        service_remote_layer_with(
            &mut self.runtime.layers,
            operation,
            || {
                atoms.read_remote(
                    self.atoms,
                    self.conn,
                    self.root,
                    self.window.window,
                    baseline,
                )
            },
            || atoms.read_support(self.conn, self.root),
            |mutation, dispatched| {
                atoms.send_marked(
                    self.conn,
                    self.root,
                    self.window.window,
                    mutation,
                    dispatched,
                )
            },
            Instant::now,
        )
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

fn remote_deadline(owner: &ControlWindow) -> Option<Instant> {
    owner.active().map(remote_operation_deadline)
}

fn remote_operation_deadline(operation: &control::ActiveOperation) -> Instant {
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
    operation.request.command == control::Command::BringTop
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
    !operation.is_some_and(remote_owns_layer)
}

fn remote_service_due(operation: &control::ActiveOperation, now: Instant) -> bool {
    let observe = match operation.request.command {
        control::Command::Show => operation.show.observe,
        control::Command::Hide => operation.hide.observe,
        control::Command::BringTop if remote_owns_layer(operation) => {
            operation.bring_top.layer_observe
        }
        control::Command::BringTop => operation.show.observe,
        control::Command::BringHere | control::Command::Recover => true,
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
        control::Command::BringHere | control::Command::Recover => {}
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
    owner.verify_active_ownership(host.conn)?;
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
                    unsupported_placement(operation)
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
    loop {
        let now = Instant::now();
        if duration_deadline.is_some_and(|dl| now >= dl) {
            // Ordered owner cleanup publishes Closing before layer/input teardown.
            return probe_termination_result(ProbeTermination::DurationExpired);
        }
        if let Some((Event::Error(error), _)) = &buffered_event {
            return Err(format!("Asynchronous X11 error: {error:?}").into());
        }
        service_owner_operation(
            owner,
            &mut NativeVisibility {
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
            return Ok(());
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

        if owner.active().is_none()
            && runtime.input_fence.blocked
            && runtime.body == BodyAvailability::Ready
        {
            restore_deferred_input(&mut NativeVisibility {
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
                return probe_termination_result(ProbeTermination::ControlEndpointLost);
            }
            if route_owner_request(
                owner,
                &event,
                &mut NativeVisibility {
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
                    return Ok(());
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
                    return Ok(());
                }
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
                                return Ok(());
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
                        return Ok(());
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
                                return Ok(());
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
                Event::ClientMessage(ev) if wm_delete_event(&ev, window) => {
                    return probe_termination_result(ProbeTermination::WmClose);
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
            return Ok(());
        }
        publish_ready_if(runtime.body, || owner.publish_ready(conn))?;
        if let Some(popup) = runtime.menu.as_mut() {
            popup.repaint_if_changed(conn, runtime.interaction.state())?;
        }
        // Look one event ahead before the batch-end flush. An interruption at the
        // batch boundary must discard the coalesced move just like one within a batch.
        if let Some((event, sequence)) = conn.poll_for_event_with_sequence()? {
            if owner.handle_event(conn, &event)? {
                return probe_termination_result(ProbeTermination::ControlEndpointLost);
            }
            if route_owner_request(
                owner,
                &event,
                &mut NativeVisibility {
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
                    return Ok(());
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
            wm_protocols: 3,
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
            wm_protocols: 3,
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
            wm_protocols: 3,
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
            wm_protocols: 3,
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
            wm_protocols: 3,
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
            wm_protocols: 3,
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
            number_of_desktops: 90,
            desktop_viewport: 91,
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
            number_of_desktops: 90,
            desktop_viewport: 91,
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
        for atom in [atoms.number_of_desktops, atoms.desktop_viewport] {
            let event = property(1, atom);
            assert!(!relevant_layout_event(&event, 1, 100, &atoms));
            assert!(!interrupts_pending_movement(&event, 1, 100, &atoms));
            assert!(!interrupts_layer(&event, 1, 100, &atoms));
            assert!(!visibility_layout_interruption(&event, 1, &atoms));
        }
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
            number_of_desktops: 90,
            desktop_viewport: 91,
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

    #[test]
    fn structured_layer_outcomes_distinguish_already_waiting_and_fresh_confirmation() {
        let now = Instant::now();
        let mut layers = LayerController::default();
        assert_eq!(
            service_layer_with(
                &mut layers,
                Some(Layer::Above),
                || Ok(Ok(ObservedLayer::Above)),
                || panic!("already matches"),
                |_| panic!("already matches"),
                || now,
            )
            .unwrap(),
            LayerOutcome::AlreadyMatches
        );
        let sends = Cell::new(0);
        assert_eq!(
            service_layer_with(
                &mut layers,
                Some(Layer::Above),
                || Ok(Ok(ObservedLayer::Normal)),
                || Ok(Ok(SUPPORT)),
                |mutation| {
                    assert_eq!(mutation, Mutation::AddAbove);
                    sends.set(sends.get() + 1);
                    Ok(())
                },
                || now,
            )
            .unwrap(),
            LayerOutcome::Waiting
        );
        assert_eq!(sends.get(), 1);
        let pending = layers.pending();
        assert_eq!(
            service_layer_with(
                &mut layers,
                Some(Layer::Below),
                || panic!("busy must not read"),
                || panic!("busy"),
                |_| panic!("busy"),
                || now,
            )
            .unwrap(),
            LayerOutcome::Busy
        );
        assert_eq!(layers.pending(), pending);
        assert_eq!(
            service_layer_with(
                &mut layers,
                None,
                || Ok(Ok(ObservedLayer::Above)),
                || panic!("confirmation"),
                |_| panic!("confirmation"),
                || now,
            )
            .unwrap(),
            LayerOutcome::Confirmed
        );
        assert!(layers.pending().is_none());
    }

    #[test]
    fn structured_support_and_property_failures_are_distinct_and_stop_mutation() {
        let now = Instant::now();
        for (support, expected) in [
            (
                Ok(LayerSupport {
                    above: false,
                    ..SUPPORT
                }),
                LayerOutcome::Unsupported,
            ),
            (Err("malformed support"), LayerOutcome::Unverifiable),
        ] {
            let mut layers = LayerController::default();
            assert_eq!(
                service_layer_with(
                    &mut layers,
                    Some(Layer::Above),
                    || Ok(Ok(ObservedLayer::Below)),
                    || Ok(support),
                    |_| panic!("no removal before complete support"),
                    || now,
                )
                .unwrap(),
                expected
            );
            assert!(layers.pending().is_none());
        }
        let mut layers = LayerController::default();
        pending_above(&mut layers, now);
        assert_eq!(
            service_layer_with(
                &mut layers,
                None,
                || Ok(Err("malformed state")),
                || panic!("unverifiable"),
                |_| panic!("unverifiable"),
                || now,
            )
            .unwrap(),
            LayerOutcome::Unverifiable
        );
        assert!(layers.pending().is_none());
    }

    #[test]
    fn contradictory_layer_flags_never_produce_structured_above_confirmation() {
        let now = Instant::now();
        let mut layers = LayerController::default();
        service_layer_with(
            &mut layers,
            Some(Layer::Above),
            || Ok(Ok(ObservedLayer::Normal)),
            || Ok(Ok(SUPPORT)),
            |_| Ok(()),
            || now,
        )
        .unwrap();
        let deadline = layers.deadline().unwrap();
        for (time, expected) in [
            (now, LayerOutcome::Waiting),
            (deadline, LayerOutcome::TimedOut(Phase::AwaitFinal)),
        ] {
            assert_eq!(
                service_layer_with(
                    &mut layers,
                    None,
                    || Ok(Ok(ObservedLayer::Conflict)),
                    || panic!("no retry or conflict repair after dispatch"),
                    |_| panic!("no retry"),
                    || time,
                )
                .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn structured_timeout_and_external_interruption_retain_the_controller_phase() {
        let now = Instant::now();
        for removal_done in [false, true] {
            let mut layers = LayerController::default();
            pending_above(&mut layers, now);
            if removal_done {
                service_layer_with(
                    &mut layers,
                    None,
                    || Ok(Ok(ObservedLayer::Normal)),
                    || Ok(Ok(SUPPORT)),
                    |_| Ok(()),
                    || now,
                )
                .unwrap();
            }
            let phase = layers.pending().unwrap().phase;
            let deadline = layers.deadline().unwrap();
            assert_eq!(
                service_layer_with(
                    &mut layers,
                    None,
                    || Ok(Ok(ObservedLayer::Normal)),
                    || panic!("deadline"),
                    |_| panic!("deadline"),
                    || deadline,
                )
                .unwrap(),
                LayerOutcome::TimedOut(phase)
            );
            assert!(layers.pending().is_none());
            pending_above(&mut layers, now);
            assert_eq!(
                abort_layer(&mut layers, "external lifecycle interruption"),
                Some(LayerOutcome::Interrupted(Phase::AwaitRemoval))
            );
            assert!(layers.pending().is_none());
            assert_eq!(abort_layer(&mut layers, "already aborted"), None);
        }
    }

    #[test]
    fn bounded_service_expiry_never_sends_and_final_fresh_read_can_confirm() {
        let now = Instant::now();
        let deadline = now + Duration::from_millis(200);
        for achieved in [false, true] {
            let mut layers = LayerController::default();
            let clock = Cell::new(now);
            let reads = Cell::new(0);
            assert_eq!(
                service_layer_bounded_with(
                    &mut layers,
                    Some(Layer::Above),
                    Some(deadline),
                    || {
                        reads.set(reads.get() + 1);
                        Ok(Ok(if achieved && reads.get() == 2 {
                            ObservedLayer::Above
                        } else {
                            ObservedLayer::Below
                        }))
                    },
                    || {
                        clock.set(deadline);
                        Ok(Ok(SUPPORT))
                    },
                    |_| panic!("support read exhausted the outer budget"),
                    || clock.get(),
                )
                .unwrap(),
                if achieved {
                    LayerOutcome::Confirmed
                } else {
                    LayerOutcome::TimedOut(Phase::AwaitRemoval)
                }
            );
            assert_eq!(reads.get(), 2);
            assert!(layers.pending().is_none());
        }
        let mut layers = LayerController::default();
        assert_eq!(
            service_layer_bounded_with(
                &mut layers,
                Some(Layer::Above),
                Some(now),
                || Ok(Ok(ObservedLayer::Normal)),
                || panic!("expired entry"),
                |_| panic!("expired entry"),
                || now,
            )
            .unwrap(),
            LayerOutcome::TimedOut(Phase::AwaitFinal)
        );
    }

    #[test]
    fn fresh_above_after_removal_skips_addition_and_checked_error_cannot_retry() {
        let now = Instant::now();
        let mut layers = LayerController::default();
        let mut reads = VecDeque::from([ObservedLayer::Below, ObservedLayer::Above]);
        let sends = Cell::new(0);
        assert_eq!(
            service_layer_with(
                &mut layers,
                Some(Layer::Above),
                || Ok(Ok(reads.pop_front().unwrap())),
                || Ok(Ok(SUPPORT)),
                |mutation| {
                    assert!(matches!(mutation, Mutation::Remove(_)));
                    sends.set(sends.get() + 1);
                    Ok(())
                },
                || now,
            )
            .unwrap(),
            LayerOutcome::Confirmed
        );
        assert_eq!(sends.get(), 1);
        let result = service_layer_with(
            &mut layers,
            Some(Layer::Above),
            || Ok(Ok(ObservedLayer::Below)),
            || Ok(Ok(SUPPORT)),
            |_| {
                sends.set(sends.get() + 1);
                Err("checked native failure".into())
            },
            || now,
        );
        assert_eq!(result.unwrap_err().to_string(), "checked native failure");
        assert!(layers.pending().is_none());
        assert_eq!(
            service_layer_with(
                &mut layers,
                None,
                || Ok(Ok(ObservedLayer::Normal)),
                || panic!("no second request after uncertainty"),
                |_| panic!("no second request"),
                || now,
            )
            .unwrap(),
            LayerOutcome::Waiting
        );
        assert_eq!(sends.get(), 2);
    }

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
                        .map(|_| ())
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
        let outcome = finish_probe_with(result.map(|_| ()), || {
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
            number_of_desktops: 90,
            desktop_viewport: 91,
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
                    .map(|_| ())
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
    fn control_commands_parse_into_independent_caller_role() {
        for (flag, command) in [
            ("--hide", Command::Hide),
            ("--show", Command::Show),
            ("--bring-top", Command::BringTop),
        ] {
            assert_eq!(parse_args([flag]), Ok(CliMode::Control(command)));
        }
    }
    #[test]
    fn guarded_placement_commands_remain_unknown_cli_options() {
        for flag in ["--bring-here", "--recover"] {
            let error = parse_args([flag]).unwrap_err();
            assert_eq!(error.exit_code(), ExitCode::Usage);
            assert!(error.0.contains(&format!("Unknown option: '{flag}'")));
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

#[cfg(test)]
mod ownership_integration_tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn ownership_then_delay_without_body_then_initialization_and_duration() {
        let start = Instant::now();
        let mut state = (Vec::new(), start, false);
        let outcome = owner_startup_with(
            &mut state,
            |state| {
                state.0.push("acquire");
                Ok(Acquisition::Owned)
            },
            |state| {
                assert_eq!(state.0, ["acquire"]);
                assert!(!state.2, "no body during delay");
                state.0.push("delay");
                state.1 += Duration::from_secs(20);
                Ok(())
            },
            |state| {
                assert_eq!(state.0, ["acquire", "delay"]);
                state.2 = true;
                state.0.push("body/renderer");
                state.1 += Duration::from_secs(2); // ordinary initialization precedes run_probe
                let deadline = probe_duration_deadline(state.1, Some(5))?.unwrap();
                assert_eq!(deadline, start + Duration::from_secs(27));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(outcome, Acquisition::Owned);
        assert!(state.2);
    }

    #[test]
    fn duplicate_and_acquisition_failure_cannot_delay_or_create_body() {
        assert_eq!(
            owner_startup_with(
                &mut (),
                |_| Ok(Acquisition::Duplicate),
                |_| panic!("duplicate delay"),
                |_| panic!("duplicate body")
            )
            .unwrap(),
            Acquisition::Duplicate
        );
        let error = owner_startup_with(
            &mut (),
            |_| Err("ungrab failed".into()),
            |_| panic!("failed acquisition delay"),
            |_| panic!("failed acquisition body"),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "ungrab failed");
    }

    #[test]
    fn loss_during_delay_blocks_body_and_reaches_cleanup() {
        let calls = RefCell::new(Vec::new());
        let result = owner_startup_with(
            &mut (),
            |_| Ok(Acquisition::Owned),
            |_| Err("selection lost during delay".into()),
            |_| panic!("body after loss"),
        );
        let error = finish_probe_with(result.map(|_| ()), || {
            owner_cleanup_with(
                &mut (),
                |_| {
                    calls.borrow_mut().push("closing check");
                    Ok(())
                },
                || {
                    calls.borrow_mut().push("empty body cleanup");
                    Ok(())
                },
                |_| {
                    calls.borrow_mut().push("control destroy");
                    Ok(())
                },
                || {
                    calls.borrow_mut().push("flush");
                    Ok(())
                },
            )
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "selection lost during delay");
        assert_eq!(
            *calls.borrow(),
            [
                "closing check",
                "empty body cleanup",
                "control destroy",
                "flush"
            ]
        );
    }

    #[test]
    fn closing_precedes_teardown_and_control_is_last_even_after_failures() {
        for original_failure in [false, true] {
            let calls = RefCell::new(Vec::new());
            let result = if original_failure {
                Err("original runtime failure".into())
            } else {
                Ok(())
            };
            let error = finish_probe_with(result, || {
                owner_cleanup_with(
                    &mut (),
                    |_| {
                        calls.borrow_mut().push("closing");
                        Err("closing failure".into())
                    },
                    || {
                        x11::resource::cleanup_all([
                            Box::new(|| {
                                calls.borrow_mut().push("input/popup");
                                Err("release failure".into())
                            })
                                as Box<dyn FnOnce() -> Result<(), HostError>>,
                            Box::new(|| {
                                calls.borrow_mut().push("renderer");
                                Err("renderer failure".into())
                            }),
                            Box::new(|| {
                                calls.borrow_mut().push("body");
                                Ok(())
                            }),
                        ])
                    },
                    |_| {
                        calls.borrow_mut().push("control");
                        Err("destroy failure".into())
                    },
                    || {
                        calls.borrow_mut().push("flush");
                        Ok(())
                    },
                )
            })
            .unwrap_err();
            assert_eq!(
                error.to_string(),
                if original_failure {
                    "original runtime failure"
                } else {
                    "closing failure"
                }
            );
            assert_eq!(
                *calls.borrow(),
                [
                    "closing",
                    "input/popup",
                    "renderer",
                    "body",
                    "control",
                    "flush"
                ]
            );
        }
    }

    #[test]
    fn runtime_control_loss_remains_primary_through_ordered_cleanup() {
        for cleanup_fails in [false, true] {
            let calls = RefCell::new(Vec::new());
            // Both the event batch and one-event lookahead use this exit classification
            // after the native ownership check has verified actual endpoint loss.
            let result = probe_termination_result(ProbeTermination::ControlEndpointLost);
            let error = finish_probe_with(result, || {
                owner_cleanup_with(
                    &mut (),
                    |_| {
                        calls.borrow_mut().push("closing ownership check");
                        Ok(()) // already lost: native closing skips descriptor publication
                    },
                    || {
                        calls.borrow_mut().push("input/popup/renderer/body");
                        if cleanup_fails {
                            Err("body cleanup failed".into())
                        } else {
                            Ok(())
                        }
                    },
                    |_| {
                        calls.borrow_mut().push("control cleanup");
                        if cleanup_fails {
                            Err("control cleanup failed".into())
                        } else {
                            Ok(())
                        }
                    },
                    || {
                        calls.borrow_mut().push("flush");
                        Ok(())
                    },
                )
            })
            .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Control endpoint lost during running probe"
            );
            assert_eq!(
                *calls.borrow(),
                [
                    "closing ownership check",
                    "input/popup/renderer/body",
                    "control cleanup",
                    "flush"
                ]
            );
        }
    }

    #[test]
    fn duration_and_wm_close_remain_normal_completion_after_cleanup() {
        for reason in [ProbeTermination::DurationExpired, ProbeTermination::WmClose] {
            let cleaned = Cell::new(false);
            finish_probe_with(probe_termination_result(reason), || {
                cleaned.set(true);
                Ok(())
            })
            .unwrap();
            assert!(cleaned.get());
        }
    }

    #[test]
    fn ready_publication_follows_existing_map_and_correction_truth() {
        let mut runtime = ProbeRuntime::new();
        let mut window = tests::test_window();
        let bounds = crate::geometry::ValidOriginBounds::new(0, 50, 0, 50);
        let actual = crate::geometry::Rect::new(100, 100, 160, 160);
        let writes = Cell::new(0);
        let publish = || {
            writes.set(writes.get() + 1);
            Ok(())
        };
        publish_ready_if(runtime.body, publish).unwrap();
        assert_eq!(writes.get(), 0);
        assert_eq!(
            probe_deadline(&runtime, None),
            None,
            "map wait has no timeout or idle polling"
        );
        runtime.body = BodyAvailability::Validating; // native map/workspace gate in run_probe
        runtime
            .placement_with(&mut window, actual, &bounds, |_, target| {
                assert_eq!(target, Point::new(50, 50));
                Ok(())
            })
            .unwrap();
        publish_ready_if(runtime.body, publish).unwrap();
        assert_eq!(writes.get(), 0);
        let deadline = runtime.correction.unwrap().1;
        runtime
            .check_correction(deadline, Point::new(50, 50), &bounds)
            .unwrap();
        publish_ready_if(runtime.body, publish).unwrap();
        assert_eq!(writes.get(), 1);
        assert!(publish_ready_if(runtime.body, || Err("descriptor write failed".into())).is_err());
        for state in [
            BodyAvailability::AwaitingMap,
            BodyAvailability::Unavailable,
            BodyAvailability::Validating,
            BodyAvailability::Destroyed,
        ] {
            publish_ready_if(state, || panic!("unready publication")).unwrap();
        }
    }

    #[test]
    fn duration_is_optional_and_does_not_add_map_timeout() {
        let runtime = ProbeRuntime::new();
        assert_eq!(probe_duration_deadline(Instant::now(), None).unwrap(), None);
        assert_eq!(probe_deadline(&runtime, None), None);
        let deadline = probe_duration_deadline(Instant::now(), Some(30)).unwrap();
        assert_eq!(probe_deadline(&runtime, deadline), deadline);
        assert!(probe_duration_deadline(Instant::now(), Some(u64::MAX)).is_err());
    }
}

#[cfg(test)]
mod show_tests {
    use super::*;
    use crate::control::{
        self, ActiveOperation, Capability, Command, Evidence, LayerObservation as L, Lifecycle,
        MapState, Reason, ShowPhase, Status, Visibility, VisibilityEvidence, WmState, Workspace,
    };
    use crate::geometry::{Rect, ValidOriginBounds};
    use std::cell::RefCell;
    use std::collections::VecDeque;

    #[derive(Clone, Copy)]
    struct Reading {
        evidence: VisibilityEvidence,
        layer: L,
        desktop: u32,
        current: u32,
    }
    fn reading(visibility: Visibility, layer: L, desktop: u32, current: u32) -> Reading {
        let workspace = if desktop == u32::MAX {
            Workspace::AllDesktops
        } else if desktop == current {
            Workspace::Current
        } else {
            Workspace::Other
        };
        let (wm_state, map_state, hidden) = match visibility {
            Visibility::Minimized => (WmState::Iconic, MapState::Unmapped, true),
            Visibility::NotMinimizedElsewhere => (WmState::Iconic, MapState::Unmapped, false),
            Visibility::Withdrawn => (WmState::Withdrawn, MapState::Unmapped, false),
            _ => (WmState::Normal, MapState::Viewable, false),
        };
        Reading {
            evidence: VisibilityEvidence {
                destroyed: visibility == Visibility::Destroyed,
                wm_state,
                map_state: Evidence::Readable(map_state),
                hidden: Evidence::Readable(hidden),
                workspace,
                capability: Capability::Supported,
                coherent: true,
            },
            layer,
            desktop,
            current,
        }
    }
    struct Fake {
        now: Instant,
        current: Reading,
        reads: VecDeque<Reading>,
        runtime: ProbeRuntime,
        window: ManagedProbeWindow,
        geometry: Rect,
        bounds: ValidOriginBounds,
        area: Rect,
        monitor: u32,
        log: Vec<&'static str>,
        fail: Option<&'static str>,
        advance: Option<(&'static str, Duration)>,
        refuse_correction: bool,
        layer_mode: u8, // 0 immediate, 1 unchanged, 2 removal only, 3 spontaneous Above
        layer_issue: Option<RemoteLayerIssue>,
        layer_support: LayerSupport,
        mutations: Vec<Mutation>,
        changes: VecDeque<(&'static str, Reading)>,
        step_failures: VecDeque<&'static str>,
        layer_reads: VecDeque<Reading>,
        support_reads: VecDeque<PropertyResult<LayerSupport>>,
        supported: Evidence<Vec<u32>>,
        fail_before_layer_dispatch: bool,
    }
    impl Fake {
        fn new(current: Reading) -> Self {
            let mut runtime = ProbeRuntime::new();
            runtime.body = BodyAvailability::Ready;
            runtime.arm_map_input_sync(500);
            Self {
                now: Instant::now(),
                current,
                reads: VecDeque::new(),
                runtime,
                window: tests::test_window(),
                geometry: Rect::new(100, 100, 160, 160),
                bounds: ValidOriginBounds::new(0, 500, 0, 500),
                area: Rect::new(0, 0, 660, 660),
                monitor: 20,
                log: vec![],
                fail: None,
                advance: None,
                refuse_correction: false,
                layer_mode: 0,
                layer_issue: None,
                layer_support: LayerSupport {
                    state: true,
                    above: true,
                    below: true,
                },
                mutations: vec![],
                changes: VecDeque::new(),
                step_failures: VecDeque::new(),
                layer_reads: VecDeque::new(),
                support_reads: VecDeque::new(),
                supported: Evidence::Readable(vec![12, 13, 14, 15, 16, 17]),
                fail_before_layer_dispatch: false,
            }
        }
        fn step(&mut self, name: &'static str) -> Result<(), HostError> {
            self.log.push(name);
            if self.changes.front().is_some_and(|(step, _)| *step == name) {
                self.current = self.changes.pop_front().unwrap().1;
            }
            if self.step_failures.front() == Some(&name) {
                self.step_failures.pop_front();
                return Err(format!("injected {name}").into());
            }
            if let Some((step, duration)) = self.advance {
                if name == step {
                    self.now += duration;
                    self.advance = None;
                }
            }
            if self.fail == Some(name) {
                return Err(format!("injected {name}").into());
            }
            Ok(())
        }
        fn operation(&self) -> ActiveOperation {
            self.command_operation(Command::Show)
        }
        fn command_operation(&self, command: Command) -> ActiveOperation {
            let mut active = None;
            control::admit(
                &mut active,
                control::Request {
                    command,
                    request_id: 99,
                    reply_xid: 88,
                    epoch: 7,
                },
                crate::control::NO_DESTINATION,
                Lifecycle::Ready,
                false,
                self.now,
            );
            let mut operation = active.unwrap();
            operation.stage = if command == Command::Hide {
                control::Stage::Hide
            } else {
                control::Stage::Show
            };
            operation.detail = control::preflight_detail(Reason::None, operation.stage);
            operation
        }
        fn run(&mut self, operation: &mut ActiveOperation) -> Option<control::OperationResult> {
            service_show_with(operation, self).unwrap()
        }
        fn count(&self, step: &str) -> usize {
            self.log.iter().filter(|s| **s == step).count()
        }
    }
    impl VisibilityHost for Fake {
        fn layer_atoms(&self) -> LayerAtoms {
            fake_layer_atoms()
        }
        fn now(&self) -> Instant {
            self.now
        }
        fn observe(&mut self) -> Result<x11::visibility::VisibilitySnapshot, HostError> {
            self.step("observe")?;
            if let Some(read) = self.reads.pop_front() {
                self.current = read;
            }
            let r = self.current;
            Ok(x11::visibility::VisibilitySnapshot {
                evidence: r.evidence,
                layer: r.layer,
                candidate: control::classify(r.evidence),
                observation: Some(x11::visibility::Observation {
                    wm_state: r.evidence.wm_state,
                    map_state: r.evidence.map_state,
                    state: match r.evidence.hidden {
                        Evidence::Readable(true) => Evidence::Readable(vec![13]),
                        Evidence::Readable(false) => Evidence::Readable(vec![]),
                        Evidence::Missing => Evidence::Missing,
                        Evidence::Malformed => Evidence::Malformed,
                    },
                    desktop: Evidence::Readable(r.desktop),
                    current_desktop: Evidence::Readable(r.current),
                    supported: self.supported.clone(),
                }),
            })
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
        fn deferred_placement(&mut self, p: &ShowPlacement) -> Result<bool, HostError> {
            if self.runtime.correction.is_some() {
                return Ok(false);
            }
            self.runtime.body = BodyAvailability::Validating;
            let mut target = None;
            self.runtime.placement_with(
                &mut self.window,
                p.input.geometry,
                &p.input.bounds,
                |_, point| {
                    target = Some(point);
                    Ok(())
                },
            )?;
            if let Some(target) = target {
                self.log.push("ordinary_correct");
                self.geometry.x = target.x;
                self.geometry.y = target.y;
            }
            Ok(self.runtime.body == BodyAvailability::Ready)
        }
        fn fence(&mut self) -> Result<(), HostError> {
            self.step("fence")?;
            let log = RefCell::new(vec![]);
            let fail = self.fail;
            let outcome = self.runtime.fence_visibility_input_with(
                &mut self.window,
                || {
                    log.borrow_mut().push("empty");
                    if fail == Some("empty") {
                        Err("injected empty".into())
                    } else {
                        Ok(70000)
                    }
                },
                |r, w| {
                    log.borrow_mut().push("release");
                    assert!(r.pending_move.is_none());
                    assert!(w.in_flight_moves.is_empty());
                    if fail == Some("release") {
                        Err("injected release".into())
                    } else {
                        Ok(())
                    }
                },
                |_| {
                    log.borrow_mut().push("popup");
                    if fail == Some("popup") {
                        Err("injected popup".into())
                    } else {
                        Ok(())
                    }
                },
            );
            self.log.extend(log.into_inner());
            outcome
        }
        fn iconify(&mut self, deadline: Instant, dispatched: &mut bool) -> Result<bool, HostError> {
            assert!(self.runtime.visibility_dispatch_eligible());
            if self.now >= deadline {
                return Ok(false);
            }
            *dispatched = true;
            self.step("iconify")?;
            Ok(true)
        }
        fn restore(&mut self, deadline: Instant, dispatched: &mut bool) -> Result<bool, HostError> {
            assert!(self.runtime.visibility_dispatch_eligible());
            if self.now >= deadline {
                return Ok(false);
            }
            *dispatched = true;
            self.step("user_time_zero")?;
            if self.now >= deadline {
                return Ok(false);
            }
            self.step("map")?;
            Ok(true) // acknowledgement alone deliberately changes no observation
        }
        fn hidden(
            &mut self,
            hidden: bool,
            deadline: Instant,
            dispatched: &mut bool,
        ) -> Result<bool, HostError> {
            assert!(self.runtime.visibility_dispatch_eligible());
            if self.now >= deadline {
                return Ok(false);
            }
            *dispatched = true;
            self.step(if hidden {
                "add_hidden"
            } else {
                "remove_hidden"
            })?;
            Ok(true) // the WM's state must arrive through a later fresh observation
        }
        fn placement(&mut self) -> Result<ShowPlacement, HostError> {
            self.step("geometry")?;
            Ok(ShowPlacement {
                area: self.area,
                current_desktop: self.current.current,
                input: InputPlacement {
                    selected_monitor: 20,
                    observed_monitor: self.monitor,
                    geometry: self.geometry,
                    bounds: self.bounds,
                },
            })
        }
        fn correct(
            &mut self,
            target: Point,
            deadline: Instant,
            dispatched: &mut bool,
        ) -> Result<Option<Instant>, HostError> {
            if self.now + CONFIRMATION_TIMEOUT > deadline {
                return Ok(None);
            }
            *dispatched = true;
            let confirmation = self.now + CONFIRMATION_TIMEOUT;
            self.step("correct")?;
            self.runtime.body = BodyAvailability::Validating;
            if !self.refuse_correction {
                self.geometry.x = target.x;
                self.geometry.y = target.y;
            }
            Ok(Some(confirmation))
        }
        fn enable(
            &mut self,
            snapshot: &x11::visibility::VisibilitySnapshot,
            p: &InputPlacement,
            deadline: Option<Instant>,
        ) -> Result<bool, HostError> {
            if deadline.is_some_and(|dl| self.now >= dl) {
                return Ok(false);
            }
            self.step("enable")?;
            self.window.confirmed_origin = p.geometry.origin();
            self.window.cancel_in_flight_moves();
            self.runtime.body = BodyAvailability::Ready;
            let fail = self.fail;
            self.runtime.restore_visibility_input_with(
                &self.window,
                snapshot.evidence,
                p,
                || {
                    if fail == Some("shape") {
                        Err("injected shape".into())
                    } else {
                        Ok(4 * 65536 + 123)
                    }
                },
            )?;
            Ok(true)
        }
    }
    impl BringTopHost for Fake {
        fn qualify_layer(
            &self,
            snapshot: &x11::visibility::VisibilitySnapshot,
            baseline: control::VisibilityBaseline,
        ) -> Result<ObservedLayer, RemoteLayerIssue> {
            fake_layer_atoms().qualify_remote(snapshot, baseline)
        }
        fn service_above(
            &mut self,
            operation: &mut ActiveOperation,
        ) -> Result<(LayerOutcome, Option<RemoteLayerIssue>), HostError> {
            let baseline = operation.show.baseline.unwrap();
            let mut layers = std::mem::take(&mut self.runtime.layers);
            let host = RefCell::new(&mut *self);
            let result = service_remote_layer_with(
                &mut layers,
                operation,
                || {
                    let mut h = host.borrow_mut();
                    h.step("layer_read")?;
                    if let Some(issue) = h.layer_issue {
                        return Ok(Err(issue));
                    }
                    if let Some(read) = h.layer_reads.pop_front() {
                        h.current = read;
                    }
                    let snapshot = h.observe()?;
                    Ok(fake_layer_atoms().qualify_remote(&snapshot, baseline))
                },
                || {
                    let mut h = host.borrow_mut();
                    h.step("layer_support")?;
                    let default = h.layer_support;
                    Ok(h.support_reads.pop_front().unwrap_or(Ok(default)))
                },
                |mutation, dispatched| {
                    let mut h = host.borrow_mut();
                    if h.fail_before_layer_dispatch {
                        return Err("injected before layer dispatch".into());
                    }
                    *dispatched = true;
                    h.mutations.push(mutation);
                    h.step(match mutation {
                        Mutation::Remove(_) => "remove_below",
                        Mutation::AddAbove => "add_above",
                        Mutation::AddBelow => panic!("G targets only Above"),
                    })?;
                    match mutation {
                        Mutation::Remove(_) if h.layer_mode == 3 => h.current.layer = L::Above,
                        Mutation::Remove(_) if h.layer_mode != 1 => h.current.layer = L::Normal,
                        Mutation::AddAbove if h.layer_mode == 0 => h.current.layer = L::Above,
                        _ => {}
                    }
                    Ok(())
                },
                || host.borrow().now,
            );
            self.runtime.layers = layers;
            result
        }
        fn abort_above(&mut self) {
            self.runtime.layers.abort();
        }
        fn layer_deadline(&self) -> Option<Instant> {
            self.runtime.layers.deadline()
        }
    }
    fn fake_layer_atoms() -> LayerAtoms {
        LayerAtoms {
            state: 12,
            supported: 18,
            above: 16,
            below: 17,
        }
    }
    fn minimized() -> Fake {
        Fake::new(reading(Visibility::Minimized, L::Normal, 2, 2))
    }
    fn visible() -> Reading {
        reading(Visibility::NotMinimizedHere, L::Normal, 2, 2)
    }
    #[test]
    fn stage_a_placement_service_cannot_call_any_body_host_method() {
        for command in [Command::BringHere, Command::Recover] {
            let mut host = Fake::new(visible());
            let mut active = None;
            control::admit(
                &mut active,
                control::Request {
                    command,
                    request_id: 99,
                    reply_xid: 88,
                    epoch: 7,
                },
                2,
                Lifecycle::Ready,
                false,
                host.now,
            );
            let mut operation = active.unwrap();
            let original = operation;
            let result = service_remote_with(&mut operation, &mut host)
                .unwrap()
                .unwrap();
            assert_eq!(
                (result.status(), result.detail().reason),
                (Status::Failed, Reason::Unsupported)
            );
            assert!(host.log.is_empty());
            assert!(host.mutations.is_empty());
            assert_eq!(operation, original);
            assert_eq!(host.runtime.body, BodyAvailability::Ready);
        }
    }

    fn assert_success(result: control::OperationResult, here: bool, dispatched: bool) {
        assert_eq!(result.status(), Status::Success);
        assert_eq!(result.exit_code(), control::ExitCode::Success);
        let d = result.detail();
        assert_eq!(d.reason, Reason::None);
        assert!(d.progress.not_minimized_confirmed);
        assert_eq!(d.progress.input_ready, here);
        assert_eq!(d.progress.placement_deferred, !here);
        assert_eq!(d.progress.host_mutation_dispatched, dispatched);
        assert_eq!(
            d.visibility,
            if here {
                Visibility::NotMinimizedHere
            } else {
                Visibility::NotMinimizedElsewhere
            }
        );
        control::Detail::decode(d.encode()).unwrap();
    }

    /// Reuse the existing visibility fake and production owner progression in
    /// the transport's coupled caller-timeout test, without exposing its internals.
    pub(crate) struct CallerTimeoutShow {
        host: Fake,
    }

    impl CallerTimeoutShow {
        pub(crate) fn new(now: Instant) -> Self {
            let mut host = minimized();
            host.now = now;
            Self { host }
        }

        pub(crate) fn service_at(
            &mut self,
            operation: &mut ActiveOperation,
            now: Instant,
        ) -> Result<Option<control::OperationResult>, HostError> {
            self.host.now = now;
            if !remote_service_due(operation, now) {
                return Ok(None);
            }
            service_remote_with(operation, &mut self.host)
        }

        pub(crate) fn observe_restored(&mut self, operation: &mut ActiveOperation) {
            // Model a WM notification and subsequent fresh readable state, not
            // a terminal result. Production Show still verifies placement/input.
            self.host.current = visible();
            mark_remote_observation(operation, false);
        }

        pub(crate) fn assert_pending(&self) {
            assert_eq!(
                control::classify(self.host.current.evidence).visibility,
                Visibility::Minimized
            );
            assert_eq!(self.host.count("map"), 1);
            assert_eq!(self.host.count("user_time_zero"), 1);
            assert_eq!(self.host.count("enable"), 0);
            assert!(self.host.runtime.input_fence.blocked);
            assert!(!self.host.runtime.accepts_input());
        }

        pub(crate) fn assert_completed(&self, result: control::OperationResult) {
            assert_success(result, true, true);
            assert_eq!(
                control::classify(self.host.current.evidence).visibility,
                Visibility::NotMinimizedHere
            );
            assert_eq!(self.host.count("map"), 1);
            assert_eq!(self.host.count("user_time_zero"), 1);
            assert_eq!(self.host.count("fence"), 1);
            assert_eq!(self.host.count("enable"), 1);
            assert!(self.host.runtime.accepts_input());
            assert!(!self.host.runtime.pointer.is_grabbed());
            assert!(self.host.runtime.menu.is_none());
        }

        pub(crate) fn trace(&self) -> &[&'static str] {
            &self.host.log
        }
    }

    /// Standalone Hide/Show snapshots for the Section-12 lifecycle regressions.
    /// Keep the fake alive through terminal delivery so shutdown can be checked
    /// against actual dispatch, geometry and input state, not just phase labels.
    pub(crate) mod h_c2 {
        use super::*;
        use control::{HidePhase, Stage};

        pub(crate) struct Fixture {
            host: Fake,
            original_error: Option<HostError>,
        }

        #[derive(Debug, PartialEq, Eq)]
        pub(crate) struct HostState {
            body: u32,
            geometry: Rect,
            availability: BodyAvailability,
            blocked: bool,
            empty_confirmed: bool,
            input_ready: bool,
        }

        impl Fixture {
            pub(crate) fn take_original_error(&mut self) -> Option<HostError> {
                self.original_error.take()
            }

            pub(crate) fn state(&self) -> HostState {
                HostState {
                    body: self.body(),
                    geometry: self.host.geometry,
                    availability: self.host.runtime.body,
                    blocked: self.host.runtime.input_fence.blocked,
                    empty_confirmed: self.host.runtime.input_fence.empty_confirmed,
                    input_ready: self.input_ready(),
                }
            }

            pub(crate) fn body(&self) -> u32 {
                self.host.window.window
            }

            pub(crate) fn trace(&self) -> &[&'static str] {
                &self.host.log
            }

            pub(crate) fn input_ready(&self) -> bool {
                self.host.runtime.accepts_input()
            }

            /// Same active-record boundary as owner servicing: notifications
            /// can only schedule existing work, never recreate a terminal record.
            pub(crate) fn notify_and_service(&mut self, active: Option<&mut ActiveOperation>) {
                if let Some(op) = active {
                    mark_remote_observation(op, false);
                    if remote_service_due(op, self.host.now) {
                        service_remote_with(op, &mut self.host).unwrap();
                    }
                }
            }

            pub(crate) fn late_notifications(&mut self, active: Option<&mut ActiveOperation>) {
                // Even convincing late WM state and an expired correction/owner
                // deadline cannot restart a completed operation.
                self.host.current = reading(Visibility::Minimized, L::Normal, 2, 2);
                self.host.now += Duration::from_secs(10);
                self.notify_and_service(active);
            }

            pub(crate) fn cleanup(&mut self) -> Result<(), HostError> {
                let h = &mut self.host;
                abort_layer(&mut h.runtime.layers, "shutdown");
                h.runtime.correction = None;
                h.runtime.cancel_menu_with(
                    &mut h.window,
                    |pointer| pointer.release_with(|| Ok(())).map(|_| ()),
                    |_| Ok(()),
                )?;
                assert!(h.runtime.interaction.is_idle());
                assert!(!h.runtime.pointer.is_grabbed());
                assert!(h.runtime.menu.is_none());
                assert!(h.runtime.layers.pending().is_none());
                Ok(())
            }
        }

        pub(crate) fn hide_loss_after_dispatch(
            destroyed: bool,
            elsewhere: bool,
        ) -> (ActiveOperation, control::OperationResult, Fixture) {
            let desktop = if elsewhere { 7 } else { 2 };
            let initial = if elsewhere {
                Visibility::NotMinimizedElsewhere
            } else {
                Visibility::NotMinimizedHere
            };
            let mut h = Fake::new(reading(initial, L::Normal, desktop, 2));
            let body = h.window.window;
            let geometry = h.geometry;
            let mut op = h.command_operation(Command::Hide);
            let deadline = op.deadline;
            assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
            assert_eq!(
                op.hide.phase,
                if elsewhere {
                    HidePhase::HiddenAddRequested
                } else {
                    HidePhase::IconifyRequested
                }
            );
            assert_eq!(h.count("iconify"), usize::from(!elsewhere));
            assert_eq!(h.count("add_hidden"), usize::from(elsewhere));
            assert_eq!(
                h.log,
                [
                    "fence",
                    "empty",
                    "release",
                    "observe",
                    if elsewhere { "add_hidden" } else { "iconify" }
                ]
            );
            assert!(h.fence_settled());
            assert!(op.detail.progress.host_mutation_dispatched);

            // Availability changes only AFTER the checked dispatch above.
            let (visibility, reason) = if destroyed {
                (Visibility::Destroyed, Reason::BodyDestroyed)
            } else {
                (Visibility::Withdrawn, Reason::Withdrawn)
            };
            let mut lost = reading(visibility, L::Unknown, desktop, 2);
            if destroyed {
                lost.evidence.wm_state = WmState::Absent;
                lost.evidence.map_state = Evidence::Missing;
                lost.evidence.hidden = Evidence::Missing;
                lost.evidence.coherent = false;
            } else {
                assert_eq!(
                    lost.evidence.map_state,
                    Evidence::Readable(MapState::Unmapped)
                );
                assert_eq!(lost.evidence.hidden, Evidence::Readable(false));
            }
            h.current = lost;
            mark_remote_observation(&mut op, false);
            assert!(remote_service_due(&op, h.now));
            let before = h.log.len();
            let result = service_remote_with(&mut op, &mut h).unwrap().unwrap();
            // Fresh failure observation plus independent safe-input observation;
            // no retry, remanagement, geometry work or enabling is performed.
            assert_eq!(&h.log[before..], ["observe", "observe"]);
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(result.exit_code(), control::ExitCode::OperationFailed);
            let d = result.detail();
            assert_eq!(
                (d.reason, d.visibility, d.stage),
                (reason, visibility, Stage::Hide)
            );
            assert_eq!(
                d.progress,
                control::Progress {
                    host_mutation_dispatched: true,
                    ..control::Progress::default()
                }
            );
            assert_eq!(control::Detail::decode(d.encode()).unwrap(), d);
            assert_eq!(op.deadline, deadline);
            assert!(h.runtime.input_fence.blocked);
            assert!(!h.runtime.accepts_input());
            assert!(h.fence_settled());
            assert_eq!((h.window.window, h.geometry), (body, geometry));
            (
                op,
                result,
                Fixture {
                    host: h,
                    original_error: None,
                },
            )
        }

        pub(crate) fn hide_phases() -> Vec<(ActiveOperation, Fixture)> {
            [
                HidePhase::Fence,
                HidePhase::Fenced,
                HidePhase::IconifyAttempted,
                HidePhase::IconifyRequested,
                HidePhase::HiddenAddAttempted,
                HidePhase::HiddenAddRequested,
            ]
            .into_iter()
            .map(|phase| {
                let elsewhere = matches!(
                    phase,
                    HidePhase::HiddenAddAttempted | HidePhase::HiddenAddRequested
                );
                let mut h = Fake::new(reading(
                    if elsewhere {
                        Visibility::NotMinimizedElsewhere
                    } else {
                        Visibility::NotMinimizedHere
                    },
                    L::Normal,
                    if elsewhere { 7 } else { 2 },
                    2,
                ));
                let mut op = h.command_operation(Command::Hide);
                let deadline = op.deadline;
                let mut original_error = None;
                if phase == HidePhase::Fenced {
                    let mut transient = h.current;
                    transient.evidence.coherent = false;
                    h.reads.push_back(transient);
                }
                let uncertain = matches!(
                    phase,
                    HidePhase::IconifyAttempted | HidePhase::HiddenAddAttempted
                );
                if uncertain {
                    h.fail = Some(if elsewhere { "add_hidden" } else { "iconify" });
                }
                if phase != HidePhase::Fence {
                    let outcome = service_remote_with(&mut op, &mut h);
                    if uncertain {
                        let error = outcome.unwrap_err();
                        assert!(error.to_string().contains(h.fail.unwrap()));
                        original_error = Some(error);
                    } else {
                        assert!(outcome.unwrap().is_none());
                    }
                    assert!(h.fence_settled());
                    assert!(!h.runtime.accepts_input());
                }
                assert_eq!(op.hide.phase, phase);
                assert_eq!(op.deadline, deadline);
                let sent = !matches!(phase, HidePhase::Fence | HidePhase::Fenced);
                assert_eq!(h.count("iconify"), usize::from(sent && !elsewhere));
                assert_eq!(h.count("add_hidden"), usize::from(sent && elsewhere));
                assert_eq!(op.detail.progress.host_mutation_dispatched, sent);
                assert_eq!(
                    h.count("map") + h.count("remove_hidden") + h.count("enable"),
                    0
                );
                if sent {
                    assert_eq!(
                        op.hide.baseline.unwrap().desktop,
                        if elsewhere { 7 } else { 2 }
                    );
                    assert_eq!(op.hide.baseline.unwrap().current_desktop, 2);
                    assert_eq!(
                        op.detail.workspace,
                        if elsewhere {
                            Workspace::Other
                        } else {
                            Workspace::Current
                        }
                    );
                }
                (
                    op,
                    Fixture {
                        host: h,
                        original_error,
                    },
                )
            })
            .collect()
        }

        pub(crate) fn show_phases() -> Vec<(ActiveOperation, Fixture)> {
            [
                ShowPhase::Observe,
                ShowPhase::Fenced,
                ShowPhase::RestoreAttempted,
                ShowPhase::RestoreRequested,
                ShowPhase::HiddenRemoveAttempted,
                ShowPhase::HiddenRemoveRequested,
                ShowPhase::EnablingInput,
                ShowPhase::InputRestored,
            ]
            .into_iter()
            .map(|phase| {
                let elsewhere = matches!(
                    phase,
                    ShowPhase::HiddenRemoveAttempted | ShowPhase::HiddenRemoveRequested
                );
                let mut h = Fake::new(reading(
                    Visibility::Minimized,
                    L::Normal,
                    if elsewhere { 7 } else { 2 },
                    2,
                ));
                h.runtime.body = BodyAvailability::Unavailable;
                let mut op = h.operation();
                let deadline = op.deadline;
                let mut original_error = None;
                if phase == ShowPhase::Fenced {
                    let mut transient = h.current;
                    transient.evidence.coherent = false;
                    h.reads = VecDeque::from([h.current, transient]);
                }
                let uncertain = matches!(
                    phase,
                    ShowPhase::RestoreAttempted | ShowPhase::HiddenRemoveAttempted
                );
                if uncertain {
                    h.fail = Some(if elsewhere {
                        "remove_hidden"
                    } else {
                        "user_time_zero"
                    });
                }
                if phase != ShowPhase::Observe {
                    let outcome = service_remote_with(&mut op, &mut h);
                    if uncertain {
                        let error = outcome.unwrap_err();
                        assert!(error.to_string().contains(h.fail.unwrap()));
                        original_error = Some(error);
                    } else {
                        assert!(outcome.unwrap().is_none());
                    }
                }
                if matches!(phase, ShowPhase::EnablingInput | ShowPhase::InputRestored) {
                    h.current = visible();
                    mark_remote_observation(&mut op, false);
                    if phase == ShowPhase::EnablingInput {
                        h.fail = Some("enable");
                        let error = service_remote_with(&mut op, &mut h).unwrap_err();
                        assert_eq!(error.to_string(), "injected enable");
                        original_error = Some(error);
                    } else {
                        let result = service_remote_with(&mut op, &mut h).unwrap().unwrap();
                        assert_success(result, true, true);
                    }
                }
                assert_eq!(op.show.phase, phase);
                assert_eq!(op.deadline, deadline);
                let sent = !matches!(phase, ShowPhase::Observe | ShowPhase::Fenced);
                assert_eq!(op.detail.progress.host_mutation_dispatched, sent);
                assert_eq!(h.count("remove_hidden"), usize::from(sent && elsewhere));
                assert_eq!(h.count("user_time_zero"), usize::from(sent && !elsewhere));
                assert_eq!(
                    h.count("map"),
                    usize::from(sent && !elsewhere && phase != ShowPhase::RestoreAttempted)
                );
                assert_eq!(
                    h.count("iconify") + h.count("add_hidden") + h.count("correct"),
                    0
                );
                assert_eq!(h.runtime.accepts_input(), phase == ShowPhase::InputRestored);
                assert_eq!(
                    op.detail.progress.input_ready,
                    phase == ShowPhase::InputRestored
                );
                assert_eq!(
                    op.detail.progress.not_minimized_confirmed,
                    phase == ShowPhase::InputRestored
                );
                if elsewhere {
                    assert_eq!(op.show.baseline.unwrap().desktop, 7);
                    assert_eq!(op.show.baseline.unwrap().current_desktop, 2);
                    assert_eq!(op.detail.workspace, Workspace::Other);
                    assert!(op.show.area.is_none());
                    assert!(op.show.correction.is_none());
                    assert!(!op.show.correction_sent);
                    assert_eq!(h.count("geometry") + h.count("enable"), 0);
                    assert!(h.runtime.input_fence.blocked);
                    assert!(!op.detail.progress.placement_deferred); // restoration still unconfirmed
                }
                (
                    op,
                    Fixture {
                        host: h,
                        original_error,
                    },
                )
            })
            .collect()
        }

        pub(crate) fn geometry_snapshots() -> Vec<(&'static str, ActiveOperation, Fixture)> {
            let mut fixtures = vec![];
            for name in [
                "placement-pending",
                "correction-pending",
                "correction-confirmed-input-pending",
            ] {
                let mut h = minimized();
                h.runtime.body = BodyAvailability::Unavailable;
                let mut op = h.operation();
                let deadline = op.deadline;
                let mut original_error = None;
                assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                h.current = visible();
                mark_remote_observation(&mut op, false);
                if name == "placement-pending" {
                    // Snapshot at the first geometry read after restoration.
                    h.fail = Some("geometry");
                    let error = service_remote_with(&mut op, &mut h).unwrap_err();
                    assert_eq!(error.to_string(), "injected geometry");
                    original_error = Some(error);
                    assert!(op.show.area.is_none());
                    assert!(op.show.correction.is_none());
                    assert!(!op.show.correction_sent);
                } else {
                    h.area = Rect::new(200, 200, 460, 460);
                    h.bounds = checked_probe_bounds(h.area).unwrap();
                    assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                    let (target, confirmation) = op.show.correction.unwrap();
                    assert_eq!(target, Point::new(200, 200));
                    assert_eq!(confirmation, h.now + CONFIRMATION_TIMEOUT);
                    assert_eq!(op.show.area, Some(h.area));
                    assert!(op.show.correction_sent);
                    assert_eq!(h.count("correct"), 1);
                    if name == "correction-confirmed-input-pending" {
                        h.now = confirmation;
                        mark_remote_observation(&mut op, false);
                        // No distinct correction-confirmed phase exists: the
                        // same production call proceeds to checked input enable.
                        h.fail = Some("enable");
                        let error = service_remote_with(&mut op, &mut h).unwrap_err();
                        assert_eq!(error.to_string(), "injected enable");
                        original_error = Some(error);
                        assert!(op.show.correction.is_none());
                        assert_eq!(op.show.phase, ShowPhase::EnablingInput);
                        assert_eq!(h.geometry.origin(), target);
                    } else {
                        assert_eq!(h.count("enable"), 0);
                        assert_eq!(op.show.phase, ShowPhase::RestoreRequested);
                    }
                }
                assert_eq!(
                    (op.stage, op.detail.stage),
                    (Stage::Geometry, Stage::Geometry)
                );
                assert_eq!(op.deadline, deadline);
                assert!(!op.show.observe);
                assert!(!op.show.layout_interrupted);
                assert!(op.detail.progress.host_mutation_dispatched);
                assert!(!op.detail.progress.input_ready);
                assert!(!op.detail.progress.not_minimized_confirmed);
                assert!(h.runtime.input_fence.blocked);
                assert!(!h.runtime.accepts_input());
                assert_eq!(h.count("map"), 1);
                assert_eq!(h.count("add_hidden") + h.count("remove_hidden"), 0);
                fixtures.push((
                    name,
                    op,
                    Fixture {
                        host: h,
                        original_error,
                    },
                ));
            }
            fixtures
        }
    }

    pub(crate) mod g3_hardening {
        use super::*;
        use control::Stage;

        fn fresh(min: bool, layer: L, desktop: u32) -> (Fake, ActiveOperation) {
            let h = Fake::new(reading(
                if min {
                    Visibility::Minimized
                } else if desktop == 7 {
                    Visibility::NotMinimizedElsewhere
                } else {
                    Visibility::NotMinimizedHere
                },
                layer,
                desktop,
                2,
            ));
            let op = h.command_operation(Command::BringTop);
            (h, op)
        }
        fn restored(h: &mut Fake, op: &mut ActiveOperation) {
            assert!(service_remote_with(op, h).unwrap().is_none());
            let r = h.current;
            h.current = reading(
                if r.desktop == 7 {
                    Visibility::NotMinimizedElsewhere
                } else {
                    Visibility::NotMinimizedHere
                },
                r.layer,
                r.desktop,
                r.current,
            );
            mark_remote_observation(op, false);
        }
        fn assert_terminal(
            r: control::OperationResult,
            status: Status,
            reason: Reason,
            stage: Stage,
        ) {
            assert_eq!(
                (r.status(), r.detail().reason, r.detail().stage),
                (status, reason, stage)
            );
            let d = r.detail();
            assert_eq!(
                d.progress.not_minimized_confirmed,
                matches!(
                    d.visibility,
                    Visibility::NotMinimizedHere | Visibility::NotMinimizedElsewhere
                )
            );
            if d.progress.above_confirmed {
                assert_eq!(d.layer, L::Above);
                assert!(d.progress.not_minimized_confirmed);
            }
            if d.progress.input_ready {
                assert_eq!(d.visibility, Visibility::NotMinimizedHere);
                assert!(!d.progress.placement_deferred);
            }
            if d.progress.placement_deferred {
                assert_eq!(d.visibility, Visibility::NotMinimizedElsewhere);
                assert!(!d.progress.input_ready);
            }
            assert_eq!(control::Detail::decode(d.encode()).unwrap(), d);
            assert_eq!(d.encode() & 0xff00_0000, 0);
        }
        fn assert_before_layer(h: &Fake) {
            assert!(h.mutations.is_empty());
            assert_eq!(h.count("layer_read"), 0);
        }

        #[test]
        fn owner_deadline_edges_allow_only_final_confirmation_without_new_layer_work() {
            for delta in [-1_i64, 0, 1] {
                for above in [false, true] {
                    let (mut h, mut op) = fresh(false, L::Normal, 2);
                    observe_visibility(&mut op, &mut h).unwrap();
                    op.stage = Stage::Layer;
                    op.detail.stage = Stage::Layer;
                    h.now += Duration::from_millis(3000);
                    op.bring_top.layer_admitted = Some(h.now);
                    h.layer_mode = 1;
                    assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                    assert_eq!(h.layer_deadline(), Some(op.deadline));
                    h.now = if delta < 0 {
                        op.deadline - Duration::from_nanos(1)
                    } else {
                        op.deadline + Duration::from_nanos(delta as u64)
                    };
                    h.current.layer = if above { L::Above } else { L::Normal };
                    let result = service_remote_with(&mut op, &mut h).unwrap();
                    if above {
                        assert_eq!(result.unwrap().status(), Status::Success);
                    } else if delta < 0 {
                        assert!(result.is_none());
                    } else {
                        assert_terminal(
                            result.unwrap(),
                            Status::Failed,
                            Reason::ConfirmationTimeout,
                            Stage::Layer,
                        );
                    }
                    assert_eq!(h.mutations, [Mutation::AddAbove]);
                }
            }
        }
        #[test]
        fn incoherent_final_state_times_out_without_stale_bits_or_erasing_restoration_history() {
            let (mut h, mut op) = fresh(true, L::Normal, 7);
            restored(&mut h, &mut op);
            h.layer_mode = 1;
            assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
            let deadline = h.layer_deadline().unwrap();
            h.current.evidence.coherent = false;
            h.current.layer = L::Unknown;
            for _ in 0..20 {
                h.now += Duration::from_millis(20);
                assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                assert_eq!(h.layer_deadline(), Some(deadline));
            }
            h.now = deadline;
            let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
            assert_terminal(
                r,
                Status::Partial,
                Reason::ConfirmationTimeout,
                Stage::Layer,
            );
            assert_eq!(
                r.detail().progress,
                control::Progress {
                    host_mutation_dispatched: true,
                    ..control::Progress::default()
                }
            );
            assert!(op.bring_top.restoration_confirmed);
            assert_eq!(h.mutations, [Mutation::AddAbove]);
        }
        #[test]
        fn selecting_popup_during_g_visibility_cleans_before_busy_and_stale_release_is_inert() {
            let (mut h, op) = fresh(false, L::Normal, 2);
            h.runtime
                .acquire_menu_with(MenuPopup::test_popup(10, 1), Point::new(0, 0), || {
                    Ok((GrabStatus::SUCCESS, 100))
                })
                .unwrap();
            let log = RefCell::new(vec![]);
            h.runtime
                .complete_menu_with(
                    &mut h.window,
                    MenuOutcome::SetLayer(Layer::Below),
                    |p| {
                        log.borrow_mut().push("release");
                        p.release_with(|| Ok(())).map(|_| ())
                    },
                    |_| {
                        log.borrow_mut().push("destroy");
                        Ok(())
                    },
                    |_, target| {
                        log.borrow_mut().push("busy");
                        local_layer_after_menu_with(
                            op.request.command == Command::BringTop,
                            target,
                            || panic!("no local layer work during remote visibility"),
                        )
                    },
                )
                .unwrap();
            assert_eq!(*log.borrow(), ["release", "destroy", "busy"]);
            assert!(h.runtime.menu.is_none());
            assert!(!h.runtime.pointer.is_grabbed());
            assert!(h.runtime.layers.pending().is_none());
            assert_eq!(
                h.runtime
                    .interaction
                    .menu_release(1, crate::geometry::MenuHit::Row(2), 101, 0),
                HostAction::None
            );
            let (mut h, mut op) = fresh(true, L::Normal, 2);
            h.runtime = visibility_fence_tests::runtime(4);
            restored(&mut h, &mut op);
            assert_eq!(
                h.runtime
                    .interaction
                    .menu_release(1, crate::geometry::MenuHit::Row(4), 101, 0),
                HostAction::None
            );
            assert!(h.runtime.menu.is_none());
            assert!(h.mutations.is_empty());
        }
        #[test]
        fn timeout_final_reread_can_confirm_already_achieved_above_without_resend() {
            let (mut h, mut op) = fresh(true, L::Normal, 2);
            restored(&mut h, &mut op);
            h.layer_mode = 1;
            assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
            h.now = h.layer_deadline().unwrap();
            h.changes = VecDeque::from([
                (
                    "observe",
                    reading(Visibility::NotMinimizedHere, L::Normal, 2, 2),
                ),
                (
                    "observe",
                    reading(Visibility::NotMinimizedHere, L::Above, 2, 2),
                ),
            ]);
            let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
            assert_terminal(r, Status::Success, Reason::None, Stage::Layer);
            assert_eq!(h.mutations, [Mutation::AddAbove]);
            assert!(r.detail().progress.above_confirmed);
        }
        #[test]
        fn user_time_only_and_checked_layer_effect_with_current_above_do_not_invent_partial() {
            let (mut h, mut op) = fresh(true, L::Normal, 2);
            h.advance = Some(("user_time_zero", op.bring_top.visibility_cutoff - h.now));
            let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
            assert_terminal(r, Status::Failed, Reason::ConfirmationTimeout, Stage::Show);
            assert!(r.detail().progress.host_mutation_dispatched);
            assert_eq!(h.count("map"), 0);
            assert!(!op.bring_top.restoration_confirmed);
            assert_before_layer(&h);
            let (mut h, mut op) = fresh(false, L::Normal, 2);
            h.fail = Some("add_above");
            h.changes.push_back((
                "add_above",
                reading(Visibility::NotMinimizedHere, L::Above, 2, 2),
            ));
            assert!(service_remote_with(&mut op, &mut h).is_err());
            let r = finish_bring_top_with(&mut op, &mut h, Reason::X11Failure, false).unwrap();
            assert_terminal(r, Status::Failed, Reason::X11Failure, Stage::Layer);
            assert!(r.detail().progress.above_confirmed);
            assert!(!op.bring_top.minimized_at_start);
            assert!(!op.bring_top.restoration_confirmed);
            assert_eq!(h.mutations, [Mutation::AddAbove]);
        }
        #[test]
        fn shutdown_cleanup_of_later_layer_phase_pointer_owners_keeps_original_failure() {
            for gesture in [1, 2, 3, 4] {
                let (mut h, mut op) = fresh(false, L::Normal, 2);
                h.layer_mode = 1;
                assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                let layers = std::mem::take(&mut h.runtime.layers);
                h.runtime = visibility_fence_tests::runtime(gesture);
                h.runtime.layers = layers;
                let release = std::cell::Cell::new(0);
                let popup = std::cell::Cell::new(0);
                let result = finish_probe_with(Err("original G shutdown cause".into()), || {
                    assert!(abort_layer(&mut h.runtime.layers, "shutdown").is_some());
                    assert!(abort_layer(&mut h.runtime.layers, "shutdown repeated").is_none());
                    h.runtime.cancel_menu_with(
                        &mut h.window,
                        |p| {
                            release.set(release.get() + 1);
                            p.release_with(|| Ok(())).map(|_| ())
                        },
                        |_| {
                            popup.set(popup.get() + 1);
                            Ok(())
                        },
                    )
                });
                assert_eq!(result.unwrap_err().to_string(), "original G shutdown cause");
                assert!(!h.runtime.pointer.is_grabbed());
                assert!(h.runtime.menu.is_none());
                assert!(h.runtime.pending_move.is_none());
                assert!(h.runtime.layers.pending().is_none());
                assert_eq!(release.get(), 1);
                assert_eq!(popup.get(), usize::from(gesture >= 3));
                assert_eq!(h.mutations, [Mutation::AddAbove]);
            }
        }
        /// Real production progression snapshots shared with transport lifecycle
        /// tests. No copied Show/Layer state machine or fabricated proof flags.
        pub(crate) fn lifecycle_fixtures() -> Vec<(&'static str, ActiveOperation, LayerController)>
        {
            let mut fixtures = vec![];
            for name in [
                "before-baseline",
                "after-fence",
                "restore-attempt",
                "restore-request",
                "after-proof",
                "geometry",
                "input-restored-before-layer",
                "before-layer-send",
                "after-remove",
                "after-add",
                "checked-layer-uncertainty",
                "layer-confirmed-before-reply",
                "transitional-final-read",
            ] {
                let layer = if name == "after-remove" {
                    L::Below
                } else if name == "transitional-final-read" {
                    L::Above
                } else {
                    L::Normal
                };
                let (mut h, mut op) = fresh(true, layer, 2);
                match name {
                    "before-baseline" => {}
                    "after-fence" => {
                        let mut transient = h.current;
                        transient.evidence.coherent = false;
                        transient.layer = L::Unknown;
                        h.reads = VecDeque::from([h.current, transient]);
                        assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                        assert_eq!(op.show.phase, ShowPhase::Fenced);
                    }
                    "restore-attempt" => {
                        h.fail = Some("map");
                        assert!(service_remote_with(&mut op, &mut h).is_err());
                    }
                    "restore-request" => {
                        assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                    }
                    _ => {
                        restored(&mut h, &mut op);
                        match name {
                            "after-proof" => {
                                h.fail = Some("geometry");
                                assert!(service_remote_with(&mut op, &mut h).is_err());
                            }
                            "geometry" => {
                                h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
                                assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                            }
                            "input-restored-before-layer" => {
                                assert!(matches!(
                                    service_visibility_stage_with(&mut op, &mut h).unwrap(),
                                    Some(VisibilityCompletion::Ready(_))
                                ));
                            }
                            "before-layer-send" => {
                                h.layer_issue = Some(RemoteLayerIssue::Waiting);
                                assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                            }
                            "after-remove" | "after-add" => {
                                h.layer_mode = 1;
                                assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                            }
                            "checked-layer-uncertainty" => {
                                h.fail = Some("add_above");
                                assert!(service_remote_with(&mut op, &mut h).is_err());
                                assert!(op.bring_top.layer_dispatched);
                                assert!(!op.bring_top.layer_checked);
                            }
                            "layer-confirmed-before-reply" => {
                                assert_eq!(
                                    service_remote_with(&mut op, &mut h)
                                        .unwrap()
                                        .unwrap()
                                        .status(),
                                    Status::Success
                                );
                            }
                            "transitional-final-read" => {
                                let above = h.current;
                                let mut transient = above;
                                transient.evidence.coherent = false;
                                transient.layer = L::Unknown;
                                h.reads =
                                    VecDeque::from([above, above, above, above, above, transient]);
                                assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                                assert!(op.bring_top.layer_finished);
                            }
                            _ => unreachable!(),
                        }
                        assert!(
                            op.bring_top.minimized_at_start && op.bring_top.restoration_confirmed
                        );
                    }
                }
                fixtures.push((name, op, h.runtime.layers));
            }
            fixtures
        }
        #[test]
        fn destroyed_and_withdrawn_at_each_visibility_boundary_keep_exact_history() {
            for unavailable in [Visibility::Destroyed, Visibility::Withdrawn] {
                for point in 0..4 {
                    let (mut h, mut op) = fresh(true, L::Normal, 2);
                    let lost = reading(unavailable, L::Unknown, 2, 2);
                    match point {
                        0 => h.current = lost,
                        1 => h.reads = VecDeque::from([h.current, lost]),
                        2 => {
                            assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                            h.current = lost;
                        }
                        _ => {
                            restored(&mut h, &mut op);
                            h.changes.push_back(("geometry", lost));
                        }
                    }
                    let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
                    assert_terminal(
                        r,
                        if point == 3 {
                            Status::Partial
                        } else {
                            Status::Failed
                        },
                        if unavailable == Visibility::Destroyed {
                            Reason::BodyDestroyed
                        } else {
                            Reason::Withdrawn
                        },
                        if point == 3 {
                            Stage::Geometry
                        } else {
                            Stage::Show
                        },
                    );
                    assert_eq!(op.bring_top.restoration_confirmed, point == 3);
                    assert!(!r.detail().progress.not_minimized_confirmed);
                    assert!(!r.detail().progress.above_confirmed);
                    assert!(!r.detail().progress.input_ready);
                    assert_eq!(h.count("map"), usize::from(point >= 2));
                    assert_before_layer(&h);
                }
            }
        }
        #[test]
        fn visibility_bad_capability_properties_and_incoherence_are_bounded_without_effects() {
            for kind in 0..5 {
                let (mut h, mut op) = fresh(true, L::Normal, 2);
                match kind {
                    0 => h.current.evidence.capability = Capability::Unsupported,
                    1 => h.current.evidence.hidden = Evidence::Malformed,
                    2 => h.current.evidence.hidden = Evidence::Missing,
                    3 => h.current.evidence.coherent = false,
                    _ => h.current.evidence.wm_state = WmState::Malformed,
                }
                h.current.layer = L::Unknown;
                let early = service_remote_with(&mut op, &mut h).unwrap();
                if kind == 0 {
                    assert_terminal(
                        early.unwrap(),
                        Status::Failed,
                        Reason::Unsupported,
                        Stage::Show,
                    );
                } else {
                    assert!(early.is_none());
                    for _ in 0..3 {
                        mark_remote_observation(&mut op, false);
                        assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                    }
                    h.now = op.bring_top.visibility_cutoff;
                    assert_terminal(
                        service_remote_with(&mut op, &mut h).unwrap().unwrap(),
                        Status::Failed,
                        Reason::ConfirmationTimeout,
                        Stage::Show,
                    );
                }
                assert!(!op.bring_top.restoration_confirmed);
                assert_eq!(h.count("fence"), 0);
                assert_eq!(h.count("map"), 0);
                assert_before_layer(&h);
            }
        }
        #[test]
        fn workspace_drift_before_mutation_after_dispatch_and_after_proof_is_not_repaired() {
            for body_changes in [false, true] {
                for point in 0..3 {
                    let (mut h, mut op) = fresh(true, L::Normal, 2);
                    let changed = reading(
                        if point == 0 {
                            Visibility::Minimized
                        } else {
                            Visibility::NotMinimizedElsewhere
                        },
                        L::Normal,
                        if body_changes { 7 } else { 2 },
                        if body_changes { 2 } else { 7 },
                    );
                    match point {
                        0 => h.reads = VecDeque::from([h.current, changed]),
                        1 => {
                            assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                            h.current = changed;
                        }
                        _ => {
                            restored(&mut h, &mut op);
                            h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
                            assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                            h.current = changed;
                        }
                    }
                    let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
                    assert_terminal(
                        r,
                        if point == 2 {
                            Status::Partial
                        } else {
                            Status::Failed
                        },
                        Reason::LayoutChanged,
                        if point == 2 {
                            Stage::Geometry
                        } else {
                            Stage::Show
                        },
                    );
                    assert_eq!(op.bring_top.restoration_confirmed, point == 2);
                    assert_eq!(h.count("map"), usize::from(point > 0));
                    assert_eq!(h.count("correct"), usize::from(point == 2));
                    assert_before_layer(&h);
                }
            }
        }
        #[test]
        fn geometry_layout_interruption_and_checked_correction_failure_never_start_layer() {
            for failure in ["layout", "geometry", "correct"] {
                let (mut h, mut op) = fresh(true, L::Normal, 2);
                restored(&mut h, &mut op);
                h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
                if failure == "layout" {
                    assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                    mark_remote_observation(&mut op, true);
                    assert_terminal(
                        service_remote_with(&mut op, &mut h).unwrap().unwrap(),
                        Status::Partial,
                        Reason::LayoutChanged,
                        Stage::Geometry,
                    );
                } else {
                    h.fail = Some(failure);
                    let err = service_remote_with(&mut op, &mut h).unwrap_err();
                    assert_eq!(err.to_string(), format!("injected {failure}"));
                    let r =
                        finish_bring_top_with(&mut op, &mut h, Reason::X11Failure, false).unwrap();
                    assert_terminal(r, Status::Partial, Reason::X11Failure, Stage::Geometry);
                }
                assert!(op.bring_top.restoration_confirmed);
                assert_eq!(h.count("map"), 1);
                assert_before_layer(&h);
            }
        }
        #[test]
        fn g_fence_failure_and_enable_refence_failure_keep_primary_error_and_capture_debt() {
            for failure in ["empty", "release", "popup"] {
                let (mut h, mut op) = fresh(true, L::Normal, 2);
                h.runtime = visibility_fence_tests::runtime(4);
                h.fail = Some(failure);
                let err = service_remote_with(&mut op, &mut h).unwrap_err();
                assert_eq!(err.to_string(), format!("injected {failure}"));
                let reason = if failure == "release" {
                    Reason::ReleaseFailed
                } else {
                    Reason::X11Failure
                };
                let r = finish_bring_top_with(&mut op, &mut h, reason, false).unwrap();
                assert_terminal(r, Status::Failed, reason, Stage::Show);
                assert!(!op.bring_top.restoration_confirmed);
                assert_eq!(h.count("release"), 1);
                assert_eq!(h.count("popup"), 1);
                assert_eq!(h.count("map"), 0);
                assert_before_layer(&h);
                assert_eq!(h.runtime.pointer.is_grabbed(), failure == "release");
            }
            let (mut h, mut op) = fresh(true, L::Normal, 2);
            restored(&mut h, &mut op);
            h.step_failures = VecDeque::from(["enable", "fence"]);
            let err = service_remote_with(&mut op, &mut h).unwrap_err();
            assert_eq!(err.to_string(), "injected enable");
            assert_eq!(h.count("fence"), 2);
            assert_eq!(h.count("enable"), 1);
            let r = finish_bring_top_with(&mut op, &mut h, Reason::X11Failure, false).unwrap();
            assert_terminal(r, Status::Partial, Reason::X11Failure, Stage::Geometry);
            assert!(!r.detail().progress.input_ready);
            assert_before_layer(&h);
        }
        #[test]
        fn layer_loss_or_conflict_before_first_send_and_after_dispatch_is_truthful() {
            for min in [false, true] {
                for desktop in [2, 7] {
                    for kind in 0..6 {
                        for after_send in [false, true] {
                            let (mut h, mut op) = fresh(min, L::Normal, desktop);
                            if min {
                                restored(&mut h, &mut op);
                            }
                            if after_send {
                                h.layer_mode = 1;
                                assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                            }
                            let mut bad = reading(
                                if desktop == 7 {
                                    Visibility::NotMinimizedElsewhere
                                } else {
                                    Visibility::NotMinimizedHere
                                },
                                L::Normal,
                                desktop,
                                2,
                            );
                            let reason = match kind {
                                0 => {
                                    bad = reading(Visibility::Destroyed, L::Unknown, desktop, 2);
                                    Reason::BodyDestroyed
                                }
                                1 => {
                                    bad = reading(Visibility::Withdrawn, L::Unknown, desktop, 2);
                                    Reason::Withdrawn
                                }
                                2 => {
                                    bad.desktop = 8;
                                    bad.evidence.workspace = Workspace::Other;
                                    Reason::LayoutChanged
                                }
                                3 => {
                                    bad.current = 8;
                                    bad.evidence.workspace = Workspace::Other;
                                    Reason::LayoutChanged
                                }
                                4 => {
                                    bad.layer = L::Conflict;
                                    Reason::Unverifiable
                                }
                                _ => {
                                    bad.evidence.hidden = Evidence::Malformed;
                                    bad.layer = L::Unknown;
                                    Reason::Unverifiable
                                }
                            };
                            if after_send {
                                h.current = bad;
                            } else {
                                h.layer_reads.push_back(bad);
                            }
                            let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
                            assert_terminal(
                                r,
                                if min { Status::Partial } else { Status::Failed },
                                reason,
                                Stage::Layer,
                            );
                            assert_eq!(h.mutations.len(), usize::from(after_send));
                            assert!(!r.detail().progress.above_confirmed);
                            assert!(h.runtime.layers.pending().is_none());
                        }
                    }
                }
            }
        }
        #[test]
        fn support_refusal_or_malformed_support_before_removal_and_before_addition() {
            let good = LayerSupport {
                state: true,
                above: true,
                below: true,
            };
            for min in [false, true] {
                for after_remove in [false, true] {
                    for malformed in [false, true] {
                        let (mut h, mut op) = fresh(min, L::Below, 2);
                        if min {
                            restored(&mut h, &mut op);
                        }
                        if after_remove {
                            h.support_reads.push_back(Ok(good));
                        }
                        h.support_reads.push_back(if malformed {
                            Err("malformed _NET_SUPPORTED")
                        } else {
                            Ok(LayerSupport {
                                above: false,
                                ..good
                            })
                        });
                        let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
                        assert_terminal(
                            r,
                            if min { Status::Partial } else { Status::Failed },
                            if malformed {
                                Reason::Unverifiable
                            } else {
                                Reason::Unsupported
                            },
                            Stage::Layer,
                        );
                        assert_eq!(h.count("remove_below"), usize::from(after_remove));
                        assert_eq!(h.count("add_above"), 0);
                        assert_eq!(
                            r.detail().layer,
                            if after_remove { L::Normal } else { L::Below }
                        );
                        assert!(!r.detail().progress.above_confirmed);
                    }
                }
            }
        }
        #[test]
        fn remote_property_support_loss_and_native_errors_preserve_dispatch_marks() {
            for min in [false, true] {
                for before_marker in [false, true] {
                    let (mut h, mut op) = fresh(min, L::Normal, 2);
                    if min {
                        restored(&mut h, &mut op);
                    }
                    h.fail_before_layer_dispatch = before_marker;
                    if !before_marker {
                        h.fail = Some("add_above");
                    }
                    let err = service_remote_with(&mut op, &mut h).unwrap_err();
                    assert!(err.to_string().contains(if before_marker {
                        "before layer dispatch"
                    } else {
                        "add_above"
                    }));
                    assert!(op.bring_top.layer_attempted);
                    assert_eq!(op.bring_top.layer_dispatched, !before_marker);
                    assert!(!op.bring_top.layer_checked);
                    let r =
                        finish_bring_top_with(&mut op, &mut h, Reason::X11Failure, false).unwrap();
                    assert_terminal(
                        r,
                        if min { Status::Partial } else { Status::Failed },
                        Reason::X11Failure,
                        Stage::Layer,
                    );
                    assert_eq!(h.mutations.len(), usize::from(!before_marker));
                    assert!(h.runtime.layers.pending().is_none());
                }
                for support in [
                    Evidence::Malformed,
                    Evidence::Missing,
                    Evidence::Readable(vec![12, 13, 14, 15]),
                ] {
                    let (mut h, mut op) = fresh(min, L::Normal, 7);
                    if min {
                        restored(&mut h, &mut op);
                    }
                    h.layer_mode = 1;
                    assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                    h.supported = support.clone();
                    h.current.layer = L::Unknown;
                    let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
                    assert_terminal(
                        r,
                        if min { Status::Partial } else { Status::Failed },
                        if matches!(support, Evidence::Readable(_)) {
                            Reason::Unsupported
                        } else {
                            Reason::Unverifiable
                        },
                        Stage::Layer,
                    );
                    assert_eq!(h.mutations, [Mutation::AddAbove]);
                    assert!(!r.detail().progress.above_confirmed);
                    for name in ["map", "user_time_zero", "geometry", "correct", "enable"] {
                        assert_eq!(h.count(name), 0);
                    }
                }
            }
        }
        #[test]
        fn exact_visibility_geometry_and_layer_edges_never_renew_budgets() {
            for delta in [-1_i64, 0, 1] {
                let (mut h, mut op) = fresh(true, L::Normal, 2);
                let cutoff = op.bring_top.visibility_cutoff;
                h.now = if delta < 0 {
                    cutoff - Duration::from_nanos(1)
                } else {
                    cutoff + Duration::from_nanos(delta as u64)
                };
                let r = service_remote_with(&mut op, &mut h).unwrap();
                assert_eq!(h.count("map"), usize::from(delta < 0));
                assert_eq!(r.is_none(), delta < 0);
                assert_before_layer(&h);
                let (mut h, mut op) = fresh(true, L::Normal, 2);
                restored(&mut h, &mut op);
                h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
                let boundary = op.bring_top.visibility_cutoff - CONFIRMATION_TIMEOUT;
                h.now = if delta < 0 {
                    boundary - Duration::from_nanos(1)
                } else {
                    boundary + Duration::from_nanos(delta as u64)
                };
                let r = service_remote_with(&mut op, &mut h).unwrap();
                assert_eq!(h.count("correct"), usize::from(delta <= 0));
                assert_eq!(r.is_none(), delta <= 0);
                assert_before_layer(&h);
                for final_above in [false, true] {
                    let (mut h, mut op) = fresh(false, L::Below, 2);
                    h.layer_mode = 1;
                    assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                    let deadline = h.layer_deadline().unwrap();
                    h.now = if delta < 0 {
                        deadline - Duration::from_nanos(1)
                    } else {
                        deadline + Duration::from_nanos(delta as u64)
                    };
                    h.current.layer = if final_above { L::Above } else { L::Normal };
                    let r = service_remote_with(&mut op, &mut h).unwrap();
                    assert_eq!(h.count("add_above"), usize::from(!final_above && delta < 0));
                    if final_above {
                        assert_eq!(r.unwrap().status(), Status::Success);
                    } else if delta >= 0 {
                        assert_eq!(r.unwrap().status(), Status::Failed);
                    } else {
                        assert!(r.is_none());
                    }
                    assert_eq!(
                        op.deadline - op.bring_top.visibility_cutoff,
                        Duration::from_millis(1000)
                    );
                }
            }
        }
        #[test]
        fn all_desktops_all_layers_visible_and_minimized_preserve_sentinel() {
            for min in [false, true] {
                for layer in [L::Normal, L::Below, L::Above] {
                    let (mut h, mut op) = fresh(min, layer, u32::MAX);
                    if min {
                        restored(&mut h, &mut op);
                    }
                    let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
                    assert_success(r, true, min || layer != L::Above);
                    assert_eq!(r.detail().workspace, Workspace::AllDesktops);
                    assert_eq!(h.current.desktop, u32::MAX);
                    assert_eq!(op.show.baseline.unwrap().desktop, u32::MAX);
                    assert_eq!(h.count("map"), usize::from(min));
                    assert_eq!(h.count("remove_hidden"), 0);
                }
            }
        }
        #[test]
        fn g_right_opening_preservation_and_restoration_fence_invalidate_stale_input() {
            let (mut h, mut op) = fresh(false, L::Normal, 2);
            h.runtime = visibility_fence_tests::runtime(2);
            assert_eq!(
                service_remote_with(&mut op, &mut h)
                    .unwrap()
                    .unwrap()
                    .status(),
                Status::Success
            );
            assert!(h.runtime.pointer.is_grabbed());
            assert!(matches!(
                h.runtime.interaction.state(),
                InteractionState::RightPressed { .. }
            ));
            assert_eq!(h.count("fence"), 0);
            for gesture in 1..=4 {
                let (mut h, mut op) = fresh(true, L::Normal, 2);
                h.runtime = visibility_fence_tests::runtime(gesture);
                h.runtime.pending_move = Some(Point::new(200, 200));
                h.window.in_flight_moves.push_back(Point::new(200, 200));
                let generation = h.runtime.menu_generation;
                restored(&mut h, &mut op);
                assert!(h.runtime.visibility_dispatch_eligible());
                assert_eq!(h.runtime.menu_generation, generation + 1);
                assert!(!h.runtime.pointer.is_grabbed());
                assert!(h.runtime.menu.is_none());
                assert_eq!(
                    service_remote_with(&mut op, &mut h)
                        .unwrap()
                        .unwrap()
                        .status(),
                    Status::Success
                );
                assert_eq!(h.count("fence"), 1);
                let release = Event::ButtonRelease(x11rb::protocol::xproto::ButtonPressEvent {
                    event: h.window.window,
                    detail: 1,
                    ..Default::default()
                });
                assert!(h
                    .runtime
                    .input_fence
                    .rejects(&release, h.window.window, 70000));
                assert!(!h.runtime.interaction.has_left_gesture());
                assert!(h.runtime.pending_move.is_none());
                let reads = h.count("observe");
                restore_deferred_input(&mut h).unwrap();
                assert_eq!(h.count("observe"), reads);
            }
        }
        #[test]
        fn completed_elsewhere_g_defers_input_and_never_resurrects_on_local_return() {
            let (mut h, mut op) = fresh(true, L::Below, 7);
            restored(&mut h, &mut op);
            let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
            assert_success(r, false, true);
            assert!(h.runtime.input_fence.blocked);
            assert_eq!(h.count("geometry"), 0);
            assert_eq!(h.count("enable"), 0);
            let mutations = h.mutations.clone();
            h.current = reading(Visibility::NotMinimizedHere, L::Above, 7, 7);
            restore_deferred_input(&mut h).unwrap();
            assert!(h.runtime.accepts_input());
            assert_eq!(h.count("enable"), 1);
            assert_eq!(h.mutations, mutations);
            restore_deferred_input(&mut h).unwrap();
            assert_eq!(h.count("enable"), 1);
            assert_eq!(h.count("remove_hidden"), 1);
        }
    }
    mod bring_top {
        use super::*;
        use control::{ExitCode, Stage};

        fn run(h: &mut Fake, op: &mut ActiveOperation) -> Option<control::OperationResult> {
            service_bring_top_with(op, h).unwrap()
        }
        fn setup(minimized: bool, layer: L, elsewhere: bool) -> (Fake, ActiveOperation) {
            let h = Fake::new(reading(
                if minimized {
                    Visibility::Minimized
                } else if elsewhere {
                    Visibility::NotMinimizedElsewhere
                } else {
                    Visibility::NotMinimizedHere
                },
                layer,
                if elsewhere { 7 } else { 2 },
                2,
            ));
            let op = h.command_operation(Command::BringTop);
            (h, op)
        }
        fn restore(h: &mut Fake, op: &mut ActiveOperation, elsewhere: bool) {
            assert!(run(h, op).is_none());
            assert!(op.bring_top.minimized_at_start);
            assert!(!op.bring_top.restoration_confirmed);
            assert!(h.mutations.is_empty());
            h.current = reading(
                if elsewhere {
                    Visibility::NotMinimizedElsewhere
                } else {
                    Visibility::NotMinimizedHere
                },
                h.current.layer,
                h.current.desktop,
                h.current.current,
            );
            mark_remote_observation(op, false);
        }
        fn expected_mutations(layer: L) -> Vec<Mutation> {
            match layer {
                L::Above => vec![],
                L::Normal => vec![Mutation::AddAbove],
                L::Below => vec![
                    Mutation::Remove(layer::Flags {
                        above: false,
                        below: true,
                    }),
                    Mutation::AddAbove,
                ],
                _ => unreachable!(),
            }
        }
        fn assert_prohibitions(h: &Fake, elsewhere: bool) {
            for name in [
                "workspace_write",
                "current_desktop_request",
                "active_window",
                "set_input_focus",
                "map_raised",
                "restack",
                "second_body",
                "retry",
                "direct_state_write",
            ] {
                assert_eq!(h.count(name), 0, "prohibited {name}");
            }
            if elsewhere {
                for name in [
                    "map",
                    "user_time_zero",
                    "geometry",
                    "correct",
                    "ordinary_correct",
                    "enable",
                ] {
                    assert_eq!(h.count(name), 0, "off-workspace {name}");
                }
            }
            assert!(h.mutations.iter().all(|m| matches!(
                m,
                Mutation::AddAbove
                    | Mutation::Remove(layer::Flags {
                        above: false,
                        below: true
                    })
            )));
        }

        #[test]
        fn local_visible_normal_below_above_matrix_preserves_ready_interaction() {
            for layer in [L::Normal, L::Below, L::Above] {
                let (mut h, mut op) = setup(false, layer, false);
                h.runtime.interaction.handle_left_press(
                    Point::new(180, 180),
                    (80, 80),
                    100,
                    Point::new(100, 100),
                );
                h.runtime.interaction.on_grab_acquired();
                h.runtime
                    .pointer
                    .acquire_with(CaptureOwner::BodyLeft, || Ok((GrabStatus::SUCCESS, 5)))
                    .unwrap();
                let result = run(&mut h, &mut op).unwrap();
                assert_success(result, true, layer != L::Above);
                assert!(result.detail().progress.above_confirmed);
                assert_eq!(h.mutations, expected_mutations(layer));
                assert_eq!(h.count("fence"), 0);
                assert_eq!(h.count("map"), 0);
                assert!(h.runtime.pointer.is_grabbed());
                assert!(h.runtime.interaction.has_left_gesture());
                assert!(!op.bring_top.restoration_confirmed);
                assert_prohibitions(&h, false);
            }
        }
        #[test]
        fn local_minimized_normal_below_above_restore_then_compose_matrix() {
            for layer in [L::Normal, L::Below, L::Above] {
                let (mut h, mut op) = setup(true, layer, false);
                restore(&mut h, &mut op, false);
                let result = run(&mut h, &mut op).unwrap();
                assert_success(result, true, true);
                assert!(op.bring_top.restoration_confirmed);
                assert!(result.detail().progress.above_confirmed);
                assert_eq!(h.mutations, expected_mutations(layer));
                for name in ["fence", "map", "user_time_zero", "enable"] {
                    assert_eq!(h.count(name), 1);
                }
                if layer != L::Above {
                    let index = |name| h.log.iter().position(|s| *s == name).unwrap();
                    assert!(index("enable") < index("add_above"));
                }
                assert_prohibitions(&h, false);
            }
        }
        #[test]
        fn off_workspace_visible_three_layer_matrix_no_visibility_or_local_mutation() {
            for layer in [L::Normal, L::Below, L::Above] {
                let (mut h, mut op) = setup(false, layer, true);
                let result = run(&mut h, &mut op).unwrap();
                assert_success(result, false, layer != L::Above);
                assert_eq!(h.mutations, expected_mutations(layer));
                assert_eq!(h.count("remove_hidden"), 0);
                assert_eq!((h.current.desktop, h.current.current), (7, 2));
                assert!(h.runtime.input_fence.blocked);
                assert_prohibitions(&h, true);
            }
        }
        #[test]
        fn off_workspace_minimized_three_layer_matrix_remove_hidden_then_above() {
            for layer in [L::Normal, L::Below, L::Above] {
                let (mut h, mut op) = setup(true, layer, true);
                restore(&mut h, &mut op, true);
                let result = run(&mut h, &mut op).unwrap();
                assert_success(result, false, true);
                assert!(op.bring_top.restoration_confirmed);
                assert_eq!(h.count("remove_hidden"), 1);
                assert_eq!(h.mutations, expected_mutations(layer));
                assert_eq!((h.current.desktop, h.current.current), (7, 2));
                let response = control::TerminalResponse {
                    status: result.status(),
                    detail: result.detail(),
                    request_id: 99,
                    control_xid: 50,
                };
                let diagnostic =
                    x11::control::CallerOutcome::Terminal(response).diagnostic(Command::BringTop);
                assert!(diagnostic.contains("another workspace"));
                assert!(!diagnostic.contains("visible here"));
                assert_prohibitions(&h, true);
            }
        }
        #[test]
        fn approved_normal_nonviewable_elsewhere_variants_are_deterministic_only() {
            for map in [MapState::Unmapped, MapState::Unviewable] {
                let (mut h, mut op) = setup(false, L::Normal, true);
                h.current.evidence.wm_state = WmState::Normal;
                h.current.evidence.map_state = Evidence::Readable(map);
                let result = run(&mut h, &mut op).unwrap();
                assert_success(result, false, true);
                assert_eq!(h.mutations, [Mutation::AddAbove]);
                assert_prohibitions(&h, true);
            }
        }
        #[test]
        fn restoration_timeout_never_starts_layer_or_creates_partial() {
            for elsewhere in [false, true] {
                let (mut h, mut op) = setup(true, L::Normal, elsewhere);
                assert!(run(&mut h, &mut op).is_none());
                h.now = op.bring_top.visibility_cutoff;
                let result = run(&mut h, &mut op).unwrap();
                assert_eq!(result.exit_code(), ExitCode::OperationFailed);
                assert!(!op.bring_top.restoration_confirmed);
                assert_eq!(h.count("layer_read"), 0);
                assert!(h.mutations.is_empty());
                assert_eq!(h.count(if elsewhere { "remove_hidden" } else { "map" }), 1);
            }
        }
        #[test]
        fn layer_timeout_is_partial_only_after_actual_restoration() {
            for minimized in [false, true] {
                for elsewhere in [false, true] {
                    let (mut h, mut op) = setup(minimized, L::Normal, elsewhere);
                    if minimized {
                        restore(&mut h, &mut op, elsewhere);
                    }
                    h.layer_mode = 1;
                    assert!(run(&mut h, &mut op).is_none());
                    assert_eq!(op.stage, Stage::Layer);
                    h.now = h.runtime.layers.deadline().unwrap();
                    let result = run(&mut h, &mut op).unwrap();
                    assert_eq!(
                        result.status(),
                        if minimized {
                            Status::Partial
                        } else {
                            Status::Failed
                        }
                    );
                    assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
                    assert!(result.detail().progress.not_minimized_confirmed);
                    assert!(!result.detail().progress.above_confirmed);
                    assert_eq!(result.detail().layer, L::Normal);
                    assert_eq!(h.mutations, [Mutation::AddAbove]);
                    assert!(h.runtime.layers.pending().is_none());
                    assert_prohibitions(&h, elsewhere);
                }
            }
        }
        #[test]
        fn layer_unsupported_and_unverifiable_use_restoration_proof() {
            for minimized in [false, true] {
                for issue in [
                    RemoteLayerIssue::Unsupported,
                    RemoteLayerIssue::Unverifiable,
                ] {
                    let (mut h, mut op) = setup(minimized, L::Normal, false);
                    if minimized {
                        restore(&mut h, &mut op, false);
                    }
                    h.layer_issue = Some(issue);
                    let result = run(&mut h, &mut op).unwrap();
                    assert_eq!(
                        result.status(),
                        if minimized {
                            Status::Partial
                        } else {
                            Status::Failed
                        }
                    );
                    assert_eq!(result.detail().reason, remote_layer_reason(issue));
                    assert!(h.mutations.is_empty());
                }
            }
        }
        #[test]
        fn below_removal_then_checked_add_error_is_failed_or_partial_without_rollback() {
            for minimized in [false, true] {
                let (mut h, mut op) = setup(minimized, L::Below, false);
                if minimized {
                    restore(&mut h, &mut op, false);
                }
                h.fail = Some("add_above");
                let original = service_bring_top_with(&mut op, &mut h).unwrap_err();
                assert_eq!(original.to_string(), "injected add_above");
                assert_eq!(h.current.layer, L::Normal);
                assert!(h.runtime.layers.pending().is_none());
                assert!(
                    op.bring_top.layer_attempted
                        && op.bring_top.layer_dispatched
                        && op.bring_top.layer_checked
                );
                assert!(op.detail.progress.host_mutation_dispatched);
                let result =
                    finish_bring_top_with(&mut op, &mut h, Reason::X11Failure, false).unwrap();
                assert_eq!(
                    result.status(),
                    if minimized {
                        Status::Partial
                    } else {
                        Status::Failed
                    }
                );
                assert_eq!(result.detail().layer, L::Normal);
                assert!(!result.detail().progress.above_confirmed);
                assert_eq!(h.mutations, expected_mutations(L::Below));
                assert_prohibitions(&h, false);
            }
        }
        #[test]
        fn checked_first_layer_error_preserves_dispatch_uncertainty_and_native_priority() {
            let (mut h, mut op) = setup(true, L::Normal, false);
            restore(&mut h, &mut op, false);
            h.fail = Some("add_above");
            assert!(service_bring_top_with(&mut op, &mut h).is_err());
            assert!(op.bring_top.layer_dispatched);
            assert!(!op.bring_top.layer_checked);
            let result = finish_bring_top_with(&mut op, &mut h, Reason::X11Failure, false).unwrap();
            assert_eq!(result.status(), Status::Partial);
            assert!(result.detail().progress.host_mutation_dispatched);
            assert!(!result.detail().progress.above_confirmed);
        }
        #[test]
        fn restoration_proof_precedes_geometry_failure_and_layer_is_never_started() {
            for failure in ["geometry", "enable", "shape"] {
                let (mut h, mut op) = setup(true, L::Normal, false);
                restore(&mut h, &mut op, false);
                h.fail = Some(failure);
                let original = service_bring_top_with(&mut op, &mut h).unwrap_err();
                assert!(original.to_string().contains(failure));
                assert!(op.bring_top.restoration_confirmed);
                h.fail = None;
                let result =
                    finish_bring_top_with(&mut op, &mut h, Reason::X11Failure, false).unwrap();
                assert_eq!(result.status(), Status::Partial);
                assert!(result.detail().progress.not_minimized_confirmed);
                assert!(!result.detail().progress.input_ready);
                assert_eq!(h.count("layer_read"), 0);
                assert_eq!(h.count("fence"), if failure == "geometry" { 1 } else { 2 });
            }
        }
        #[test]
        fn geometry_correction_and_input_enable_finish_before_layer() {
            let (mut h, mut op) = setup(true, L::Normal, false);
            restore(&mut h, &mut op, false);
            h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
            assert!(run(&mut h, &mut op).is_none());
            let deadline = op.show.correction.unwrap().1;
            assert_eq!(deadline, h.now + Duration::from_millis(150));
            assert_eq!(h.count("layer_read"), 0);
            h.now = deadline;
            assert_success(run(&mut h, &mut op).unwrap(), true, true);
            let index = |name| h.log.iter().position(|s| *s == name).unwrap();
            assert!(index("correct") < index("enable"));
            assert!(index("enable") < index("add_above"));
        }
        #[test]
        fn insufficient_geometry_budget_is_partial_with_no_correction_or_layer() {
            let (mut h, mut op) = setup(true, L::Normal, false);
            restore(&mut h, &mut op, false);
            h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
            h.now = op.bring_top.visibility_cutoff - Duration::from_millis(149);
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.status(), Status::Partial);
            assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
            assert_eq!(h.count("correct"), 0);
            assert_eq!(h.count("layer_read"), 0);
        }
        #[test]
        fn g_allows_coherent_layer_change_during_visibility_but_show_does_not() {
            for command in [Command::Show, Command::BringTop] {
                let (mut h, _) = setup(true, L::Below, false);
                let mut op = h.command_operation(command);
                assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                h.current = reading(Visibility::NotMinimizedHere, L::Above, 2, 2);
                let result = service_remote_with(&mut op, &mut h).unwrap().unwrap();
                assert_eq!(
                    result.status(),
                    if command == Command::Show {
                        Status::Failed
                    } else {
                        Status::Success
                    }
                );
                assert_eq!(
                    result.detail().reason,
                    if command == Command::Show {
                        Reason::LayerChanged
                    } else {
                        Reason::None
                    }
                );
                assert!(h.mutations.is_empty());
            }
        }
        #[test]
        fn workspace_drift_during_either_phase_is_terminal_without_compensation() {
            for minimized in [false, true] {
                for phase in [Stage::Show, Stage::Layer] {
                    let (mut h, mut op) = setup(minimized, L::Normal, false);
                    if minimized {
                        restore(&mut h, &mut op, false);
                    }
                    if phase == Stage::Layer {
                        h.layer_mode = 1;
                        assert!(run(&mut h, &mut op).is_none());
                    }
                    if phase == Stage::Show && !minimized {
                        observe_visibility(&mut op, &mut h).unwrap();
                    }
                    h.current.current = 3;
                    h.current.evidence.workspace = Workspace::Other;
                    h.current.evidence.wm_state = WmState::Iconic;
                    h.current.evidence.map_state = Evidence::Readable(MapState::Unmapped);
                    let result = run(&mut h, &mut op).unwrap();
                    assert_eq!(result.detail().reason, Reason::LayoutChanged);
                    assert_eq!(
                        result.status(),
                        if minimized && phase == Stage::Layer {
                            Status::Partial
                        } else {
                            Status::Failed
                        }
                    );
                    assert!(result.detail().progress.not_minimized_confirmed);
                    assert!(!result.detail().progress.placement_deferred);
                    assert!(!result.detail().progress.above_confirmed);
                    assert_prohibitions(&h, false);
                }
            }
        }
        #[test]
        fn historical_restoration_survives_evidence_loss_without_stale_current_bits() {
            for final_visibility in [
                Visibility::Destroyed,
                Visibility::Withdrawn,
                Visibility::Minimized,
            ] {
                let (mut h, mut op) = setup(true, L::Normal, false);
                restore(&mut h, &mut op, false);
                h.layer_mode = 1;
                assert!(run(&mut h, &mut op).is_none());
                h.current = reading(final_visibility, L::Normal, 2, 2);
                let result = run(&mut h, &mut op).unwrap();
                assert_eq!(result.status(), Status::Partial);
                assert!(!result.detail().progress.not_minimized_confirmed);
                assert!(!result.detail().progress.above_confirmed);
                assert!(!result.detail().progress.input_ready);
                assert!(!result.detail().progress.placement_deferred);
            }
        }
        #[test]
        fn spontaneous_above_after_removal_skips_redundant_addition() {
            let (mut h, mut op) = setup(false, L::Below, false);
            h.layer_mode = 3;
            assert_eq!(run(&mut h, &mut op).unwrap().status(), Status::Success);
            assert_eq!(h.mutations.len(), 1);
            assert_eq!(h.count("add_above"), 0);
        }
        #[test]
        fn below_reappearance_after_removal_and_conflict_are_terminal() {
            for layer in [L::Below, L::Conflict] {
                let (mut h, mut op) = setup(false, L::Below, false);
                h.layer_mode = 2;
                assert!(run(&mut h, &mut op).is_none());
                h.current.layer = layer;
                let result = run(&mut h, &mut op).unwrap();
                assert_eq!(result.status(), Status::Failed);
                assert_eq!(
                    result.detail().reason,
                    if layer == L::Below {
                        Reason::LayerChanged
                    } else {
                        Reason::Unverifiable
                    }
                );
                assert_eq!(h.mutations.len(), 2);
                assert!(!result.detail().progress.above_confirmed);
                assert!(h.runtime.layers.pending().is_none());
            }
        }
        #[test]
        fn layer_admission_caps_one_second_and_never_resets_owner_deadline() {
            for millis in [0, 2400, 2499] {
                let (mut h, mut op) = setup(true, L::Normal, false);
                let t0 = h.now;
                restore(&mut h, &mut op, false);
                h.now = t0 + Duration::from_millis(millis);
                h.layer_mode = 1;
                assert!(run(&mut h, &mut op).is_none());
                assert_eq!(op.deadline, t0 + Duration::from_millis(3500));
                assert_eq!(
                    op.bring_top.visibility_cutoff,
                    t0 + Duration::from_millis(2500)
                );
                assert_eq!(
                    h.layer_deadline(),
                    Some((h.now + Duration::from_millis(1000)).min(op.deadline))
                );
                for _ in 0..3 {
                    mark_remote_observation(&mut op, false);
                    assert!(run(&mut h, &mut op).is_none());
                    assert_eq!(op.deadline, t0 + Duration::from_millis(3500));
                }
            }
        }
        #[test]
        fn late_layer_entry_is_capped_by_total_owner_budget() {
            let (mut h, mut op) = setup(false, L::Normal, false);
            op.stage = Stage::Layer;
            op.detail.stage = Stage::Layer;
            op.show.baseline = Some(control::VisibilityBaseline {
                desktop: 2,
                current_desktop: 2,
                layer: L::Normal,
            });
            h.now += Duration::from_millis(3100);
            op.bring_top.layer_admitted = Some(h.now);
            h.layer_mode = 1;
            assert!(run(&mut h, &mut op).is_none());
            assert_eq!(h.layer_deadline(), Some(op.deadline));
            h.now = op.deadline;
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(h.mutations.len(), 1);
        }
        #[test]
        fn visibility_cutoff_allows_final_reads_but_no_new_mutation_or_layer_start() {
            for minimized in [false, true] {
                for layer in [L::Normal, L::Above] {
                    let (mut h, mut op) = setup(minimized, layer, false);
                    h.now = op.bring_top.visibility_cutoff;
                    let result = run(&mut h, &mut op).unwrap();
                    assert_eq!(
                        result.status(),
                        if !minimized && layer == L::Above {
                            Status::Success
                        } else {
                            Status::Failed
                        }
                    );
                    for name in ["fence", "map", "user_time_zero", "correct", "enable"] {
                        assert_eq!(h.count(name), 0);
                    }
                    assert!(h.mutations.is_empty());
                }
            }
        }
        #[test]
        fn final_layer_read_at_cutoff_can_confirm_achieved_above_without_resend() {
            let (mut h, mut op) = setup(false, L::Normal, false);
            h.layer_mode = 1;
            assert!(run(&mut h, &mut op).is_none());
            h.now = h.layer_deadline().unwrap();
            h.current.layer = L::Above;
            assert_eq!(run(&mut h, &mut op).unwrap().status(), Status::Success);
            assert_eq!(h.mutations, [Mutation::AddAbove]);
        }
        #[test]
        fn transient_qualification_preserves_pending_controller_and_fixed_deadline() {
            let (mut h, mut op) = setup(false, L::Normal, false);
            h.layer_mode = 1;
            assert!(run(&mut h, &mut op).is_none());
            let pending = h.runtime.layers.pending();
            h.layer_issue = Some(RemoteLayerIssue::Waiting);
            h.now += Duration::from_millis(200);
            assert!(run(&mut h, &mut op).is_none());
            assert_eq!(h.runtime.layers.pending(), pending);
            h.now = pending.unwrap().deadline;
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
            assert_eq!(h.mutations, [Mutation::AddAbove]);
        }
        #[test]
        fn layer_property_evidence_schedules_remote_read_and_configure_stays_ordinary() {
            use x11rb::protocol::xproto::{ConfigureNotifyEvent, PropertyNotifyEvent};
            let (mut h, mut op) = setup(false, L::Normal, false);
            h.layer_mode = 1;
            assert!(run(&mut h, &mut op).is_none());
            let event = Event::PropertyNotify(PropertyNotifyEvent {
                window: 100,
                atom: 12,
                ..Default::default()
            });
            assert!(remote_consumes_visibility_event(&op, &event, 100));
            mark_remote_observation(&mut op, false);
            assert!(remote_service_due(&op, h.now));
            let reads = h.count("layer_read");
            h.current.layer = L::Above;
            assert_eq!(run(&mut h, &mut op).unwrap().status(), Status::Success);
            assert!(h.count("layer_read") > reads);
            // The production route leaves ConfigureNotify on the existing
            // translated-root movement reconciliation path while G owns Layer.
            let event = ConfigureNotifyEvent {
                window: 100,
                width: 160,
                height: 160,
                ..Default::default()
            };
            assert!(!remote_consumes_visibility_event(
                &op,
                &Event::ConfigureNotify(event),
                100
            ));
            let target = Point::new(150, 160);
            h.window.in_flight_moves.push_back(target);
            h.window.requested_origin = target;
            assert!(
                matches!(h.window.handle_configure_notify_with(&event, || Ok(target)).unwrap(), ConfigureReconciliation::Confirmed {origin,..} if origin == target)
            );
            assert!(h.window.in_flight_moves.is_empty());
        }
        #[test]
        fn local_menu_cleanup_precedes_busy_and_cannot_steal_g_controller() {
            let (mut h, mut op) = setup(false, L::Normal, false);
            h.layer_mode = 1;
            assert!(run(&mut h, &mut op).is_none());
            let pending = h.runtime.layers.pending();
            h.runtime
                .acquire_menu_with(MenuPopup::test_popup(10, 1), Point::new(0, 0), || {
                    Ok((GrabStatus::SUCCESS, 100))
                })
                .unwrap();
            let log = RefCell::new(vec![]);
            h.runtime
                .complete_menu_with(
                    &mut h.window,
                    MenuOutcome::SetLayer(Layer::Below),
                    |p| {
                        log.borrow_mut().push("release");
                        p.release_with(|| Ok(())).map(|_| ())
                    },
                    |_| {
                        log.borrow_mut().push("destroy");
                        Ok(())
                    },
                    |_, target| {
                        log.borrow_mut().push("busy");
                        local_layer_after_menu_with(true, target, || {
                            panic!("remote controller must not be stolen")
                        })
                    },
                )
                .unwrap();
            assert_eq!(*log.borrow(), ["release", "destroy", "busy"]);
            assert_eq!(h.runtime.layers.pending(), pending);
            assert!(!h.runtime.pointer.is_grabbed());
            assert!(h.runtime.menu.is_none());
        }
        #[test]
        fn an_aborted_remote_controller_never_restarts_or_retries() {
            let (mut h, mut op) = setup(false, L::Normal, false);
            h.layer_mode = 1;
            assert!(run(&mut h, &mut op).is_none());
            h.runtime.layers.abort();
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(result.detail().reason, Reason::LayerChanged);
            assert_eq!(h.mutations, [Mutation::AddAbove]);
        }
        #[test]
        fn already_ready_popup_is_preserved_by_layer_only_g() {
            let (mut h, mut op) = setup(false, L::Normal, false);
            h.runtime
                .acquire_menu_with(MenuPopup::test_popup(10, 1), Point::new(0, 0), || {
                    Ok((GrabStatus::SUCCESS, 100))
                })
                .unwrap();
            assert_eq!(run(&mut h, &mut op).unwrap().status(), Status::Success);
            assert!(h.runtime.menu.is_some());
            assert!(h.runtime.pointer.is_grabbed());
            assert_eq!(h.count("fence"), 0);
        }
        #[test]
        fn expiry_during_layer_support_or_removal_never_sends_late_addition() {
            for step in ["layer_support", "remove_below"] {
                let (mut h, mut op) = setup(false, L::Below, false);
                h.advance = Some((step, Duration::from_millis(1000)));
                let result = run(&mut h, &mut op).unwrap();
                assert_eq!(result.status(), Status::Failed);
                assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
                assert_eq!(h.count("add_above"), 0);
                assert_eq!(h.count("remove_below"), usize::from(step == "remove_below"));
                assert!(!result.detail().progress.above_confirmed);
            }
        }
        #[test]
        fn visibility_read_crossing_cutoff_cannot_enable_input_or_start_layer() {
            let (mut h, mut op) = setup(true, L::Normal, false);
            restore(&mut h, &mut op, false);
            h.advance = Some(("geometry", op.bring_top.visibility_cutoff - h.now));
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.status(), Status::Partial);
            assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
            assert_eq!(h.count("enable"), 0);
            assert!(h.mutations.is_empty());
        }
        #[test]
        fn placement_busy_is_preflight_busy_without_any_g_mutation() {
            let (mut h, mut op) = setup(false, L::Normal, false);
            h.runtime.correction = Some((Point::new(200, 200), h.now + CONFIRMATION_TIMEOUT));
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.status(), Status::Busy);
            assert!(!result.detail().progress.host_mutation_dispatched);
            assert!(h.mutations.is_empty());
            assert_eq!(h.count("fence"), 0);
            assert_eq!(h.count("observe"), 0);
        }
        #[test]
        fn all_desktop_body_follows_local_path_and_preserves_desktop_sentinel() {
            let (mut h, mut op) = setup(true, L::Below, false);
            h.current.desktop = u32::MAX;
            h.current.evidence.workspace = Workspace::AllDesktops;
            assert!(run(&mut h, &mut op).is_none());
            h.current = reading(Visibility::NotMinimizedHere, L::Below, u32::MAX, 2);
            let result = run(&mut h, &mut op).unwrap();
            assert_success(result, true, true);
            assert_eq!(result.detail().workspace, Workspace::AllDesktops);
            assert_eq!(op.show.baseline.unwrap().desktop, u32::MAX);
            assert_eq!(h.count("map"), 1);
            assert_eq!(h.count("remove_hidden"), 0);
        }
        #[test]
        fn transitional_final_read_keeps_operation_without_restarting_completed_layer() {
            let (mut h, mut op) = setup(false, L::Above, false);
            let above = h.current;
            let mut transitional = above;
            transitional.evidence.coherent = false;
            transitional.layer = L::Unknown;
            h.reads = VecDeque::from([above, above, above, above, above, transitional]);
            assert!(run(&mut h, &mut op).is_none());
            assert!(op.bring_top.layer_finished);
            assert!(h.runtime.layers.pending().is_none());
            assert!(!remote_service_due(&op, h.now));
            assert_eq!(h.count("layer_read"), 1);
            h.current = above;
            mark_remote_observation(&mut op, false);
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.status(), Status::Success);
            assert_eq!(h.count("layer_read"), 1);
            assert!(h.mutations.is_empty());
        }
        #[test]
        fn final_layer_drift_cannot_report_failed_with_reason_none() {
            let (mut h, mut op) = setup(false, L::Above, false);
            observe_visibility(&mut op, &mut h).unwrap();
            op.stage = Stage::Layer;
            op.detail.stage = Stage::Layer;
            h.current.layer = L::Normal;
            let result = finish_bring_top_with(&mut op, &mut h, Reason::None, true).unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(result.detail().reason, Reason::LayerChanged);
            assert_eq!(result.detail().layer, L::Normal);
            assert!(result.detail().progress.not_minimized_confirmed);
            assert!(!result.detail().progress.above_confirmed);
        }
        #[test]
        fn local_layer_deadline_route_is_excluded_only_for_remote_layer_phase() {
            let (mut h, mut op) = setup(false, L::Normal, false);
            assert!(local_layer_service_allowed(None));
            assert!(local_layer_service_allowed(Some(&op)));
            h.layer_mode = 1;
            assert!(run(&mut h, &mut op).is_none());
            assert!(!local_layer_service_allowed(Some(&op)));
            assert_eq!(remote_operation_deadline(&op), h.layer_deadline().unwrap());
            assert_ne!(
                remote_operation_deadline(&op),
                op.bring_top.visibility_cutoff
            );
        }
    }
    mod scheduling {
        use super::*;

        #[derive(Clone, Copy)]
        enum Queued {
            Request(control::Request),
            Ordinary(&'static str),
            Visible,
            Minimized,
            Observed(Reading),
        }

        #[derive(Default)]
        struct Owner {
            active: Option<ActiveOperation>,
            replies: Vec<(Command, Status, Reason, Instant)>,
        }
        impl Owner {
            // Admission is injected at the validated transport boundary. The
            // scheduling seam and Show readiness/deadline logic are production code.
            fn route(&mut self, host: &mut Fake, event: Queued) -> Result<bool, HostError> {
                let Queued::Request(request) = event else {
                    return Ok(false);
                };
                host.step("request")?;
                match control::admit(
                    &mut self.active,
                    request,
                    crate::control::NO_DESTINATION,
                    Lifecycle::Ready,
                    false,
                    host.now,
                ) {
                    control::Admission::Admitted => {
                        host.step("admit")?;
                        let op = self.active.as_mut().unwrap();
                        op.stage = if request.command == Command::Hide {
                            control::Stage::Hide
                        } else {
                            control::Stage::Show
                        };
                        op.detail = control::preflight_detail(Reason::None, op.stage);
                    }
                    control::Admission::Duplicate => host.step("duplicate")?,
                    control::Admission::Reply(status, reason, _) => {
                        self.replies
                            .push((request.command, status, reason, host.now));
                    }
                }
                Ok(true)
            }
            fn service(&mut self, host: &mut Fake) -> Result<(), HostError> {
                let Some(op) = self.active.as_mut() else {
                    return Ok(());
                };
                if !remote_service_due(op, host.now) {
                    return Ok(());
                }
                if let Some(result) = service_remote_with(op, host)? {
                    self.replies.push((
                        op.request.command,
                        result.status(),
                        result.detail().reason,
                        host.now,
                    ));
                    self.active = None;
                }
                Ok(())
            }
        }

        struct Loop {
            owner: Owner,
            host: Fake,
            waits: Vec<Option<Instant>>,
            event_cost: Duration,
        }
        impl Loop {
            fn new() -> Self {
                Self {
                    owner: Owner::default(),
                    host: minimized(),
                    waits: vec![],
                    event_cost: Duration::ZERO,
                }
            }
            fn route(&mut self, event: Queued) -> Result<bool, HostError> {
                route_owner_request_with(
                    &mut self.owner,
                    &mut self.host,
                    |owner, host| owner.route(host, event),
                    Owner::service,
                )
            }
            // One outer iteration, including the real bounded drain, both
            // routing sites, and an injected socket wait (no X server or sleeps).
            fn iteration(&mut self, queue: &mut VecDeque<Queued>) -> Result<(), HostError> {
                self.owner.service(&mut self.host)?;
                for event in
                    drain_events_bounded(&mut None, 64, || Ok::<_, HostError>(queue.pop_front()))?
                {
                    if self.route(event)? {
                        continue;
                    }
                    if self
                        .owner
                        .active
                        .as_ref()
                        .is_some_and(|op| self.host.now >= remote_operation_deadline(op))
                    {
                        self.owner.service(&mut self.host)?;
                    }
                    match event {
                        Queued::Ordinary(name) => self.host.step(name)?,
                        Queued::Visible | Queued::Minimized => {
                            self.host.current = if matches!(event, Queued::Visible) {
                                visible()
                            } else {
                                reading(Visibility::Minimized, L::Normal, 2, 2)
                            };
                            if let Some(op) = self.owner.active.as_mut() {
                                mark_remote_observation(op, false);
                            }
                        }
                        Queued::Observed(reading) => {
                            self.host.current = reading;
                            if let Some(op) = self.owner.active.as_mut() {
                                mark_remote_observation(op, false);
                            }
                        }
                        Queued::Request(_) => unreachable!("control event consumed"),
                    }
                    self.host.now += self.event_cost;
                }
                self.owner.service(&mut self.host)?;
                if let Some(event) = queue.pop_front() {
                    if !self.route(event)? {
                        queue.push_front(event); // one-event lookahead retained
                    }
                    return Ok(()); // lookahead continues the outer loop
                }
                let deadline = probe_deadline(&self.host.runtime, None)
                    .into_iter()
                    .chain(self.owner.active.as_ref().map(remote_operation_deadline))
                    .min();
                self.host.step("wait")?;
                self.waits.push(deadline);
                Ok(())
            }
        }

        fn request(command: Command, id: u64) -> Queued {
            Queued::Request(control::Request {
                command,
                request_id: id,
                reply_xid: 88,
                epoch: 7,
            })
        }

        fn off_workspace_loop(command: Command) -> Loop {
            let mut s = Loop::new();
            s.host.current = reading(
                if command == Command::Hide {
                    Visibility::NotMinimizedElsewhere
                } else {
                    Visibility::Minimized
                },
                L::Normal,
                7,
                2,
            );
            s
        }

        #[test]
        fn g_more_than_64_events_before_visibility_or_layer_evidence_finish_without_wait() {
            for layer_phase in [false, true] {
                let mut s = Loop::new();
                if layer_phase {
                    s.host.current = visible();
                    s.host.layer_mode = 1;
                }
                s.route(request(Command::BringTop, 99)).unwrap();
                s.event_cost = Duration::from_millis(1);
                let mut q = VecDeque::from(vec![Queued::Ordinary("traffic"); 130]);
                q.push_back(Queued::Observed(reading(
                    Visibility::NotMinimizedHere,
                    if layer_phase { L::Above } else { L::Normal },
                    2,
                    2,
                )));
                while s.owner.active.is_some() {
                    s.iteration(&mut q).unwrap();
                }
                assert_eq!(
                    s.owner.replies,
                    [(Command::BringTop, Status::Success, Reason::None, s.host.now)]
                );
                assert_eq!(s.host.count("traffic"), 130);
                assert_eq!(s.host.count("map"), usize::from(!layer_phase));
                assert_eq!(s.host.count("add_above"), 1);
                assert_eq!(s.waits, [None]);
            }
        }
        #[test]
        fn sustained_g_traffic_cannot_starve_visibility_layer_or_total_deadlines() {
            for phase in 0..3 {
                let mut s = Loop::new();
                let t0 = s.host.now;
                if phase > 0 {
                    s.host.current = visible();
                    s.host.layer_mode = 1;
                }
                if phase == 2 {
                    // Adversarial late Layer admission exercises the total cap
                    // through the production adapter, with no controller renewal.
                    let mut op = s.host.command_operation(Command::BringTop);
                    observe_visibility(&mut op, &mut s.host).unwrap();
                    s.host.now = t0 + Duration::from_millis(3000);
                    op.stage = control::Stage::Layer;
                    op.detail.stage = control::Stage::Layer;
                    op.bring_top.layer_admitted = Some(s.host.now);
                    op.bring_top.layer_observe = true;
                    s.owner.active = Some(op);
                    s.owner.service(&mut s.host).unwrap();
                } else {
                    s.route(request(Command::BringTop, 99)).unwrap();
                }
                let expected = if phase == 0 {
                    2500
                } else if phase == 1 {
                    1000
                } else {
                    3500
                };
                s.event_cost = Duration::from_millis(if phase == 2 { 1 } else { 10 });
                let mut q = VecDeque::from(vec![Queued::Ordinary("traffic"); 1000]);
                while s.owner.active.is_some() {
                    s.iteration(&mut q).unwrap();
                }
                assert_eq!(
                    s.owner.replies,
                    [(
                        Command::BringTop,
                        Status::Failed,
                        Reason::ConfirmationTimeout,
                        t0 + Duration::from_millis(expected)
                    )]
                );
                assert!(!q.is_empty());
                assert!(s.waits.is_empty());
                assert_eq!(s.host.count("map"), usize::from(phase == 0));
                assert_eq!(s.host.count("add_above"), usize::from(phase > 0));
            }
        }
        #[test]
        fn g_wait_uses_earliest_movement_pointer_or_remote_deadline_without_idle_reads() {
            let mut s = Loop::new();
            s.host.current = visible();
            s.host.layer_mode = 1;
            let t0 = s.host.now;
            s.route(request(Command::BringTop, 99)).unwrap();
            let reads = s.host.count("observe");
            s.host.runtime.confirmation_deadline = Some(t0 + Duration::from_millis(150));
            s.host.runtime.safety.deadline = Some(t0 + Duration::from_millis(250));
            s.iteration(&mut VecDeque::new()).unwrap();
            assert_eq!(s.waits.last(), Some(&Some(t0 + Duration::from_millis(150))));
            s.host.runtime.confirmation_deadline = None;
            s.iteration(&mut VecDeque::new()).unwrap();
            assert_eq!(s.waits.last(), Some(&Some(t0 + Duration::from_millis(250))));
            s.host.runtime.safety.deadline = None;
            s.iteration(&mut VecDeque::new()).unwrap();
            assert_eq!(s.waits.last(), Some(&s.host.layer_deadline()));
            assert_eq!(s.host.count("observe"), reads);
            assert_eq!(s.host.count("add_above"), 1);
        }
        #[test]
        fn checked_restore_with_buffered_visibility_event_is_serviced_before_wait() {
            let mut s = Loop::new();
            let visible = visible();
            s.host.changes.push_back(("map", visible));
            s.route(request(Command::BringTop, 99)).unwrap();
            assert_eq!(s.host.count("map"), 1);
            assert!(s.owner.replies.is_empty());
            // The checked request buffered its notification; drain the real
            // bounded-batch seam rather than inventing another wake event.
            s.iteration(&mut VecDeque::from([Queued::Observed(visible)]))
                .unwrap();
            assert_eq!(s.owner.replies[0].1, Status::Success);
            assert_eq!(s.host.count("map"), 1);
            assert_eq!(s.waits, [None]);
        }
        #[test]
        fn all_remote_pairings_progress_original_only_and_keep_duplicate_deadlines() {
            for first in [Command::Hide, Command::Show, Command::BringTop] {
                for second in [Command::Hide, Command::Show, Command::BringTop] {
                    let mut s = Loop::new();
                    if first == Command::Hide {
                        s.host.current = visible();
                    }
                    s.route(request(first, 99)).unwrap();
                    let original = s.owner.active.unwrap();
                    let maps = s.host.count("map");
                    let hides = s.host.count("iconify");
                    let mutations = s.host.mutations.clone();
                    s.host.now += Duration::from_millis(20);
                    s.route(request(first, 99)).unwrap();
                    s.route(request(second, 100)).unwrap();
                    assert_eq!(s.owner.active, Some(original));
                    assert_eq!(s.host.count("map"), maps);
                    assert_eq!(s.host.count("iconify"), hides);
                    assert_eq!(s.host.mutations, mutations);
                    assert_eq!(s.owner.replies.len(), 1);
                    assert_eq!(s.owner.replies[0].1, Status::Busy);
                    s.host.current = reading(
                        if first == Command::Hide {
                            Visibility::Minimized
                        } else {
                            Visibility::NotMinimizedHere
                        },
                        L::Normal,
                        2,
                        2,
                    );
                    mark_remote_observation(s.owner.active.as_mut().unwrap(), false);
                    s.owner.service(&mut s.host).unwrap();
                    s.owner.service(&mut s.host).unwrap();
                    assert_eq!(s.owner.replies.len(), 2);
                    assert_eq!(s.owner.replies[1].0, first);
                    assert_eq!(s.owner.replies[1].1, Status::Success);
                    assert!(s.owner.active.is_none());
                }
            }
        }
        #[test]
        fn bring_top_lookahead_begins_before_outer_continue() {
            let mut s = Loop::new();
            let mut queue = VecDeque::from(vec![Queued::Ordinary("before"); 64]);
            queue.push_back(request(Command::BringTop, 99));
            s.iteration(&mut queue).unwrap();
            assert_eq!(s.host.count("map"), 1);
            assert!(s.waits.is_empty());
            assert_eq!(s.host.log.last(), Some(&"map"));
            assert_eq!(s.owner.active.unwrap().request.command, Command::BringTop);
        }
        #[test]
        fn bring_top_visibility_ready_and_immediate_layer_complete_in_same_service() {
            for layer in [L::Normal, L::Below, L::Above] {
                let mut s = Loop::new();
                s.host.current.layer = layer;
                s.iteration(&mut VecDeque::from([
                    request(Command::BringTop, 99),
                    Queued::Observed(reading(Visibility::NotMinimizedHere, layer, 2, 2)),
                ]))
                .unwrap();
                assert!(s.owner.active.is_none());
                assert_eq!(
                    s.owner.replies,
                    [(Command::BringTop, Status::Success, Reason::None, s.host.now)]
                );
                assert_eq!(s.waits, [None]);
                assert_eq!(s.host.count("map"), 1);
                assert_eq!(s.host.count("enable"), 1);
                assert_eq!(s.host.count("add_above"), usize::from(layer != L::Above));
            }
        }
        #[test]
        fn already_ready_g_layer_work_needs_no_future_event_or_poll() {
            for layer in [L::Normal, L::Below, L::Above] {
                let mut s = Loop::new();
                s.host.current = reading(Visibility::NotMinimizedHere, layer, 2, 2);
                s.iteration(&mut VecDeque::from([request(Command::BringTop, 99)]))
                    .unwrap();
                assert!(s.owner.active.is_none());
                assert_eq!(s.owner.replies[0].1, Status::Success);
                assert_eq!(s.waits, [None]);
                assert_eq!(s.host.count("fence"), 0);
                let count = s.host.count("observe");
                s.iteration(&mut VecDeque::new()).unwrap();
                assert_eq!(s.host.count("observe"), count);
            }
        }
        #[test]
        fn g_retains_identity_across_visibility_layer_and_competitors_are_busy() {
            let mut s = Loop::new();
            s.host.layer_mode = 1;
            let t0 = s.host.now;
            s.iteration(&mut VecDeque::from([
                request(Command::BringTop, 99),
                Queued::Visible,
            ]))
            .unwrap();
            let original = s.owner.active.unwrap();
            assert_eq!(original.request.command, Command::BringTop);
            assert_eq!(original.stage, control::Stage::Layer);
            assert_eq!(original.deadline, t0 + Duration::from_millis(3500));
            assert!(s.owner.replies.is_empty());
            s.host.now += Duration::from_millis(100);
            s.iteration(&mut VecDeque::from([
                request(Command::BringTop, 99),
                request(Command::BringTop, 100),
                request(Command::Hide, 101),
                request(Command::Show, 102),
            ]))
            .unwrap();
            assert_eq!(s.owner.active, Some(original));
            assert_eq!(s.host.count("duplicate"), 1);
            assert!(s.owner.replies.iter().all(|r| r.1 == Status::Busy));
            assert_eq!(s.host.count("map"), 1);
            assert_eq!(s.host.count("add_above"), 1);
            s.host.current.layer = L::Above;
            mark_remote_observation(s.owner.active.as_mut().unwrap(), false);
            s.owner.service(&mut s.host).unwrap();
            s.owner.service(&mut s.host).unwrap();
            assert_eq!(
                s.owner
                    .replies
                    .iter()
                    .filter(|r| r.0 == Command::BringTop && r.1 == Status::Success)
                    .count(),
                1
            );
        }
        #[test]
        fn last_batch_layer_evidence_completes_g_before_socket_wait() {
            let mut s = Loop::new();
            s.host.current = visible();
            s.host.layer_mode = 1;
            s.iteration(&mut VecDeque::from([request(Command::BringTop, 99)]))
                .unwrap();
            let mut queue = VecDeque::from(vec![Queued::Ordinary("traffic"); 63]);
            queue.push_back(Queued::Observed(reading(
                Visibility::NotMinimizedHere,
                L::Above,
                2,
                2,
            )));
            s.iteration(&mut queue).unwrap();
            assert!(s.owner.active.is_none());
            assert_eq!(s.owner.replies[0].1, Status::Success);
            assert_eq!(s.waits.last(), Some(&None));
            assert_eq!(s.host.count("add_above"), 1);
        }
        #[test]
        fn off_workspace_only_event_dispatches_hidden_request_before_socket_wait() {
            for command in [Command::Hide, Command::Show] {
                let mut s = off_workspace_loop(command);
                let admitted = s.host.now;
                s.iteration(&mut VecDeque::from([request(command, 99)]))
                    .unwrap();
                let step = if command == Command::Hide {
                    "add_hidden"
                } else {
                    "remove_hidden"
                };
                assert_eq!(s.host.count(step), 1);
                let index = |step| s.host.log.iter().position(|s| *s == step).unwrap();
                assert!(index(step) < index("wait"));
                assert_eq!(
                    s.waits,
                    [Some(
                        admitted
                            + if command == Command::Hide {
                                Duration::from_millis(1000)
                            } else {
                                Duration::from_millis(2500)
                            }
                    )]
                );
                assert!(s.owner.replies.is_empty());
                for step in ["iconify", "user_time_zero", "map", "geometry", "enable"] {
                    assert_eq!(s.host.count(step), 0);
                }
            }
        }

        #[test]
        fn off_workspace_lookahead_dispatches_hidden_request_before_continue() {
            for command in [Command::Hide, Command::Show] {
                let mut s = off_workspace_loop(command);
                let mut queue = VecDeque::from(vec![Queued::Ordinary("before"); 64]);
                queue.push_back(request(command, 99));
                s.iteration(&mut queue).unwrap();
                let step = if command == Command::Hide {
                    "add_hidden"
                } else {
                    "remove_hidden"
                };
                assert_eq!(s.host.log.last(), Some(&step));
                assert_eq!(s.host.count(step), 1);
                assert!(s.waits.is_empty());
            }
        }

        #[test]
        fn off_workspace_sustained_traffic_cannot_starve_or_reset_fixed_deadlines() {
            for command in [Command::Hide, Command::Show] {
                let mut s = off_workspace_loop(command);
                let admitted = s.host.now;
                s.event_cost = Duration::from_millis(10);
                let mut queue = VecDeque::from([request(command, 99)]);
                queue.extend(vec![Queued::Ordinary("traffic"); 400]);
                while s.owner.replies.is_empty() {
                    s.iteration(&mut queue).unwrap();
                }
                assert_eq!(
                    s.owner.replies,
                    [(
                        command,
                        Status::Failed,
                        Reason::ConfirmationTimeout,
                        admitted
                            + if command == Command::Hide {
                                Duration::from_millis(1000)
                            } else {
                                Duration::from_millis(2500)
                            }
                    )]
                );
                assert!(s.host.count("traffic") > 64);
                assert!(!queue.is_empty());
                assert!(s.waits.is_empty());
                assert_eq!(
                    s.host.count("add_hidden"),
                    usize::from(command == Command::Hide)
                );
                assert_eq!(
                    s.host.count("remove_hidden"),
                    usize::from(command == Command::Show)
                );
                for step in ["iconify", "user_time_zero", "map", "geometry", "enable"] {
                    assert_eq!(s.host.count(step), 0);
                }
            }
        }

        #[test]
        fn off_workspace_last_batch_notification_confirms_before_wait_without_restart() {
            for command in [Command::Hide, Command::Show] {
                let mut s = off_workspace_loop(command);
                let mut queue = VecDeque::from([request(command, 99)]);
                queue.extend(vec![Queued::Ordinary("traffic"); 62]);
                queue.push_back(Queued::Observed(reading(
                    if command == Command::Hide {
                        Visibility::Minimized
                    } else {
                        Visibility::NotMinimizedElsewhere
                    },
                    L::Normal,
                    7,
                    2,
                )));
                s.iteration(&mut queue).unwrap();
                assert_eq!(
                    s.owner.replies,
                    [(command, Status::Success, Reason::None, s.host.now)]
                );
                assert!(s.owner.active.is_none());
                assert_eq!(s.waits, [None]);
                assert_eq!(
                    s.host.count("add_hidden"),
                    usize::from(command == Command::Hide)
                );
                assert_eq!(
                    s.host.count("remove_hidden"),
                    usize::from(command == Command::Show)
                );
                let observations = s.host.count("observe");
                s.owner.service(&mut s.host).unwrap();
                assert_eq!(s.host.count("observe"), observations);
                assert_eq!(s.owner.replies.len(), 1);
            }
        }

        #[test]
        fn hide_only_event_fences_and_dispatches_before_wait_without_polling() {
            let mut s = Loop::new();
            s.host.current = visible();
            let admitted = s.host.now;
            s.iteration(&mut VecDeque::from([request(Command::Hide, 99)]))
                .unwrap();
            assert_eq!(
                s.host.log,
                ["request", "admit", "fence", "empty", "release", "observe", "iconify", "wait"]
            );
            assert_eq!(s.waits, [Some(admitted + Duration::from_millis(1000))]);
            assert_eq!(
                s.owner.active.unwrap().hide.phase,
                control::HidePhase::IconifyRequested
            );
            let reads = s.host.count("observe");
            s.iteration(&mut VecDeque::new()).unwrap();
            assert_eq!(s.host.count("observe"), reads);
            assert_eq!(s.host.count("iconify"), 1);
            assert_eq!(s.host.count("map"), 0);
        }
        #[test]
        fn hide_lookahead_dispatches_before_outer_continue_and_discards_pending_motion() {
            let mut s = Loop::new();
            s.host.current = visible();
            s.host.runtime.pending_move = Some(Point::new(200, 200));
            let mut queue = VecDeque::from(vec![Queued::Ordinary("before"); 64]);
            queue.push_back(request(Command::Hide, 99));
            s.iteration(&mut queue).unwrap();
            assert_eq!(s.host.count("before"), 64);
            assert_eq!(s.host.count("iconify"), 1);
            assert!(s.host.runtime.pending_move.is_none());
            assert!(s.waits.is_empty());
            assert_eq!(s.host.log.last(), Some(&"iconify"));
        }
        #[test]
        fn hide_notifications_at_end_of_batch_complete_before_wait_in_event_order() {
            let mut s = Loop::new();
            s.host.current = visible();
            let mut queue = VecDeque::from([
                Queued::Ordinary("before"),
                request(Command::Hide, 99),
                Queued::Ordinary("after"),
            ]);
            queue.extend(vec![Queued::Ordinary("traffic"); 60]);
            queue.push_back(Queued::Minimized);
            s.iteration(&mut queue).unwrap();
            assert!(s.owner.active.is_none());
            assert_eq!(s.owner.replies[0].0, Command::Hide);
            assert_eq!(s.owner.replies[0].1, Status::Success);
            assert_eq!(s.waits, [None]);
            assert_eq!(s.host.count("iconify"), 1);
            let index = |name| s.host.log.iter().position(|s| *s == name).unwrap();
            assert!(index("before") < index("iconify"));
            assert!(index("iconify") < index("after"));
            assert!(index("after") < index("wait"));
        }
        #[test]
        fn hide_duplicates_do_not_extend_deadline_or_retry_and_show_competes_as_busy() {
            let mut s = Loop::new();
            s.host.current = visible();
            let admitted = s.host.now;
            s.iteration(&mut VecDeque::from([
                request(Command::Hide, 99),
                request(Command::Hide, 99),
                request(Command::Show, 100),
            ]))
            .unwrap();
            assert_eq!(s.host.count("duplicate"), 1);
            assert_eq!(
                s.owner.replies,
                [(Command::Show, Status::Busy, Reason::Busy, admitted)]
            );
            assert_eq!(
                s.owner.active.unwrap().deadline,
                admitted + Duration::from_millis(1000)
            );
            assert_eq!(s.host.count("iconify"), 1);
            assert_eq!(s.host.count("map"), 0);
        }
        #[test]
        fn sustained_traffic_cannot_starve_hide_deadline_or_failed_input_restoration() {
            let mut s = Loop::new();
            s.host.current = visible();
            let admitted = s.host.now;
            s.event_cost = Duration::from_millis(10);
            let mut queue = VecDeque::from([request(Command::Hide, 99)]);
            queue.extend(vec![Queued::Ordinary("traffic"); 400]);
            while s.owner.replies.is_empty() {
                s.iteration(&mut queue).unwrap();
            }
            assert_eq!(
                s.owner.replies,
                [(
                    Command::Hide,
                    Status::Failed,
                    Reason::ConfirmationTimeout,
                    admitted + Duration::from_millis(1000)
                )]
            );
            assert!(s.host.count("traffic") > 64);
            assert!(!queue.is_empty());
            assert!(s.waits.is_empty());
            assert_eq!(s.host.count("iconify"), 1);
            assert!(s.host.runtime.accepts_input());
        }
        #[test]
        fn expected_unmap_and_state_notifications_schedule_hide_without_layout_abort() {
            let layout = x11::monitors::LayoutAtoms {
                number_of_desktops: 90,
                desktop_viewport: 91,
                workarea: 10,
                current_desktop: 11,
                wm_desktop: 12,
            };
            let visibility = x11::visibility::VisibilityAtoms {
                wm_state: 20,
                change_state: 21,
                state: 22,
                hidden: 23,
                above: 24,
                below: 25,
                desktop: 12,
                current_desktop: 11,
                supported: 26,
                user_time: 27,
            };
            for event in [
                Event::UnmapNotify(x11rb::protocol::xproto::UnmapNotifyEvent {
                    window: 100,
                    ..Default::default()
                }),
                Event::PropertyNotify(x11rb::protocol::xproto::PropertyNotifyEvent {
                    window: 100,
                    atom: 20,
                    ..Default::default()
                }),
                Event::PropertyNotify(x11rb::protocol::xproto::PropertyNotifyEvent {
                    window: 100,
                    atom: 22,
                    ..Default::default()
                }),
            ] {
                assert!(visibility_event(&event, 1, 100, &visibility, &layout));
                let h = Fake::new(visible());
                let mut op = h.command_operation(Command::Hide);
                op.hide.observe = false;
                mark_remote_observation(
                    &mut op,
                    visibility_layout_interruption(&event, 1, &layout),
                );
                assert!(remote_service_due(&op, h.now));
                assert!(!op.hide.layout_interrupted);
            }
            for atom in [layout.number_of_desktops, layout.desktop_viewport] {
                let event = Event::PropertyNotify(x11rb::protocol::xproto::PropertyNotifyEvent {
                    window: 1,
                    atom,
                    ..Default::default()
                });
                assert!(!visibility_event(&event, 1, 100, &visibility, &layout));
                assert!(!visibility_layout_interruption(&event, 1, &layout));
            }
        }

        #[test]
        fn only_show_event_starts_restore_before_socket_wait_without_another_event() {
            let mut scheduler = Loop::new();
            let admission = scheduler.host.now;
            assert!(scheduler.owner.active.is_none());
            let mut queue = VecDeque::from([request(Command::Show, 99)]);
            scheduler.iteration(&mut queue).unwrap();
            assert!(queue.is_empty());
            assert_eq!(scheduler.host.count("user_time_zero"), 1);
            assert_eq!(scheduler.host.count("map"), 1);
            assert_eq!(scheduler.host.now, admission);
            assert_eq!(
                scheduler.owner.active.unwrap().show.phase,
                ShowPhase::RestoreRequested
            );
            assert_eq!(
                scheduler.waits,
                [Some(admission + Duration::from_millis(2500))]
            );
            assert_eq!(scheduler.host.log.last(), Some(&"wait"));
        }

        #[test]
        fn early_show_preserves_events_before_and_after_it_in_the_drained_batch() {
            let mut scheduler = Loop::new();
            let mut queue = VecDeque::from([
                Queued::Ordinary("before_first"),
                Queued::Ordinary("before_second"),
                request(Command::Show, 99),
                Queued::Ordinary("after_first"),
                Queued::Ordinary("after_second"),
            ]);
            scheduler.iteration(&mut queue).unwrap();
            assert!(queue.is_empty());
            assert_eq!(
                scheduler.host.log,
                [
                    "before_first",
                    "before_second",
                    "request",
                    "admit",
                    "observe",
                    "fence",
                    "empty",
                    "release",
                    "observe",
                    "user_time_zero",
                    "map",
                    "after_first",
                    "after_second",
                    "wait",
                ]
            );
        }

        #[test]
        fn duplicate_does_not_retry_map_and_competing_request_is_busy() {
            let mut scheduler = Loop::new();
            let admission = scheduler.host.now;
            let mut queue = VecDeque::from([
                request(Command::Show, 99),
                request(Command::Show, 99),
                request(Command::Show, 100),
            ]);
            scheduler.iteration(&mut queue).unwrap();
            assert_eq!(scheduler.host.count("map"), 1);
            assert_eq!(scheduler.host.count("user_time_zero"), 1);
            assert_eq!(scheduler.host.count("duplicate"), 1);
            assert_eq!(scheduler.host.count("observe"), 2);
            assert_eq!(
                scheduler.owner.replies,
                [(Command::Show, Status::Busy, Reason::Busy, admission)]
            );
            let op = scheduler.owner.active.unwrap();
            assert_eq!(op.request.request_id, 99);
            assert_eq!(op.deadline, admission + Duration::from_millis(2500));
        }

        #[test]
        fn bring_top_request_only_batch_starts_restore_before_wait() {
            let mut scheduler = Loop::new();
            let admitted = scheduler.host.now;
            scheduler
                .iteration(&mut VecDeque::from([request(Command::BringTop, 2)]))
                .unwrap();
            let op = scheduler.owner.active.unwrap();
            assert_eq!(op.request.command, Command::BringTop);
            assert_eq!(op.deadline, admitted + Duration::from_millis(3500));
            assert_eq!(scheduler.host.count("map"), 1);
            assert!(scheduler.host.mutations.is_empty());
            assert_eq!(
                scheduler.waits,
                [Some(admitted + Duration::from_millis(2500))]
            );
            assert!(scheduler.owner.replies.is_empty());
        }

        #[test]
        fn lookahead_request_gets_the_same_immediate_progression_before_outer_continue() {
            let mut scheduler = Loop::new();
            let admission = scheduler.host.now;
            let mut queue = VecDeque::from(vec![Queued::Ordinary("before"); 64]);
            queue.push_back(request(Command::Show, 99));
            scheduler.iteration(&mut queue).unwrap();
            assert!(queue.is_empty());
            assert_eq!(scheduler.host.count("before"), 64);
            assert_eq!(scheduler.host.count("map"), 1);
            assert_eq!(scheduler.host.now, admission);
            assert!(scheduler.waits.is_empty()); // lookahead continues, after servicing
            assert_eq!(scheduler.host.log.last(), Some(&"map"));
            scheduler.iteration(&mut queue).unwrap();
            assert_eq!(
                scheduler.waits,
                [Some(admission + Duration::from_millis(2500))]
            );
            assert_eq!(scheduler.host.count("map"), 1);
        }

        #[test]
        fn idle_and_pending_show_reach_socket_wait_without_repeated_work_or_spin() {
            let mut scheduler = Loop::new();
            scheduler.iteration(&mut VecDeque::new()).unwrap();
            assert_eq!(scheduler.waits, [None]);
            assert_eq!(scheduler.host.log, ["wait"]);
            scheduler
                .iteration(&mut VecDeque::from([request(Command::Show, 99)]))
                .unwrap();
            let observations = scheduler.host.count("observe");
            let deadline = scheduler.owner.active.unwrap().deadline;
            scheduler.iteration(&mut VecDeque::new()).unwrap();
            assert_eq!(scheduler.waits, [None, Some(deadline), Some(deadline)]);
            assert_eq!(scheduler.host.count("observe"), observations);
            assert_eq!(scheduler.host.count("map"), 1);
            assert!(!remote_service_due(
                scheduler.owner.active.as_ref().unwrap(),
                scheduler.host.now
            ));
        }

        #[test]
        fn last_batch_notification_finishes_show_before_socket_wait() {
            let mut scheduler = Loop::new();
            scheduler
                .iteration(&mut VecDeque::from([
                    request(Command::Show, 99),
                    Queued::Visible,
                ]))
                .unwrap();
            assert!(scheduler.owner.active.is_none());
            assert_eq!(scheduler.owner.replies[0].1, Status::Success);
            assert_eq!(scheduler.host.count("map"), 1);
            assert_eq!(scheduler.host.count("enable"), 1);
            assert_eq!(scheduler.waits, [None]);
        }

        #[test]
        fn correction_wait_uses_150_ms_and_wakes_without_another_event() {
            let mut scheduler = Loop::new();
            let admission = scheduler.host.now;
            scheduler.host.bounds = ValidOriginBounds::new(200, 500, 200, 500);
            scheduler
                .iteration(&mut VecDeque::from([
                    request(Command::Show, 99),
                    Queued::Visible,
                ]))
                .unwrap();
            let op = scheduler.owner.active.unwrap();
            let confirmation = admission + Duration::from_millis(150);
            assert_eq!(op.show.correction.unwrap().1, confirmation);
            assert_eq!(scheduler.waits, [Some(confirmation)]);
            assert_eq!(scheduler.host.count("enable"), 0);
            scheduler.host.now = confirmation - Duration::from_millis(1);
            assert!(!remote_service_due(&op, scheduler.host.now));
            scheduler.host.now = confirmation;
            scheduler.iteration(&mut VecDeque::new()).unwrap();
            assert!(scheduler.owner.active.is_none());
            assert_eq!(scheduler.owner.replies[0].1, Status::Success);
            assert_eq!(scheduler.host.count("correct"), 1);
            assert_eq!(scheduler.host.count("enable"), 1);
            assert_eq!(scheduler.host.count("map"), 1);
            assert_eq!(scheduler.waits.last(), Some(&None));
        }

        #[test]
        fn sustained_batches_cannot_starve_fixed_remote_deadline() {
            let mut scheduler = Loop::new();
            let admission = scheduler.host.now;
            scheduler.event_cost = Duration::from_millis(10);
            let mut queue = VecDeque::from([request(Command::Show, 99)]);
            queue.extend(vec![Queued::Ordinary("traffic"); 400]);
            while scheduler.owner.replies.is_empty() {
                scheduler.iteration(&mut queue).unwrap();
            }
            assert_eq!(
                scheduler.owner.replies,
                [(
                    Command::Show,
                    Status::Failed,
                    Reason::ConfirmationTimeout,
                    admission + Duration::from_millis(2500)
                )]
            );
            assert!(!queue.is_empty());
            assert!(scheduler.host.count("traffic") > 64);
            assert!(scheduler.waits.is_empty());
            assert_eq!(scheduler.host.count("user_time_zero"), 1);
            assert_eq!(scheduler.host.count("map"), 1);
            assert_eq!(scheduler.host.count("correct"), 0);
            assert_eq!(scheduler.host.count("enable"), 0);
        }
    }

    #[test]
    fn acknowledged_map_is_pending_until_fresh_wm_geometry_and_input_confirmation() {
        let mut h = minimized();
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        assert_eq!(
            h.log,
            [
                "observe",
                "fence",
                "empty",
                "release",
                "observe",
                "user_time_zero",
                "map"
            ]
        );
        assert_eq!(op.show.phase, ShowPhase::RestoreRequested);
        assert!(!op.detail.progress.not_minimized_confirmed);
        assert!(!op.detail.progress.input_ready);
        assert!(h.run(&mut op).is_none()); // notification is not proof either
        assert_eq!(h.count("map"), 1);
        h.current = visible();
        assert_success(h.run(&mut op).unwrap(), true, true);
        assert_eq!(h.count("map"), 1);
        assert_eq!(h.count("user_time_zero"), 1);
        assert!(
            h.log.iter().position(|s| *s == "geometry").unwrap()
                < h.log.iter().position(|s| *s == "enable").unwrap()
        );
        assert_eq!(h.runtime.input_fence.enabled_after, Some(4 * 65536 + 123));
        assert_eq!(h.runtime.pending_map_input, None);
    }
    #[test]
    fn fresh_above_normal_below_and_all_desktops_are_preserved_for_restore_and_noop() {
        for layer in [L::Above, L::Normal, L::Below] {
            for desktop in [2, u32::MAX] {
                for restore in [false, true] {
                    let mut h = Fake::new(reading(
                        if restore {
                            Visibility::Minimized
                        } else {
                            Visibility::NotMinimizedHere
                        },
                        layer,
                        desktop,
                        2,
                    ));
                    let mut op = h.operation();
                    if restore {
                        assert!(h.run(&mut op).is_none());
                        h.current = reading(Visibility::NotMinimizedHere, layer, desktop, 2);
                    }
                    let result = h.run(&mut op).unwrap();
                    assert_success(result, true, restore);
                    assert_eq!(op.show.baseline.unwrap().desktop, desktop);
                    assert_eq!(result.detail().layer, layer);
                    assert_eq!(h.count("map"), usize::from(restore));
                }
            }
        }
    }
    #[test]
    fn local_noop_preserves_idle_drag_opening_and_popup_without_shape_or_release() {
        for gesture in 0..=4 {
            let mut h = Fake::new(visible());
            h.runtime = visibility_fence_tests::runtime(gesture);
            let state = h.runtime.interaction.state();
            let owner = h.runtime.pointer.owner();
            let mut op = h.operation();
            assert_success(h.run(&mut op).unwrap(), true, false);
            assert_eq!(h.runtime.interaction.state(), state);
            assert_eq!(h.runtime.pointer.owner(), owner);
            for step in [
                "fence", "empty", "release", "popup", "map", "enable", "correct",
            ] {
                assert_eq!(h.count(step), 0);
            }
        }
    }
    #[test]
    fn off_workspace_iconic_and_normal_nonviewable_noops_never_map_or_query_geometry() {
        for (wm, map) in [
            (WmState::Iconic, MapState::Unmapped),
            (WmState::Normal, MapState::Unmapped),
            (WmState::Normal, MapState::Unviewable),
        ] {
            let mut r = reading(Visibility::NotMinimizedElsewhere, L::Below, 7, 2);
            r.evidence.wm_state = wm;
            r.evidence.map_state = Evidence::Readable(map);
            let mut h = Fake::new(r);
            let mut op = h.operation();
            assert_success(h.run(&mut op).unwrap(), false, false);
            assert_eq!(h.count("map"), 0);
            assert_eq!(h.count("remove_hidden"), 0);
            assert_eq!(h.count("geometry"), 0);
            assert_eq!(h.count("correct"), 0);
            assert!(h.runtime.input_fence.blocked);
            assert!(!h.runtime.accepts_input());
        }
    }
    #[test]
    fn actually_minimized_elsewhere_can_finish_iconic_unmapped_without_local_normal_transition() {
        let mut h = Fake::new(reading(Visibility::Minimized, L::Above, 7, 2));
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        assert_eq!(op.show.phase, ShowPhase::HiddenRemoveRequested);
        assert_eq!(h.count("remove_hidden"), 1);
        assert_eq!(h.count("user_time_zero"), 0);
        assert_eq!(h.count("map"), 0);
        assert_eq!(h.runtime.pending_map_input, None);
        assert!(h.run(&mut op).is_none()); // checked REMOVE is not confirmation
        h.current = reading(Visibility::NotMinimizedElsewhere, L::Above, 7, 2);
        assert_success(h.run(&mut op).unwrap(), false, true);
        assert_eq!(h.count("geometry"), 0);
        assert_eq!(h.count("enable"), 0);
        assert_eq!(h.count("correct"), 0);
        assert_eq!(h.count("remove_hidden"), 1);
        assert!(h.runtime.input_fence.blocked);
        assert!(!h.runtime.accepts_input());
        assert_eq!(op.show.baseline.unwrap().desktop, 7);
    }

    #[test]
    fn off_workspace_remove_refusal_times_out_without_retry_map_or_input() {
        let mut h = Fake::new(reading(Visibility::Minimized, L::Below, 7, 2));
        let mut op = h.operation();
        let deadline = op.deadline;
        assert!(h.run(&mut op).is_none());
        for _ in 0..3 {
            mark_remote_observation(&mut op, false);
            assert!(h.run(&mut op).is_none());
            assert_eq!(op.deadline, deadline);
        }
        h.now = deadline;
        let result = h.run(&mut op).unwrap();
        assert_eq!(result.status(), Status::Failed);
        assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
        assert_eq!(h.count("remove_hidden"), 1);
        for step in ["user_time_zero", "map", "geometry", "correct", "enable"] {
            assert_eq!(h.count(step), 0);
        }
        assert!(h.runtime.input_fence.blocked);
    }

    #[test]
    fn off_workspace_show_needs_fresh_final_no_hidden_and_never_arms_map_shape_sync() {
        let mut h = Fake::new(reading(Visibility::Minimized, L::Above, 7, 2));
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        let remote = reading(Visibility::NotMinimizedElsewhere, L::Above, 7, 2);
        h.reads = VecDeque::from([remote, h.current]);
        assert!(h.run(&mut op).is_none()); // HIDDEN returned on the final fresh read
        assert!(!op.detail.progress.not_minimized_confirmed);
        assert_eq!(h.count("remove_hidden"), 1);
        let event = MapNotifyEvent {
            response_type: x11rb::protocol::xproto::MAP_NOTIFY_EVENT,
            event: h.window.window,
            window: h.window.window,
            ..Default::default()
        };
        h.runtime
            .synchronize_map_input_with(&event, h.window.window, 70001, || {
                panic!("REMOVE-HIDDEN must not arm post-map SHAPE or enable remote input")
            })
            .unwrap();
        assert_eq!(h.runtime.pending_map_input, None);
        assert!(h.runtime.input_fence.blocked);
        // Another approved coherent representation may finish without mapping.
        h.current = remote;
        h.current.evidence.wm_state = WmState::Normal;
        h.current.evidence.map_state = Evidence::Readable(MapState::Unviewable);
        assert_success(h.run(&mut op).unwrap(), false, true);
        for step in ["map", "user_time_zero", "geometry", "enable"] {
            assert_eq!(h.count(step), 0);
        }
    }

    #[test]
    fn off_workspace_checked_failures_consume_distinct_attempts_without_retry() {
        for command in [Command::Hide, Command::Show] {
            let (visibility, step) = if command == Command::Hide {
                (Visibility::NotMinimizedElsewhere, "add_hidden")
            } else {
                (Visibility::Minimized, "remove_hidden")
            };
            let mut h = Fake::new(reading(visibility, L::Normal, 7, 2));
            h.fail = Some(step);
            let mut op = h.command_operation(command);
            let deadline = op.deadline;
            assert!(service_visibility_with(&mut op, &mut h).is_err());
            if command == Command::Hide {
                assert_eq!(op.hide.phase, control::HidePhase::HiddenAddAttempted);
            } else {
                assert_eq!(op.show.phase, ShowPhase::HiddenRemoveAttempted);
            }
            assert!(op.detail.progress.host_mutation_dispatched);
            h.fail = None;
            assert!(service_visibility_with(&mut op, &mut h).unwrap().is_none());
            assert_eq!(h.count(step), 1);
            assert_eq!(op.deadline, deadline);
            for step in ["iconify", "user_time_zero", "map", "geometry", "enable"] {
                assert_eq!(h.count(step), 0);
            }
        }
    }

    #[test]
    fn off_workspace_workspace_current_and_layer_drift_fail_without_compensation() {
        for command in [Command::Hide, Command::Show] {
            for case in 0..3 {
                let before = if command == Command::Hide {
                    Visibility::NotMinimizedElsewhere
                } else {
                    Visibility::Minimized
                };
                let after = if command == Command::Hide {
                    Visibility::Minimized
                } else {
                    Visibility::NotMinimizedElsewhere
                };
                let mut h = Fake::new(reading(before, L::Above, 7, 2));
                let mut op = h.command_operation(command);
                assert!(service_visibility_with(&mut op, &mut h).unwrap().is_none());
                h.current = match case {
                    0 => reading(after, L::Above, 8, 2),
                    1 => reading(after, L::Above, 7, 3),
                    _ => reading(after, L::Below, 7, 2),
                };
                let result = service_visibility_with(&mut op, &mut h).unwrap().unwrap();
                assert_eq!(result.status(), Status::Failed);
                assert_eq!(
                    result.detail().reason,
                    if case == 2 {
                        Reason::LayerChanged
                    } else {
                        Reason::LayoutChanged
                    }
                );
                assert_eq!(h.count("add_hidden"), usize::from(command == Command::Hide));
                assert_eq!(
                    h.count("remove_hidden"),
                    usize::from(command == Command::Show)
                );
                for step in [
                    "iconify",
                    "user_time_zero",
                    "map",
                    "geometry",
                    "correct",
                    "enable",
                ] {
                    assert_eq!(h.count(step), 0);
                }
            }
        }
    }

    #[test]
    fn off_workspace_expiry_before_dispatch_never_sends_hidden_request() {
        for command in [Command::Hide, Command::Show] {
            for step in ["admission", "fence", "observe"] {
                let visibility = if command == Command::Hide {
                    Visibility::NotMinimizedElsewhere
                } else {
                    Visibility::Minimized
                };
                let mut h = Fake::new(reading(visibility, L::Normal, 7, 2));
                let mut op = h.command_operation(command);
                if step == "admission" {
                    h.now = op.deadline;
                } else {
                    h.advance = Some((step, op.deadline.duration_since(h.now)));
                }
                let result = service_visibility_with(&mut op, &mut h).unwrap().unwrap();
                assert_eq!(result.status(), Status::Failed);
                assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
                assert!(!result.detail().progress.host_mutation_dispatched);
                for step in [
                    "add_hidden",
                    "remove_hidden",
                    "iconify",
                    "user_time_zero",
                    "map",
                    "enable",
                ] {
                    assert_eq!(h.count(step), 0);
                }
            }
        }
    }

    #[test]
    fn off_workspace_fence_cancels_every_gesture_before_hidden_dispatch() {
        for command in [Command::Hide, Command::Show] {
            for gesture in 0..5 {
                let visibility = if command == Command::Hide {
                    Visibility::NotMinimizedElsewhere
                } else {
                    Visibility::Minimized
                };
                let mut h = Fake::new(reading(visibility, L::Normal, 7, 2));
                h.runtime = visibility_fence_tests::runtime(gesture);
                h.runtime.pending_move = Some(Point::new(200, 200));
                h.runtime.final_target = Some(Point::new(210, 210));
                let mut op = h.command_operation(command);
                assert!(service_visibility_with(&mut op, &mut h).unwrap().is_none());
                let request = if command == Command::Hide {
                    "add_hidden"
                } else {
                    "remove_hidden"
                };
                let index = |step| h.log.iter().position(|s| *s == step).unwrap();
                assert!(index("empty") < index("release"));
                assert!(index("release") < index(request));
                if gesture == 3 || gesture == 4 {
                    assert!(index("popup") < index(request));
                }
                assert_eq!(h.count(request), 1);
                assert!(h.runtime.visibility_dispatch_eligible());
                assert!(h.runtime.input_fence.blocked);
                assert!(!h.runtime.pointer.is_grabbed());
                assert!(h.runtime.pending_move.is_none());
                assert!(h.runtime.final_target.is_none());
                assert_eq!(h.runtime.pending_map_input, None);
            }
        }
    }

    #[test]
    fn off_workspace_fence_failure_blocks_both_hidden_requests() {
        for command in [Command::Hide, Command::Show] {
            for fail in ["empty", "release", "popup"] {
                let visibility = if command == Command::Hide {
                    Visibility::NotMinimizedElsewhere
                } else {
                    Visibility::Minimized
                };
                let mut h = Fake::new(reading(visibility, L::Normal, 7, 2));
                h.runtime = visibility_fence_tests::runtime(4);
                h.fail = Some(fail);
                let mut op = h.command_operation(command);
                assert!(service_visibility_with(&mut op, &mut h).is_err());
                assert_eq!(h.count("popup"), 1);
                assert_eq!(h.count("add_hidden"), 0);
                assert_eq!(h.count("remove_hidden"), 0);
                assert!(!op.detail.progress.host_mutation_dispatched);
                assert!(!h.runtime.visibility_dispatch_eligible());
            }
        }
    }
    #[test]
    fn malformed_missing_unsupported_and_incoherent_evidence_never_dispatch() {
        for case in 0..8 {
            let mut h = minimized();
            match case {
                0 => h.current.evidence.wm_state = WmState::Malformed,
                1 => h.current.evidence.hidden = Evidence::Missing,
                2 => h.current.evidence.hidden = Evidence::Malformed,
                3 => h.current.evidence.capability = Capability::Unsupported,
                4 => h.current.evidence.workspace = Workspace::Unknown,
                5 => h.current.evidence.coherent = false,
                6 => h.current.evidence.hidden = Evidence::Readable(false),
                _ => h.current.evidence.capability = Capability::Unverifiable,
            }
            let mut op = h.operation();
            let first = h.run(&mut op);
            if case == 3 {
                assert_eq!(first.unwrap().detail().reason, Reason::Unsupported);
            } else {
                assert!(first.is_none());
                h.now = op.deadline;
                assert_eq!(
                    h.run(&mut op).unwrap().detail().reason,
                    Reason::ConfirmationTimeout
                );
            }
            assert_eq!(h.count("map"), 0);
            assert_eq!(h.count("fence"), 0);
        }
    }
    #[test]
    fn withdrawn_destroyed_and_conflicting_layer_baseline_fail_without_mutation() {
        for (visibility, layer, reason) in [
            (Visibility::Withdrawn, L::Normal, Reason::Withdrawn),
            (Visibility::Destroyed, L::Normal, Reason::BodyDestroyed),
            (Visibility::Minimized, L::Conflict, Reason::Unverifiable),
            (Visibility::Minimized, L::Unknown, Reason::Unverifiable),
        ] {
            let mut h = Fake::new(reading(visibility, layer, 2, 2));
            let mut op = h.operation();
            let result = h.run(&mut op).unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(result.detail().reason, reason);
            assert_eq!(h.count("map"), 0);
            assert_eq!(h.count("fence"), 0);
        }
    }
    #[test]
    fn layer_workspace_and_current_desktop_drift_fail_without_compensating_writes() {
        for case in 0..4 {
            let mut h = minimized();
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            h.current = visible();
            match case {
                0 => h.current.layer = L::Above,
                1 => {
                    h.current.desktop = 7;
                    h.current.evidence.workspace = Workspace::Other;
                }
                2 => {
                    h.current.current = 3;
                    h.current.evidence.workspace = Workspace::Other;
                }
                _ => h.current.layer = L::Conflict,
            }
            let result = h.run(&mut op).unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(
                result.detail().reason,
                if case == 0 || case == 3 {
                    Reason::LayerChanged
                } else {
                    Reason::LayoutChanged
                }
            );
            assert_eq!(h.count("map"), 1);
            assert_eq!(h.count("enable"), 0);
            assert_eq!(h.count("correct"), 0);
        }
    }
    #[test]
    fn minimized_fence_settles_idle_stale_drag_automatic_right_and_popup_before_map() {
        for gesture in 0..=4 {
            let mut h = minimized();
            h.runtime = visibility_fence_tests::runtime(gesture);
            h.runtime.pending_move = Some(Point::new(120, 120));
            h.runtime.final_target = Some(Point::new(120, 120));
            h.runtime.confirmation_deadline = Some(h.now);
            h.window.in_flight_moves.push_back(Point::new(120, 120));
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            assert!(h.runtime.visibility_dispatch_eligible());
            assert!(h.runtime.menu.is_none());
            assert!(!h.runtime.pointer.is_grabbed());
            assert!(
                h.log.iter().position(|s| *s == "release").unwrap()
                    < h.log.iter().position(|s| *s == "map").unwrap()
            );
            if gesture >= 3 {
                assert!(
                    h.log.iter().position(|s| *s == "popup").unwrap()
                        < h.log.iter().position(|s| *s == "map").unwrap()
                );
            }
        }
    }
    #[test]
    fn failed_release_empty_shape_or_popup_cleanup_blocks_map_and_preserves_debt() {
        for failure in ["empty", "release", "popup"] {
            let mut h = minimized();
            h.runtime = visibility_fence_tests::runtime(3);
            h.fail = Some(failure);
            let mut op = h.operation();
            let error = service_show_with(&mut op, &mut h).unwrap_err();
            assert!(error.to_string().contains(failure));
            assert_eq!(h.count("popup"), 1);
            assert_eq!(h.count("release"), 1);
            assert_eq!(h.count("map"), 0);
            assert!(h.runtime.input_fence.blocked);
            assert!(!h.runtime.visibility_dispatch_eligible());
            if failure == "release" {
                assert!(h.runtime.pointer.is_grabbed());
            }
        }
        // No tracked owner is not proof of no server automatic capture.
        let mut h = minimized();
        h.fail = Some("release");
        let mut op = h.operation();
        assert!(service_show_with(&mut op, &mut h).is_err());
        assert!(h.runtime.pointer.is_grabbed());
        assert_eq!(h.count("map"), 0);
    }
    #[test]
    fn native_restore_error_is_fatal_and_never_retried_or_confirmed() {
        for failure in ["user_time_zero", "map"] {
            let mut h = minimized();
            h.fail = Some(failure);
            let mut op = h.operation();
            assert!(service_show_with(&mut op, &mut h).is_err());
            assert_eq!(op.show.phase, ShowPhase::RestoreAttempted);
            assert_eq!(op.detail.reason, Reason::X11Failure);
            assert!(!op.detail.progress.not_minimized_confirmed);
            assert!(op.detail.progress.host_mutation_dispatched);
            assert_eq!(h.count("user_time_zero"), 1);
            assert_eq!(h.count("map"), usize::from(failure == "map"));
            assert_eq!(h.count("enable"), 0);
            h.fail = None;
            assert!(h.run(&mut op).is_none());
            assert_eq!(h.count("user_time_zero"), 1);
            assert_eq!(h.count("map"), usize::from(failure == "map"));
        }
    }
    #[test]
    fn changed_workarea_requires_one_correction_and_full_confirmation_before_enable() {
        let mut h = minimized();
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.current = visible();
        h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
        h.area = Rect::new(200, 200, 460, 460);
        assert!(h.run(&mut op).is_none());
        assert_eq!(h.count("correct"), 1);
        assert_eq!(h.count("enable"), 0);
        let (_, confirmation) = op.show.correction.unwrap();
        h.now = confirmation - Duration::from_millis(1);
        assert!(h.run(&mut op).is_none());
        h.now = confirmation;
        assert_success(h.run(&mut op).unwrap(), true, true);
        assert_eq!(h.count("correct"), 1);
        assert_eq!(h.geometry.origin(), Point::new(200, 200));
    }
    #[test]
    fn insufficient_budget_never_begins_correction_or_enables_input() {
        let mut h = minimized();
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.current = visible();
        h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
        h.now = op.deadline - Duration::from_millis(149);
        assert_eq!(
            h.run(&mut op).unwrap().detail().reason,
            Reason::ConfirmationTimeout
        );
        assert_eq!(h.count("correct"), 0);
        assert_eq!(h.count("enable"), 0);
    }
    #[test]
    fn refused_correction_removed_monitor_and_fixed_size_change_remain_fatal() {
        for case in 0..3 {
            let mut h = minimized();
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            h.current = visible();
            match case {
                0 => {
                    h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
                    h.refuse_correction = true;
                    assert!(h.run(&mut op).is_none());
                    h.now = op.show.correction.unwrap().1;
                }
                1 => h.monitor = 99,
                _ => h.geometry.width = 161,
            }
            assert!(service_show_with(&mut op, &mut h).is_err());
            assert_eq!(h.count("enable"), 0);
            assert!(h.count("correct") <= 1);
        }
    }
    #[test]
    fn layout_drift_during_correction_invalidates_success_without_second_move() {
        let mut h = minimized();
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.current = visible();
        h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
        assert!(h.run(&mut op).is_none());
        h.area.x += 1;
        h.now = op.show.correction.unwrap().1;
        assert_eq!(
            h.run(&mut op).unwrap().detail().reason,
            Reason::LayoutChanged
        );
        assert_eq!(h.count("correct"), 1);
        assert_eq!(h.count("enable"), 0);
    }
    #[test]
    fn pending_local_placement_is_busy_before_show_effects() {
        let mut h = minimized();
        h.runtime.correction = Some((Point::new(100, 100), h.now + CONFIRMATION_TIMEOUT));
        let mut op = h.operation();
        let result = h.run(&mut op).unwrap();
        assert_eq!(result.status(), Status::Busy);
        assert_eq!(result.exit_code(), control::ExitCode::Busy);
        assert_eq!(h.count("fence"), 0);
        assert_eq!(h.count("map"), 0);
    }
    #[test]
    fn reenable_failure_never_reports_ready_and_retains_safe_fence() {
        let mut h = minimized();
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.current = visible();
        h.fail = Some("shape");
        assert!(service_show_with(&mut op, &mut h).is_err());
        assert!(h.runtime.input_fence.blocked);
        assert!(!h.runtime.input_fence.empty_confirmed);
        assert!(!op.detail.progress.input_ready);
        assert_eq!(h.count("map"), 1);
    }
    #[test]
    fn repeated_events_keep_single_deadline_and_final_expiry_observation_cannot_mutate() {
        let mut h = minimized();
        let mut op = h.operation();
        let deadline = op.deadline;
        assert!(h.run(&mut op).is_none());
        for _ in 0..62 {
            h.now += Duration::from_millis(40);
            op.show.observe = true;
            assert!(h.run(&mut op).is_none());
            assert_eq!(op.deadline, deadline);
        }
        h.now = deadline;
        let before = h.log.len();
        let result = h.run(&mut op).unwrap();
        assert_eq!(result.status(), Status::Failed);
        assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
        assert_eq!(&h.log[before..], ["observe"]);
        assert_eq!(h.count("map"), 1);
    }
    #[test]
    fn expiry_before_restore_admission_effects_is_observation_only() {
        let mut h = minimized();
        let mut op = h.operation();
        h.now = op.deadline;
        let result = h.run(&mut op).unwrap();
        assert_eq!(result.status(), Status::Failed);
        assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
        assert!(!result.detail().progress.host_mutation_dispatched);
        assert_eq!(op.show.phase, ShowPhase::Observe);
        assert_eq!(h.log, ["observe"]);
        assert_eq!(h.count("user_time_zero"), 0);
        assert_eq!(h.count("map"), 0);
    }
    #[test]
    fn expiry_between_user_time_and_map_never_claims_restore_request() {
        let mut h = minimized();
        let mut op = h.operation();
        h.advance = Some(("user_time_zero", op.deadline - h.now));
        let result = h.run(&mut op).unwrap();
        assert_eq!(result.status(), Status::Failed);
        assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
        assert!(result.detail().progress.host_mutation_dispatched);
        assert_eq!(op.show.phase, ShowPhase::RestoreAttempted);
        assert_eq!(h.count("user_time_zero"), 1);
        assert_eq!(h.count("map"), 0);
        assert_eq!(
            h.log,
            [
                "observe",
                "fence",
                "empty",
                "release",
                "observe",
                "user_time_zero",
                "observe"
            ]
        );
        assert!(h.runtime.input_fence.blocked);
        let before = h.log.len();
        op.show.observe = true;
        assert_eq!(h.run(&mut op).unwrap().status(), Status::Failed);
        assert_eq!(&h.log[before..], ["observe"]);
    }
    #[test]
    fn expiry_after_remove_hidden_can_only_succeed_from_fresh_elsewhere_evidence() {
        let mut h = Fake::new(reading(Visibility::Minimized, L::Below, 7, 2));
        let mut op = h.operation();
        h.advance = Some(("remove_hidden", op.deadline - h.now));
        h.reads = VecDeque::from([
            h.current,
            h.current,
            reading(Visibility::NotMinimizedElsewhere, L::Below, 7, 2),
        ]);
        assert_success(h.run(&mut op).unwrap(), false, true);
        assert_eq!(op.show.phase, ShowPhase::HiddenRemoveRequested);
        assert_eq!(h.count("remove_hidden"), 1);
        assert_eq!(h.count("user_time_zero"), 0);
        assert_eq!(h.count("map"), 0);
        assert_eq!(h.count("enable"), 0);
        assert_eq!(h.count("geometry"), 0);
    }
    #[test]
    fn slower_host_completes_map_correction_and_input_within_fixed_2500_ms() {
        assert_eq!(CONFIRMATION_TIMEOUT, Duration::from_millis(150));
        let mut h = minimized();
        let start = h.now;
        let mut op = h.operation();
        let deadline = start + Duration::from_millis(2500);
        assert_eq!(op.deadline, deadline);
        h.advance = Some(("user_time_zero", Duration::from_millis(1200)));
        assert!(h.run(&mut op).is_none());
        assert_eq!(h.now - start, Duration::from_millis(1200));
        assert_eq!(op.show.phase, ShowPhase::RestoreRequested);
        assert_eq!(h.count("map"), 1);
        h.now += Duration::from_millis(300); // WM response
        h.current = visible();
        h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
        h.advance = Some(("geometry", Duration::from_millis(300)));
        assert!(h.run(&mut op).is_none());
        let confirmation = op.show.correction.unwrap().1;
        assert_eq!(confirmation, start + Duration::from_millis(1950));
        h.now = confirmation;
        h.advance = Some(("enable", Duration::from_millis(300)));
        assert_success(h.run(&mut op).unwrap(), true, true);
        assert_eq!(h.now - start, Duration::from_millis(2250));
        assert_eq!(op.deadline, deadline);
        assert_eq!(op.show.phase, ShowPhase::InputRestored);
        assert_eq!(h.count("user_time_zero"), 1);
        assert_eq!(h.count("map"), 1);
        assert_eq!(h.count("correct"), 1);
        assert_eq!(h.count("enable"), 1);
        assert_eq!(h.log.last(), Some(&"observe"));
        assert!(h.runtime.accepts_input());
    }
    #[test]
    fn just_over_2500_ms_never_dispatches_map_correction_or_input_late() {
        for step in ["user_time_zero", "geometry"] {
            let mut h = minimized();
            let start = h.now;
            let mut op = h.operation();
            if step == "geometry" {
                assert!(h.run(&mut op).is_none());
                h.current = visible();
                h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
            }
            h.advance = Some((step, Duration::from_millis(2501)));
            let result = h.run(&mut op).unwrap();
            assert_eq!(h.now - start, Duration::from_millis(2501));
            assert_eq!(op.deadline, start + Duration::from_millis(2500));
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
            assert!(result.detail().progress.host_mutation_dispatched);
            assert_eq!(h.count("map"), usize::from(step == "geometry"));
            assert_eq!(h.count("correct"), 0);
            assert_eq!(h.count("enable"), 0);
            assert_eq!(h.log.last(), Some(&"observe"));
        }
    }
    #[test]
    fn expiry_during_user_time_or_geometry_uses_final_read_without_new_mutations() {
        for step in ["user_time_zero", "geometry"] {
            let mut h = minimized();
            let mut op = h.operation();
            if step == "geometry" {
                assert!(h.run(&mut op).is_none());
                h.current = visible();
            }
            h.advance = Some((step, op.deadline - h.now));
            let result = h.run(&mut op).unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
            assert_eq!(h.log.last(), Some(&"observe"));
            assert_eq!(h.count("enable"), 0);
            assert_eq!(h.count("correct"), 0);
            assert_eq!(h.count("map"), usize::from(step == "geometry"));
        }
    }
    #[test]
    fn final_read_at_expiry_can_confirm_existing_ready_noop_but_never_restore_input() {
        let mut h = Fake::new(visible());
        let mut op = h.operation();
        h.now = op.deadline;
        assert_success(h.run(&mut op).unwrap(), true, false);
        assert_eq!(h.count("enable"), 0);
        assert_eq!(h.count("fence"), 0);
        let mut h = minimized();
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.current = visible();
        h.now = op.deadline;
        assert_eq!(h.run(&mut op).unwrap().status(), Status::Failed);
        assert_eq!(h.count("enable"), 0);
    }
    #[test]
    fn buffered_traffic_over_64_events_cannot_starve_remote_deadline() {
        let mut h = minimized();
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        let mut events = VecDeque::from(vec![(); 400]);
        let mut count = 0;
        let mut final_result = None;
        while !events.is_empty() {
            let batch =
                drain_events_bounded(&mut None, 64, || Ok::<_, HostError>(events.pop_front()))
                    .unwrap();
            for _ in batch {
                h.now += Duration::from_millis(10);
                count += 1;
                if h.now >= op.deadline {
                    final_result = h.run(&mut op);
                    break;
                }
            }
            if final_result.is_some() {
                break;
            }
        }
        assert_eq!(count, 250);
        assert!(!events.is_empty());
        assert_eq!(final_result.unwrap().status(), Status::Failed);
        assert_eq!(h.count("map"), 1);
    }
    #[test]
    fn wm_delete_requires_body_protocols_format_and_protocol_atom() {
        let w = tests::test_window();
        let good = x11rb::protocol::xproto::ClientMessageEvent::new(
            32,
            w.window,
            w.wm_protocols,
            [w.wm_delete_window, 0, 0, 0, 0],
        );
        assert!(wm_delete_event(&good, &w));
        for case in 0..4 {
            let mut e = good;
            match case {
                0 => e.window += 1,
                1 => e.type_ += 1,
                2 => e.format = 8,
                _ => e.data = [w.wm_delete_window + 1, 0, 0, 0, 0].into(),
            }
            assert!(!wm_delete_event(&e, &w));
        }
    }
    #[test]
    fn completed_elsewhere_show_defers_then_ordinary_workspace_return_validates_before_input() {
        for was_minimized in [false, true] {
            let mut h = Fake::new(reading(
                if was_minimized {
                    Visibility::Minimized
                } else {
                    Visibility::NotMinimizedElsewhere
                },
                L::Normal,
                7,
                2,
            ));
            let mut op = h.operation();
            if was_minimized {
                assert!(h.run(&mut op).is_none());
                h.current = reading(Visibility::NotMinimizedElsewhere, L::Normal, 7, 2);
            }
            assert_success(h.run(&mut op).unwrap(), false, was_minimized);
            assert_eq!(h.count("remove_hidden"), usize::from(was_minimized));
            assert_eq!(h.runtime.pending_map_input, None);
            let maps = h.count("map");
            restore_deferred_input(&mut h).unwrap();
            assert_eq!(h.count("geometry"), 0);
            // This is a later ordinary workspace event, with no active remote record.
            h.current = reading(Visibility::NotMinimizedHere, L::Normal, 7, 7);
            h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
            restore_deferred_input(&mut h).unwrap();
            assert_eq!(h.count("ordinary_correct"), 1);
            assert_eq!(h.count("enable"), 0);
            restore_deferred_input(&mut h).unwrap();
            assert_eq!(h.count("ordinary_correct"), 1);
            let (_, deadline) = h.runtime.correction.unwrap();
            h.runtime
                .check_correction(deadline, h.geometry.origin(), &h.bounds)
                .unwrap();
            h.window.cancel_in_flight_moves();
            restore_deferred_input(&mut h).unwrap();
            assert_eq!(h.count("enable"), 1);
            assert!(h.runtime.accepts_input());
            assert_eq!(h.count("map"), maps);
            assert!(op.detail.progress.placement_deferred); // old terminal result is unchanged
            restore_deferred_input(&mut h).unwrap();
            assert_eq!(h.count("enable"), 1);
            assert_eq!(h.count("user_time_zero"), 0);
            assert_eq!(h.count("map"), 0);
            assert_eq!(h.count("remove_hidden"), usize::from(was_minimized));
        }
    }
    #[test]
    fn show_map_notify_cannot_consume_old_startup_allowance_or_enable_early() {
        let mut h = minimized();
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        let event = MapNotifyEvent {
            response_type: x11rb::protocol::xproto::MAP_NOTIFY_EVENT,
            event: h.window.window,
            window: h.window.window,
            ..Default::default()
        };
        h.runtime
            .synchronize_map_input_with(&event, h.window.window, 600, || {
                panic!("Show must stay fenced")
            })
            .unwrap();
        assert_eq!(h.runtime.pending_map_input, None);
        assert!(h.runtime.input_fence.blocked);
        h.current = visible();
        assert_success(h.run(&mut op).unwrap(), true, true);
        h.runtime
            .synchronize_map_input_with(&event, h.window.window, 4 * 65536 + 200, || {
                panic!("no second SHAPE allowance")
            })
            .unwrap();
        assert_eq!(h.runtime.input_fence.enabled_after, Some(4 * 65536 + 123));
    }
    #[test]
    fn layout_notifications_invalidate_show_without_visibility_or_geometry_retries() {
        let atoms = x11::monitors::LayoutAtoms {
            number_of_desktops: 90,
            desktop_viewport: 91,
            workarea: 10,
            current_desktop: 11,
            wm_desktop: 12,
        };
        let event = Event::PropertyNotify(x11rb::protocol::xproto::PropertyNotifyEvent {
            window: 1,
            atom: 10,
            ..Default::default()
        });
        assert!(visibility_layout_interruption(&event, 1, &atoms));
        let mut h = minimized();
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        op.show.layout_interrupted = true;
        h.current = visible();
        assert_eq!(
            h.run(&mut op).unwrap().detail().reason,
            Reason::LayoutChanged
        );
        assert_eq!(h.count("map"), 1);
        assert_eq!(h.count("enable"), 0);
        assert_eq!(h.count("correct"), 0);
    }
    #[test]
    fn untracked_transition_debt_blocks_even_off_workspace_completion() {
        let mut h = Fake::new(reading(Visibility::NotMinimizedElsewhere, L::Normal, 7, 2));
        h.runtime.input_fence.blocked = true;
        h.runtime.input_fence.empty_confirmed = true;
        h.runtime.pointer.require_transition_release();
        let mut op = h.operation();
        assert!(service_show_with(&mut op, &mut h).is_err());
        assert_eq!(op.detail.reason, Reason::ReleaseFailed);
        assert_eq!(h.count("map"), 0);
    }
    #[test]
    fn late_drift_after_enabling_is_failure_and_must_not_overstate_ready() {
        let mut h = minimized();
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        let mut changed = visible();
        changed.layer = L::Above;
        h.reads = VecDeque::from([visible(), visible(), changed]);
        let result = service_show_safely_with(&mut op, &mut h).unwrap().unwrap();
        assert_eq!(result.detail().reason, Reason::LayerChanged);
        assert_eq!(op.show.phase, ShowPhase::InputRestored);
        assert!(!result.detail().progress.input_ready);
        assert!(h.runtime.input_fence.blocked);
        assert_eq!(h.count("empty"), 2);
    }
    #[test]
    fn failed_enabling_attempt_re_fences_once_and_keeps_original_error() {
        let mut h = minimized();
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.current = visible();
        h.fail = Some("shape");
        let error = service_show_safely_with(&mut op, &mut h).unwrap_err();
        assert!(error.to_string().contains("shape"));
        assert!(h.runtime.input_fence.blocked);
        assert!(h.runtime.input_fence.empty_confirmed);
        assert!(h.runtime.visibility_dispatch_eligible());
        assert_eq!(h.count("empty"), 2);
        assert_eq!(h.count("enable"), 1);
        assert_eq!(h.count("map"), 1);
    }
    #[test]
    fn local_layer_busy_is_rejected_only_after_popup_and_pointer_cleanup() {
        let log = RefCell::new(vec![]);
        let mut r = visibility_fence_tests::runtime(3);
        let mut w = tests::test_window();
        let mut active = Some(minimized().operation());
        let desired = r.layers.desired;
        r.complete_menu_with(
            &mut w,
            MenuOutcome::SetLayer(Layer::Above),
            |p| {
                p.release_with(|| {
                    log.borrow_mut().push("release");
                    Ok(())
                })
                .map(|_| ())
            },
            |_| {
                log.borrow_mut().push("popup");
                Ok(())
            },
            |_, target| {
                local_layer_after_menu_with(active.is_some(), target, || {
                    log.borrow_mut().push("layer");
                    Ok(())
                })
            },
        )
        .unwrap();
        assert_eq!(*log.borrow(), ["release", "popup"]);
        assert!(r.menu.is_none());
        assert!(!r.pointer.is_grabbed());
        assert_eq!(r.layers.desired, desired);
        active.take();
        local_layer_after_menu_with(active.is_some(), Layer::Above, || {
            log.borrow_mut().push("layer");
            Ok(())
        })
        .unwrap();
        assert_eq!(*log.borrow(), ["release", "popup", "layer"]);
    }
    mod hide_tests {
        use super::*;
        use control::HidePhase;
        use x11rb::protocol::xproto::{ButtonPressEvent, ButtonReleaseEvent, MotionNotifyEvent};

        fn run(h: &mut Fake, op: &mut ActiveOperation) -> Option<control::OperationResult> {
            service_visibility_with(op, h).unwrap()
        }
        fn assert_hidden(r: control::OperationResult, sent: bool) {
            assert_eq!(r.status(), Status::Success);
            assert_eq!(r.exit_code(), control::ExitCode::Success);
            let d = r.detail();
            assert_eq!(d.visibility, Visibility::Minimized);
            assert_ne!(d.workspace, Workspace::Unknown);
            assert_eq!(d.stage, control::Stage::Hide);
            assert_eq!(d.progress.host_mutation_dispatched, sent);
            assert!(!d.progress.input_ready);
            assert!(!d.progress.not_minimized_confirmed);
            assert!(!d.progress.above_confirmed);
        }
        #[test]
        fn local_and_all_desktops_require_fresh_minimization_after_checked_dispatch() {
            for desktop in [2, u32::MAX] {
                let mut h = Fake::new(reading(Visibility::NotMinimizedHere, L::Below, desktop, 2));
                let mut op = h.command_operation(Command::Hide);
                assert!(run(&mut h, &mut op).is_none());
                assert_eq!(h.log, ["fence", "empty", "release", "observe", "iconify"]);
                assert_eq!(op.hide.phase, HidePhase::IconifyRequested);
                assert_eq!(h.count("add_hidden"), 0);
                assert!(run(&mut h, &mut op).is_none()); // acknowledgement is not success
                assert_eq!(h.count("iconify"), 1);
                h.current = reading(Visibility::Minimized, L::Below, desktop, 2);
                assert_hidden(run(&mut h, &mut op).unwrap(), true);
                assert!(h.runtime.input_fence.blocked);
                assert_eq!(h.count("map"), 0);
                assert_eq!(h.count("geometry"), 0);
                assert_eq!(h.count("enable"), 0);
            }
        }
        #[test]
        fn both_other_workspace_representations_must_request_and_confirm_hidden() {
            for normal in [false, true] {
                let mut r = reading(Visibility::NotMinimizedElsewhere, L::Above, 7, 2);
                if normal {
                    r.evidence.wm_state = WmState::Normal;
                    r.evidence.map_state = Evidence::Readable(MapState::Unviewable);
                }
                let mut h = Fake::new(r);
                let xid = h.window.window;
                let geometry = h.geometry;
                let mut op = h.command_operation(Command::Hide);
                assert!(run(&mut h, &mut op).is_none());
                assert!(run(&mut h, &mut op).is_none());
                assert_eq!(op.hide.phase, HidePhase::HiddenAddRequested);
                assert_eq!(h.count("add_hidden"), 1);
                assert_eq!(h.count("iconify"), 0);
                assert_eq!(h.count("map"), 0);
                assert_eq!(h.count("geometry"), 0);
                assert_eq!(h.count("correct"), 0);
                h.current = reading(Visibility::Minimized, L::Above, 7, 2);
                let result = run(&mut h, &mut op).unwrap();
                assert_hidden(result, true);
                assert_eq!(result.detail().workspace, Workspace::Other);
                assert_eq!((h.window.window, h.geometry), (xid, geometry));
            }
        }
        #[test]
        fn off_workspace_refusal_fails_without_placement_or_input_recovery() {
            let mut h = Fake::new(reading(Visibility::NotMinimizedElsewhere, L::Normal, 7, 2));
            let mut op = h.command_operation(Command::Hide);
            assert!(run(&mut h, &mut op).is_none());
            h.now = op.deadline;
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(result.exit_code(), control::ExitCode::OperationFailed);
            assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
            assert_eq!(
                result.detail().visibility,
                Visibility::NotMinimizedElsewhere
            );
            assert!(h.runtime.input_fence.blocked);
            assert_eq!(h.count("add_hidden"), 1);
            assert_eq!(h.count("iconify"), 0);
            assert_eq!(h.count("geometry"), 0);
        }
        #[test]
        fn already_minimized_still_fences_untracked_capture_and_never_enables_input() {
            let mut h = minimized();
            h.runtime.pointer.require_transition_release();
            let mut op = h.command_operation(Command::Hide);
            assert_hidden(run(&mut h, &mut op).unwrap(), false);
            assert_eq!(h.log, ["fence", "empty", "release", "observe", "observe"]);
            assert!(!h.runtime.pointer.is_grabbed());
            assert!(h.runtime.input_fence.blocked);
        }
        #[test]
        fn every_gesture_and_pending_correction_is_invalidated_before_iconification() {
            for case in 0..6 {
                let mut h = Fake::new(visible());
                h.runtime = visibility_fence_tests::runtime(case);
                if case == 5 {
                    h.runtime.interaction.handle_left_press(
                        Point::new(180, 180),
                        (80, 80),
                        100,
                        Point::new(100, 100),
                    );
                    h.runtime.interaction.on_grab_acquired();
                    h.runtime.pointer.set_grabbed(true);
                }
                h.runtime.pending_move = Some(Point::new(200, 200));
                h.runtime.final_target = Some(Point::new(210, 210));
                h.runtime.confirmation_deadline = Some(h.now + CONFIRMATION_TIMEOUT);
                h.runtime.correction = Some((Point::new(220, 220), h.now + CONFIRMATION_TIMEOUT));
                let generation = h.runtime.menu_generation;
                let mut op = h.command_operation(Command::Hide);
                assert!(run(&mut h, &mut op).is_none());
                assert!(h.runtime.visibility_dispatch_eligible());
                assert_eq!(h.runtime.menu_generation, generation + 1);
                assert_eq!(h.runtime.correction, None);
                assert!(h.window.in_flight_moves.is_empty());
                assert_eq!(h.count("iconify"), 1);
                assert_eq!(h.count("geometry"), 0);
                assert!(
                    h.log.iter().position(|s| *s == "release").unwrap()
                        < h.log.iter().position(|s| *s == "iconify").unwrap()
                );
                if case == 3 || case == 4 {
                    assert_eq!(h.count("popup"), 1);
                }
            }
        }
        #[test]
        fn fence_failures_attempt_cleanup_preserve_error_and_block_dispatch() {
            for fail in ["empty", "release", "popup"] {
                let mut h = minimized();
                h.runtime = visibility_fence_tests::runtime(4);
                h.fail = Some(fail);
                let mut op = h.command_operation(Command::Hide);
                let err = service_visibility_with(&mut op, &mut h).unwrap_err();
                assert_eq!(err.to_string(), format!("injected {fail}"));
                assert_eq!(h.log, ["fence", "empty", "release", "popup"]);
                assert!(!h.runtime.visibility_dispatch_eligible());
                assert_eq!(h.count("iconify"), 0);
                assert_eq!(h.count("enable"), 0);
                if fail == "release" {
                    assert!(h.runtime.pointer.is_grabbed());
                    assert!(h.runtime.menu.is_none());
                }
                if fail == "popup" {
                    assert!(h.runtime.menu.is_some());
                }
            }
        }
        #[test]
        fn missing_malformed_unsupported_and_contradictory_evidence_never_dispatches() {
            for case in 0..8 {
                let mut r = visible();
                match case {
                    0 => r.evidence.hidden = Evidence::Missing,
                    1 => r.evidence.hidden = Evidence::Malformed,
                    2 => r.evidence.capability = Capability::Unsupported,
                    3 => r.evidence.capability = Capability::Unverifiable,
                    4 => r.evidence.coherent = false,
                    5 => r.evidence.hidden = Evidence::Readable(true),
                    6 => r.evidence.workspace = Workspace::Unknown,
                    _ => r.evidence.wm_state = WmState::Malformed,
                }
                let mut h = Fake::new(r);
                let mut op = h.command_operation(Command::Hide);
                let initial = run(&mut h, &mut op);
                h.now = op.deadline;
                let result = initial.unwrap_or_else(|| run(&mut h, &mut op).unwrap());
                assert_eq!(result.status(), Status::Failed);
                assert_eq!(h.count("iconify"), 0);
                assert_eq!(h.count("enable"), 0);
                assert!(h.runtime.input_fence.blocked);
            }
        }
        #[test]
        fn transitional_evidence_waits_for_notification_then_dispatches_once() {
            let mut h = Fake::new(visible());
            h.current.evidence.coherent = false;
            let mut op = h.command_operation(Command::Hide);
            assert!(run(&mut h, &mut op).is_none());
            assert!(!remote_service_due(&op, h.now));
            h.current = visible();
            mark_remote_observation(&mut op, false);
            assert!(remote_service_due(&op, h.now));
            assert!(run(&mut h, &mut op).is_none());
            assert_eq!(h.count("iconify"), 1);
            assert_eq!(h.count("fence"), 1);
        }
        #[test]
        fn withdrawn_and_destroyed_fail_without_remanagement() {
            for (visibility, reason) in [
                (Visibility::Withdrawn, Reason::Withdrawn),
                (Visibility::Destroyed, Reason::BodyDestroyed),
            ] {
                let mut h = Fake::new(reading(visibility, L::Normal, 2, 2));
                let mut op = h.command_operation(Command::Hide);
                let result = run(&mut h, &mut op).unwrap();
                assert_eq!(result.status(), Status::Failed);
                assert_eq!(result.detail().reason, reason);
                assert_eq!(h.count("iconify"), 0);
                assert_eq!(h.count("map"), 0);
            }
        }
        #[test]
        fn checked_dispatch_error_retains_attempt_and_dispatch_without_retry() {
            let mut h = Fake::new(visible());
            h.fail = Some("iconify");
            let mut op = h.command_operation(Command::Hide);
            assert!(service_visibility_with(&mut op, &mut h).is_err());
            assert_eq!(op.hide.phase, HidePhase::IconifyAttempted);
            assert!(op.detail.progress.host_mutation_dispatched);
            h.fail = None;
            assert!(run(&mut h, &mut op).is_none());
            assert_eq!(h.count("iconify"), 1);
        }
        #[test]
        fn no_iconification_at_expiry_or_after_fence_or_observation_consumes_budget() {
            for step in ["admission", "fence", "observe"] {
                let mut h = Fake::new(visible());
                let mut op = h.command_operation(Command::Hide);
                if step == "admission" {
                    h.now = op.deadline;
                } else {
                    h.advance = Some((step, Duration::from_millis(1000)));
                }
                let result = run(&mut h, &mut op).unwrap();
                assert_eq!(result.status(), Status::Failed);
                assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
                assert!(!result.detail().progress.host_mutation_dispatched);
                assert_eq!(h.count("iconify"), 0);
            }
        }
        #[test]
        fn expiry_can_confirm_fresh_minimization_but_cannot_skip_input_obligations() {
            for settled in [false, true] {
                let mut h = Fake::new(visible());
                let mut op = h.command_operation(Command::Hide);
                if settled {
                    assert!(run(&mut h, &mut op).is_none());
                }
                h.current = reading(Visibility::Minimized, L::Normal, 2, 2);
                h.now = op.deadline;
                let result = run(&mut h, &mut op).unwrap();
                assert_eq!(
                    result.status(),
                    if settled {
                        Status::Success
                    } else {
                        Status::Failed
                    }
                );
                assert_eq!(h.count("iconify"), usize::from(settled));
                assert_eq!(h.count("enable"), 0);
            }
        }
        #[test]
        fn final_minimized_candidate_requires_another_coherent_read() {
            let mut h = minimized();
            h.reads = VecDeque::from([h.current, visible()]);
            let mut op = h.command_operation(Command::Hide);
            assert!(run(&mut h, &mut op).is_none());
            assert_eq!(h.count("iconify"), 0);
            h.now = op.deadline;
            assert_eq!(run(&mut h, &mut op).unwrap().status(), Status::Failed);
            assert!(h.runtime.accepts_input());
        }
        #[test]
        fn layer_workspace_and_layout_drift_fail_without_compensating_requests() {
            for case in 0..4 {
                let mut h = Fake::new(visible());
                let mut op = h.command_operation(Command::Hide);
                assert!(run(&mut h, &mut op).is_none());
                h.current = reading(Visibility::Minimized, L::Normal, 2, 2);
                match case {
                    0 => h.current.layer = L::Above,
                    1 => h.current.desktop = 3,
                    2 => h.current.current = 3,
                    _ => mark_remote_observation(&mut op, true),
                }
                let result = run(&mut h, &mut op).unwrap();
                assert_eq!(result.status(), Status::Failed);
                assert_eq!(
                    result.detail().reason,
                    if case == 0 {
                        Reason::LayerChanged
                    } else {
                        Reason::LayoutChanged
                    }
                );
                assert_eq!(h.count("iconify"), 1);
                assert_eq!(h.count("map"), 0);
                assert_eq!(h.count("geometry"), 0);
            }
        }
        #[test]
        fn local_refusal_restores_input_only_after_fresh_placement_and_keeps_failed_result() {
            let mut h = Fake::new(visible());
            h.runtime = visibility_fence_tests::runtime(1);
            let mut op = h.command_operation(Command::Hide);
            assert!(run(&mut h, &mut op).is_none());
            h.now = op.deadline;
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert!(!result.detail().progress.input_ready);
            assert!(h.runtime.accepts_input());
            assert_eq!(h.count("iconify"), 1);
            assert_eq!(h.count("enable"), 1);
            assert_eq!(h.count("geometry"), 2);
            assert!(!h.runtime.interaction.is_dragging());
            assert!(h.runtime.pending_move.is_none());
            assert_eq!(h.runtime.input_fence.enabled_after, Some(4 * 65536 + 123));
            for event in [
                Event::ButtonPress(ButtonPressEvent {
                    event: h.window.window,
                    ..Default::default()
                }),
                Event::ButtonRelease(ButtonReleaseEvent {
                    event: h.window.window,
                    ..Default::default()
                }),
                Event::MotionNotify(MotionNotifyEvent {
                    event: h.window.window,
                    ..Default::default()
                }),
            ] {
                assert!(h
                    .runtime
                    .input_fence
                    .rejects(&event, h.window.window, 70001));
            }
        }
        #[test]
        fn failure_recovery_defers_uncertain_minimized_and_off_workspace_until_local_return() {
            for case in 0..3 {
                let mut h = Fake::new(visible());
                let mut op = h.command_operation(Command::Hide);
                assert!(run(&mut h, &mut op).is_none());
                op.hide.layout_interrupted = true;
                h.current = match case {
                    0 => reading(Visibility::Minimized, L::Normal, 2, 2),
                    1 => reading(Visibility::NotMinimizedElsewhere, L::Normal, 2, 7),
                    _ => {
                        let mut r = visible();
                        r.evidence.coherent = false;
                        r
                    }
                };
                let result = run(&mut h, &mut op).unwrap();
                assert_eq!(result.status(), Status::Failed);
                assert!(h.runtime.input_fence.blocked);
                assert_eq!(h.count("geometry"), 0);
                h.current = visible();
                restore_deferred_input(&mut h).unwrap();
                assert!(h.runtime.accepts_input());
                assert_eq!(h.count("iconify"), 1);
                assert_eq!(result.status(), Status::Failed);
            }
        }
        #[test]
        fn failed_hide_reuses_bounded_deferred_correction_before_enabling_input() {
            let mut h = Fake::new(visible());
            let mut op = h.command_operation(Command::Hide);
            assert!(run(&mut h, &mut op).is_none());
            h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
            h.now = op.deadline;
            assert_eq!(run(&mut h, &mut op).unwrap().status(), Status::Failed);
            assert_eq!(h.count("ordinary_correct"), 1);
            assert_eq!(h.count("enable"), 0);
            let (_, deadline) = h.runtime.correction.unwrap();
            h.runtime
                .check_correction(deadline, h.geometry.origin(), &h.bounds)
                .unwrap();
            h.window.cancel_in_flight_moves();
            restore_deferred_input(&mut h).unwrap();
            assert!(h.runtime.accepts_input());
            assert_eq!(h.count("ordinary_correct"), 1);
            assert_eq!(h.count("iconify"), 1);
        }
        #[test]
        fn failed_recovery_shape_is_refenced_with_original_fatal_error() {
            let mut h = Fake::new(visible());
            let mut op = h.command_operation(Command::Hide);
            assert!(run(&mut h, &mut op).is_none());
            h.fail = Some("shape");
            h.now = op.deadline;
            let err = service_visibility_with(&mut op, &mut h).unwrap_err();
            assert!(err.to_string().contains("shape"));
            assert!(h.runtime.visibility_dispatch_eligible());
            assert!(h.runtime.input_fence.blocked);
            assert_eq!(h.count("empty"), 2);
            assert_eq!(h.count("iconify"), 1);
        }
        #[test]
        fn recovery_rechecks_visibility_after_placement_before_enabling() {
            let mut h = Fake::new(visible());
            let mut op = h.command_operation(Command::Hide);
            assert!(run(&mut h, &mut op).is_none());
            h.now = op.deadline;
            // Terminal failure, local recovery candidate, then unavailable again
            // after geometry reads. No silhouette may be enabled from the old read.
            h.reads = VecDeque::from([
                visible(),
                visible(),
                reading(Visibility::Minimized, L::Normal, 2, 2),
            ]);
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert!(h.runtime.input_fence.blocked);
            assert_eq!(h.count("enable"), 0);
            h.current = visible();
            restore_deferred_input(&mut h).unwrap();
            assert!(h.runtime.accepts_input());
            assert_eq!(h.count("iconify"), 1);
        }
        #[test]
        fn invalid_retained_monitor_or_resized_body_never_recovers_input() {
            for resized in [false, true] {
                let mut h = Fake::new(visible());
                let mut op = h.command_operation(Command::Hide);
                assert!(run(&mut h, &mut op).is_none());
                if resized {
                    h.geometry.width += 1;
                } else {
                    h.monitor += 1;
                }
                h.now = op.deadline;
                assert!(service_visibility_with(&mut op, &mut h).is_err());
                assert!(h.runtime.input_fence.blocked);
                assert_eq!(op.detail.reason, Reason::X11Failure);
                assert_eq!(h.count("iconify"), 1);
            }
        }
        #[test]
        fn repeated_hide_show_cycles_preserve_body_workspace_layer_and_fresh_input() {
            for layer in [L::Normal, L::Above, L::Below] {
                let mut h = Fake::new(reading(Visibility::NotMinimizedHere, layer, 2, 2));
                let xid = h.window.window;
                for _ in 0..20 {
                    let mut hide = h.command_operation(Command::Hide);
                    assert!(run(&mut h, &mut hide).is_none());
                    h.current = reading(Visibility::Minimized, layer, 2, 2);
                    assert_hidden(run(&mut h, &mut hide).unwrap(), true);
                    let mut noop = h.command_operation(Command::Hide);
                    assert_hidden(run(&mut h, &mut noop).unwrap(), false);
                    let mut show = h.operation();
                    assert!(run(&mut h, &mut show).is_none());
                    assert!(h.runtime.input_fence.blocked);
                    h.current = reading(Visibility::NotMinimizedHere, layer, 2, 2);
                    assert_success(run(&mut h, &mut show).unwrap(), true, true);
                    assert!(h.runtime.accepts_input());
                    assert_eq!(h.window.window, xid);
                    assert!(h.runtime.pending_move.is_none());
                    assert!(h.runtime.final_target.is_none());
                    assert_eq!(h.current.layer, layer);
                    assert_eq!(h.current.desktop, 2);
                }
                assert_eq!(h.count("iconify"), 20);
                assert_eq!(h.count("map"), 20);
                assert_eq!(h.count("correct"), 0);
            }
        }

        #[test]
        fn repeated_off_workspace_cycles_preserve_xid_layer_workspaces_and_fence() {
            for layer in [L::Normal, L::Above, L::Below] {
                let mut h = Fake::new(reading(Visibility::NotMinimizedElsewhere, layer, 7, 2));
                let xid = h.window.window;
                let geometry = h.geometry;
                for _ in 0..20 {
                    let mut hide = h.command_operation(Command::Hide);
                    assert!(run(&mut h, &mut hide).is_none());
                    h.current = reading(Visibility::Minimized, layer, 7, 2);
                    assert_hidden(run(&mut h, &mut hide).unwrap(), true);
                    let mut noop_hide = h.command_operation(Command::Hide);
                    assert_hidden(run(&mut h, &mut noop_hide).unwrap(), false);
                    let mut show = h.operation();
                    assert!(run(&mut h, &mut show).is_none());
                    h.current = reading(Visibility::NotMinimizedElsewhere, layer, 7, 2);
                    assert_success(run(&mut h, &mut show).unwrap(), false, true);
                    let mut noop_show = h.operation();
                    assert_success(run(&mut h, &mut noop_show).unwrap(), false, false);
                    assert_eq!((h.window.window, h.geometry), (xid, geometry));
                    assert_eq!(
                        (h.current.desktop, h.current.current, h.current.layer),
                        (7, 2, layer)
                    );
                    assert!(h.runtime.input_fence.blocked);
                    assert!(!h.runtime.accepts_input());
                    assert!(!h.runtime.pointer.is_grabbed());
                    assert!(h.runtime.menu.is_none());
                    assert!(h.runtime.pending_move.is_none());
                    assert!(h.runtime.final_target.is_none());
                    assert_eq!(h.runtime.pending_map_input, None);
                }
                assert_eq!(h.count("add_hidden"), 20);
                assert_eq!(h.count("remove_hidden"), 20);
                for step in [
                    "iconify",
                    "user_time_zero",
                    "map",
                    "geometry",
                    "correct",
                    "enable",
                ] {
                    assert_eq!(h.count(step), 0);
                }
            }
        }

        #[test]
        fn local_hide_then_manual_workspace_change_selects_remote_show() {
            let mut h = Fake::new(visible());
            let xid = h.window.window;
            let mut hide = h.command_operation(Command::Hide);
            assert!(run(&mut h, &mut hide).is_none());
            h.current = reading(Visibility::Minimized, L::Normal, 2, 2);
            assert_hidden(run(&mut h, &mut hide).unwrap(), true);
            // The owner has completed Hide. The user changes workspace before
            // admitting a new Show; its fresh baseline must select REMOVE-HIDDEN.
            h.current = reading(Visibility::Minimized, L::Normal, 2, 7);
            let mut show = h.operation();
            assert!(run(&mut h, &mut show).is_none());
            h.current = reading(Visibility::NotMinimizedElsewhere, L::Normal, 2, 7);
            assert_success(run(&mut h, &mut show).unwrap(), false, true);
            assert_eq!(h.count("iconify"), 1);
            assert_eq!(h.count("remove_hidden"), 1);
            assert_eq!(h.count("add_hidden"), 0);
            assert_eq!(h.window.window, xid);
            assert_eq!((h.current.desktop, h.current.current), (2, 7));
            for step in ["user_time_zero", "map", "geometry", "enable"] {
                assert_eq!(h.count(step), 0);
            }
        }
    }
}
