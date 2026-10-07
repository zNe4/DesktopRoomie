use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, Char2b, ConnectionExt, CreateGCAux, CreateWindowAux, EventMask, Font, Gcontext,
    PropMode, Rectangle, Screen, Window, WindowClass,
};
use x11rb::wrapper::ConnectionExt as WrapperExt;

use super::resource::{cleanup_all, OwnedResource};
use crate::geometry::{place_menu, MenuHit, MenuLayout, Point, Rect};
use crate::interaction::{InteractionState, MenuGesture, MenuItem};

type HostError = Box<dyn std::error::Error>;
const LABELS: [&[u8]; 2] = [b"Dismiss", b"Quit"];

/// Input belongs to this popup only after its explicit acquisition request.
#[derive(Debug, Clone, Copy)]
pub struct PopupInputGate {
    pub window: Window,
    pub generation: u64,
    pub acquisition_sequence: u64,
    pub creation_sequence: u64,
}
impl PopupInputGate {
    pub fn accepts(&self, window: Window, generation: u64, sequence: u64, synthetic: bool) -> bool {
        !synthetic
            && window == self.window
            && generation == self.generation
            && sequence >= self.acquisition_sequence
    }
}

/// Short-lived resources only. Pointer ownership belongs to ProbeRuntime.
pub struct MenuPopup {
    pub window: Window,
    pub window_resource: OwnedResource,
    gc: Gcontext,
    gc_resource: OwnedResource,
    font: Font,
    font_resource: OwnedResource,
    pub rect: Rect,
    layout: MenuLayout,
    ascent: i16,
    descent: i16,
    black: u32,
    white: u32,
    pub input: PopupInputGate,
    painted_state: InteractionState,
}

impl MenuPopup {
    /// A placement failure is recoverable; X11 and cleanup failures are fatal.
    pub fn create(
        conn: &impl Connection,
        screen: &Screen,
        area: Rect,
        anchor: Point,
        generation: u64,
    ) -> Result<Option<Self>, HostError> {
        let font = conn.generate_id()?;
        conn.open_font(font, b"fixed")?.check()?;
        let font_resource = OwnedResource::default();
        let measured = (|| -> Result<_, HostError> {
            let metrics = conn.query_font(font)?.reply()?;
            let mut width = 0;
            for label in LABELS {
                let chars: Vec<_> = label
                    .iter()
                    .map(|byte| Char2b {
                        byte1: 0,
                        byte2: *byte,
                    })
                    .collect();
                let extents = conn.query_text_extents(font, &chars)?.reply()?;
                if extents.overall_left < 0 || extents.overall_width < 0 {
                    return Err("Unsupported fixed font text metrics".into());
                }
                width = width.max(extents.overall_width.max(extents.overall_right));
            }
            let height = i32::from(metrics.font_ascent) + i32::from(metrics.font_descent);
            if height <= 0 || metrics.font_ascent < 0 || metrics.font_descent < 0 {
                return Err("Unsupported fixed font height".into());
            }
            let layout = MenuLayout::new(
                u32::try_from(width)?
                    .checked_add(24)
                    .ok_or("Font width overflow")?
                    .max(120),
                u32::try_from(height)?
                    .checked_add(12)
                    .ok_or("Font height overflow")?
                    .max(28),
            );
            Ok((layout, metrics.font_ascent, metrics.font_descent))
        })();
        let (layout, ascent, descent) = match measured {
            Ok(metrics) => metrics,
            Err(error) => {
                let _ = cleanup_all([Box::new(|| {
                    font_resource.release_with(|| {
                        conn.close_font(font)?.check()?;
                        Ok(())
                    })
                })]);
                return Err(error);
            }
        };
        let layout = match layout {
            Ok(layout) => layout,
            Err(error) => {
                eprintln!("[WARN] Cannot size menu: {error}");
                font_resource.release_with(|| {
                    conn.close_font(font)?.check()?;
                    Ok(())
                })?;
                return Ok(None);
            }
        };
        let rect = match place_menu(area, layout.size, anchor, 4) {
            Ok(rect) => rect,
            Err(error) => {
                eprintln!("[WARN] Cannot place menu: {error}");
                font_resource.release_with(|| {
                    conn.close_font(font)?.check()?;
                    Ok(())
                })?;
                return Ok(None);
            }
        };
        let creation = (|| -> Result<_, HostError> {
            let window = conn.generate_id()?;
            let cookie = conn.create_window(
                screen.root_depth,
                window,
                screen.root,
                i16::try_from(rect.x)?,
                i16::try_from(rect.y)?,
                u16::try_from(rect.width)?,
                u16::try_from(rect.height)?,
                0,
                WindowClass::INPUT_OUTPUT,
                screen.root_visual,
                &CreateWindowAux::new()
                    .override_redirect(1)
                    .background_pixel(screen.white_pixel)
                    .border_pixel(screen.black_pixel)
                    .colormap(screen.default_colormap)
                    .event_mask(
                        EventMask::EXPOSURE
                            | EventMask::STRUCTURE_NOTIFY
                            | EventMask::BUTTON_PRESS
                            | EventMask::BUTTON_RELEASE
                            | EventMask::POINTER_MOTION,
                    ),
            )?;
            let sequence = cookie.sequence_number();
            cookie.check()?;
            Ok((window, sequence))
        })();
        let (window, creation_sequence) = match creation {
            Ok(window) => window,
            Err(error) => {
                let _ = cleanup_all([Box::new(|| {
                    font_resource.release_with(|| {
                        conn.close_font(font)?.check()?;
                        Ok(())
                    })
                })]);
                return Err(error);
            }
        };
        let window_resource = OwnedResource::default();
        let gc_creation = (|| -> Result<_, HostError> {
            let gc = conn.generate_id()?;
            conn.create_gc(
                gc,
                window,
                &CreateGCAux::new()
                    .font(font)
                    .foreground(screen.black_pixel)
                    .background(screen.white_pixel),
            )?
            .check()?;
            Ok(gc)
        })();
        let gc = match gc_creation {
            Ok(gc) => gc,
            Err(error) => {
                let _ = cleanup_all([
                    Box::new(|| {
                        window_resource.release_with(|| {
                            conn.destroy_window(window)?.check()?;
                            Ok(())
                        })
                    }),
                    Box::new(|| {
                        font_resource.release_with(|| {
                            conn.close_font(font)?.check()?;
                            Ok(())
                        })
                    }),
                ]);
                return Err(error);
            }
        };
        let popup = Self {
            window,
            window_resource,
            gc,
            gc_resource: OwnedResource::default(),
            font,
            font_resource,
            rect,
            layout,
            ascent,
            descent,
            black: screen.black_pixel,
            white: screen.white_pixel,
            input: PopupInputGate {
                window,
                generation,
                acquisition_sequence: u64::MAX,
                creation_sequence,
            },
            painted_state: InteractionState::Idle,
        };
        let initialization = (|| -> Result<(), HostError> {
            let property = conn
                .intern_atom(false, b"_NET_WM_WINDOW_TYPE")?
                .reply()?
                .atom;
            let popup_type = conn
                .intern_atom(false, b"_NET_WM_WINDOW_TYPE_POPUP_MENU")?
                .reply()?
                .atom;
            conn.change_property32(
                PropMode::REPLACE,
                window,
                property,
                AtomEnum::ATOM,
                &[popup_type],
            )?
            .check()?;
            let mut hints = x11rb::properties::WmHints::new();
            hints.input = Some(false);
            hints.set(conn, window)?.check()?;
            popup.paint(conn, InteractionState::Idle)?;
            conn.map_window(window)?.check()?;
            Ok(())
        })();
        if let Err(error) = initialization {
            let _ = popup.destroy(conn);
            return Err(error);
        }
        Ok(Some(popup))
    }

    pub fn hit(&self, root: Point, same_screen: bool) -> MenuHit {
        if !same_screen {
            return MenuHit::Outside;
        }
        self.layout
            .hit(Point::new(root.x - self.rect.x, root.y - self.rect.y))
    }

    pub fn paint(&self, conn: &impl Connection, state: InteractionState) -> Result<(), HostError> {
        let (hover, pressed) = match state {
            InteractionState::MenuOpen {
                hover,
                gesture: MenuGesture::ItemPressed { item, .. },
            } => (hover, Some(item)),
            InteractionState::MenuOpen { hover, .. } => (hover, None),
            _ => (None, None),
        };
        let fill = |rect: Rect, pixel: u32| -> Result<(), HostError> {
            conn.change_gc(
                self.gc,
                &x11rb::protocol::xproto::ChangeGCAux::new().foreground(pixel),
            )?
            .check()?;
            conn.poly_fill_rectangle(
                self.window,
                self.gc,
                &[Rectangle {
                    x: i16::try_from(rect.x)?,
                    y: i16::try_from(rect.y)?,
                    width: u16::try_from(rect.width)?,
                    height: u16::try_from(rect.height)?,
                }],
            )?
            .check()?;
            Ok(())
        };
        fill(
            Rect::new(0, 0, self.rect.width, self.rect.height),
            self.black,
        )?;
        for (index, label) in LABELS.iter().enumerate() {
            let row = self.layout.rows[index];
            let item = if index == 0 {
                MenuItem::Dismiss
            } else {
                MenuItem::Quit
            };
            let highlighted = hover == Some(item);
            let (background, foreground) = if highlighted {
                (self.black, self.white)
            } else {
                (self.white, self.black)
            };
            fill(row, background)?;
            conn.change_gc(
                self.gc,
                &x11rb::protocol::xproto::ChangeGCAux::new()
                    .foreground(foreground)
                    .background(background),
            )?
            .check()?;
            let text_height = i32::from(self.ascent) + i32::from(self.descent);
            let baseline = row.y + (row.height as i32 - text_height) / 2 + i32::from(self.ascent);
            conn.image_text8(self.window, self.gc, 12, i16::try_from(baseline)?, label)?
                .check()?;
            if pressed == Some(item) {
                conn.poly_rectangle(
                    self.window,
                    self.gc,
                    &[Rectangle {
                        x: 3,
                        y: i16::try_from(row.y + 2)?,
                        width: u16::try_from(row.width - 5)?,
                        height: u16::try_from(row.height - 5)?,
                    }],
                )?
                .check()?;
            }
        }
        Ok(())
    }

    pub fn repaint_if_changed(
        &mut self,
        conn: &impl Connection,
        state: InteractionState,
    ) -> Result<(), HostError> {
        if state != self.painted_state {
            self.paint(conn, state)?;
            self.painted_state = state;
        }
        Ok(())
    }

    pub fn destroy(&self, conn: &impl Connection) -> Result<(), HostError> {
        self.destroy_with(
            || {
                conn.destroy_window(self.window)?.check()?;
                Ok(())
            },
            || {
                conn.free_gc(self.gc)?.check()?;
                Ok(())
            },
            || {
                conn.close_font(self.font)?.check()?;
                Ok(())
            },
            || {
                conn.flush()?;
                Ok(())
            },
        )
    }

    fn destroy_with(
        &self,
        window: impl FnOnce() -> Result<(), HostError>,
        gc: impl FnOnce() -> Result<(), HostError>,
        font: impl FnOnce() -> Result<(), HostError>,
        flush: impl FnOnce() -> Result<(), HostError>,
    ) -> Result<(), HostError> {
        cleanup_all([
            Box::new(|| self.window_resource.release_with(window)),
            Box::new(|| self.gc_resource.release_with(gc)),
            Box::new(|| self.font_resource.release_with(font)),
            Box::new(flush),
        ])
    }

    #[cfg(test)]
    pub fn test_popup(window: Window, generation: u64) -> Self {
        let layout = MenuLayout::new(120, 28).unwrap();
        Self {
            window,
            window_resource: OwnedResource::default(),
            gc: 2,
            gc_resource: OwnedResource::default(),
            font: 3,
            font_resource: OwnedResource::default(),
            rect: Rect::new(-200, 48, layout.size.width, layout.size.height),
            layout,
            ascent: 10,
            descent: 3,
            black: 0,
            white: 1,
            input: PopupInputGate {
                window,
                generation,
                acquisition_sequence: 70000,
                creation_sequence: 69900,
            },
            painted_state: InteractionState::Idle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn input_gate_rejects_previous_generation_pre_acquisition_and_synthetic_events() {
        let gate = MenuPopup::test_popup(10, 2).input;
        assert!(gate.accepts(10, 2, 70000, false));
        assert!(gate.accepts(10, 2, 70001, false));
        assert!(!gate.accepts(9, 2, 70001, false));
        assert!(!gate.accepts(10, 1, 70001, false));
        assert!(!gate.accepts(10, 2, 69999, false));
        assert!(!gate.accepts(10, 2, 70001, true));
    }

    #[test]
    fn captured_root_coordinates_outside_and_on_another_screen_do_not_hit_rows() {
        let popup = MenuPopup::test_popup(10, 1);
        assert_eq!(popup.hit(Point::new(-188, 60), true), MenuHit::Row(0));
        assert_eq!(popup.hit(Point::new(-188, 60), false), MenuHit::Outside);
        assert_eq!(popup.hit(Point::new(-201, 60), true), MenuHit::Outside);
        assert_eq!(popup.hit(Point::new(1920, 600), true), MenuHit::Outside);
    }

    #[test]
    fn popup_cleanup_attempts_every_resource_in_order_and_is_acknowledged_idempotent() {
        let popup = MenuPopup::test_popup(10, 1);
        let calls = RefCell::new(Vec::new());
        popup
            .destroy_with(
                || {
                    calls.borrow_mut().push("window");
                    Ok(())
                },
                || {
                    calls.borrow_mut().push("gc");
                    Ok(())
                },
                || {
                    calls.borrow_mut().push("font");
                    Ok(())
                },
                || {
                    calls.borrow_mut().push("flush");
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(*calls.borrow(), ["window", "gc", "font", "flush"]);
        popup
            .destroy_with(
                || panic!("window repeated"),
                || panic!("gc repeated"),
                || panic!("font repeated"),
                || Ok(()),
            )
            .unwrap();
    }

    #[test]
    fn failed_window_cleanup_retains_uncertainty_and_still_frees_gc_and_font() {
        let popup = MenuPopup::test_popup(10, 1);
        let calls = RefCell::new(Vec::new());
        let error = popup
            .destroy_with(
                || {
                    calls.borrow_mut().push("window");
                    Err("window acknowledgement lost".into())
                },
                || {
                    calls.borrow_mut().push("gc");
                    Ok(())
                },
                || {
                    calls.borrow_mut().push("font");
                    Ok(())
                },
                || {
                    calls.borrow_mut().push("flush");
                    Ok(())
                },
            )
            .unwrap_err();
        assert_eq!(error.to_string(), "window acknowledgement lost");
        assert_eq!(*calls.borrow(), ["window", "gc", "font", "flush"]);
        assert!(popup
            .destroy_with(
                || panic!("unknown destroy repeated"),
                || panic!("gc repeated"),
                || panic!("font repeated"),
                || Ok(())
            )
            .is_err());
    }

    #[test]
    fn external_destruction_skips_window_but_releases_independent_resources() {
        let popup = MenuPopup::test_popup(10, 1);
        popup.window_resource.externally_destroyed();
        let calls = RefCell::new(Vec::new());
        popup
            .destroy_with(
                || panic!("external destroy repeated"),
                || {
                    calls.borrow_mut().push("gc");
                    Ok(())
                },
                || {
                    calls.borrow_mut().push("font");
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        assert_eq!(*calls.borrow(), ["gc", "font"]);
    }
}
