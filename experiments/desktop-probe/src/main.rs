mod x11;

use std::env;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::render::ConnectionExt as RenderExt;
use x11rb::protocol::shape::ConnectionExt as ShapeExt;
use x11rb::protocol::xproto::VisualClass;
use x11rb::protocol::Event;

use crate::x11::monitors::query_desktop_layout;
use crate::x11::render::Renderer;
use crate::x11::visual::find_alpha_visual;
use crate::x11::window::ManagedProbeWindow;

struct Config {
    diagnose: bool,
    duration_secs: Option<u64>,
}

fn parse_args() -> Result<Config, String> {
    let mut diagnose = false;
    let mut duration_secs = None;

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

    // M01.2: Render borderless transparent test body
    println!("Mission M01.2: Rendering borderless transparent test body...");

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

    // 2. Report target placement
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

    // 3. Create managed borderless window
    let probe_window = match ManagedProbeWindow::create(
        &conn,
        screen.root,
        alpha_vis.visual_id,
        layout.usable_area,
    ) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[ERROR] Failed to create managed probe window: {}", e);
            process::exit(1);
        }
    };

    println!(
        "  Window created: ID=0x{:x}, geometry={}x{} at +{}+{}",
        probe_window.window,
        probe_window.width,
        probe_window.height,
        probe_window.x,
        probe_window.y
    );
    println!("  Window properties: borderless (_MOTIF_WM_HINTS), managed, type=UTILITY");

    // 4. Initialize double-buffered renderer
    let renderer = match Renderer::new(&conn, probe_window.window, probe_window.colormap) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[ERROR] Failed to initialize renderer: {}", e);
            let _ = probe_window.destroy(&conn);
            process::exit(1);
        }
    };

    // Perform initial paint
    if let Err(e) = renderer.paint(&conn, probe_window.window) {
        eprintln!("[WARN] Initial paint error: {}", e);
    }

    println!("\n[Running Probe]");
    if let Some(sec) = config.duration_secs {
        println!("  Probe running for {} seconds (or until closed)...", sec);
    } else {
        println!(
            "  Probe is visible on desktop. Close window or press Ctrl+C in terminal to exit."
        );
    }

    // 5. Event loop
    let start_time = Instant::now();
    let mut running = true;

    while running {
        // Check timeout if configured
        if let Some(sec) = config.duration_secs {
            if start_time.elapsed() >= Duration::from_secs(sec) {
                println!("  Duration limit reached ({}s). Exiting cleanly.", sec);
                break;
            }
        }

        // Process incoming X11 events
        while let Ok(Some(event)) = conn.poll_for_event() {
            match event {
                Event::Expose(exp) if exp.window == probe_window.window => {
                    let _ = renderer.paint(&conn, probe_window.window);
                }
                Event::ClientMessage(msg) if msg.window == probe_window.window => {
                    let data = msg.data.as_data32();
                    if data[0] == probe_window.wm_delete_window {
                        println!("  Received WM_DELETE_WINDOW. Exiting cleanly.");
                        running = false;
                        break;
                    }
                }
                _ => {}
            }
        }

        thread::sleep(Duration::from_millis(15));
    }

    // 6. Cleanup
    println!("\n[Cleanup]");
    let _ = probe_window.destroy(&conn);
    println!("  Resources released cleanly.");
    println!("  M01.2: Borderless transparent test body executed successfully.");
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
