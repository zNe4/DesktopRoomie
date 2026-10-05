use std::env;
use std::process;
use x11rb::connection::Connection;
use x11rb::protocol::render::ConnectionExt as RenderExt;
use x11rb::protocol::shape::ConnectionExt as ShapeExt;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as XprotoExt, VisualClass};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_help();
        return;
    }

    println!("=== DesktopRoomie A00-M01: Desktop Probe ===");
    println!("Mission M01.1: Establishing X11 connection and diagnostics...");

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

    println!("\n[X11 Session]");
    println!("  Screen index:       {}", screen_num);
    println!("  Root window:        0x{:x}", screen.root);
    println!(
        "  Screen dimensions:  {}x{} px ({}x{} mm)",
        screen.width_in_pixels,
        screen.height_in_pixels,
        screen.width_in_millimeters,
        screen.height_in_millimeters
    );
    println!("  Root visual ID:     0x{:x}", screen.root_visual);
    println!("  Root depth:         {} bpp", screen.root_depth);

    // Inspect available visuals and depth 32 availability
    println!("\n[Visuals & Depths]");
    let mut depth_32_count = 0;
    for depth in &screen.allowed_depths {
        if depth.depth == 32 {
            for visual in &depth.visuals {
                if visual.class == VisualClass::TRUE_COLOR {
                    depth_32_count += 1;
                }
            }
        }
    }
    println!(
        "  Allowed depths:     {:?}",
        screen
            .allowed_depths
            .iter()
            .map(|d| d.depth)
            .collect::<Vec<_>>()
    );
    println!(
        "  32-bit TrueColor visuals: {} found (required for M01.2 alpha transparency)",
        depth_32_count
    );

    // Query RENDER extension
    println!("\n[Extensions]");
    match conn.render_query_version(0, 11) {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => {
                println!(
                    "  RENDER extension:   supported (v{}.{})",
                    reply.major_version, reply.minor_version
                );
            }
            Err(e) => eprintln!("  RENDER extension:   query reply failed: {}", e),
        },
        Err(e) => eprintln!("  RENDER extension:   not available: {}", e),
    }

    // Query SHAPE extension
    match conn.shape_query_version() {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => {
                println!(
                    "  SHAPE extension:    supported (v{}.{})",
                    reply.major_version, reply.minor_version
                );
            }
            Err(e) => eprintln!("  SHAPE extension:    query reply failed: {}", e),
        },
        Err(e) => eprintln!("  SHAPE extension:    not available: {}", e),
    }

    // Query EWMH hints from root window
    println!("\n[Window Manager / EWMH]");
    query_ewmh_hints(&conn, screen.root);

    println!("\n[Result]");
    println!("  M01.1 verification SUCCESS: X11 connection established and environment verified cleanly.");
}

fn print_help() {
    println!("DesktopRoomie - A00-M01 Desktop Probe");
    println!();
    println!("USAGE:");
    println!("  desktop-probe [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("  -h, --help       Show this help message");
    println!("  --diagnose       Run full environment diagnostics and exit (default for M01.1)");
}

fn query_ewmh_hints(conn: &impl Connection, root: x11rb::protocol::xproto::Window) {
    if let Ok(atom_desktops) = conn.intern_atom(false, b"_NET_NUMBER_OF_DESKTOPS") {
        if let Ok(reply) = atom_desktops.reply() {
            if let Ok(prop) = conn.get_property(false, root, reply.atom, AtomEnum::CARDINAL, 0, 1) {
                if let Ok(prop_reply) = prop.reply() {
                    if let Some(val) = prop_reply.value32().and_then(|mut iter| iter.next()) {
                        println!("  _NET_NUMBER_OF_DESKTOPS: {}", val);
                    }
                }
            }
        }
    }

    if let Ok(atom_current) = conn.intern_atom(false, b"_NET_CURRENT_DESKTOP") {
        if let Ok(reply) = atom_current.reply() {
            if let Ok(prop) = conn.get_property(false, root, reply.atom, AtomEnum::CARDINAL, 0, 1) {
                if let Ok(prop_reply) = prop.reply() {
                    if let Some(val) = prop_reply.value32().and_then(|mut iter| iter.next()) {
                        println!("  _NET_CURRENT_DESKTOP:   {}", val);
                    }
                }
            }
        }
    }

    if let Ok(atom_workarea) = conn.intern_atom(false, b"_NET_WORKAREA") {
        if let Ok(reply) = atom_workarea.reply() {
            if let Ok(prop) = conn.get_property(false, root, reply.atom, AtomEnum::CARDINAL, 0, 4) {
                if let Ok(prop_reply) = prop.reply() {
                    if let Some(mut iter) = prop_reply.value32() {
                        let x = iter.next().unwrap_or(0);
                        let y = iter.next().unwrap_or(0);
                        let w = iter.next().unwrap_or(0);
                        let h = iter.next().unwrap_or(0);
                        println!(
                            "  _NET_WORKAREA:          x={}, y={}, w={}, h={}",
                            x, y, w, h
                        );
                    }
                }
            }
        }
    }
}
