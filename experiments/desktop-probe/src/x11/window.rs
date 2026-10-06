use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, Colormap, ColormapAlloc, ConnectionExt as XprotoExt, CreateWindowAux, EventMask,
    PropMode, Visualid, Window, WindowClass,
};
use x11rb::wrapper::ConnectionExt as WrapperExt;

use super::monitors::Rect;
use super::render::{WINDOW_HEIGHT, WINDOW_WIDTH};
use super::shape::apply_body_input_shape;

pub struct ManagedProbeWindow {
    pub window: Window,
    pub colormap: Colormap,
    pub wm_delete_window: u32,
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
}

impl ManagedProbeWindow {
    pub fn create(
        conn: &impl Connection,
        root: Window,
        visual_id: Visualid,
        usable_area: Rect,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // 1. Create a colormap for the 32-bit visual (required in X11 to prevent BadMatch)
        let colormap = conn.generate_id()?;
        conn.create_colormap(ColormapAlloc::NONE, colormap, root, visual_id)?
            .check()?;

        // 2. Position the window centered inside the usable area on the primary display
        let width = WINDOW_WIDTH;
        let height = WINDOW_HEIGHT;
        let x = usable_area.x + ((usable_area.width.saturating_sub(width as u32)) / 2) as i32;
        let y = usable_area.y + ((usable_area.height.saturating_sub(height as u32)) / 2) as i32;

        // 3. Create the managed 32-bit window (override_redirect is false by default)
        // Request exposure, structure, and mouse pointer events (enter, leave, button clicks)
        let window = conn.generate_id()?;
        let win_aux = CreateWindowAux::new()
            .background_pixel(0)
            .border_pixel(0)
            .colormap(colormap)
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::STRUCTURE_NOTIFY
                    | EventMask::ENTER_WINDOW
                    | EventMask::LEAVE_WINDOW
                    | EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE,
            );

        conn.create_window(
            32, // depth
            window,
            root,
            x as i16,
            y as i16,
            width,
            height,
            0, // border_width
            WindowClass::INPUT_OUTPUT,
            visual_id,
            &win_aux,
        )?
        .check()?;

        // Set WM_NORMAL_HINTS (UserSpecified position and size) to direct Openbox placement
        let mut size_hints = x11rb::properties::WmSizeHints::new();
        size_hints.position = Some((
            x11rb::properties::WmSizeHintsSpecification::UserSpecified,
            x,
            y,
        ));
        size_hints.size = Some((
            x11rb::properties::WmSizeHintsSpecification::UserSpecified,
            width as i32,
            height as i32,
        ));
        size_hints.min_size = Some((width as i32, height as i32));
        size_hints.max_size = Some((width as i32, height as i32));
        size_hints.set_normal_hints(conn, window)?.check()?;

        // Set WM_HINTS: explicitly request no keyboard focus (ICCCM No-Input model)
        let mut wm_hints = x11rb::properties::WmHints::new();
        wm_hints.input = Some(false);
        wm_hints.set(conn, window)?.check()?;

        // 4. Motif hints: borderless (decorations = 0)
        let motif_atom = conn.intern_atom(false, b"_MOTIF_WM_HINTS")?.reply()?.atom;
        // flags = 2 (MWM_HINTS_DECORATIONS), decorations = 0
        let motif_hints = [2u32, 0, 0, 0, 0];
        conn.change_property32(
            PropMode::REPLACE,
            window,
            motif_atom,
            motif_atom,
            &motif_hints,
        )?
        .check()?;

        // 5. EWMH Window Type: UTILITY (non-disruptive desktop element)
        let net_wm_window_type = conn
            .intern_atom(false, b"_NET_WM_WINDOW_TYPE")?
            .reply()?
            .atom;
        let net_wm_window_type_utility = conn
            .intern_atom(false, b"_NET_WM_WINDOW_TYPE_UTILITY")?
            .reply()?
            .atom;
        conn.change_property32(
            PropMode::REPLACE,
            window,
            net_wm_window_type,
            AtomEnum::ATOM,
            &[net_wm_window_type_utility],
        )?
        .check()?;

        // 6. Set WM_PROTOCOLS: WM_DELETE_WINDOW (clean graceful close; omits WM_TAKE_FOCUS)
        let wm_protocols = conn.intern_atom(false, b"WM_PROTOCOLS")?.reply()?.atom;
        let wm_delete_window = conn.intern_atom(false, b"WM_DELETE_WINDOW")?.reply()?.atom;
        conn.change_property32(
            PropMode::REPLACE,
            window,
            wm_protocols,
            AtomEnum::ATOM,
            &[wm_delete_window],
        )?
        .check()?;

        // EWMH _NET_WM_USER_TIME = 0: explicitly informs window manager not to take focus on map
        let net_wm_user_time = conn.intern_atom(false, b"_NET_WM_USER_TIME")?.reply()?.atom;
        conn.change_property32(
            PropMode::REPLACE,
            window,
            net_wm_user_time,
            AtomEnum::CARDINAL,
            &[0],
        )?
        .check()?;

        // 7. Set window title and class
        conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            b"DesktopRoomie-Probe",
        )?
        .check()?;
        conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"desktop-probe\0DesktopRoomie\0",
        )?
        .check()?;

        // 8. Apply X11 Shape extension: restrict pointer input exclusively to the test shapes
        apply_body_input_shape(conn, window, width, height)?;

        // 9. Map window to screen
        conn.map_window(window)?.check()?;
        conn.flush()?;

        Ok(Self {
            window,
            colormap,
            wm_delete_window,
            x,
            y,
            width,
            height,
        })
    }

    pub fn destroy(&self, conn: &impl Connection) -> Result<(), Box<dyn std::error::Error>> {
        let _ = conn.destroy_window(self.window);
        let _ = conn.free_colormap(self.colormap);
        let _ = conn.flush();
        Ok(())
    }
}
