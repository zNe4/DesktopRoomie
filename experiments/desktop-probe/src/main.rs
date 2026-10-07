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

    let layout = query_desktop_layout(
        &conn,
        screen.root,
        screen.width_in_pixels,
        screen.height_in_pixels,
    );

    // If pure diagnosis requested, print diagnostics and exit cleanly (M01.1 mode)
    if config.diagnose {
        run_diagnostics(&conn, screen, &layout);
        println!("\n[Result]");
        println!("  M01.1: X11 connection established; diagnostics completed.");
        return;
    }

    // M02.3: Bounded dragging and stable grab offsets
    println!("Mission M02.4: WM confirmation and event efficiency...");

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
    let valid_bounds = match compute_valid_origin_bounds(layout.usable_area, body_size) {
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
            let _ = probe_window.destroy(&conn);
            process::exit(1);
        }
    };

    // Perform initial paint (fatal on failure)
    if let Err(e) = renderer.paint(&conn, probe_window.window) {
        eprintln!("[ERROR] Initial paint failed: {}", e);
        let _ = renderer.destroy(&conn);
        let _ = probe_window.destroy(&conn);
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

    // 5. Event loop
    let conn_fd = conn.stream().as_raw_fd();
    let start_time = Instant::now();
    let mut running = true;
    let mut mapped_logged = false;
    let mut interaction = InteractionManager::new();
    let mut pointer_tracker = PointerCaptureTracker::new();
    let mut pending_move: Option<Point> = None;
    let mut buffered_event: Option<Event> = None;
    let mut awaiting_final_confirmation: Option<Point> = None;
    let mut confirmation_deadline: Option<Instant> = None;

    const MAX_EVENT_BATCH: usize = 64;

    while running {
        // Check duration deadline at start of each iteration
        if let Some(sec) = config.duration_secs {
            let duration = Duration::from_secs(sec);
            let elapsed = start_time.elapsed();
            if elapsed >= duration {
                println!("  Duration limit reached ({}s). Exiting cleanly.", sec);
                if let Err(e) = pointer_tracker.release_if_held(&conn, x11rb::CURRENT_TIME) {
                    fatal_host_error(
                        &format!("Failed to release pointer capture on timeout: {}", e),
                        &conn,
                        &mut pointer_tracker,
                        &renderer,
                        &probe_window,
                    );
                }
                break;
            }
        }

        // Check expired confirmation deadlines alongside duration deadline, before processing another batch
        check_confirmation_timeout(
            &conn,
            screen.root,
            &mut confirmation_deadline,
            &mut probe_window,
            &mut awaiting_final_confirmation,
            &mut interaction,
            &mut pointer_tracker,
            &renderer,
            &mut pending_move,
        );

        // 1. Drain a bounded batch of available X11 events
        let event_batch = match drain_events_bounded(&mut buffered_event, MAX_EVENT_BATCH, || {
            conn.poll_for_event()
        }) {
            Ok(b) => b,
            Err(e) => {
                fatal_host_error(
                    &format!("X11 connection poll error: {}", e),
                    &conn,
                    &mut pointer_tracker,
                    &renderer,
                    &probe_window,
                );
            }
        };

        // 2. Process drained events
        for event in event_batch {
            match event {
                Event::MotionNotify(ev) if ev.event == probe_window.window => {
                    let pointer_root = Point::new(ev.root_x as i32, ev.root_y as i32);
                    let was_dragging = interaction.is_dragging();
                    if let HostAction::MoveWindow { target } =
                        interaction.handle_motion(pointer_root, ev.time, &valid_bounds)
                    {
                        if !was_dragging {
                            println!(
                                "[INPUT] Drag started at root ({}, {}), target origin {}",
                                ev.root_x, ev.root_y, target
                            );
                        }
                        pending_move = Some(target);
                    }
                }
                other_event => {
                    // Flush pending move before handling any non-motion event
                    flush_pending_move(
                        &mut pending_move,
                        &mut probe_window,
                        &conn,
                        &mut pointer_tracker,
                        &renderer,
                        &mut confirmation_deadline,
                    );

                    match other_event {
                        Event::MapNotify(ev) if ev.window == probe_window.window => {
                            if !mapped_logged {
                                mapped_logged = true;
                                match probe_window.query_actual_root_geometry(&conn, screen.root) {
                                    Ok(actual_geometry) => {
                                        println!("\n[Placement Verification (MapNotify)]");
                                        println!(
                                            "  Startup placement: requested={}, actual={}",
                                            requested_origin,
                                            actual_geometry.origin()
                                        );
                                        println!(
                                            "  Actual mapped geometry: {} (size: {})",
                                            actual_geometry,
                                            actual_geometry.size()
                                        );
                                    }
                                    Err(e) => {
                                        fatal_host_error(
                                            &format!(
                                                "Failed to query actual root geometry on MapNotify: {}",
                                                e
                                            ),
                                            &conn,
                                            &mut pointer_tracker,
                                            &renderer,
                                            &probe_window,
                                        );
                                    }
                                }
                            }
                        }
                        Event::ConfigureNotify(ev) if ev.window == probe_window.window => {
                            match probe_window.handle_configure_notify(&conn, screen.root, &ev) {
                                Ok(ConfigureReconciliation::Confirmed { origin, .. }) => {
                                    if let Some(expected_final) = awaiting_final_confirmation {
                                        if origin == expected_final {
                                            awaiting_final_confirmation = None;
                                            println!(
                                                "[INPUT] Final placement confirmed by WM at {}",
                                                origin
                                            );
                                        } else if probe_window.in_flight_moves.is_empty() {
                                            eprintln!(
                                                "[WARN] Final placement adjusted by WM to {} (expected {}); cancelling interaction.",
                                                origin, expected_final
                                            );
                                            awaiting_final_confirmation = None;
                                            if let HostAction::ReleaseGrab { time } = interaction
                                                .cancel_on_wm_mismatch(x11rb::CURRENT_TIME)
                                            {
                                                if let Err(e) =
                                                    pointer_tracker.release_if_held(&conn, time)
                                                {
                                                    fatal_host_error(
                                                        &format!(
                                                            "Failed to release pointer capture on mismatch cancellation: {}",
                                                            e
                                                        ),
                                                        &conn,
                                                        &mut pointer_tracker,
                                                        &renderer,
                                                        &probe_window,
                                                    );
                                                }
                                            }
                                            pending_move = None;
                                            probe_window.cancel_in_flight_moves();
                                        }
                                    }
                                    if probe_window.in_flight_moves.is_empty()
                                        && awaiting_final_confirmation.is_none()
                                    {
                                        confirmation_deadline = None;
                                    }
                                }
                                Ok(ConfigureReconciliation::InFlightCatchUp { .. }) => {
                                    confirmation_deadline =
                                        Some(Instant::now() + CONFIRMATION_TIMEOUT);
                                }
                                Ok(ConfigureReconciliation::StaleHistorical { .. }) => {}
                                Ok(ConfigureReconciliation::GenuineMismatch {
                                    requested,
                                    confirmed,
                                }) => {
                                    eprintln!(
                                        "[WARN] Window manager refused or adjusted requested movement: requested {}, confirmed {}; cancelling interaction.",
                                        requested, confirmed
                                    );
                                    if let Some(expected_final) = awaiting_final_confirmation.take()
                                    {
                                        eprintln!(
                                            "[WARN] Final placement adjusted by WM to {} (expected {})",
                                            confirmed, expected_final
                                        );
                                    }
                                    if let HostAction::ReleaseGrab { time } =
                                        interaction.cancel_on_wm_mismatch(x11rb::CURRENT_TIME)
                                    {
                                        if let Err(e) = pointer_tracker.release_if_held(&conn, time)
                                        {
                                            fatal_host_error(
                                                &format!(
                                                    "Failed to release pointer capture on mismatch cancellation: {}",
                                                    e
                                                ),
                                                &conn,
                                                &mut pointer_tracker,
                                                &renderer,
                                                &probe_window,
                                            );
                                        }
                                    }
                                    pending_move = None;
                                    probe_window.cancel_in_flight_moves();
                                    confirmation_deadline = None;
                                }
                                Ok(ConfigureReconciliation::UnexpectedSizeChange {
                                    expected,
                                    actual,
                                }) => {
                                    fatal_host_error(
                                        &format!(
                                            "Window manager altered probe dimensions unexpectedly: expected {}, received {}",
                                            expected, actual
                                        ),
                                        &conn,
                                        &mut pointer_tracker,
                                        &renderer,
                                        &probe_window,
                                    );
                                }
                                Err(e) => {
                                    fatal_host_error(
                                        &format!("Failed to reconcile ConfigureNotify: {}", e),
                                        &conn,
                                        &mut pointer_tracker,
                                        &renderer,
                                        &probe_window,
                                    );
                                }
                            }
                        }
                        Event::Expose(exp) if exp.window == probe_window.window => {
                            // Blit only on exp.count == 0 (final sub-rectangle of composite exposure)
                            if exp.count == 0 {
                                if let Err(e) = renderer.paint(&conn, probe_window.window) {
                                    fatal_host_error(
                                        &format!("Repaint on Expose failed: {}", e),
                                        &conn,
                                        &mut pointer_tracker,
                                        &renderer,
                                        &probe_window,
                                    );
                                }
                            }
                        }
                        Event::EnterNotify(ev) if ev.event == probe_window.window => {
                            println!(
                                "[INPUT] Pointer entered body at ({}, {})",
                                ev.event_x, ev.event_y
                            );
                        }
                        Event::LeaveNotify(ev) if ev.event == probe_window.window => {
                            println!(
                                "[INPUT] Pointer left body at ({}, {})",
                                ev.event_x, ev.event_y
                            );
                        }
                        Event::ButtonPress(ev) if ev.event == probe_window.window => {
                            match ev.detail {
                                1 => {
                                    let pointer_root =
                                        Point::new(ev.root_x as i32, ev.root_y as i32);
                                    let local = (ev.event_x, ev.event_y);
                                    let actual_origin = match probe_window
                                        .query_actual_root_origin(&conn, screen.root)
                                    {
                                        Ok(origin) => origin,
                                        Err(e) => {
                                            fatal_host_error(
                                                &format!(
                                                    "Failed to query actual window origin on press: {}",
                                                    e
                                                ),
                                                &conn,
                                                &mut pointer_tracker,
                                                &renderer,
                                                &probe_window,
                                            );
                                        }
                                    };

                                    match interaction.handle_left_press(
                                        pointer_root,
                                        local,
                                        ev.time,
                                        actual_origin,
                                    ) {
                                        HostAction::AcquireGrab { time } => {
                                            match grab_pointer(&conn, probe_window.window, time) {
                                                Ok(GrabStatus::SUCCESS) => {
                                                    pointer_tracker.set_grabbed(true);
                                                    interaction.on_grab_acquired();
                                                    awaiting_final_confirmation = None;
                                                    println!(
                                                        "[INPUT] ButtonPress: button=1 (Left) at ({}, {}) -> pointer capture acquired",
                                                        ev.event_x, ev.event_y
                                                    );
                                                }
                                                Ok(status) => {
                                                    interaction.on_grab_denied();
                                                    println!(
                                                        "[INPUT] ButtonPress: button=1 (Left) -> pointer capture denied ({:?}), recovered to Idle",
                                                        status
                                                    );
                                                }
                                                Err(e) => {
                                                    fatal_host_error(
                                                        &format!(
                                                            "GrabPointer request failed: {}",
                                                            e
                                                        ),
                                                        &conn,
                                                        &mut pointer_tracker,
                                                        &renderer,
                                                        &probe_window,
                                                    );
                                                }
                                            }
                                        }
                                        _ => {
                                            println!(
                                                "[INPUT] ButtonPress: button=1 (Left) at ({}, {}) (outside interactive shape)",
                                                ev.event_x, ev.event_y
                                            );
                                        }
                                    }
                                }
                                3 => match interaction.handle_right_press(ev.time) {
                                    HostAction::ReleaseGrab { time } => {
                                        if let Err(e) = pointer_tracker.release_if_held(&conn, time)
                                        {
                                            fatal_host_error(
                                                &format!(
                                                    "Failed to release pointer capture on cancel: {}",
                                                    e
                                                ),
                                                &conn,
                                                &mut pointer_tracker,
                                                &renderer,
                                                &probe_window,
                                            );
                                        }
                                        pending_move = None;
                                        probe_window.cancel_in_flight_moves();
                                        confirmation_deadline = None;
                                        awaiting_final_confirmation = None;
                                        println!(
                                            "[INPUT] ButtonPress: button=3 (Right) -> cancelled active gesture and released capture"
                                        );
                                    }
                                    HostAction::ExitCleanly => {
                                        println!(
                                            "[INPUT] ButtonPress: button=3 (Right) on body at ({}, {}) -> clean exit requested.",
                                            ev.event_x, ev.event_y
                                        );
                                        running = false;
                                        break;
                                    }
                                    _ => {}
                                },
                                other => {
                                    interaction.handle_other_button_press(other, ev.time);
                                    println!(
                                        "[INPUT] ButtonPress: button={} at ({}, {}) (ignored)",
                                        other, ev.event_x, ev.event_y
                                    );
                                }
                            }
                        }
                        Event::ButtonRelease(ev) if ev.event == probe_window.window => {
                            match ev.detail {
                                1 => {
                                    let pointer_root =
                                        Point::new(ev.root_x as i32, ev.root_y as i32);
                                    let local = (ev.event_x, ev.event_y);
                                    match interaction.handle_left_release(
                                        pointer_root,
                                        local,
                                        ev.time,
                                        &valid_bounds,
                                    ) {
                                        HostAction::ReleaseGrabAndMoveWindow { time, target } => {
                                            if target != probe_window.requested_origin() {
                                                if let Err(e) =
                                                    probe_window.configure_position(&conn, target)
                                                {
                                                    fatal_host_error(
                                                        &format!(
                                                            "Failed to finalize window position at {}: {}",
                                                            target, e
                                                        ),
                                                        &conn,
                                                        &mut pointer_tracker,
                                                        &renderer,
                                                        &probe_window,
                                                    );
                                                }
                                            }
                                            if let Err(e) =
                                                pointer_tracker.release_if_held(&conn, time)
                                            {
                                                fatal_host_error(
                                                    &format!(
                                                        "Failed to release pointer capture on drag completion: {}",
                                                        e
                                                    ),
                                                    &conn,
                                                    &mut pointer_tracker,
                                                    &renderer,
                                                    &probe_window,
                                                );
                                            }
                                            if probe_window.in_flight_moves.is_empty()
                                                && probe_window.confirmed_origin() == target
                                            {
                                                awaiting_final_confirmation = None;
                                                confirmation_deadline = None;
                                                println!(
                                                    "[INPUT] Drag completed at root ({}, {}), target origin {} (already confirmed by WM)",
                                                    ev.root_x, ev.root_y, target
                                                );
                                            } else {
                                                awaiting_final_confirmation = Some(target);
                                                confirmation_deadline =
                                                    Some(Instant::now() + CONFIRMATION_TIMEOUT);
                                                println!(
                                                    "[INPUT] Drag completed at root ({}, {}), requested origin {}; awaiting WM confirmation",
                                                    ev.root_x, ev.root_y, target
                                                );
                                            }
                                        }
                                        HostAction::ReleaseGrabAndToggleColor { time } => {
                                            if let Err(e) =
                                                pointer_tracker.release_if_held(&conn, time)
                                            {
                                                fatal_host_error(
                                                    &format!(
                                                        "Failed to release pointer capture on click release: {}",
                                                        e
                                                    ),
                                                    &conn,
                                                    &mut pointer_tracker,
                                                    &renderer,
                                                    &probe_window,
                                                );
                                            }
                                            match renderer.toggle_color(&conn, probe_window.window)
                                            {
                                                Ok(theme) => {
                                                    println!(
                                                        "[INPUT] ButtonRelease: button=1 (Left) at ({}, {}) on shape -> completed click, toggled body color to {}",
                                                        ev.event_x,
                                                        ev.event_y,
                                                        theme.name()
                                                    );
                                                }
                                                Err(e) => {
                                                    fatal_host_error(
                                                        &format!(
                                                            "Color toggle on ButtonRelease failed: {}",
                                                            e
                                                        ),
                                                        &conn,
                                                        &mut pointer_tracker,
                                                        &renderer,
                                                        &probe_window,
                                                    );
                                                }
                                            }
                                        }
                                        HostAction::ReleaseGrab { time } => {
                                            if let Err(e) =
                                                pointer_tracker.release_if_held(&conn, time)
                                            {
                                                fatal_host_error(
                                                    &format!(
                                                        "Failed to release pointer capture on click cancellation: {}",
                                                        e
                                                    ),
                                                    &conn,
                                                    &mut pointer_tracker,
                                                    &renderer,
                                                    &probe_window,
                                                );
                                            }
                                            println!(
                                                "[INPUT] ButtonRelease: button=1 (Left) at ({}, {}) outside shape -> click cancelled without toggle",
                                                ev.event_x, ev.event_y
                                            );
                                        }
                                        HostAction::None => {
                                            println!(
                                                "[INPUT] ButtonRelease: button=1 (Left) at ({}, {}) (stale/suppressed)",
                                                ev.event_x, ev.event_y
                                            );
                                        }
                                        _ => {}
                                    }
                                }
                                other => {
                                    interaction.handle_other_button_release(other, ev.time);
                                    println!(
                                        "[INPUT] ButtonRelease: button={} at ({}, {}) (ignored)",
                                        other, ev.event_x, ev.event_y
                                    );
                                }
                            }
                        }
                        Event::ClientMessage(msg) if msg.window == probe_window.window => {
                            let data = msg.data.as_data32();
                            if data[0] == probe_window.wm_delete_window {
                                println!("  Received WM_DELETE_WINDOW. Exiting cleanly.");
                                if let Err(e) =
                                    pointer_tracker.release_if_held(&conn, x11rb::CURRENT_TIME)
                                {
                                    fatal_host_error(
                                        &format!(
                                            "Failed to release pointer capture on WM_DELETE_WINDOW: {}",
                                            e
                                        ),
                                        &conn,
                                        &mut pointer_tracker,
                                        &renderer,
                                        &probe_window,
                                    );
                                }
                                running = false;
                                break;
                            }
                        }
                        Event::Error(xerr) => {
                            fatal_host_error(
                                &format!("Asynchronous X11 protocol error received: {:?}", xerr),
                                &conn,
                                &mut pointer_tracker,
                                &renderer,
                                &probe_window,
                            );
                        }
                        _ => {}
                    }
                }
            }
            if !running {
                break;
            }
        }

        if !running {
            break;
        }

        // Flush any remaining pending move before sleeping
        flush_pending_move(
            &mut pending_move,
            &mut probe_window,
            &conn,
            &mut pointer_tracker,
            &renderer,
            &mut confirmation_deadline,
        );

        if !running {
            break;
        }

        // Flush any pending requests before waiting
        if let Err(e) = conn.flush() {
            fatal_host_error(
                &format!("Failed to flush X11 connection: {}", e),
                &conn,
                &mut pointer_tracker,
                &renderer,
                &probe_window,
            );
        }

        // Before sleeping on the socket, verify no events remain buffered in x11rb
        // (which can occur if the drained batch reached MAX_EVENT_BATCH or if
        // synchronous reply() calls received and queued incoming server events).
        if buffered_event.is_some() {
            continue;
        }
        match conn.poll_for_event() {
            Ok(Some(ev)) => {
                buffered_event = Some(ev);
                continue;
            }
            Ok(None) => {}
            Err(e) => {
                fatal_host_error(
                    &format!("X11 connection poll error: {}", e),
                    &conn,
                    &mut pointer_tracker,
                    &renderer,
                    &probe_window,
                );
            }
        }

        // Check if movement confirmation deadline has expired before blocking
        check_confirmation_timeout(
            &conn,
            screen.root,
            &mut confirmation_deadline,
            &mut probe_window,
            &mut awaiting_final_confirmation,
            &mut interaction,
            &mut pointer_tracker,
            &renderer,
            &mut pending_move,
        );

        // Compute timeout until duration limit or confirmation deadline
        let mut timeout_ms = -1;
        let now = Instant::now();

        if let Some(sec) = config.duration_secs {
            let duration = Duration::from_secs(sec);
            let elapsed = start_time.elapsed();
            if elapsed >= duration {
                continue; // Loop top will perform clean shutdown
            }
            let remaining = duration - elapsed;
            let ms = remaining.as_millis().min(i32::MAX as u128) as i32;
            timeout_ms = if timeout_ms == -1 {
                ms
            } else {
                timeout_ms.min(ms)
            };
        }

        if let Some(dl) = confirmation_deadline {
            if now >= dl {
                continue; // Loop around to process expired confirmation deadline immediately
            }
            let remaining = dl - now;
            let ms = remaining.as_millis().min(i32::MAX as u128) as i32;
            timeout_ms = if timeout_ms == -1 {
                ms
            } else {
                timeout_ms.min(ms)
            };
        }

        let mut pfd = PollFd {
            fd: conn_fd,
            events: POLLIN,
            revents: 0,
        };

        let ret = unsafe { poll(&mut pfd as *mut _, 1, timeout_ms) };
        if ret < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::Interrupted {
                fatal_host_error(
                    &format!("Socket poll error: {}", err),
                    &conn,
                    &mut pointer_tracker,
                    &renderer,
                    &probe_window,
                );
            }
        }
    }

    // 6. Cleanup after clean exit
    println!("\n[Cleanup]");
    let mut cleanup_failed = false;
    if let Err(e) = pointer_tracker.release_if_held(&conn, x11rb::CURRENT_TIME) {
        eprintln!("[ERROR] Failed to release pointer capture: {}", e);
        cleanup_failed = true;
    }
    if let Err(e) = renderer.destroy(&conn) {
        eprintln!("[ERROR] Failed to destroy renderer resources: {}", e);
        cleanup_failed = true;
    }
    if let Err(e) = probe_window.destroy(&conn) {
        eprintln!("[ERROR] Failed to destroy window resources: {}", e);
        cleanup_failed = true;
    }

    if cleanup_failed {
        eprintln!("[ERROR] Server-side resource cleanup failed.");
        process::exit(1);
    }

    println!("  Resources released cleanly.");
    println!("  Probe exited cleanly.");
}

/// Drains available X11 events into a bounded batch, prepending any previously buffered event.
pub fn drain_events_bounded<E, F>(
    buffered_event: &mut Option<Event>,
    max_batch: usize,
    mut poll_fn: F,
) -> Result<Vec<Event>, E>
where
    F: FnMut() -> Result<Option<Event>, E>,
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
fn check_confirmation_timeout_with<F, R>(
    confirmation_deadline: &mut Option<Instant>,
    probe_window: &mut ManagedProbeWindow,
    awaiting_final_confirmation: &mut Option<Point>,
    interaction: &mut InteractionManager,
    pending_move: &mut Option<Point>,
    mut query_fn: F,
    mut release_grab_fn: R,
) -> Result<bool, Box<dyn std::error::Error>>
where
    F: FnMut() -> Result<Point, Box<dyn std::error::Error>>,
    R: FnMut(u32) -> Result<(), Box<dyn std::error::Error>>,
{
    if let Some(dl) = *confirmation_deadline {
        if Instant::now() >= dl {
            let live_origin = query_fn()?;
            let size = Size::new(probe_window.width as u32, probe_window.height as u32);
            let rec = probe_window.reconcile_live_snapshot(live_origin, size);
            *confirmation_deadline = None;

            match rec {
                ConfigureReconciliation::Confirmed { origin, .. } => {
                    if let Some(final_target) = awaiting_final_confirmation.take() {
                        if origin == final_target {
                            println!("[INPUT] Final placement confirmed by WM at {}", origin);
                        } else {
                            eprintln!(
                                "[WARN] Final placement adjusted by WM to {} (expected {}); cancelling interaction.",
                                origin, final_target
                            );
                            if let HostAction::ReleaseGrab { time } =
                                interaction.cancel_on_wm_mismatch(x11rb::CURRENT_TIME)
                            {
                                release_grab_fn(time)?;
                            }
                            *pending_move = None;
                            probe_window.cancel_in_flight_moves();
                        }
                    }
                }
                ConfigureReconciliation::GenuineMismatch {
                    requested,
                    confirmed,
                } => {
                    eprintln!(
                        "[WARN] Movement confirmation timed out: requested {}, confirmed {}; cancelling interaction.",
                        requested, confirmed
                    );
                    if let Some(final_target) = awaiting_final_confirmation.take() {
                        eprintln!(
                            "[WARN] Final placement adjusted by WM to {} (expected {})",
                            confirmed, final_target
                        );
                    }
                    if let HostAction::ReleaseGrab { time } =
                        interaction.cancel_on_wm_mismatch(x11rb::CURRENT_TIME)
                    {
                        release_grab_fn(time)?;
                    }
                    *pending_move = None;
                    probe_window.cancel_in_flight_moves();
                }
                _ => {}
            }
            return Ok(true);
        }
    }
    Ok(false)
}

#[allow(clippy::too_many_arguments)]
fn check_confirmation_timeout(
    conn: &impl Connection,
    screen_root: x11rb::protocol::xproto::Window,
    confirmation_deadline: &mut Option<Instant>,
    probe_window: &mut ManagedProbeWindow,
    awaiting_final_confirmation: &mut Option<Point>,
    interaction: &mut InteractionManager,
    pointer_tracker: &mut PointerCaptureTracker,
    renderer: &Renderer,
    pending_move: &mut Option<Point>,
) {
    let window_id = probe_window.window;
    if let Err(e) = check_confirmation_timeout_with(
        confirmation_deadline,
        probe_window,
        awaiting_final_confirmation,
        interaction,
        pending_move,
        || ManagedProbeWindow::query_window_actual_root_origin(conn, window_id, screen_root),
        |time| pointer_tracker.release_if_held(conn, time).map(|_| ()),
    ) {
        fatal_host_error(
            &format!("Failed during confirmation timeout handling: {}", e),
            conn,
            pointer_tracker,
            renderer,
            probe_window,
        );
    }
}

fn flush_pending_move(
    pending_move: &mut Option<Point>,
    probe_window: &mut ManagedProbeWindow,
    conn: &impl Connection,
    pointer_tracker: &mut PointerCaptureTracker,
    renderer: &Renderer,
    confirmation_deadline: &mut Option<Instant>,
) {
    if let Some(target) = pending_move.take() {
        if target != probe_window.requested_origin() {
            if let Err(e) = probe_window.configure_position(conn, target) {
                fatal_host_error(
                    &format!("Failed to move window to {}: {}", target, e),
                    conn,
                    pointer_tracker,
                    renderer,
                    probe_window,
                );
            }
            *confirmation_deadline = Some(Instant::now() + CONFIRMATION_TIMEOUT);
        }
    }
}

fn fatal_host_error(
    err_msg: &str,
    conn: &impl Connection,
    pointer_tracker: &mut PointerCaptureTracker,
    renderer: &Renderer,
    probe_window: &ManagedProbeWindow,
) -> ! {
    eprintln!("[ERROR] {}", err_msg);
    if let Err(e) = pointer_tracker.release_if_held(conn, x11rb::CURRENT_TIME) {
        eprintln!(
            "[ERROR] Failed to release pointer capture during error exit: {}",
            e
        );
    }
    if let Err(e) = renderer.destroy(conn) {
        eprintln!(
            "[ERROR] Failed to destroy renderer during error exit: {}",
            e
        );
    }
    if let Err(e) = probe_window.destroy(conn) {
        eprintln!(
            "[ERROR] Failed to destroy probe window during error exit: {}",
            e
        );
    }
    process::exit(1);
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
}
