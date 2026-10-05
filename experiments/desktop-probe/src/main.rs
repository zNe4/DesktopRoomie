use std::cmp::{max, min};
use std::env;
use std::process;
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as RandrExt;
use x11rb::protocol::render::ConnectionExt as RenderExt;
use x11rb::protocol::shape::ConnectionExt as ShapeExt;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as XprotoExt, VisualClass};

#[derive(Debug, Clone)]
pub struct MonitorGeometry {
    pub name: String,
    pub is_primary: bool,
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

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

    println!("\n[X11 Session & Display]");
    println!("  Screen index:           {}", screen_num);
    println!("  Root window:            0x{:x}", screen.root);
    println!(
        "  Total screen spanning:  {}x{} px ({}x{} mm)",
        screen.width_in_pixels,
        screen.height_in_pixels,
        screen.width_in_millimeters,
        screen.height_in_millimeters
    );
    println!("  Root visual ID:         0x{:x}", screen.root_visual);
    println!("  Root depth:             {} bpp", screen.root_depth);

    // Query RandR monitors to isolate the primary/main screen (1920x1080)
    let monitors = query_monitors(&conn, screen.root);
    println!("\n[RandR Monitors]");
    let mut primary_monitor: Option<MonitorGeometry> = None;
    if monitors.is_empty() {
        println!("  No RandR monitor information available; using root screen geometry.");
    } else {
        for mon in &monitors {
            let primary_tag = if mon.is_primary {
                " (PRIMARY / MAIN)"
            } else {
                ""
            };
            println!(
                "  Monitor '{}': {}x{} at +{}+{} ({} mm x {} mm){}",
                mon.name, mon.width, mon.height, mon.x, mon.y, mon.width, mon.height, primary_tag
            );
            if mon.is_primary {
                primary_monitor = Some(mon.clone());
            }
        }
        if primary_monitor.is_none() && !monitors.is_empty() {
            // Default to first monitor if none explicitly marked primary
            primary_monitor = Some(monitors[0].clone());
        }
    }

    if let Some(ref pm) = primary_monitor {
        println!(
            "  -> Target main screen for DesktopRoomie: {}x{} at +{}+{}",
            pm.width, pm.height, pm.x, pm.y
        );
    }

    // Inspect available visuals and depth 32 candidates (softened claim)
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

    // Query RENDER extension
    println!("\n[Extensions]");
    match conn.render_query_version(0, 11) {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => {
                println!(
                    "  RENDER extension:       supported (v{}.{})",
                    reply.major_version, reply.minor_version
                );
            }
            Err(e) => eprintln!("  RENDER extension:       query reply failed: {}", e),
        },
        Err(e) => eprintln!("  RENDER extension:       not available: {}", e),
    }

    // Query SHAPE extension
    match conn.shape_query_version() {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => {
                println!(
                    "  SHAPE extension:        supported (v{}.{})",
                    reply.major_version, reply.minor_version
                );
            }
            Err(e) => eprintln!("  SHAPE extension:        query reply failed: {}", e),
        },
        Err(e) => eprintln!("  SHAPE extension:        not available: {}", e),
    }

    // Query EWMH hints: desktops and selected desktop's workarea
    println!("\n[Window Manager / EWMH]");
    let current_desktop = query_current_desktop(&conn, screen.root);
    let workarea_opt = query_desktop_workarea(&conn, screen.root, current_desktop);

    if let Some(wa) = workarea_opt {
        println!(
            "  Current desktop workarea: x={}, y={}, w={}, h={}",
            wa.x, wa.y, wa.width, wa.height
        );

        if let Some(ref pm) = primary_monitor {
            // Compute intersection of current desktop workarea and primary monitor bounds
            let eff_x = max(pm.x as i32, wa.x);
            let eff_y = max(pm.y as i32, wa.y);
            let eff_right = min((pm.x as i32) + (pm.width as i32), wa.x + (wa.width as i32));
            let eff_bottom = min(
                (pm.y as i32) + (pm.height as i32),
                wa.y + (wa.height as i32),
            );

            if eff_right > eff_x && eff_bottom > eff_y {
                let eff_w = (eff_right - eff_x) as u32;
                let eff_h = (eff_bottom - eff_y) as u32;
                println!(
                    "  -> Usable area on main screen (desktop {}): x={}, y={}, w={}, h={}",
                    current_desktop, eff_x, eff_y, eff_w, eff_h
                );
            }
        }
    }

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

fn query_monitors(
    conn: &impl Connection,
    root: x11rb::protocol::xproto::Window,
) -> Vec<MonitorGeometry> {
    let mut monitors = Vec::new();
    if let Ok(cookie) = conn.randr_get_monitors(root, true) {
        if let Ok(reply) = cookie.reply() {
            for mon in reply.monitors {
                let name = if let Ok(atom_name) = conn.get_atom_name(mon.name) {
                    if let Ok(name_reply) = atom_name.reply() {
                        String::from_utf8_lossy(&name_reply.name).into_owned()
                    } else {
                        format!("0x{:x}", mon.name)
                    }
                } else {
                    format!("0x{:x}", mon.name)
                };

                monitors.push(MonitorGeometry {
                    name,
                    is_primary: mon.primary,
                    x: mon.x,
                    y: mon.y,
                    width: mon.width,
                    height: mon.height,
                });
            }
        }
    }
    monitors
}

fn query_current_desktop(conn: &impl Connection, root: x11rb::protocol::xproto::Window) -> u32 {
    if let Ok(atom_desktops) = conn.intern_atom(false, b"_NET_NUMBER_OF_DESKTOPS") {
        if let Ok(reply) = atom_desktops.reply() {
            if let Ok(prop) = conn.get_property(false, root, reply.atom, AtomEnum::CARDINAL, 0, 1) {
                if let Ok(prop_reply) = prop.reply() {
                    if let Some(val) = prop_reply.value32().and_then(|mut iter| iter.next()) {
                        println!("  _NET_NUMBER_OF_DESKTOPS:  {}", val);
                    }
                }
            }
        }
    }

    let mut current_desktop = 0;
    if let Ok(atom_current) = conn.intern_atom(false, b"_NET_CURRENT_DESKTOP") {
        if let Ok(reply) = atom_current.reply() {
            if let Ok(prop) = conn.get_property(false, root, reply.atom, AtomEnum::CARDINAL, 0, 1) {
                if let Ok(prop_reply) = prop.reply() {
                    if let Some(val) = prop_reply.value32().and_then(|mut iter| iter.next()) {
                        println!("  _NET_CURRENT_DESKTOP:    {}", val);
                        current_desktop = val;
                    }
                }
            }
        }
    }
    current_desktop
}

fn query_desktop_workarea(
    conn: &impl Connection,
    root: x11rb::protocol::xproto::Window,
    current_desktop: u32,
) -> Option<Rect> {
    let atom_workarea = conn
        .intern_atom(false, b"_NET_WORKAREA")
        .ok()?
        .reply()
        .ok()?
        .atom;
    // EWMH defines _NET_WORKAREA as an array of 4 CARDINALs per desktop.
    // Read up to 256 32-bit values (supporting up to 64 virtual desktops).
    let prop = conn
        .get_property(false, root, atom_workarea, AtomEnum::CARDINAL, 0, 256)
        .ok()?
        .reply()
        .ok()?;
    let values: Vec<u32> = prop.value32()?.collect();

    let offset = (current_desktop as usize) * 4;
    if offset + 3 < values.len() {
        let x = values[offset] as i32;
        let y = values[offset + 1] as i32;
        let width = values[offset + 2];
        let height = values[offset + 3];
        Some(Rect {
            x,
            y,
            width,
            height,
        })
    } else {
        eprintln!(
            "  [WARN] _NET_WORKAREA contains {} values, insufficient for desktop index {}",
            values.len(),
            current_desktop
        );
        None
    }
}
