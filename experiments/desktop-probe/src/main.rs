mod cli;
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

#[cfg(test)]
use crate::cli::HELP;
use crate::cli::{parse_args, print_help, CliMode};
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
#[path = "probe_tests/here_service_tests.rs"]
mod here_service_tests;
