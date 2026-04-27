//! Signature manager dialog: a free-standing window that lets the user draw,
//! import, and manage signatures saved in the on-disk library.
//!
//! Built with imperative gtk-rs (no relm4 sub-component) — keeps state
//! contained and avoids interleaving the dialog into AppModel.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use libadwaita as adw;
use relm4::adw::prelude::*;
use relm4::gtk::cairo;
use relm4::gtk::{self, gdk, gio, glib};

use previewer_signature::{
    ImportOptions, Library, Signature, SignatureId, SignatureKind, Stroke, StrokePoint,
    import_png_signature,
};

const THUMB_W: i32 = 96;
const THUMB_H: i32 = 32;

/// Open a picker popover anchored to `anchor`. Calls `on_pick` with the
/// chosen signature when the user clicks one.
pub fn open_picker(anchor: &impl IsA<gtk::Widget>, on_pick: impl Fn(Signature) + 'static) {
    let library = Library::default_user_library();
    let signatures = library.load_all().unwrap_or_default();

    let popover = gtk::Popover::new();
    popover.set_parent(anchor);
    popover.set_position(gtk::PositionType::Bottom);

    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();

    if signatures.is_empty() {
        let lbl = gtk::Label::builder()
            .label("No signatures yet.\nOpen the Signatures dialog\nto draw or import one.")
            .justify(gtk::Justification::Center)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(16)
            .margin_end(16)
            .build();
        outer.append(&lbl);
    } else {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        let on_pick = std::rc::Rc::new(on_pick);
        for sig in signatures {
            let row = adw::ActionRow::new();
            row.set_title(&sig.name);
            row.set_activatable(true);

            let thumb = signature_thumbnail(&sig);
            let pic = gtk::Picture::for_paintable(&thumb);
            pic.set_can_shrink(true);
            pic.set_content_fit(gtk::ContentFit::Contain);
            pic.set_size_request(64, 24);
            pic.add_css_class("card");
            pic.set_valign(gtk::Align::Center);
            row.add_prefix(&pic);

            let on_pick = on_pick.clone();
            let popover_clone = popover.clone();
            let sig_clone = sig.clone();
            row.connect_activated(move |_| {
                (on_pick)(sig_clone.clone());
                popover_clone.popdown();
            });
            list.append(&row);
        }
        outer.append(&list);
    }

    popover.set_child(Some(&outer));
    popover.popup();
}

/// Open the signature manager as a modal child of `parent`.
pub fn open(parent: &impl IsA<gtk::Window>) {
    let library = Rc::new(Library::default_user_library());

    let window = adw::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Signatures")
        .default_width(640)
        .default_height(480)
        .build();

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar.add_top_bar(&header);

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);

    let switcher = adw::ViewSwitcher::new();
    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
    let view_stack = adw::ViewStack::new();
    switcher.set_stack(Some(&view_stack));
    header.set_title_widget(Some(&switcher));

    // Track all the listboxes that show library state so we can refresh them
    // after saves/deletes.
    let library_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let refresh_library: Rc<RefCell<Box<dyn Fn()>>> = Rc::new(RefCell::new(Box::new(|| {})));
    {
        let lib = library.clone();
        let list = library_list.clone();
        let refresh_inner = refresh_library.clone();
        let refresh_fn: Box<dyn Fn()> = Box::new(move || {
            populate_library_list(&list, &lib, refresh_inner.clone());
        });
        *refresh_library.borrow_mut() = refresh_fn;
    }
    (refresh_library.borrow())();

    view_stack.add_titled(
        &draw_tab(library.clone(), refresh_library.clone()),
        Some("draw"),
        "Draw",
    );
    view_stack.add_titled(
        &import_tab(library.clone(), refresh_library.clone(), &window),
        Some("import"),
        "Import",
    );

    let library_scroll = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .child(&library_list)
        .build();
    let library_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    library_box.append(&library_scroll);
    view_stack.add_titled(&library_box, Some("library"), "Library");

    toolbar.set_content(Some(&view_stack));
    window.set_content(Some(&toolbar));
    window.present();
}

fn populate_library_list(
    list: &gtk::ListBox,
    library: &Library,
    refresh: Rc<RefCell<Box<dyn Fn()>>>,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let signatures = match library.load_all() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to load signature library");
            return;
        }
    };
    if signatures.is_empty() {
        let placeholder = gtk::Label::builder()
            .label("No signatures yet. Draw or import one from the other tabs.")
            .margin_top(24)
            .margin_bottom(24)
            .build();
        list.append(&placeholder);
        return;
    }
    for sig in signatures {
        let row = adw::ActionRow::new();
        row.set_title(&sig.name);
        let kind_label = match &sig.kind {
            SignatureKind::Vector { strokes } => {
                format!(
                    "{} stroke{}",
                    strokes.len(),
                    if strokes.len() == 1 { "" } else { "s" }
                )
            }
            SignatureKind::Raster { width, height, .. } => format!("Raster · {width}×{height}"),
        };
        row.set_subtitle(&kind_label);

        let thumb = signature_thumbnail(&sig);
        let pic = gtk::Picture::for_paintable(&thumb);
        pic.set_can_shrink(true);
        pic.set_content_fit(gtk::ContentFit::Contain);
        pic.set_size_request(THUMB_W, THUMB_H);
        pic.add_css_class("card");
        pic.set_valign(gtk::Align::Center);
        row.add_prefix(&pic);

        let delete = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .valign(gtk::Align::Center)
            .css_classes(["destructive-action", "circular"])
            .tooltip_text("Delete signature")
            .build();
        let lib_clone = Rc::new(library.dir().to_path_buf());
        let id = sig.id;
        let refresh_clone = refresh.clone();
        delete.connect_clicked(move |_| {
            let lib = Library::at(lib_clone.as_ref());
            if let Err(e) = lib.delete(id) {
                tracing::error!(error = %e, "delete failed");
            }
            (refresh_clone.borrow())();
        });
        row.add_suffix(&delete);
        list.append(&row);
    }
}

fn draw_tab(library: Rc<Library>, refresh: Rc<RefCell<Box<dyn Fn()>>>) -> gtk::Box {
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .spacing(12)
        .build();

    let canvas = gtk::DrawingArea::builder()
        .content_width(560)
        .content_height(180)
        .css_classes(["card"])
        .build();
    canvas.set_hexpand(true);

    // Shared mutable state: completed strokes + active stroke being drawn.
    let strokes: Rc<RefCell<Vec<Stroke>>> = Rc::new(RefCell::new(Vec::new()));
    let active: Rc<RefCell<Option<Stroke>>> = Rc::new(RefCell::new(None));

    {
        let strokes = strokes.clone();
        let active = active.clone();
        canvas.set_draw_func(move |area, cr, w, h| {
            // White background.
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.rectangle(0.0, 0.0, w as f64, h as f64);
            let _ = cr.fill();

            cr.set_source_rgb(0.05, 0.05, 0.05);
            cr.set_line_width(2.0);
            cr.set_line_cap(cairo::LineCap::Round);
            cr.set_line_join(cairo::LineJoin::Round);

            for s in strokes.borrow().iter().chain(active.borrow().iter()) {
                if s.points.is_empty() {
                    continue;
                }
                cr.move_to(s.points[0].x, s.points[0].y);
                for p in &s.points[1..] {
                    cr.line_to(p.x, p.y);
                }
                let _ = cr.stroke();
            }
            let _ = area;
        });
    }

    let drag = gtk::GestureDrag::new();
    drag.set_button(gdk::BUTTON_PRIMARY);
    {
        let active = active.clone();
        let canvas = canvas.clone();
        drag.connect_drag_begin(move |_, x, y| {
            let mut s = Stroke::new();
            s.push(StrokePoint {
                x,
                y,
                pressure: 1.0,
            });
            *active.borrow_mut() = Some(s);
            canvas.queue_draw();
        });
    }
    {
        let active = active.clone();
        let canvas = canvas.clone();
        drag.connect_drag_update(move |g, dx, dy| {
            if let Some((sx, sy)) = g.start_point() {
                if let Some(s) = active.borrow_mut().as_mut() {
                    s.push(StrokePoint {
                        x: sx + dx,
                        y: sy + dy,
                        pressure: 1.0,
                    });
                }
                canvas.queue_draw();
            }
        });
    }
    {
        let active = active.clone();
        let strokes = strokes.clone();
        let canvas = canvas.clone();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(s) = active.borrow_mut().take()
                && !s.points.is_empty()
            {
                strokes.borrow_mut().push(s.simplified(0.5));
            }
            canvas.queue_draw();
        });
    }
    canvas.add_controller(drag);
    outer.append(&canvas);

    let name_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let name_entry = gtk::Entry::builder()
        .placeholder_text("Signature name")
        .hexpand(true)
        .build();
    name_row.append(&name_entry);
    outer.append(&name_row);

    let buttons = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();

    let clear_btn = gtk::Button::with_label("Clear");
    {
        let strokes = strokes.clone();
        let canvas = canvas.clone();
        clear_btn.connect_clicked(move |_| {
            strokes.borrow_mut().clear();
            canvas.queue_draw();
        });
    }
    buttons.append(&clear_btn);

    let save_btn = gtk::Button::builder()
        .label("Save")
        .css_classes(["suggested-action"])
        .build();
    {
        let strokes = strokes.clone();
        let canvas = canvas.clone();
        let name_entry = name_entry.clone();
        let library = library.clone();
        let refresh = refresh.clone();
        save_btn.connect_clicked(move |_| {
            let s = strokes.borrow();
            if s.is_empty() {
                return;
            }
            let name = name_entry.text().trim().to_string();
            let name = if name.is_empty() {
                format!("Drawn {}", chrono_like_stamp())
            } else {
                name
            };
            let sig = Signature {
                id: SignatureId::random(),
                name,
                kind: SignatureKind::Vector { strokes: s.clone() },
            };
            drop(s);
            if let Err(e) = library.save(&sig) {
                tracing::error!(error = %e, "failed to save signature");
                return;
            }
            tracing::info!(id = ?sig.id, "saved drawn signature");
            strokes.borrow_mut().clear();
            name_entry.set_text("");
            canvas.queue_draw();
            (refresh.borrow())();
        });
    }
    buttons.append(&save_btn);
    outer.append(&buttons);
    outer
}

fn import_tab(
    library: Rc<Library>,
    refresh: Rc<RefCell<Box<dyn Fn()>>>,
    parent: &adw::Window,
) -> gtk::Box {
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .spacing(12)
        .build();

    let blurb = gtk::Label::builder()
        .label(
            "Import a signature from a transparent PNG. The image is auto-trimmed to its alpha bounding box so any margin around the ink doesn't end up in the stamp.",
        )
        .wrap(true)
        .xalign(0.0)
        .build();
    outer.append(&blurb);

    let pick_btn = gtk::Button::builder()
        .label("Pick PNG…")
        .css_classes(["suggested-action"])
        .halign(gtk::Align::Start)
        .build();
    let parent = parent.clone();
    {
        let library = library.clone();
        let refresh = refresh.clone();
        pick_btn.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::new();
            dialog.set_title("Pick signature PNG");
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("PNG with alpha"));
            filter.add_mime_type("image/png");
            filter.add_pattern("*.png");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));

            let library = library.clone();
            let refresh = refresh.clone();
            dialog.open(
                Some(&parent),
                gio::Cancellable::NONE,
                move |result| match result {
                    Ok(file) => {
                        if let Some(path) = file.path() {
                            handle_import(path, &library, &refresh);
                        }
                    }
                    Err(e) => tracing::debug!(?e, "import dialog dismissed"),
                },
            );
        });
    }
    outer.append(&pick_btn);
    outer
}

fn handle_import(path: PathBuf, library: &Library, refresh: &Rc<RefCell<Box<dyn Fn()>>>) {
    match import_png_signature(&path, ImportOptions::default()) {
        Ok(sig) => {
            if let Err(e) = library.save(&sig) {
                tracing::error!(error = %e, "library save failed");
                return;
            }
            tracing::info!(name = %sig.name, "imported signature");
            (refresh.borrow())();
        }
        Err(e) => tracing::warn!(error = %e, path = %path.display(), "PNG import failed"),
    }
}

/// Build a small `gdk::MemoryTexture` previewing the signature for use in
/// the library list. Vector sigs are rendered to a Cairo surface; rasters
/// use their native RGBA bytes directly (gtk::Picture handles fit-to-size).
pub(crate) fn signature_thumbnail(sig: &Signature) -> gdk::MemoryTexture {
    match &sig.kind {
        SignatureKind::Raster {
            width,
            height,
            pixels,
        } => {
            let bytes = glib::Bytes::from(pixels.as_slice());
            gdk::MemoryTexture::new(
                *width as i32,
                *height as i32,
                gdk::MemoryFormat::R8g8b8a8,
                &bytes,
                *width as usize * 4,
            )
        }
        SignatureKind::Vector { strokes } => {
            let surface = render_strokes_to_surface(strokes, THUMB_W * 2, THUMB_H * 2);
            let w = surface.width();
            let h = surface.height();
            let rgba = surface_argb32_to_rgba(surface);
            let bytes = glib::Bytes::from(&rgba[..]);
            gdk::MemoryTexture::new(w, h, gdk::MemoryFormat::R8g8b8a8, &bytes, (w as usize) * 4)
        }
    }
}

/// Rasterise `strokes` into a `width × height` Cairo surface, fitting all
/// strokes proportionally with a small padding. Strokes are drawn black on
/// transparent so they composite cleanly into list rows.
fn render_strokes_to_surface(strokes: &[Stroke], width: i32, height: i32) -> cairo::ImageSurface {
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height)
        .expect("create thumbnail surface");
    let cr = cairo::Context::new(&surface).expect("cairo context");

    // Compute the union bounding box of every point — skip non-finite values
    // so a corrupted disk file can't NaN-poison the bbox and tip Cairo over.
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut any = false;
    for s in strokes {
        for p in &s.points {
            if !p.x.is_finite() || !p.y.is_finite() {
                continue;
            }
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
            any = true;
        }
    }
    if !any {
        return surface;
    }
    let bbox_w = (max_x - min_x).max(1.0);
    let bbox_h = (max_y - min_y).max(1.0);

    let pad = 4.0;
    let avail_w = width as f64 - 2.0 * pad;
    let avail_h = height as f64 - 2.0 * pad;
    let scale = (avail_w / bbox_w).min(avail_h / bbox_h);
    let offset_x = pad + (avail_w - bbox_w * scale) / 2.0;
    let offset_y = pad + (avail_h - bbox_h * scale) / 2.0;

    cr.set_source_rgb(0.05, 0.05, 0.05);
    cr.set_line_width(2.0);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    for s in strokes {
        if s.points.is_empty() {
            continue;
        }
        let map = |p: &StrokePoint| {
            (
                (p.x - min_x) * scale + offset_x,
                (p.y - min_y) * scale + offset_y,
            )
        };
        let (x0, y0) = map(&s.points[0]);
        cr.move_to(x0, y0);
        for p in &s.points[1..] {
            let (x, y) = map(p);
            cr.line_to(x, y);
        }
        let _ = cr.stroke();
    }
    drop(cr);
    surface
}

/// Convert a Cairo `ARgb32` surface (BGRA premultiplied on little-endian)
/// to straight RGBA8, the format `gdk::MemoryTexture::R8g8b8a8` expects.
///
/// Takes the surface by value because `data()` needs `&mut self` and cloning
/// an `ImageSurface` doesn't reliably grant a separate mutable borrow under
/// cairo-rs.
fn surface_argb32_to_rgba(mut surface: cairo::ImageSurface) -> Vec<u8> {
    let w = surface.width() as usize;
    let h = surface.height() as usize;
    let stride = surface.stride() as usize;
    let data = surface.data().expect("surface data");

    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let src = y * stride + x * 4;
            let dst = (y * w + x) * 4;
            let b = data[src];
            let g = data[src + 1];
            let r = data[src + 2];
            let a = data[src + 3];
            let (r, g, b) = if a == 0 {
                (0, 0, 0)
            } else if a == 255 {
                (r, g, b)
            } else {
                (
                    ((r as u32 * 255) / a as u32).min(255) as u8,
                    ((g as u32 * 255) / a as u32).min(255) as u8,
                    ((b as u32 * 255) / a as u32).min(255) as u8,
                )
            };
            out[dst] = r;
            out[dst + 1] = g;
            out[dst + 2] = b;
            out[dst + 3] = a;
        }
    }
    out
}

fn chrono_like_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("#{secs}")
}

// Suppress an unused-imports warning when this module is the only consumer
// of these types.
#[allow(dead_code)]
fn _force_use(_: &SignatureId, _: glib::ControlFlow) {}
