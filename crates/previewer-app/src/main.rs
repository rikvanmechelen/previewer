//! Previewer — entry point.

mod signature_manager;

use std::cell::Cell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use libadwaita as adw;
use relm4::adw::prelude::*;
use relm4::gtk::{self, cairo, gdk, gio, glib};
use relm4::prelude::*;

use previewer_core::{
    Annotation, AnnotationLayer, ArrowEnds, CoalesceKey, Color, FontSpec, Point, Rect, Settings,
    StampImage, Stroke, StrokeStyle, UndoStack, load_layer, save_layer, sidecar_path,
};
use previewer_image::{DecodedImage, decode_to_rgba};
use previewer_pdf::{Document as PdfDoc, RenderedPage, TextMatch};
use previewer_render::{
    DragKind, HitKind, ViewTransform, apply_drag, hit_test, paint_annotations, paint_selection,
};
use previewer_signature::{Signature, SignatureKind};

const APP_ID: &str = "org.moma.Previewer";

/// PDF Type 1 standard families we support without font embedding. Anything
/// the user picks here maps cleanly to a `/BaseFont` in the lopdf write
/// path (see `previewer-pdf/src/lopdf_pass.rs`).
const FONT_FAMILIES: &[&str] = &["Helvetica", "Times", "Courier"];

/// Stroke styles in the order presented by the dropdown. Index here is
/// also the `gtk::DropDown::selected()` value.
const STROKE_STYLE_LABELS: &[&str] = &["Solid", "Dashed", "Dotted"];

fn stroke_style_index(s: StrokeStyle) -> u32 {
    match s {
        StrokeStyle::Solid => 0,
        StrokeStyle::Dashed => 1,
        StrokeStyle::Dotted => 2,
    }
}

fn stroke_style_from_index(i: u32) -> Option<StrokeStyle> {
    match i {
        0 => Some(StrokeStyle::Solid),
        1 => Some(StrokeStyle::Dashed),
        2 => Some(StrokeStyle::Dotted),
        _ => None,
    }
}

/// Initial text for a freshly-placed FreeText annotation. Rendered dim,
/// cleared the first time the user enters edit mode, restored on empty
/// commit.
const PLACEHOLDER_TEXT: &str = "Enter some text";

fn family_index(family: &str) -> u32 {
    FONT_FAMILIES
        .iter()
        .position(|f| f.eq_ignore_ascii_case(family))
        .unwrap_or(0) as u32
}

fn color_to_rgba(c: Color) -> gdk::RGBA {
    let (r, g, b, a) = c.to_unit_rgba();
    gdk::RGBA::new(r as f32, g as f32, b as f32, a as f32)
}

fn rgba_to_color(rgba: &gdk::RGBA) -> Color {
    Color::rgba(
        (rgba.red() * 255.0).round().clamp(0.0, 255.0) as u8,
        (rgba.green() * 255.0).round().clamp(0.0, 255.0) as u8,
        (rgba.blue() * 255.0).round().clamp(0.0, 255.0) as u8,
        (rgba.alpha() * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

const ZOOM_STEP: f64 = 1.25;
const ZOOM_MIN: f64 = 0.10;
const ZOOM_MAX: f64 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Pan,
    Rect,
    Ellipse,
    Line,
    Arrow,
    DoubleArrow,
    Highlight,
    Text,
}

impl Tool {
    fn label(self) -> &'static str {
        match self {
            Tool::Pan => "Pan",
            Tool::Rect => "Rect",
            Tool::Ellipse => "Ellipse",
            Tool::Line => "Line",
            Tool::Arrow => "Arrow",
            Tool::DoubleArrow => "Double arrow",
            Tool::Highlight => "Highlight",
            Tool::Text => "Text",
        }
    }

    /// True for the shape-style tools that live behind the Draw popover.
    fn is_draw_shape(self) -> bool {
        matches!(
            self,
            Tool::Rect | Tool::Ellipse | Tool::Line | Tool::Arrow | Tool::DoubleArrow
        )
    }
}

#[derive(Debug, Clone)]
struct TextPromptInfo {
    widget_x: f64,
    widget_y: f64,
    image_x: f64,
    image_y: f64,
    /// Pre-fill the editor with this text (re-edit flow).
    initial_text: String,
    /// If set, commit replaces this index in `annotations.items` instead of
    /// appending a new annotation.
    replace_index: Option<usize>,
    /// Style applied live to the inline TextView so what the user types looks
    /// like what the rendered annotation will look like.
    font: FontSpec,
    color: Color,
    /// Current display zoom — feeds into the inline editor's font-size so a
    /// 14pt annotation at 200% zoom shows the editor at 28 widget pixels,
    /// matching what the rendered annotation will look like once committed.
    zoom: f64,
}

/// Whether the current zoom is being driven by the user (a manual value)
/// or by the "fit to width" mode that re-fits on viewport resize. The
/// middle zoom button toggles between the two: clicking it from
/// `FitWidth` jumps to a manual 100%, clicking from `Manual` re-enters
/// fit-width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZoomMode {
    FitWidth,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentKind {
    Image,
    Pdf,
}

#[derive(Debug)]
enum ViewState {
    Empty,
    Loaded { path: PathBuf, kind: DocumentKind },
    Error { path: PathBuf, message: String },
}

impl ViewState {
    fn page(&self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Loaded { .. } => "loaded",
            Self::Error { .. } => "error",
        }
    }

    fn is_pdf(&self) -> bool {
        matches!(
            self,
            Self::Loaded {
                kind: DocumentKind::Pdf,
                ..
            }
        )
    }

    fn is_loaded(&self) -> bool {
        matches!(self, Self::Loaded { .. })
    }
}

struct AppModel {
    state: ViewState,
    /// Cairo surface for the currently displayed page/image (pre-rotation).
    /// `ViewTransform` handles rotation + zoom at draw time.
    surface: Option<cairo::ImageSurface>,
    /// Pixel dimensions of `surface` (pre-rotation).
    original_size: (u32, u32),
    rotation_quarters: u8,
    zoom: f64,
    zoom_mode: ZoomMode,

    /// PDF-only: the open document and current page index.
    pdf_doc: Option<PdfDoc>,
    pdf_page: u32,

    /// PDF search state.
    search_active: bool,
    search_query: String,
    search_matches: Vec<TextMatch>,
    /// Selected match index. `None` if no current match.
    search_index: Option<usize>,

    /// Picked signature ready to be stamped on the next click-drag.
    active_signature: Option<Signature>,

    annotations: AnnotationLayer,
    tool: Tool,
    /// Live shape during a click-drag.
    draft: Option<Annotation>,
    /// Last press position in image-space, captured at drag-begin.
    draft_start: Option<Point>,

    /// Index into `annotations.items` of the currently selected annotation,
    /// if any. Selection persists across page changes only if the annotation
    /// is on the current page (we don't auto-clear on page change).
    selected: Option<usize>,
    /// Live drag (move/resize) state on a selected annotation.
    drag: Option<DragState>,

    /// Set in `update` when a Text-tool click should open the entry popover.
    /// `post_view` reads (and clears) this and spawns the popover with access
    /// to the actual `overlay_area` widget. RefCell because the payload is
    /// non-Copy (carries the optional initial text for re-editing).
    pending_text_prompt: std::cell::RefCell<Option<TextPromptInfo>>,
    /// While the inline TextView is open editing an existing annotation,
    /// this is `Some(index)` — the overlay paint skips that annotation so it
    /// doesn't show through the live editor.
    currently_editing_index: Cell<Option<usize>>,

    /// Annotation history. Snapshots of `annotations` are pushed *before*
    /// each mutation; Ctrl+Z replays them. Mutations like dragging a font
    /// spinbutton coalesce under the same `CoalesceKey` so a single Ctrl+Z
    /// reverts the whole gesture instead of one event.
    undo: UndoStack<AnnotationLayer>,

    /// Default font/color used for new FreeText annotations and shown in the
    /// font bar when no FreeText is selected. Edits to a selected FreeText
    /// also write back here so the next text the user starts inherits the
    /// last-chosen style.
    current_font: FontSpec,
    current_font_color: Color,

    /// Default stroke for newly-drawn shapes (Rect, Ellipse, Arrow). Like
    /// the font defaults, edits to a selected shape's stroke write back
    /// here.
    current_stroke: Stroke,

    /// Paths the user has explicitly confirmed they want to overwrite this
    /// session. PDFs are warned-on first in-place save because pdfium can
    /// choke mid-write and corrupt the original; once acknowledged we trust
    /// the user knows the risk.
    inplace_save_confirmed: HashSet<PathBuf>,

    /// Set by `ToggleSearch` (Ctrl+F) so `post_view` can grab focus on
    /// the inline search entry. Cleared after the focus call. RefCell-free
    /// since `bool` is `Copy`.
    pending_search_focus: Cell<bool>,

    /// Whether the page-thumbnails sidebar is currently visible. Toggled
    /// by the dedicated header-bar button; PDF-only.
    show_sidebar: bool,

    /// Set on PDF load — the sidebar's ListBox is then rebuilt from
    /// scratch in `post_view`. Bool because the ListBox itself isn't on
    /// the model (it lives in `widgets`).
    sidebar_dirty: Cell<bool>,

    /// HiDPI device-pixel-ratio for the DrawingArea, mirrored from
    /// `Widget::scale_factor()`. Used to supersample PDF page rasters so
    /// each pdfium pixel lands on a real device pixel rather than being
    /// upscaled by the compositor. 1 on standard displays, 2 on most
    /// HiDPI screens.
    surface_scale_factor: i32,

    /// Reference to the ScrolledWindow that hosts the page (used for
    /// adjustments + as the host widget for the pan gesture).
    scroll_window: Option<gtk::ScrolledWindow>,
    /// Mirrored model state, updated each `post_view` so the standalone pan
    /// gesture closures (which can't see `self`) can gate their behaviour.
    tool_signal: Rc<Cell<Tool>>,
    selected_signal: Rc<Cell<bool>>,
    active_signature_signal: Rc<Cell<bool>>,
}

#[derive(Debug, Clone)]
struct DragState {
    kind: DragKind,
    origin: Point,
    original: Annotation,
    index: usize,
}

impl AppModel {
    fn title(&self) -> String {
        match &self.state {
            ViewState::Empty => "Previewer".into(),
            ViewState::Loaded { path, .. } | ViewState::Error { path, .. } => path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Previewer")
                .to_string(),
        }
    }

    fn subtitle(&self) -> String {
        match (&self.state, &self.pdf_doc) {
            (
                ViewState::Loaded {
                    kind: DocumentKind::Pdf,
                    ..
                },
                Some(doc),
            ) => {
                let zoom_pct = (self.zoom * 100.0).round() as i32;
                let total = doc.page_count();
                format!(
                    "Page {} of {total} · {zoom_pct}% · {}",
                    self.pdf_page + 1,
                    self.tool.label()
                )
            }
            (
                ViewState::Loaded {
                    kind: DocumentKind::Image,
                    ..
                },
                _,
            ) => {
                let (w, h) = self.original_size;
                let zoom_pct = (self.zoom * 100.0).round() as i32;
                format!("{w} × {h} · {zoom_pct}% · {}", self.tool.label())
            }
            _ => String::new(),
        }
    }

    fn error_description(&self) -> String {
        match &self.state {
            ViewState::Error { path, message } => {
                format!("{}\n\n{}", path.display(), message)
            }
            _ => String::new(),
        }
    }

    fn transform(&self) -> ViewTransform {
        ViewTransform::for_image(self.original_size, self.zoom, self.rotation_quarters)
    }

    fn picture_width(&self) -> i32 {
        self.transform().widget_size().0.round().max(1.0) as i32
    }

    fn picture_height(&self) -> i32 {
        self.transform().widget_size().1.round().max(1.0) as i32
    }

    fn pdf_page_count(&self) -> Option<u32> {
        self.pdf_doc.as_ref().map(|d| d.page_count())
    }

    /// Re-render the current PDF page at the current zoom level so the
    /// rasterised text/lines remain crisp. No-op for non-PDF state.
    fn refresh_pdf_render(&mut self) {
        let scale = self.pdf_render_scale();
        let Some(doc) = self.pdf_doc.as_ref() else {
            return;
        };
        match doc.render_page(self.pdf_page, scale) {
            Ok(rendered) => {
                // image_native_size stays at the **scale=1.0** dimensions —
                // annotation coords don't change. Only the surface gets fatter.
                self.surface = Some(surface_from_rendered(&rendered));
            }
            Err(e) => tracing::error!(error = %e, "PDF re-render at zoom failed"),
        }
    }

    /// Compute the zoom level needed to fit the page's effective width
    /// (post-rotation) into the ScrolledWindow's viewport. Returns `None`
    /// if the viewport hasn't been allocated yet (e.g. immediately after
    /// load on a freshly-created window).
    fn fit_width_zoom(&self) -> Option<f64> {
        let scroll = self.scroll_window.as_ref()?;
        let viewport_w = scroll.hadjustment().page_size();
        if viewport_w <= 0.0 {
            return None;
        }
        let t = ViewTransform::for_image(self.original_size, 1.0, self.rotation_quarters);
        let (effective_w, _) = t.widget_size();
        if effective_w <= 0.0 {
            return None;
        }
        Some((viewport_w / effective_w).clamp(ZOOM_MIN, ZOOM_MAX))
    }

    fn pdf_render_scale(&self) -> f64 {
        // Render the PDF at `zoom × device-pixel-ratio` so each pdfium
        // pixel lands on a real device pixel. Without the DPR multiplier,
        // 1× zoom on a HiDPI display rasterises at half the device
        // resolution and the compositor upscales — noticeably soft
        // compared to the vector annotations we paint via Cairo, which go
        // straight to device pixels.
        //
        // Below 1.0 zoom we still render at 1× (Cairo's bilinear
        // downsample is cheap and looks fine). Above the cap the rasters
        // get expensive in memory; 4× of device pixels is plenty for
        // anything realistic.
        let dpr = self.surface_scale_factor.clamp(1, 4) as f64;
        (self.zoom * dpr).clamp(dpr, 4.0 * dpr)
    }

    /// The page index that newly drawn annotations should target.
    fn current_page(&self) -> u32 {
        if self.state.is_pdf() {
            self.pdf_page
        } else {
            0
        }
    }

    fn search_status(&self) -> String {
        if self.search_query.is_empty() {
            return String::new();
        }
        if self.search_matches.is_empty() {
            return "0 matches".into();
        }
        let i = self.search_index.unwrap_or(0);
        format!("{} of {}", i + 1, self.search_matches.len())
    }

    /// Returns the search matches that fall on the currently displayed PDF
    /// page, with the currently selected match marked.
    fn current_page_match_highlights(&self) -> Vec<Annotation> {
        if !self.state.is_pdf() {
            return Vec::new();
        }
        let selected_match = self.search_index.and_then(|i| self.search_matches.get(i));
        self.search_matches
            .iter()
            .filter(|m| m.page == self.pdf_page)
            .map(|m| {
                // Highlight selected match more strongly than the rest.
                let is_selected = selected_match.map(|s| std::ptr::eq(s, m)).unwrap_or(false);
                let color = if is_selected {
                    Color::rgba(255, 165, 0, 160) // orange-ish
                } else {
                    Color::rgba(255, 235, 0, 96)
                };
                Annotation::Highlight {
                    page: m.page,
                    bbox: m.bbox,
                    color,
                }
            })
            .collect()
    }

    fn clear_selection(&mut self) {
        self.selected = None;
        self.drag = None;
    }

    fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    /// Snapshot the persistable bits of the model into a `Settings` and
    /// flush to disk. Called on changes that should survive across
    /// launches (e.g. sidebar visibility). Errors are logged, not
    /// surfaced — a failed write isn't worth interrupting the user.
    fn save_settings(&self) {
        let settings = Settings {
            show_sidebar: self.show_sidebar,
        };
        if let Err(e) = settings.save() {
            tracing::warn!(error = %e, "failed to save settings");
        }
    }

    /// `Some((font, color))` if a FreeText is currently selected. The font bar
    /// reads this to mirror the selected annotation's style; otherwise it
    /// shows `current_font` / `current_font_color`.
    fn selected_freetext_style(&self) -> Option<(&FontSpec, Color)> {
        let idx = self.selected?;
        match self.annotations.items.get(idx)? {
            Annotation::FreeText { font, color, .. } => Some((font, *color)),
            _ => None,
        }
    }

    fn effective_font(&self) -> FontSpec {
        self.selected_freetext_style()
            .map(|(f, _)| f.clone())
            .unwrap_or_else(|| self.current_font.clone())
    }

    fn effective_font_color(&self) -> Color {
        self.selected_freetext_style()
            .map(|(_, c)| c)
            .unwrap_or(self.current_font_color)
    }

    /// `Some(stroke)` if a shape with a stroke (Rect / Ellipse / Arrow) is
    /// currently selected. Drives stroke-bar mirroring + write-back from
    /// the toolbar widgets.
    fn selected_shape_stroke(&self) -> Option<&Stroke> {
        let idx = self.selected?;
        match self.annotations.items.get(idx)? {
            Annotation::Rect { stroke, .. }
            | Annotation::Ellipse { stroke, .. }
            | Annotation::Arrow { stroke, .. } => Some(stroke),
            _ => None,
        }
    }

    fn effective_stroke(&self) -> Stroke {
        self.selected_shape_stroke()
            .cloned()
            .unwrap_or_else(|| self.current_stroke.clone())
    }

    /// Stroke controls show whenever a doc is loaded AND the font controls
    /// aren't taking the slot. The user said: "always show, unless text
    /// is selected (i.e. the text tools show)".
    fn stroke_bar_visible(&self) -> bool {
        self.state.is_loaded() && !self.font_bar_visible()
    }

    /// Visible whenever the user is plausibly working with text:
    /// the Text tool is armed, an inline editor is open, or a FreeText
    /// annotation is currently selected.
    fn font_bar_visible(&self) -> bool {
        if !self.state.is_loaded() {
            return false;
        }
        if self.tool == Tool::Text || self.currently_editing_index.get().is_some() {
            return true;
        }
        self.selected_freetext_style().is_some()
    }

    /// Push every annotation in `self.annotations` into the open pdfium
    /// document and write to `path`. After the write, re-extract from pdfium
    /// so the annotations stay editable in this session and aren't double-
    /// applied on the next save. Returns Ok on success.
    fn save_pdf_to(&mut self, path: &Path) -> Result<(), previewer_pdf::Error> {
        let Some(doc) = self.pdf_doc.as_mut() else {
            return Ok(());
        };
        let mut applied = 0;
        let mut errors = 0;
        for ann in self.annotations.items.iter() {
            match doc.add_annotation(ann) {
                Ok(()) => applied += 1,
                Err(e) => {
                    tracing::warn!(error = %e, "skipped unsupported annotation");
                    errors += 1;
                }
            }
        }
        doc.save(path)?;
        tracing::info!(
            path = %path.display(),
            applied,
            errors,
            "saved PDF with annotations"
        );

        self.annotations = if let Some(doc) = self.pdf_doc.as_mut() {
            AnnotationLayer {
                items: doc.extract_annotations(),
            }
        } else {
            AnnotationLayer::new()
        };
        // Save is the new "canonical" state; undo before save no longer
        // matches what's on disk, so don't let Ctrl+Z roll past it.
        self.undo.clear();
        self.clear_selection();
        // Native (scale=1.0) page dimensions drive the annotation coord
        // space; capture them once, then re-render at display scale for
        // crispness. Without this two-step the post-save raster ends up at
        // 1× and Cairo bilinear-upsamples it on every paint.
        if let Some(doc) = self.pdf_doc.as_ref()
            && let Ok(native) = doc.render_page(self.pdf_page, 1.0)
        {
            self.original_size = native.dimensions();
        }
        let scale = self.pdf_render_scale();
        if let Some(doc) = self.pdf_doc.as_ref()
            && let Ok(rendered) = doc.render_page(self.pdf_page, scale)
        {
            self.surface = Some(surface_from_rendered(&rendered));
        }
        Ok(())
    }

    fn go_to_pdf_page(&mut self, target: u32) {
        let scale = self.pdf_render_scale();
        let Some(doc) = self.pdf_doc.as_ref() else {
            return;
        };
        let total = doc.page_count();
        if total == 0 {
            return;
        }
        let target = target.min(total - 1);
        if target == self.pdf_page && self.surface.is_some() {
            return;
        }
        // Render at current zoom for crispness, but capture the page's
        // image-coord (scale=1.0) dimensions separately so annotations stay
        // anchored.
        match doc.render_page(target, 1.0) {
            Ok(native) => self.original_size = native.dimensions(),
            Err(e) => {
                tracing::error!(error = %e, page = target, "PDF native size probe failed");
                return;
            }
        }
        match doc.render_page(target, scale) {
            Ok(rendered) => {
                self.surface = Some(surface_from_rendered(&rendered));
                self.pdf_page = target;
                self.draft = None;
                self.draft_start = None;
                self.selected = None;
                self.drag = None;
            }
            Err(e) => tracing::error!(error = %e, page = target, "PDF page render failed"),
        }
    }
}

#[derive(Debug)]
enum AppMsg {
    OpenDialog,
    LoadPath(PathBuf),
    ZoomIn,
    ZoomOut,
    ZoomReset,
    /// Click on the contextual middle zoom button. From FitWidth we go to
    /// Manual @ 100%; from Manual we go to FitWidth.
    ZoomToggleFitWidth,
    /// The ScrolledWindow's viewport changed size — if we're in
    /// `ZoomMode::FitWidth`, recompute the zoom and re-render.
    ViewportResized,
    RotateRight,
    SelectTool(Tool),
    DrawBegin {
        wx: f64,
        wy: f64,
    },
    DrawUpdate {
        wx: f64,
        wy: f64,
    },
    DrawCommit,
    DrawCancel,
    Undo,
    Redo,
    /// GTK reports a new device-pixel-ratio for the page (e.g. window
    /// dragged from a 1× to a 2× monitor). PDFs re-rasterise at the new
    /// scale so they stay crisp.
    ScaleFactorChanged(i32),
    DeleteSelected,
    /// Re-open the inline editor on the currently selected FreeText annotation.
    EditSelectedText,
    /// The inline editor was dismissed without committing.
    EditCancelled,
    CommitFreeText {
        image_x: f64,
        image_y: f64,
        text: String,
        replace_index: Option<usize>,
    },
    /// Font-bar inputs: update either the selected FreeText annotation (if
    /// any) or the current default that drives the next text the user types.
    SetFontFamily(String),
    SetFontSize(f64),
    SetFontColor(Color),
    /// Stroke-bar inputs: update either the selected shape's stroke (if
    /// any) or the default that drives the next shape the user draws.
    SetStrokeColor(Color),
    SetStrokeWidth(f64),
    SetStrokeStyle(StrokeStyle),
    /// In-place save. For PDFs this triggers a warning dialog the first time
    /// per file (per session); for images it writes the sidecar directly.
    Save,
    /// User confirmed they want to overwrite the original PDF in-place. Skips
    /// the warning and runs the save.
    ConfirmedSaveInPlace,
    /// Open the "Save As…" file picker.
    SaveAs,
    /// Picker callback: write the document to `path` and re-target subsequent
    /// saves there.
    SaveAsTo(PathBuf),
    OpenSignatureManager,
    NextPage,
    PrevPage,
    FirstPage,
    LastPage,
    /// Jump directly to a specific page (sidebar thumbnail click).
    GoToPage(u32),
    /// Show / hide the page-thumbnails sidebar.
    ToggleSidebar,
    ToggleSearch,
    SearchQuery(String),
    SearchNext,
    SearchPrev,
    CloseSearch,
    SelectSignature(Signature),
}

#[relm4::component]
impl SimpleComponent for AppModel {
    type Init = Option<PathBuf>;
    type Input = AppMsg;
    type Output = ();

    view! {
        #[root]
        adw::ApplicationWindow {
            set_title: Some("Previewer"),
            set_icon_name: Some(APP_ID),
            set_default_size: (1024, 768),

            #[wrap(Some)]
            set_content = &adw::ToolbarView {
                // Main bar: file/view actions on the left, search +
                // editing actions (undo/redo, save, save as, rotate, zoom)
                // on the right. Drawing tools live on the second bar
                // below.
                add_top_bar = &adw::HeaderBar {
                    pack_start = &gtk::Button {
                        set_icon_name: "document-open-symbolic",
                        set_tooltip_text: Some("Open image (Ctrl+O)"),
                        connect_clicked[sender] => move |_| {
                            sender.input(AppMsg::OpenDialog);
                        },
                    },
                    pack_start = &gtk::ToggleButton {
                        set_icon_name: "sidebar-show-symbolic",
                        set_tooltip_text: Some("Page thumbnails (F9)"),
                        set_margin_start: 4,
                        #[watch]
                        set_visible: model.state.is_pdf(),
                        #[watch]
                        set_active: model.show_sidebar,
                        connect_clicked[sender] => move |_| {
                            sender.input(AppMsg::ToggleSidebar);
                        },
                    },
                    pack_start = &gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 0,
                        set_margin_start: 12,
                        add_css_class: "linked",
                        #[watch]
                        set_visible: model.state.is_pdf(),

                        gtk::Button {
                            set_icon_name: "go-previous-symbolic",
                            set_tooltip_text: Some("Previous page (Page Up)"),
                            #[watch]
                            set_sensitive: model.pdf_page > 0,
                            connect_clicked[sender] => move |_| {
                                sender.input(AppMsg::PrevPage);
                            },
                        },
                        gtk::Button {
                            set_icon_name: "go-next-symbolic",
                            set_tooltip_text: Some("Next page (Page Down)"),
                            #[watch]
                            set_sensitive: model.pdf_page + 1
                                < model.pdf_page_count().unwrap_or(0),
                            connect_clicked[sender] => move |_| {
                                sender.input(AppMsg::NextPage);
                            },
                        },
                    },

                    #[wrap(Some)]
                    set_title_widget = &adw::WindowTitle {
                        #[watch]
                        set_title: &model.title(),
                        #[watch]
                        set_subtitle: &model.subtitle(),
                    },

                    // pack_end widgets stack right→left: the FIRST `pack_end`
                    // declared here is the rightmost on screen. So we
                    // declare them in the order: search, undo/redo, save,
                    // save-as, rotate, zoom — to land left→right as
                    // [zoom, rotate, save-as, save, undo/redo, search].
                    pack_end = &gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 4,
                        #[watch]
                        set_visible: model.state.is_pdf(),

                        #[name = "search_entry"]
                        gtk::SearchEntry {
                            set_placeholder_text: Some("Search PDF…"),
                            set_width_chars: 18,
                            connect_search_changed[sender] => move |e| {
                                sender.input(AppMsg::SearchQuery(e.text().to_string()));
                            },
                            connect_activate[sender] => move |_| {
                                sender.input(AppMsg::SearchNext);
                            },
                            connect_stop_search[sender] => move |_| {
                                sender.input(AppMsg::CloseSearch);
                            },
                        },
                        gtk::Label {
                            #[watch]
                            set_label: &model.search_status(),
                            #[watch]
                            set_visible: !model.search_matches.is_empty(),
                            set_width_chars: 7,
                        },
                        gtk::Button {
                            set_icon_name: "go-up-symbolic",
                            set_tooltip_text: Some("Previous match (Shift+Enter)"),
                            #[watch]
                            set_visible: !model.search_matches.is_empty(),
                            connect_clicked[sender] => move |_| {
                                sender.input(AppMsg::SearchPrev);
                            },
                        },
                        gtk::Button {
                            set_icon_name: "go-down-symbolic",
                            set_tooltip_text: Some("Next match (Enter)"),
                            #[watch]
                            set_visible: !model.search_matches.is_empty(),
                            connect_clicked[sender] => move |_| {
                                sender.input(AppMsg::SearchNext);
                            },
                        },
                    },
                    pack_end = &gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 0,
                        add_css_class: "linked",
                        set_margin_end: 6,

                        gtk::Button {
                            set_icon_name: "edit-undo-symbolic",
                            set_tooltip_text: Some("Undo (Ctrl+Z)"),
                            #[watch]
                            set_sensitive: model.can_undo(),
                            connect_clicked[sender] => move |_| {
                                sender.input(AppMsg::Undo);
                            },
                        },
                        gtk::Button {
                            set_icon_name: "edit-redo-symbolic",
                            set_tooltip_text: Some("Redo (Ctrl+Shift+Z)"),
                            #[watch]
                            set_sensitive: model.can_redo(),
                            connect_clicked[sender] => move |_| {
                                sender.input(AppMsg::Redo);
                            },
                        },
                    },
                    pack_end = &gtk::Button {
                        set_icon_name: "document-save-symbolic",
                        set_tooltip_text: Some("Save annotations (Ctrl+S)"),
                        #[watch]
                        set_sensitive: matches!(model.state, ViewState::Loaded { .. }),
                        connect_clicked[sender] => move |_| {
                            sender.input(AppMsg::Save);
                        },
                    },
                    pack_end = &gtk::Button {
                        set_icon_name: "document-save-as-symbolic",
                        set_tooltip_text: Some("Save a copy as… (Ctrl+Shift+S)"),
                        #[watch]
                        set_sensitive: matches!(
                            model.state,
                            ViewState::Loaded { kind: DocumentKind::Pdf, .. }
                        ),
                        connect_clicked[sender] => move |_| {
                            sender.input(AppMsg::SaveAs);
                        },
                    },
                    pack_end = &gtk::Button {
                        set_icon_name: "object-rotate-right-symbolic",
                        set_tooltip_text: Some("Rotate 90° clockwise (R)"),
                        #[watch]
                        set_sensitive: matches!(model.state, ViewState::Loaded { .. }),
                        connect_clicked[sender] => move |_| {
                            sender.input(AppMsg::RotateRight);
                        },
                    },
                    pack_end = &gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 0,
                        add_css_class: "linked",

                        gtk::Button {
                            set_icon_name: "zoom-out-symbolic",
                            set_tooltip_text: Some("Zoom out (Ctrl+−)"),
                            #[watch]
                            set_sensitive: matches!(model.state, ViewState::Loaded { .. }),
                            connect_clicked[sender] => move |_| {
                                sender.input(AppMsg::ZoomOut);
                            },
                        },
                        gtk::Button {
                            // Contextual: when fit-to-width is active the
                            // affordance is "go back to 100%" (the [1]
                            // reset icon); otherwise it's "fit width to
                            // window".
                            #[watch]
                            set_icon_name: if matches!(model.zoom_mode, ZoomMode::FitWidth) {
                                "zoom-original-symbolic"
                            } else {
                                "previewer-fit-width-symbolic"
                            },
                            #[watch]
                            set_tooltip_text: Some(
                                if matches!(model.zoom_mode, ZoomMode::FitWidth) {
                                    "Reset zoom to 100% (Ctrl+0)"
                                } else {
                                    "Fit page width to window"
                                },
                            ),
                            #[watch]
                            set_sensitive: matches!(model.state, ViewState::Loaded { .. }),
                            connect_clicked[sender] => move |_| {
                                sender.input(AppMsg::ZoomToggleFitWidth);
                            },
                        },
                        gtk::Button {
                            set_icon_name: "zoom-in-symbolic",
                            set_tooltip_text: Some("Zoom in (Ctrl++)"),
                            #[watch]
                            set_sensitive: matches!(model.state, ViewState::Loaded { .. }),
                            connect_clicked[sender] => move |_| {
                                sender.input(AppMsg::ZoomIn);
                            },
                        },
                    },
                },

                // Tools bar — drawing tools on the left, font controls
                // on the right (visible only when text-mode is active).
                add_top_bar = &gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    set_margin_top: 4,
                    set_margin_bottom: 4,
                    set_margin_start: 12,
                    set_margin_end: 12,
                    add_css_class: "toolbar",
                    #[watch]
                    set_visible: model.state.is_loaded(),

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 0,
                        add_css_class: "linked",

                        gtk::ToggleButton {
                            set_icon_name: "previewer-pan-symbolic",
                            set_tooltip_text: Some("Pan / select (no drawing)"),
                            #[watch]
                            set_active: model.tool == Tool::Pan,
                            connect_clicked[sender] => move |_| {
                                sender.input(AppMsg::SelectTool(Tool::Pan));
                            },
                        },
                        gtk::MenuButton {
                            set_icon_name: "previewer-shapes-symbolic",
                            set_tooltip_text: Some(
                                "Draw shape — rectangle, ellipse, line, or arrow",
                            ),
                            #[watch]
                            set_css_classes: if model.tool.is_draw_shape() {
                                &["suggested-action"]
                            } else {
                                &[]
                            },
                            #[wrap(Some)]
                            set_popover = &gtk::Popover {
                                gtk::Box {
                                    set_orientation: gtk::Orientation::Vertical,
                                    set_spacing: 4,
                                    set_margin_top: 6,
                                    set_margin_bottom: 6,
                                    set_margin_start: 6,
                                    set_margin_end: 6,

                                    gtk::Button {
                                        set_label: "▭   Rectangle",
                                        add_css_class: "flat",
                                        connect_clicked[sender] => move |btn| {
                                            sender.input(AppMsg::SelectTool(Tool::Rect));
                                            popdown_ancestor(btn);
                                        },
                                    },
                                    gtk::Button {
                                        set_label: "◯   Ellipse",
                                        add_css_class: "flat",
                                        connect_clicked[sender] => move |btn| {
                                            sender.input(AppMsg::SelectTool(Tool::Ellipse));
                                            popdown_ancestor(btn);
                                        },
                                    },
                                    gtk::Button {
                                        set_label: "─   Line",
                                        add_css_class: "flat",
                                        connect_clicked[sender] => move |btn| {
                                            sender.input(AppMsg::SelectTool(Tool::Line));
                                            popdown_ancestor(btn);
                                        },
                                    },
                                    gtk::Button {
                                        set_label: "→   Arrow",
                                        add_css_class: "flat",
                                        connect_clicked[sender] => move |btn| {
                                            sender.input(AppMsg::SelectTool(Tool::Arrow));
                                            popdown_ancestor(btn);
                                        },
                                    },
                                    gtk::Button {
                                        set_label: "↔   Double arrow",
                                        add_css_class: "flat",
                                        connect_clicked[sender] => move |btn| {
                                            sender.input(AppMsg::SelectTool(Tool::DoubleArrow));
                                            popdown_ancestor(btn);
                                        },
                                    },
                                },
                            },
                        },
                        gtk::ToggleButton {
                            set_tooltip_text: Some("Highlight"),
                            #[watch]
                            set_active: model.tool == Tool::Highlight,
                            #[wrap(Some)]
                            set_child = &gtk::Label {
                                set_use_markup: true,
                                set_markup: "<span background=\"#ffeb00\" foreground=\"#222\"><b> A </b></span>",
                            },
                            connect_clicked[sender] => move |_| {
                                sender.input(AppMsg::SelectTool(Tool::Highlight));
                            },
                        },
                        gtk::ToggleButton {
                            set_label: "Aa",
                            set_tooltip_text: Some("Text — click on the page, then type"),
                            #[watch]
                            set_active: model.tool == Tool::Text,
                            connect_clicked[sender] => move |_| {
                                sender.input(AppMsg::SelectTool(Tool::Text));
                            },
                        },
                        #[name = "sign_button"]
                        gtk::Button {
                            set_icon_name: "previewer-sign-symbolic",
                            set_tooltip_text: Some("Sign — pick a signature to place on the page"),
                            #[watch]
                            set_visible: model.state.is_pdf(),
                            #[watch]
                            set_css_classes: if model.active_signature.is_some() {
                                &["suggested-action"]
                            } else {
                                &[]
                            },
                            connect_clicked[sender] => move |btn| {
                                let s = sender.clone();
                                signature_manager::open_picker(btn, move |sig| {
                                    s.input(AppMsg::SelectSignature(sig));
                                });
                            },
                        },
                    },

                    gtk::Button {
                        set_icon_name: "user-info-symbolic",
                        set_tooltip_text: Some("Manage signatures…"),
                        set_margin_start: 4,
                        connect_clicked[sender] => move |_| {
                            sender.input(AppMsg::OpenSignatureManager);
                        },
                    },

                    // Spring spacer pushes the font controls to the right.
                    gtk::Box {
                        set_hexpand: true,
                    },

                    // Font controls — visible only when text-mode is in
                    // play (Text tool, an active inline edit, or a
                    // FreeText annotation selected).
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 6,
                        #[watch]
                        set_visible: model.font_bar_visible(),

                        gtk::Label { set_label: "Font" },
                        #[name = "font_family_dd"]
                        gtk::DropDown::from_strings(FONT_FAMILIES) {
                            set_tooltip_text: Some("Font family (PDF standard typefaces)"),
                            #[watch]
                            set_selected: family_index(&model.effective_font().family),
                            connect_selected_notify[sender] => move |dd| {
                                let i = dd.selected() as usize;
                                if let Some(name) = FONT_FAMILIES.get(i) {
                                    sender.input(AppMsg::SetFontFamily((*name).to_string()));
                                }
                            },
                        },
                        gtk::Label { set_label: "Size", set_margin_start: 6 },
                        #[name = "font_size_spin"]
                        gtk::SpinButton::with_range(4.0, 144.0, 1.0) {
                            set_digits: 0,
                            set_numeric: true,
                            set_tooltip_text: Some("Font size in points"),
                            #[watch]
                            set_value: model.effective_font().size,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(AppMsg::SetFontSize(sb.value()));
                            },
                        },
                        gtk::Label { set_label: "Color", set_margin_start: 6 },
                        #[name = "font_color_btn"]
                        gtk::ColorDialogButton {
                            set_dialog: &gtk::ColorDialog::builder()
                                .title("Pick text color")
                                .with_alpha(true)
                                .build(),
                            set_tooltip_text: Some("Text color"),
                            #[watch]
                            set_rgba: &color_to_rgba(model.effective_font_color()),
                            connect_rgba_notify[sender] => move |b| {
                                sender.input(AppMsg::SetFontColor(rgba_to_color(&b.rgba())));
                            },
                        },
                    },

                    // Stroke controls — defaults whenever a doc is loaded;
                    // mirror the selected shape's stroke when one is
                    // selected. Hidden while the font controls take the
                    // slot (text-mode).
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 6,
                        #[watch]
                        set_visible: model.stroke_bar_visible(),

                        gtk::Label { set_label: "Stroke" },
                        #[name = "stroke_color_btn"]
                        gtk::ColorDialogButton {
                            set_dialog: &gtk::ColorDialog::builder()
                                .title("Pick stroke color")
                                .with_alpha(true)
                                .build(),
                            set_tooltip_text: Some("Stroke color"),
                            #[watch]
                            set_rgba: &color_to_rgba(model.effective_stroke().color),
                            connect_rgba_notify[sender] => move |b| {
                                sender.input(AppMsg::SetStrokeColor(rgba_to_color(&b.rgba())));
                            },
                        },
                        gtk::Label { set_label: "Width", set_margin_start: 6 },
                        #[name = "stroke_width_spin"]
                        gtk::SpinButton::with_range(0.5, 32.0, 0.5) {
                            set_digits: 1,
                            set_numeric: true,
                            set_tooltip_text: Some("Stroke width in points"),
                            #[watch]
                            set_value: model.effective_stroke().width,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(AppMsg::SetStrokeWidth(sb.value()));
                            },
                        },
                        gtk::Label { set_label: "Style", set_margin_start: 6 },
                        #[name = "stroke_style_dd"]
                        gtk::DropDown::from_strings(STROKE_STYLE_LABELS) {
                            set_tooltip_text: Some("Line style"),
                            #[watch]
                            set_selected: stroke_style_index(model.effective_stroke().style),
                            connect_selected_notify[sender] => move |dd| {
                                if let Some(s) = stroke_style_from_index(dd.selected()) {
                                    sender.input(AppMsg::SetStrokeStyle(s));
                                }
                            },
                        },
                    },
                },

                #[wrap(Some)]
                set_content = &gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 0,

                    #[name = "sidebar_scroll"]
                    gtk::ScrolledWindow {
                        set_size_request: (160, -1),
                        set_hscrollbar_policy: gtk::PolicyType::Never,
                        set_vscrollbar_policy: gtk::PolicyType::Automatic,
                        add_css_class: "background",
                        #[watch]
                        set_visible: model.show_sidebar && model.state.is_pdf(),

                        #[name = "sidebar_list"]
                        gtk::ListBox {
                            set_selection_mode: gtk::SelectionMode::Single,
                            add_css_class: "navigation-sidebar",
                        },
                    },

                    gtk::Stack {
                    set_transition_type: gtk::StackTransitionType::Crossfade,
                    set_hexpand: true,
                    set_vexpand: true,

                    #[watch]
                    set_visible_child_name: model.state.page(),

                    add_named[Some("empty")] = &adw::StatusPage {
                        set_icon_name: Some("image-x-generic-symbolic"),
                        set_title: "Previewer",
                        set_description: Some("Open an image to get started."),
                    },

                    #[name = "loaded_scroll"]
                    add_named[Some("loaded")] = &gtk::ScrolledWindow {
                        set_hexpand: true,
                        set_vexpand: true,
                        set_hscrollbar_policy: gtk::PolicyType::Automatic,
                        set_vscrollbar_policy: gtk::PolicyType::Automatic,

                        gtk::Overlay {
                            set_halign: gtk::Align::Center,
                            set_valign: gtk::Align::Center,

                            #[wrap(Some)]
                            #[name = "overlay_area"]
                            set_child = &gtk::DrawingArea {
                                set_halign: gtk::Align::Fill,
                                set_valign: gtk::Align::Fill,
                                set_hexpand: true,
                                set_vexpand: true,
                                set_focusable: true,
                                set_can_focus: true,
                                #[watch]
                                set_width_request: model.picture_width(),
                                #[watch]
                                set_height_request: model.picture_height(),
                            },

                            #[name = "text_layer"]
                            add_overlay = &gtk::Fixed {
                                set_halign: gtk::Align::Fill,
                                set_valign: gtk::Align::Fill,
                                // Pass-through by default — only the inline
                                // TextView (when present) intercepts clicks.
                                set_can_target: false,
                            },
                        },
                    },

                    add_named[Some("error")] = &adw::StatusPage {
                        set_icon_name: Some("dialog-error-symbolic"),
                        set_title: "Could not open file",
                        #[watch]
                        set_description: Some(&model.error_description()),
                    },
                    },  // Stack
                },  // content Box
            },
        }
    }

    fn init(
        initial_path: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let settings = Settings::load();
        let mut model = AppModel {
            undo: UndoStack::new(),
            current_font: FontSpec::default(),
            current_font_color: Color::BLACK,
            current_stroke: Stroke::new(Color::RED, 2.0),
            inplace_save_confirmed: HashSet::new(),
            state: ViewState::Empty,
            surface: None,
            original_size: (1, 1),
            rotation_quarters: 0,
            zoom: 1.0,
            zoom_mode: ZoomMode::Manual,
            pdf_doc: None,
            pdf_page: 0,
            search_active: false,
            search_query: String::new(),
            search_matches: Vec::new(),
            search_index: None,
            active_signature: None,
            annotations: AnnotationLayer::new(),
            tool: Tool::Pan,
            draft: None,
            draft_start: None,
            selected: None,
            drag: None,
            pending_text_prompt: std::cell::RefCell::new(None),
            currently_editing_index: Cell::new(None),
            pending_search_focus: Cell::new(false),
            show_sidebar: settings.show_sidebar,
            sidebar_dirty: Cell::new(false),
            surface_scale_factor: 1,
            scroll_window: None,
            tool_signal: Rc::new(Cell::new(Tool::Pan)),
            selected_signal: Rc::new(Cell::new(false)),
            active_signature_signal: Rc::new(Cell::new(false)),
        };
        let widgets = view_output!();
        model.scroll_window = Some(widgets.loaded_scroll.clone());

        // Pan-drag attached to the ScrolledWindow itself. Its coord frame is
        // stable (the ScrolledWindow doesn't move while its child scrolls),
        // so the gesture's offset doesn't feed back into more scrolling.
        // Gated by the mirrored `tool_signal` / `selected_signal` /
        // `active_signature_signal` so it doesn't fight with the overlay's
        // annotation drag or with signature placement.
        let pan_drag = gtk::GestureDrag::new();
        pan_drag.set_button(gdk::BUTTON_PRIMARY);
        let scroll = widgets.loaded_scroll.clone();
        let tool_sig = model.tool_signal.clone();
        let selected_sig = model.selected_signal.clone();
        let active_sig = model.active_signature_signal.clone();
        let pan_init: Rc<Cell<Option<(f64, f64)>>> = Rc::new(Cell::new(None));
        {
            let scroll = scroll.clone();
            let pan_init = pan_init.clone();
            pan_drag.connect_drag_begin(move |_, _, _| {
                let active_pan =
                    tool_sig.get() == Tool::Pan && !selected_sig.get() && !active_sig.get();
                if active_pan {
                    pan_init.set(Some((
                        scroll.hadjustment().value(),
                        scroll.vadjustment().value(),
                    )));
                } else {
                    pan_init.set(None);
                }
            });
        }
        {
            let scroll = scroll.clone();
            let pan_init = pan_init.clone();
            pan_drag.connect_drag_update(move |_, dx, dy| {
                if let Some((init_h, init_v)) = pan_init.get() {
                    let h = scroll.hadjustment();
                    let v = scroll.vadjustment();
                    let max_h = (h.upper() - h.page_size()).max(h.lower());
                    let max_v = (v.upper() - v.page_size()).max(v.lower());
                    h.set_value((init_h - dx).clamp(h.lower(), max_h));
                    v.set_value((init_v - dy).clamp(v.lower(), max_v));
                }
            });
        }
        {
            let pan_init = pan_init.clone();
            pan_drag.connect_drag_end(move |_, _, _| {
                pan_init.set(None);
            });
        }
        widgets.loaded_scroll.add_controller(pan_drag);

        // Wire shortcuts on the root window.
        root.add_controller(install_shortcuts(sender.clone()));

        // Wire click-drag drawing on the overlay.
        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        let s = sender.clone();
        let area_for_focus = widgets.overlay_area.clone();
        let scroll_for_focus = widgets.loaded_scroll.clone();
        drag.connect_drag_begin(move |_, x, y| {
            // Pull focus to the DrawingArea so any active TextView in the
            // overlay loses focus → its focus-leave handler commits the
            // text. Without this, the user is stuck inside the inline
            // editor.
            //
            // GTK's ScrolledWindow auto-scrolls to the newly-focused child;
            // since the DrawingArea is far taller than the viewport, that
            // snaps the view to its top edge — losing the user's scroll
            // position even though the click happened in-view. Snapshot
            // the scroll offsets, grab focus, then restore on the next
            // idle tick so any scroll triggered by the focus change is
            // undone before the next paint.
            let v_adj = scroll_for_focus.vadjustment();
            let h_adj = scroll_for_focus.hadjustment();
            let saved_v = v_adj.value();
            let saved_h = h_adj.value();
            area_for_focus.grab_focus();
            glib::source::idle_add_local_once(move || {
                v_adj.set_value(saved_v);
                h_adj.set_value(saved_h);
            });
            s.input(AppMsg::DrawBegin { wx: x, wy: y });
        });
        let s = sender.clone();
        drag.connect_drag_update(move |g, dx, dy| {
            if let Some((sx, sy)) = g.start_point() {
                s.input(AppMsg::DrawUpdate {
                    wx: sx + dx,
                    wy: sy + dy,
                });
            }
        });
        let s = sender.clone();
        drag.connect_drag_end(move |_, _, _| {
            s.input(AppMsg::DrawCommit);
        });
        widgets.overlay_area.add_controller(drag);

        // Double-click → re-edit selected FreeText.
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        let s = sender.clone();
        click.connect_pressed(move |_, n_press, _, _| {
            if n_press == 2 {
                s.input(AppMsg::EditSelectedText);
            }
        });
        widgets.overlay_area.add_controller(click);

        // Return key on the overlay → re-edit selected FreeText. (Doesn't
        // fire when the inline TextView has focus — TextView captures Return
        // for newline insertion.)
        let key_ctl = gtk::EventControllerKey::new();
        let s = sender.clone();
        key_ctl.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Return || key == gdk::Key::KP_Enter {
                s.input(AppMsg::EditSelectedText);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        widgets.overlay_area.add_controller(key_ctl);

        // Pick up the initial HiDPI scale factor + watch it for changes
        // (window dragged onto another monitor, fractional-scaling toggle).
        // The notify signal also fires once after first realisation, which
        // is when the widget actually knows its real scale on Wayland.
        let initial_scale = widgets.overlay_area.scale_factor();
        if initial_scale > 1 {
            sender.input(AppMsg::ScaleFactorChanged(initial_scale));
        }
        let s = sender.clone();
        widgets.overlay_area.connect_scale_factor_notify(move |w| {
            s.input(AppMsg::ScaleFactorChanged(w.scale_factor()));
        });

        // Watch the ScrolledWindow's horizontal viewport size — fit-to-
        // width recomputes whenever the pane changes width (window
        // resize, sidebar toggle, etc.).
        let s = sender.clone();
        widgets
            .loaded_scroll
            .hadjustment()
            .connect_page_size_notify(move |_| {
                s.input(AppMsg::ViewportResized);
            });

        // Wire the draw_func. Re-installed on each update so it captures the
        // current annotations + transform.
        refresh_overlay_draw(&widgets.overlay_area, &model);

        if let Some(path) = initial_path {
            sender.input(AppMsg::LoadPath(path));
        }

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            AppMsg::OpenDialog => {
                let dialog = gtk::FileDialog::new();
                dialog.set_title("Open image");
                dialog.set_modal(true);
                dialog.set_filters(Some(&image_filters()));

                let parent = relm4::main_application().active_window();
                dialog.open(
                    parent.as_ref(),
                    gio::Cancellable::NONE,
                    move |result| match result {
                        Ok(file) => {
                            if let Some(path) = file.path() {
                                sender.input(AppMsg::LoadPath(path));
                            }
                        }
                        Err(e) => tracing::debug!(?e, "file dialog dismissed"),
                    },
                );
            }
            AppMsg::LoadPath(path) => {
                self.rotation_quarters = 0;
                self.zoom = 1.0;
                // Default to fit-to-width on every fresh open. The actual
                // zoom factor is computed once the loaded ScrolledWindow
                // reports its viewport size — either right below at the
                // end of the success branch, or via the ViewportResized
                // signal once GTK allocates the widget.
                self.zoom_mode = ZoomMode::FitWidth;
                self.draft = None;
                self.draft_start = None;
                self.annotations = AnnotationLayer::new();
                self.undo.clear();
                self.pdf_doc = None;
                self.pdf_page = 0;
                self.search_query.clear();
                self.search_matches.clear();
                self.search_index = None;
                self.search_active = false;

                if is_pdf_path(&path) {
                    match PdfDoc::open(&path) {
                        Ok(mut doc) => match doc.render_page(0, 1.0) {
                            Ok(rendered) => {
                                self.original_size = rendered.dimensions();
                                self.surface = Some(surface_from_rendered(&rendered));
                                // Seed `self.annotations` with the editable
                                // subset of pre-existing /Annots; pdfium
                                // strips those from its in-memory doc so a
                                // re-save doesn't duplicate them.
                                let extracted = doc.extract_annotations();
                                if !extracted.is_empty() {
                                    tracing::info!(
                                        count = extracted.len(),
                                        "loaded existing PDF annotations"
                                    );
                                    self.annotations = AnnotationLayer { items: extracted };
                                    // Re-render with the extracted ones now
                                    // gone from pdfium's view, so the
                                    // overlay paint is the only place they
                                    // show up (and so the user can edit).
                                    if let Ok(rerendered) = doc.render_page(0, 1.0) {
                                        self.surface = Some(surface_from_rendered(&rerendered));
                                    }
                                }
                                self.pdf_doc = Some(doc);
                                self.pdf_page = 0;
                                self.state = ViewState::Loaded {
                                    path,
                                    kind: DocumentKind::Pdf,
                                };
                                // Apply fit-to-width if the viewport is
                                // already sized; otherwise the
                                // ViewportResized signal will catch up
                                // once GTK allocates the ScrolledWindow.
                                if let Some(z) = self.fit_width_zoom() {
                                    self.zoom = z;
                                }
                                // The 1× rasters above seed annotation
                                // coords; immediately re-render at the
                                // current display scale (incl. HiDPI) so
                                // the page is crisp from the first frame.
                                self.refresh_pdf_render();
                                // Sidebar thumbnails are document-scoped;
                                // post_view will rebuild when it sees the
                                // dirty flag.
                                self.sidebar_dirty.set(true);
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "PDF render failed");
                                self.surface = None;
                                self.original_size = (1, 1);
                                self.state = ViewState::Error {
                                    path,
                                    message: e.to_string(),
                                };
                            }
                        },
                        Err(e) => {
                            tracing::warn!(path = %path.display(), error = %e, "PDF open failed");
                            self.surface = None;
                            self.original_size = (1, 1);
                            self.state = ViewState::Error {
                                path,
                                message: e.to_string(),
                            };
                        }
                    }
                } else {
                    match decode_to_rgba(&path) {
                        Ok(image) => {
                            self.original_size = image.dimensions();
                            self.surface = Some(surface_from_rgba(&image));

                            let sidecar = sidecar_path(&path);
                            self.annotations = if sidecar.exists() {
                                match load_layer(&sidecar) {
                                    Ok(layer) => {
                                        tracing::info!(
                                            sidecar = %sidecar.display(),
                                            items = layer.len(),
                                            "loaded sidecar annotations"
                                        );
                                        layer
                                    }
                                    Err(e) => {
                                        tracing::warn!(error = %e, "sidecar present but failed to load");
                                        AnnotationLayer::new()
                                    }
                                }
                            } else {
                                AnnotationLayer::new()
                            };

                            self.state = ViewState::Loaded {
                                path,
                                kind: DocumentKind::Image,
                            };
                            if let Some(z) = self.fit_width_zoom() {
                                self.zoom = z;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(path = %path.display(), error = %e, "decode failed");
                            self.surface = None;
                            self.original_size = (1, 1);
                            self.state = ViewState::Error {
                                path,
                                message: e.to_string(),
                            };
                        }
                    }
                }
            }
            AppMsg::ZoomIn => {
                if matches!(self.state, ViewState::Loaded { .. }) {
                    self.zoom = (self.zoom * ZOOM_STEP).min(ZOOM_MAX);
                    self.zoom_mode = ZoomMode::Manual;
                    if self.state.is_pdf() {
                        self.refresh_pdf_render();
                    }
                }
            }
            AppMsg::ZoomOut => {
                if matches!(self.state, ViewState::Loaded { .. }) {
                    self.zoom = (self.zoom / ZOOM_STEP).max(ZOOM_MIN);
                    self.zoom_mode = ZoomMode::Manual;
                    if self.state.is_pdf() {
                        self.refresh_pdf_render();
                    }
                }
            }
            AppMsg::ZoomReset => {
                if matches!(self.state, ViewState::Loaded { .. }) {
                    self.zoom = 1.0;
                    self.zoom_mode = ZoomMode::Manual;
                    if self.state.is_pdf() {
                        self.refresh_pdf_render();
                    }
                }
            }
            AppMsg::ZoomToggleFitWidth => {
                if !matches!(self.state, ViewState::Loaded { .. }) {
                    return;
                }
                match self.zoom_mode {
                    ZoomMode::FitWidth => {
                        // Currently fit-width → exit to a manual 100% (the
                        // "[1] reset zoom" affordance the button shows in
                        // this state).
                        self.zoom = 1.0;
                        self.zoom_mode = ZoomMode::Manual;
                    }
                    ZoomMode::Manual => {
                        self.zoom_mode = ZoomMode::FitWidth;
                        if let Some(z) = self.fit_width_zoom() {
                            self.zoom = z;
                        }
                    }
                }
                if self.state.is_pdf() {
                    self.refresh_pdf_render();
                }
            }
            AppMsg::ViewportResized => {
                if matches!(self.zoom_mode, ZoomMode::FitWidth)
                    && matches!(self.state, ViewState::Loaded { .. })
                    && let Some(z) = self.fit_width_zoom()
                    && (z - self.zoom).abs() > 0.001
                {
                    self.zoom = z;
                    if self.state.is_pdf() {
                        self.refresh_pdf_render();
                    }
                }
            }
            AppMsg::RotateRight => {
                if matches!(self.state, ViewState::Loaded { .. }) {
                    self.rotation_quarters = (self.rotation_quarters + 1) % 4;
                    // No surface rebuild — ViewTransform applies the rotation at draw time.
                }
            }
            AppMsg::SelectTool(tool) => {
                self.tool = tool;
                self.draft = None;
                self.draft_start = None;
                self.selected = None;
                self.drag = None;
                // Picking any tool implicitly disarms a queued signature —
                // otherwise click-drag would still place stamps.
                self.active_signature = None;
            }
            AppMsg::DrawBegin { wx, wy } => {
                if !self.state.is_loaded() {
                    return;
                }
                let p = self.transform().widget_to_image(Point::new(wx, wy));

                // Sig placement always wins.
                if let Some(sig) = self.active_signature.clone() {
                    self.draft_start = Some(p);
                    self.draft = Some(signature_to_annotation(
                        &sig,
                        self.current_page(),
                        Rect::from_corners(p, p),
                    ));
                    return;
                }

                // Selection: handle of currently selected first, then any body.
                let cp = self.current_page();
                let tol = 4.0;
                if let Some(idx) = self.selected
                    && let Some(ann) = self.annotations.items.get(idx)
                    && ann.page() == cp
                    && let Some(HitKind::Handle(h)) = hit_test(ann, p, tol, true)
                {
                    // Snapshot before the drag begins so Ctrl+Z reverts the
                    // entire move/resize gesture, not each intermediate frame.
                    self.undo.push(self.annotations.clone());
                    self.drag = Some(DragState {
                        kind: DragKind::Resize(h),
                        origin: p,
                        original: ann.clone(),
                        index: idx,
                    });
                    return;
                }
                for (i, ann) in self.annotations.items.iter().enumerate().rev() {
                    if ann.page() != cp {
                        continue;
                    }
                    if hit_test(ann, p, tol, false).is_some() {
                        self.selected = Some(i);
                        self.undo.push(self.annotations.clone());
                        self.drag = Some(DragState {
                            kind: DragKind::Move,
                            origin: p,
                            original: ann.clone(),
                            index: i,
                        });
                        return;
                    }
                }

                // No annotation hit — clear selection.
                self.selected = None;

                // Pan tool with no annotation hit: pan is handled by a
                // separate GestureDrag on the ScrolledWindow (stable coord
                // frame). Just bail out here.
                if self.tool == Tool::Pan {
                    return;
                }
                if self.tool == Tool::Text {
                    // Drop a placeholder annotation at the click point and
                    // auto-select it. The user gets immediate visual
                    // feedback (a dim "Enter some text") and the font bar
                    // stays live so they can adjust styling before typing.
                    // Double-click (or Return) opens the inline editor and
                    // strips the placeholder; an empty commit restores it.
                    let placeholder = Annotation::FreeText {
                        page: cp,
                        position: p,
                        text: PLACEHOLDER_TEXT.to_string(),
                        font: self.current_font.clone(),
                        color: self.current_font_color,
                        is_placeholder: true,
                    };
                    self.undo.push(self.annotations.clone());
                    self.annotations.push(placeholder);
                    self.selected = Some(self.annotations.items.len() - 1);
                    self.tool = Tool::Pan;
                    return;
                }
                self.draft_start = Some(p);
                self.draft = Some(make_draft(self.tool, cp, p, p, &self.current_stroke));
            }
            AppMsg::DrawUpdate { wx, wy } => {
                let p = self.transform().widget_to_image(Point::new(wx, wy));
                // Active drag on a selected annotation takes priority.
                if let Some(drag) = self.drag.clone() {
                    let dx = p.x - drag.origin.x;
                    let dy = p.y - drag.origin.y;
                    let updated = apply_drag(&drag.original, drag.kind, dx, dy);
                    if let Some(slot) = self.annotations.items.get_mut(drag.index) {
                        *slot = updated;
                    }
                    return;
                }
                if let Some(start) = self.draft_start {
                    if let Some(sig) = self.active_signature.clone() {
                        self.draft = Some(signature_to_annotation(
                            &sig,
                            self.current_page(),
                            Rect::from_corners(start, p),
                        ));
                    } else {
                        self.draft = Some(make_draft(
                            self.tool,
                            self.current_page(),
                            start,
                            p,
                            &self.current_stroke,
                        ));
                    }
                }
            }
            AppMsg::DrawCommit => {
                if self.drag.take().is_some() {
                    // Drag finished; the annotation was already mutated in
                    // place and we already pushed a snapshot at DrawBegin.
                    return;
                }
                if let Some(draft) = self.draft.take()
                    && !is_degenerate(&draft)
                {
                    self.undo.push(self.annotations.clone());
                    self.annotations.push(draft);
                    // Auto-select the freshly added annotation and drop back
                    // to Pan so the next click edits this selection rather
                    // than re-triggering the placement / drawing tool. To
                    // place another shape or signature, the user re-picks
                    // the tool / signature.
                    self.selected = Some(self.annotations.items.len() - 1);
                    self.tool = Tool::Pan;
                    self.active_signature = None;
                }
                self.draft_start = None;
            }
            AppMsg::DrawCancel => {
                self.draft = None;
                self.draft_start = None;
                self.drag = None;
                self.selected = None;
                self.active_signature = None;
            }
            AppMsg::DeleteSelected => {
                if let Some(idx) = self.selected.take() {
                    if idx < self.annotations.items.len() {
                        self.undo.push(self.annotations.clone());
                        self.annotations.items.remove(idx);
                    }
                    self.drag = None;
                }
            }
            AppMsg::CommitFreeText {
                image_x,
                image_y,
                text,
                replace_index,
            } => {
                self.currently_editing_index.set(None);
                let trimmed_empty = text.trim().is_empty();
                match replace_index {
                    Some(idx) => {
                        // Re-edit branch — the inline editor was opened on
                        // an existing annotation. Behaviour depends on
                        // whether it was a fresh placeholder (in which case
                        // an empty exit restores "Enter some text" and
                        // keeps the placeholder live) or real content
                        // (empty exit deletes).
                        let was_placeholder = matches!(
                            self.annotations.items.get(idx),
                            Some(Annotation::FreeText {
                                is_placeholder: true,
                                ..
                            })
                        );
                        self.undo.push(self.annotations.clone());
                        if trimmed_empty {
                            if was_placeholder {
                                if let Some(Annotation::FreeText { text: t, .. }) =
                                    self.annotations.items.get_mut(idx)
                                {
                                    *t = PLACEHOLDER_TEXT.to_string();
                                }
                                self.selected = Some(idx);
                            } else if idx < self.annotations.items.len() {
                                self.annotations.items.remove(idx);
                            }
                        } else if let Some(Annotation::FreeText {
                            text: t,
                            position: pos,
                            is_placeholder: ip,
                            ..
                        }) = self.annotations.items.get_mut(idx)
                        {
                            *t = text;
                            pos.x = image_x;
                            pos.y = image_y;
                            *ip = false;
                            self.selected = Some(idx);
                        }
                    }
                    None => {
                        // Legacy "click + immediately type" entry-point.
                        // The current Tool::Text flow drops a placeholder
                        // annotation rather than spawning the editor inline,
                        // so this branch is only hit by external callers
                        // (none today). Kept for safety: if it ever fires
                        // with empty text we just no-op.
                        if trimmed_empty {
                            return;
                        }
                        let new_ann = Annotation::FreeText {
                            page: self.current_page(),
                            position: Point::new(image_x, image_y),
                            text,
                            font: self.current_font.clone(),
                            color: self.current_font_color,
                            is_placeholder: false,
                        };
                        self.undo.push(self.annotations.clone());
                        self.annotations.push(new_ann);
                        self.selected = Some(self.annotations.items.len() - 1);
                        self.tool = Tool::Pan;
                    }
                }
            }
            AppMsg::EditSelectedText => {
                let Some(idx) = self.selected else {
                    return;
                };
                let Some(Annotation::FreeText {
                    position,
                    text,
                    font,
                    color,
                    is_placeholder,
                    ..
                }) = self.annotations.items.get(idx).cloned()
                else {
                    return;
                };
                let widget = self.transform().image_to_widget(position);
                // First edit on a placeholder: open with an empty buffer so
                // the user types into a clean canvas. Subsequent edits
                // (after they've committed real content) keep prefilling
                // the existing text as before.
                let initial_text = if is_placeholder { String::new() } else { text };
                *self.pending_text_prompt.borrow_mut() = Some(TextPromptInfo {
                    widget_x: widget.x,
                    widget_y: widget.y,
                    image_x: position.x,
                    image_y: position.y,
                    initial_text,
                    replace_index: Some(idx),
                    font,
                    color,
                    zoom: self.zoom,
                });
                self.currently_editing_index.set(Some(idx));
            }
            AppMsg::EditCancelled => {
                self.currently_editing_index.set(None);
            }
            AppMsg::Undo => {
                if let Some(prev) = self.undo.pop_undo(self.annotations.clone()) {
                    self.annotations = prev;
                    self.selected = None;
                    self.drag = None;
                    self.draft = None;
                    self.draft_start = None;
                    self.currently_editing_index.set(None);
                }
            }
            AppMsg::Redo => {
                if let Some(next) = self.undo.pop_redo(self.annotations.clone()) {
                    self.annotations = next;
                    self.selected = None;
                    self.drag = None;
                    self.draft = None;
                    self.draft_start = None;
                    self.currently_editing_index.set(None);
                }
            }
            AppMsg::ScaleFactorChanged(scale) => {
                let scale = scale.max(1);
                if scale != self.surface_scale_factor {
                    self.surface_scale_factor = scale;
                    self.refresh_pdf_render();
                }
            }
            AppMsg::SetFontFamily(family) => {
                self.current_font.family = family.clone();
                if let Some(idx) = self.selected
                    && matches!(
                        self.annotations.items.get(idx),
                        Some(Annotation::FreeText { .. })
                    )
                {
                    self.undo
                        .push_coalesced(CoalesceKey::FontFamily(idx), self.annotations.clone());
                    if let Some(Annotation::FreeText { font, .. }) =
                        self.annotations.items.get_mut(idx)
                    {
                        font.family = family;
                    }
                }
                apply_inline_editor_style(
                    &self.effective_font(),
                    self.effective_font_color(),
                    self.zoom,
                );
            }
            AppMsg::SetFontSize(size) => {
                let size = size.clamp(4.0, 144.0);
                self.current_font.size = size;
                if let Some(idx) = self.selected
                    && matches!(
                        self.annotations.items.get(idx),
                        Some(Annotation::FreeText { .. })
                    )
                {
                    self.undo
                        .push_coalesced(CoalesceKey::FontSize(idx), self.annotations.clone());
                    if let Some(Annotation::FreeText { font, .. }) =
                        self.annotations.items.get_mut(idx)
                    {
                        font.size = size;
                    }
                }
                apply_inline_editor_style(
                    &self.effective_font(),
                    self.effective_font_color(),
                    self.zoom,
                );
            }
            AppMsg::SetFontColor(c) => {
                self.current_font_color = c;
                if let Some(idx) = self.selected
                    && matches!(
                        self.annotations.items.get(idx),
                        Some(Annotation::FreeText { .. })
                    )
                {
                    self.undo
                        .push_coalesced(CoalesceKey::FontColor(idx), self.annotations.clone());
                    if let Some(Annotation::FreeText { color, .. }) =
                        self.annotations.items.get_mut(idx)
                    {
                        *color = c;
                    }
                }
                apply_inline_editor_style(
                    &self.effective_font(),
                    self.effective_font_color(),
                    self.zoom,
                );
            }
            AppMsg::SetStrokeColor(c) => {
                self.current_stroke.color = c;
                if let Some(idx) = self.selected
                    && self.selected_shape_stroke().is_some()
                {
                    self.undo
                        .push_coalesced(CoalesceKey::StrokeColor(idx), self.annotations.clone());
                    if let Some(stroke) = stroke_at_mut(&mut self.annotations, idx) {
                        stroke.color = c;
                    }
                }
            }
            AppMsg::SetStrokeWidth(w) => {
                let w = w.clamp(0.5, 32.0);
                self.current_stroke.width = w;
                if let Some(idx) = self.selected
                    && self.selected_shape_stroke().is_some()
                {
                    self.undo
                        .push_coalesced(CoalesceKey::StrokeWidth(idx), self.annotations.clone());
                    if let Some(stroke) = stroke_at_mut(&mut self.annotations, idx) {
                        stroke.width = w;
                    }
                }
            }
            AppMsg::SetStrokeStyle(s) => {
                self.current_stroke.style = s;
                if let Some(idx) = self.selected
                    && self.selected_shape_stroke().is_some()
                {
                    self.undo
                        .push_coalesced(CoalesceKey::StrokeStyle(idx), self.annotations.clone());
                    if let Some(stroke) = stroke_at_mut(&mut self.annotations, idx) {
                        stroke.style = s;
                    }
                }
            }
            AppMsg::Save => match &self.state {
                ViewState::Loaded {
                    path,
                    kind: DocumentKind::Image,
                } => {
                    let sidecar = sidecar_path(path);
                    // Strip any in-flight placeholders before persisting —
                    // they're a UI hint, not user content.
                    let to_save = AnnotationLayer {
                        items: self
                            .annotations
                            .items
                            .iter()
                            .filter(|a| {
                                !matches!(
                                    a,
                                    Annotation::FreeText {
                                        is_placeholder: true,
                                        ..
                                    }
                                )
                            })
                            .cloned()
                            .collect(),
                    };
                    match save_layer(&to_save, &sidecar) {
                        Ok(()) => tracing::info!(
                            sidecar = %sidecar.display(),
                            items = to_save.len(),
                            "saved annotations"
                        ),
                        Err(e) => tracing::error!(error = %e, "failed to save annotations"),
                    }
                }
                ViewState::Loaded {
                    path,
                    kind: DocumentKind::Pdf,
                } => {
                    let path = path.clone();
                    if self.inplace_save_confirmed.contains(&path) {
                        if let Err(e) = self.save_pdf_to(&path) {
                            tracing::error!(error = %e, "PDF save failed");
                        }
                    } else {
                        prompt_inplace_save(&path, sender.clone());
                    }
                }
                _ => {}
            },
            AppMsg::ConfirmedSaveInPlace => {
                if let ViewState::Loaded {
                    path,
                    kind: DocumentKind::Pdf,
                } = &self.state
                {
                    let path = path.clone();
                    self.inplace_save_confirmed.insert(path.clone());
                    if let Err(e) = self.save_pdf_to(&path) {
                        tracing::error!(error = %e, "PDF save failed");
                    }
                }
            }
            AppMsg::SaveAs => {
                if !matches!(
                    self.state,
                    ViewState::Loaded {
                        kind: DocumentKind::Pdf,
                        ..
                    }
                ) {
                    return;
                }
                let dialog = gtk::FileDialog::new();
                dialog.set_title("Save PDF as");
                dialog.set_modal(true);
                dialog.set_filters(Some(&pdf_save_filters()));
                if let ViewState::Loaded { path, .. } = &self.state {
                    dialog.set_initial_name(Some(&suggest_save_as_name(path)));
                    if let Some(parent) = path.parent() {
                        dialog.set_initial_folder(Some(&gio::File::for_path(parent)));
                    }
                }
                let parent = relm4::main_application().active_window();
                let s = sender.clone();
                dialog.save(
                    parent.as_ref(),
                    gio::Cancellable::NONE,
                    move |result| match result {
                        Ok(file) => {
                            if let Some(path) = file.path() {
                                s.input(AppMsg::SaveAsTo(path));
                            }
                        }
                        Err(e) => tracing::debug!(?e, "save-as dialog dismissed"),
                    },
                );
            }
            AppMsg::SaveAsTo(target) => {
                if !matches!(
                    self.state,
                    ViewState::Loaded {
                        kind: DocumentKind::Pdf,
                        ..
                    }
                ) {
                    return;
                }
                if let Err(e) = self.save_pdf_to(&target) {
                    tracing::error!(error = %e, "PDF Save As failed");
                    return;
                }
                // Re-target subsequent saves at the new file. The new path is
                // inherently user-chosen, so no warning needed for in-place
                // saves of it later.
                self.inplace_save_confirmed.insert(target.clone());
                self.state = ViewState::Loaded {
                    path: target,
                    kind: DocumentKind::Pdf,
                };
            }
            AppMsg::NextPage => self.go_to_pdf_page(self.pdf_page.saturating_add(1)),
            AppMsg::PrevPage => self.go_to_pdf_page(self.pdf_page.saturating_sub(1)),
            AppMsg::FirstPage => self.go_to_pdf_page(0),
            AppMsg::LastPage => {
                if let Some(doc) = &self.pdf_doc {
                    self.go_to_pdf_page(doc.page_count().saturating_sub(1));
                }
            }
            AppMsg::GoToPage(p) => {
                self.go_to_pdf_page(p);
            }
            AppMsg::ToggleSidebar => {
                if self.state.is_pdf() {
                    self.show_sidebar = !self.show_sidebar;
                    self.save_settings();
                }
            }
            AppMsg::ToggleSearch => {
                // Search lives inline in the toolbar now (no popup bar to
                // toggle). Ctrl+F just grabs focus on the entry; post_view
                // does the actual `grab_focus` because the model can't
                // touch widgets directly.
                if self.state.is_pdf() {
                    self.search_active = true;
                    self.pending_search_focus.set(true);
                }
            }
            AppMsg::SearchQuery(q) => {
                self.search_query = q.clone();
                if let Some(doc) = &self.pdf_doc {
                    if q.is_empty() {
                        self.search_matches.clear();
                        self.search_index = None;
                    } else {
                        match doc.find_text(&q) {
                            Ok(matches) => {
                                self.search_matches = matches;
                                self.search_index = if self.search_matches.is_empty() {
                                    None
                                } else {
                                    Some(0)
                                };
                                if let Some(m) = self.search_matches.first() {
                                    self.go_to_pdf_page(m.page);
                                }
                            }
                            Err(e) => tracing::warn!(error = %e, "search failed"),
                        }
                    }
                }
            }
            AppMsg::SearchNext => {
                if !self.search_matches.is_empty() {
                    let next = self
                        .search_index
                        .map(|i| (i + 1) % self.search_matches.len())
                        .unwrap_or(0);
                    self.search_index = Some(next);
                    let target_page = self.search_matches[next].page;
                    self.go_to_pdf_page(target_page);
                }
            }
            AppMsg::SearchPrev => {
                if !self.search_matches.is_empty() {
                    let len = self.search_matches.len();
                    let prev = self
                        .search_index
                        .map(|i| (i + len - 1) % len)
                        .unwrap_or(len - 1);
                    self.search_index = Some(prev);
                    let target_page = self.search_matches[prev].page;
                    self.go_to_pdf_page(target_page);
                }
            }
            AppMsg::CloseSearch => {
                self.search_active = false;
                self.search_query.clear();
                self.search_matches.clear();
                self.search_index = None;
            }
            AppMsg::OpenSignatureManager => {
                if let Some(window) = relm4::main_application().active_window() {
                    signature_manager::open(&window);
                }
            }
            AppMsg::SelectSignature(sig) => {
                tracing::info!(name = %sig.name, "armed signature for placement");
                self.active_signature = Some(sig);
                self.draft = None;
                self.draft_start = None;
            }
        }
    }

    fn post_view() {
        // Mirror state into the signal cells so the pan gesture closure can
        // see "current" values without going through AppMsg.
        model.tool_signal.set(model.tool);
        model.selected_signal.set(model.selected.is_some());
        model
            .active_signature_signal
            .set(model.active_signature.is_some());

        let pending = model.pending_text_prompt.borrow_mut().take();
        if let Some(info) = pending {
            spawn_inline_text_editor(&widgets.text_layer, info, &sender);
        }
        if model.pending_search_focus.replace(false) {
            widgets.search_entry.grab_focus();
        }
        if model.sidebar_dirty.replace(false)
            && let Some(doc) = model.pdf_doc.as_ref()
        {
            populate_sidebar(&widgets.sidebar_list, doc, &sender);
        }
        sync_sidebar_selection(&widgets.sidebar_list, model.pdf_page);
        refresh_overlay_draw(&widgets.overlay_area, model);
    }
}

/// Get a mutable reference to the `Stroke` of the annotation at `idx`,
/// if it has one (Rect / Ellipse / Arrow). Used by stroke-bar handlers
/// that need to write back through a borrow they couldn't hold while
/// pushing to the undo stack.
fn stroke_at_mut(layer: &mut AnnotationLayer, idx: usize) -> Option<&mut Stroke> {
    match layer.items.get_mut(idx)? {
        Annotation::Rect { stroke, .. }
        | Annotation::Ellipse { stroke, .. }
        | Annotation::Arrow { stroke, .. } => Some(stroke),
        _ => None,
    }
}

fn make_draft(tool: Tool, page: u32, a: Point, b: Point, stroke: &Stroke) -> Annotation {
    let bbox = Rect::from_corners(a, b);
    match tool {
        Tool::Pan | Tool::Rect | Tool::Text => Annotation::Rect {
            page,
            bbox,
            stroke: stroke.clone(),
            fill: None,
        },
        Tool::Ellipse => Annotation::Ellipse {
            page,
            bbox,
            stroke: stroke.clone(),
            fill: None,
        },
        Tool::Arrow => Annotation::Arrow {
            page,
            from: a,
            to: b,
            stroke: stroke.clone(),
            ends: ArrowEnds::End,
        },
        Tool::DoubleArrow => Annotation::Arrow {
            page,
            from: a,
            to: b,
            stroke: stroke.clone(),
            ends: ArrowEnds::Both,
        },
        Tool::Line => Annotation::Arrow {
            page,
            from: a,
            to: b,
            stroke: stroke.clone(),
            ends: ArrowEnds::None,
        },
        Tool::Highlight => Annotation::Highlight {
            page,
            bbox,
            color: Color::rgba(255, 235, 0, 96),
        },
    }
}

fn is_degenerate(ann: &Annotation) -> bool {
    match ann {
        Annotation::Rect { bbox, .. }
        | Annotation::Ellipse { bbox, .. }
        | Annotation::Highlight { bbox, .. }
        | Annotation::Stamp { bbox, .. } => bbox.width < 2.0 || bbox.height < 2.0,
        Annotation::Arrow { from, to, .. } => {
            let dx = to.x - from.x;
            let dy = to.y - from.y;
            (dx * dx + dy * dy).sqrt() < 4.0
        }
        Annotation::FreeText { text, .. } => text.is_empty(),
        Annotation::Ink { strokes, .. } => strokes.iter().all(|s| s.len() < 2),
    }
}

fn signature_to_annotation(sig: &Signature, page: u32, bbox: Rect) -> Annotation {
    match &sig.kind {
        SignatureKind::Raster {
            width,
            height,
            pixels,
        } => Annotation::Stamp {
            page,
            bbox,
            image: StampImage {
                width: *width,
                height: *height,
                pixels: pixels.clone(),
            },
        },
        SignatureKind::Vector { strokes } => {
            let mapped = remap_vector_strokes_to_bbox(strokes, bbox);
            Annotation::Ink {
                page,
                strokes: mapped,
                color: Color::BLACK,
                width: 1.5,
            }
        }
    }
}

/// Map signature strokes (in their own canvas coords) into `target` rect on
/// the page, preserving aspect via fit-inside. Empty input → empty output.
fn remap_vector_strokes_to_bbox(
    strokes: &[previewer_signature::Stroke],
    target: Rect,
) -> Vec<Vec<Point>> {
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for s in strokes {
        for p in &s.points {
            if !p.x.is_finite() || !p.y.is_finite() {
                continue;
            }
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
        }
    }
    if !min_x.is_finite() {
        return Vec::new();
    }
    let src_w = (max_x - min_x).max(1.0);
    let src_h = (max_y - min_y).max(1.0);
    // Fit inside target while preserving aspect.
    let scale = (target.width / src_w).min(target.height / src_h);
    let mapped_w = src_w * scale;
    let mapped_h = src_h * scale;
    let off_x = target.x + (target.width - mapped_w) / 2.0;
    let off_y = target.y + (target.height - mapped_h) / 2.0;

    strokes
        .iter()
        .map(|s| {
            s.points
                .iter()
                .filter(|p| p.x.is_finite() && p.y.is_finite())
                .map(|p| Point::new(off_x + (p.x - min_x) * scale, off_y + (p.y - min_y) * scale))
                .collect()
        })
        .collect()
}

/// Spawn an inline TextView at the click point inside `text_layer` (a
/// `gtk::Fixed`). Commits on focus-out, cancels on Esc.
fn spawn_inline_text_editor(
    text_layer: &gtk::Fixed,
    info: TextPromptInfo,
    sender: &ComponentSender<AppModel>,
) {
    use std::cell::Cell as StdCell;
    use std::rc::Rc;

    let buffer = gtk::TextBuffer::new(None);
    if !info.initial_text.is_empty() {
        buffer.set_text(&info.initial_text);
    }
    let text_view = gtk::TextView::with_buffer(&buffer);
    text_view.set_wrap_mode(gtk::WrapMode::WordChar);
    text_view.set_left_margin(0);
    text_view.set_right_margin(0);
    text_view.set_top_margin(0);
    text_view.set_bottom_margin(0);
    text_view.set_pixels_above_lines(0);
    text_view.set_pixels_below_lines(0);
    text_view.set_monospace(false);
    // Apply our transparent-bg + dynamic-style CSS class so what you see
    // while typing matches what gets saved to the PDF.
    text_view.add_css_class("previewer-inline-text");
    apply_inline_editor_style(&info.font, info.color, info.zoom);
    // Size the entry box from the actual font/zoom so the glyphs aren't
    // clipped at large sizes and don't float in a tiny strip at small
    // ones. Same heuristics as `freetext_bbox_size` (0.6 char-w, 1.4
    // line-h) but scaled into widget pixels.
    let lines = info.initial_text.split('\n').count().max(1);
    let max_chars = info
        .initial_text
        .split('\n')
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0)
        .max(20);
    let char_w = info.font.size * 0.6 * info.zoom;
    let line_h = info.font.size * 1.4 * info.zoom;
    let init_w = ((max_chars as f64) * char_w).max(60.0) as i32;
    let init_h = ((lines as f64) * line_h).max(line_h) as i32;
    text_view.set_size_request(init_w, init_h);

    text_layer.put(&text_view, info.widget_x, info.widget_y);
    let replace_index = info.replace_index;

    // `finalised` plus deferred removal makes the editor robust against
    // re-entrant calls and the "remove during event handler" GTK assert.
    let finalised = Rc::new(StdCell::new(false));

    let finish = {
        let finalised = finalised.clone();
        let text_layer = text_layer.clone();
        let text_view = text_view.clone();
        let buffer = buffer.clone();
        let sender = sender.clone();
        move |commit: bool| {
            if finalised.replace(true) {
                return;
            }
            if commit {
                let start = buffer.start_iter();
                let end = buffer.end_iter();
                let text = buffer.text(&start, &end, false).to_string();
                if !text.trim().is_empty() {
                    sender.input(AppMsg::CommitFreeText {
                        image_x: info.image_x,
                        image_y: info.image_y,
                        text,
                        replace_index,
                    });
                } else if replace_index.is_some() {
                    // User wiped out a re-edit's text — that means delete.
                    sender.input(AppMsg::DeleteSelected);
                }
            } else {
                sender.input(AppMsg::EditCancelled);
            }
            // Defer the actual widget removal to the next idle tick so we're
            // not mutating the widget tree from inside a focus / key handler
            // (which can trip GTK's parent-pointer assertions on Wayland).
            let text_layer = text_layer.clone();
            let text_view = text_view.clone();
            glib::source::idle_add_local_once(move || {
                if text_view.parent().is_some() {
                    text_layer.remove(&text_view);
                }
            });
        }
    };

    // Esc → commit and exit edit mode (annotation stays selected). This
    // matches user expectation that Esc just "leaves data entry" without
    // discarding their changes.
    let key_ctl = gtk::EventControllerKey::new();
    {
        let finish = finish.clone();
        key_ctl.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                finish(true);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    text_view.add_controller(key_ctl);

    // Focus-out → commit.
    let focus_ctl = gtk::EventControllerFocus::new();
    {
        let finish = finish.clone();
        focus_ctl.connect_leave(move |_| {
            finish(true);
        });
    }
    text_view.add_controller(focus_ctl);

    text_view.grab_focus();
}

fn refresh_overlay_draw(area: &gtk::DrawingArea, model: &AppModel) {
    let current_page = model.current_page();
    let editing_idx = model.currently_editing_index.get();
    let layer = AnnotationLayer {
        items: model
            .annotations
            .items
            .iter()
            .enumerate()
            .filter(|(i, a)| a.page() == current_page && Some(*i) != editing_idx)
            .map(|(_, a)| a.clone())
            .collect(),
    };
    let selected_annotation = model
        .selected
        .filter(|i| Some(*i) != editing_idx)
        .and_then(|i| model.annotations.items.get(i))
        .filter(|a| a.page() == current_page)
        .cloned();
    let draft = model.draft.clone();
    let transform = model.transform();
    let surface = model.surface.clone();
    let image_native_size = model.original_size;
    let search_highlights = model.current_page_match_highlights();
    area.set_draw_func(move |_, cr, _, _| {
        cr.save().unwrap();
        transform.apply(cr);

        // Image first (in image-space, anchored at 0,0). The surface may have
        // been rasterised at a different pixel scale than the image-coord
        // space (e.g. PDFs are re-rendered at zoom for sharpness), so scale
        // the source to fit [0..native_w, 0..native_h] in image-coord units.
        if let Some(s) = &surface {
            cr.save().unwrap();
            let sw = s.width() as f64;
            let sh = s.height() as f64;
            let (iw, ih) = (image_native_size.0 as f64, image_native_size.1 as f64);
            if sw > 0.0 && sh > 0.0 && iw > 0.0 && ih > 0.0 {
                cr.scale(iw / sw, ih / sh);
            }
            cr.set_source_surface(s, 0.0, 0.0).unwrap();
            cr.paint().unwrap();
            cr.restore().unwrap();
        }

        if !search_highlights.is_empty() {
            let highlights = AnnotationLayer {
                items: search_highlights.clone(),
            };
            paint_annotations(cr, &highlights);
        }

        paint_annotations(cr, &layer);
        if let Some(d) = &draft {
            let single = AnnotationLayer {
                items: vec![d.clone()],
            };
            paint_annotations(cr, &single);
        }
        if let Some(sel) = &selected_annotation {
            paint_selection(cr, sel);
        }
        cr.restore().unwrap();
    });
    area.queue_draw();
}

fn surface_from_rendered(page: &RenderedPage) -> cairo::ImageSurface {
    surface_from_rgba_pixels(page.width(), page.height(), page.pixels())
}

/// Width of a sidebar thumbnail in CSS pixels. Generous enough to read
/// page numbers / page contents at a glance, narrow enough that the
/// sidebar doesn't dominate the window.
const SIDEBAR_THUMB_W: i32 = 120;

/// Rebuild the page-thumbnail sidebar from `doc`. Synchronous: renders
/// each page at low scale and inserts a `gtk::Picture` row. For typical
/// office docs (≤ ~50 pages) this finishes well under 100ms; longer
/// docs will benefit from a future async / lazy variant.
fn populate_sidebar(list: &gtk::ListBox, doc: &PdfDoc, sender: &ComponentSender<AppModel>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let total = doc.page_count();
    for page_idx in 0..total {
        // Pick a render scale so the rasterised width hits the thumb
        // target. Falls back to a small constant if the page reports a
        // zero / unusable width.
        let scale = match doc.render_page(page_idx, 1.0) {
            Ok(r) if r.width() > 0 => SIDEBAR_THUMB_W as f64 / r.width() as f64,
            _ => 0.18,
        };
        let Ok(rendered) = doc.render_page(page_idx, scale) else {
            continue;
        };
        let texture = thumbnail_to_texture(&rendered);
        let pic = gtk::Picture::for_paintable(&texture);
        pic.set_size_request(SIDEBAR_THUMB_W, -1);
        pic.set_can_shrink(true);
        pic.set_content_fit(gtk::ContentFit::Contain);
        pic.add_css_class("card");

        let label = gtk::Label::new(Some(&format!("{}", page_idx + 1)));
        label.add_css_class("dim-label");
        label.add_css_class("caption");

        let row_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(6)
            .margin_end(6)
            .build();
        row_box.append(&pic);
        row_box.append(&label);

        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&row_box));
        // Stash page index on the row so the activate handler can
        // recover which page was clicked.
        unsafe {
            row.set_data::<u32>("page-index", page_idx);
        }
        list.append(&row);
    }
    // Single click anywhere in a row → jump to that page.
    let s = sender.clone();
    list.connect_row_activated(move |_, row| {
        let page = unsafe { row.data::<u32>("page-index").map(|p| *p.as_ref()) };
        if let Some(p) = page {
            s.input(AppMsg::GoToPage(p));
        }
    });
}

fn sync_sidebar_selection(list: &gtk::ListBox, page: u32) {
    if let Some(row) = list.row_at_index(page as i32) {
        let already_selected = list
            .selected_row()
            .is_some_and(|r| r.index() == row.index());
        if !already_selected {
            list.select_row(Some(&row));
        }
    }
}

/// Convert a rendered RGBA8 page buffer into a `gdk::Texture` suitable
/// for `gtk::Picture::for_paintable`.
fn thumbnail_to_texture(page: &RenderedPage) -> gdk::MemoryTexture {
    let bytes = glib::Bytes::from(page.pixels());
    gdk::MemoryTexture::new(
        page.width() as i32,
        page.height() as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        page.width() as usize * 4,
    )
}

fn surface_from_rgba(img: &DecodedImage) -> cairo::ImageSurface {
    surface_from_rgba_pixels(img.width(), img.height(), img.pixels())
}

/// Build a Cairo `ImageSurface` from straight-alpha RGBA8 pixels.
///
/// Cairo's `ARgb32` is BGRA-in-memory on little-endian and **premultiplied**,
/// so we permute channels and multiply colour by alpha here.
fn surface_from_rgba_pixels(width: u32, height: u32, src: &[u8]) -> cairo::ImageSurface {
    let w = width as i32;
    let h = height as i32;
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h)
        .expect("cairo: create ImageSurface");
    let stride = surface.stride() as usize;
    {
        let mut data = surface.data().expect("cairo: surface.data()");
        for y in 0..h as usize {
            let dst_row = y * stride;
            let src_row = y * w as usize * 4;
            for x in 0..w as usize {
                let s = src_row + x * 4;
                let d = dst_row + x * 4;
                let r = src[s];
                let g = src[s + 1];
                let b = src[s + 2];
                let a = src[s + 3];
                // Premultiply (no-op for opaque pixels).
                let pr = ((r as u16 * a as u16) / 255) as u8;
                let pg = ((g as u16 * a as u16) / 255) as u8;
                let pb = ((b as u16 * a as u16) / 255) as u8;
                // ARgb32 on little-endian = BGRA in memory.
                data[d] = pb;
                data[d + 1] = pg;
                data[d + 2] = pr;
                data[d + 3] = a;
            }
        }
    }
    surface.mark_dirty();
    surface
}

fn image_filters() -> gio::ListStore {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Images & PDFs"));
    for mime in [
        "image/png",
        "image/jpeg",
        "image/webp",
        "image/heic",
        "image/heif",
        "application/pdf",
    ] {
        filter.add_mime_type(mime);
    }
    for pat in [
        "*.png", "*.jpg", "*.jpeg", "*.webp", "*.heic", "*.heif", "*.pdf",
    ] {
        filter.add_pattern(pat);
    }
    let store = gio::ListStore::new::<gtk::FileFilter>();
    store.append(&filter);
    store
}

/// Walk up the widget tree from `btn` until we find a `gtk::Popover`, then
/// dismiss it. Lets popover items dispatch their action and close the
/// popover without holding a direct reference to it.
fn popdown_ancestor(btn: &impl IsA<gtk::Widget>) {
    let mut node: Option<gtk::Widget> = btn.upcast_ref::<gtk::Widget>().parent();
    while let Some(w) = node {
        if let Some(p) = w.downcast_ref::<gtk::Popover>() {
            p.popdown();
            return;
        }
        node = w.parent();
    }
}

fn pdf_save_filters() -> gio::ListStore {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("PDF documents"));
    filter.add_mime_type("application/pdf");
    filter.add_pattern("*.pdf");
    let store = gio::ListStore::new::<gtk::FileFilter>();
    store.append(&filter);
    store
}

/// Suggest `<stem> (annotated).pdf` as the default Save-As name.
fn suggest_save_as_name(original: &Path) -> String {
    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document");
    format!("{stem} (annotated).pdf")
}

/// Modal warning before overwriting the original PDF in-place. Buttons:
/// Cancel / Save As… / Overwrite.
fn prompt_inplace_save(path: &Path, sender: ComponentSender<AppModel>) {
    let dialog = gtk::AlertDialog::builder()
        .modal(true)
        .message("Overwrite the original PDF?")
        .detail(format!(
            "Saving will rewrite {} in place. The annotations will be embedded into the file. \
             Choose Save As… to keep the original untouched.",
            path.display()
        ))
        .buttons(["Cancel", "Save As…", "Overwrite"])
        .cancel_button(0)
        .default_button(1)
        .build();
    let parent = relm4::main_application().active_window();
    dialog.choose(
        parent.as_ref(),
        gio::Cancellable::NONE,
        move |result| match result {
            Ok(1) => sender.input(AppMsg::SaveAs),
            Ok(2) => sender.input(AppMsg::ConfirmedSaveInPlace),
            Ok(_) => {} // Cancel
            Err(e) => tracing::debug!(?e, "save warning dismissed"),
        },
    );
}

fn is_pdf_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|s| s.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

fn install_shortcuts(sender: ComponentSender<AppModel>) -> gtk::ShortcutController {
    let controller = gtk::ShortcutController::new();
    controller.set_scope(gtk::ShortcutScope::Global);

    let bindings: &[(&str, AppMsg)] = &[
        ("<Control>o", AppMsg::OpenDialog),
        ("<Control>s", AppMsg::Save),
        ("<Control><Shift>s", AppMsg::SaveAs),
        ("Page_Down", AppMsg::NextPage),
        ("Page_Up", AppMsg::PrevPage),
        ("<Control>Home", AppMsg::FirstPage),
        ("<Control>End", AppMsg::LastPage),
        ("<Control>f", AppMsg::ToggleSearch),
        ("F9", AppMsg::ToggleSidebar),
        ("<Control>plus", AppMsg::ZoomIn),
        ("<Control>equal", AppMsg::ZoomIn),
        ("<Control>KP_Add", AppMsg::ZoomIn),
        ("<Control>minus", AppMsg::ZoomOut),
        ("<Control>KP_Subtract", AppMsg::ZoomOut),
        ("<Control>0", AppMsg::ZoomReset),
        ("<Control>KP_0", AppMsg::ZoomReset),
        ("r", AppMsg::RotateRight),
        ("R", AppMsg::RotateRight),
        ("Escape", AppMsg::DrawCancel),
        ("Delete", AppMsg::DeleteSelected),
        ("KP_Delete", AppMsg::DeleteSelected),
        ("BackSpace", AppMsg::DeleteSelected),
        ("<Control>z", AppMsg::Undo),
        ("<Control>y", AppMsg::Redo),
        ("<Control><Shift>z", AppMsg::Redo),
    ];

    for (accel, msg) in bindings {
        let trigger = gtk::ShortcutTrigger::parse_string(accel).expect("valid accel");
        let s = sender.clone();
        let m = clone_msg(msg);
        let action = gtk::CallbackAction::new(move |_, _| {
            s.input(clone_msg(&m));
            glib::Propagation::Stop
        });
        let shortcut = gtk::Shortcut::new(Some(trigger), Some(action));
        controller.add_shortcut(shortcut);
    }

    controller
}

fn clone_msg(msg: &AppMsg) -> AppMsg {
    match msg {
        AppMsg::OpenDialog => AppMsg::OpenDialog,
        AppMsg::ZoomIn => AppMsg::ZoomIn,
        AppMsg::ZoomOut => AppMsg::ZoomOut,
        AppMsg::ZoomReset => AppMsg::ZoomReset,
        AppMsg::ZoomToggleFitWidth => AppMsg::ZoomToggleFitWidth,
        AppMsg::ViewportResized => AppMsg::ViewportResized,
        AppMsg::RotateRight => AppMsg::RotateRight,
        AppMsg::SelectTool(t) => AppMsg::SelectTool(*t),
        AppMsg::LoadPath(p) => AppMsg::LoadPath(p.clone()),
        AppMsg::DrawBegin { wx, wy } => AppMsg::DrawBegin { wx: *wx, wy: *wy },
        AppMsg::DrawUpdate { wx, wy } => AppMsg::DrawUpdate { wx: *wx, wy: *wy },
        AppMsg::DrawCommit => AppMsg::DrawCommit,
        AppMsg::DrawCancel => AppMsg::DrawCancel,
        AppMsg::Undo => AppMsg::Undo,
        AppMsg::Redo => AppMsg::Redo,
        AppMsg::ScaleFactorChanged(s) => AppMsg::ScaleFactorChanged(*s),
        AppMsg::DeleteSelected => AppMsg::DeleteSelected,
        AppMsg::EditSelectedText => AppMsg::EditSelectedText,
        AppMsg::EditCancelled => AppMsg::EditCancelled,
        AppMsg::CommitFreeText {
            image_x,
            image_y,
            text,
            replace_index,
        } => AppMsg::CommitFreeText {
            image_x: *image_x,
            image_y: *image_y,
            text: text.clone(),
            replace_index: *replace_index,
        },
        AppMsg::SetFontFamily(s) => AppMsg::SetFontFamily(s.clone()),
        AppMsg::SetFontSize(v) => AppMsg::SetFontSize(*v),
        AppMsg::SetFontColor(c) => AppMsg::SetFontColor(*c),
        AppMsg::SetStrokeColor(c) => AppMsg::SetStrokeColor(*c),
        AppMsg::SetStrokeWidth(v) => AppMsg::SetStrokeWidth(*v),
        AppMsg::SetStrokeStyle(s) => AppMsg::SetStrokeStyle(*s),
        AppMsg::Save => AppMsg::Save,
        AppMsg::ConfirmedSaveInPlace => AppMsg::ConfirmedSaveInPlace,
        AppMsg::SaveAs => AppMsg::SaveAs,
        AppMsg::SaveAsTo(p) => AppMsg::SaveAsTo(p.clone()),
        AppMsg::NextPage => AppMsg::NextPage,
        AppMsg::PrevPage => AppMsg::PrevPage,
        AppMsg::FirstPage => AppMsg::FirstPage,
        AppMsg::LastPage => AppMsg::LastPage,
        AppMsg::GoToPage(p) => AppMsg::GoToPage(*p),
        AppMsg::ToggleSidebar => AppMsg::ToggleSidebar,
        AppMsg::ToggleSearch => AppMsg::ToggleSearch,
        AppMsg::SearchQuery(q) => AppMsg::SearchQuery(q.clone()),
        AppMsg::SearchNext => AppMsg::SearchNext,
        AppMsg::SearchPrev => AppMsg::SearchPrev,
        AppMsg::CloseSearch => AppMsg::CloseSearch,
        AppMsg::OpenSignatureManager => AppMsg::OpenSignatureManager,
        AppMsg::SelectSignature(s) => AppMsg::SelectSignature(s.clone()),
    }
}

/// Register the bundled icon directory with GTK's icon theme so the app
/// icon resolves by name (`APP_ID`) without requiring a system install.
fn install_icon_search_path() {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let theme = gtk::IconTheme::for_display(&display);
    // Resolve `data/icons` relative to the workspace root (CARGO_MANIFEST_DIR
    // points at `crates/previewer-app`, so go up two levels). For installed
    // builds the icon will live under the system XDG icon dirs and this
    // search path is a no-op.
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(workspace) = manifest.parent().and_then(|p| p.parent()) {
        let icons = workspace.join("data/icons");
        if icons.exists() {
            theme.add_search_path(&icons);
        }
    }
}

thread_local! {
    /// Provider whose CSS rule for `.previewer-inline-text` is rewritten each
    /// time the inline editor opens, so the live TextView shows the current
    /// font / size / color the user picked in the font bar.
    static INLINE_EDITOR_PROVIDER: std::cell::RefCell<Option<gtk::CssProvider>> =
        const { std::cell::RefCell::new(None) };
}

fn install_inline_editor_css() {
    let provider = gtk::CssProvider::new();
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    INLINE_EDITOR_PROVIDER.with(|p| *p.borrow_mut() = Some(provider));
    apply_inline_editor_style(&FontSpec::default(), Color::BLACK, 1.0);
}

/// Rewrite the inline editor's CSS rule. Cheap — replaces the provider's
/// stylesheet and GTK re-resolves styles on the live TextView immediately.
///
/// `zoom` is the current display zoom; the editor's font-size is set in
/// CSS px to `font.size * zoom`. CSS `pt` would re-introduce a DPI/zoom
/// mismatch — the committed annotation is rendered in image-coord units
/// (1 point) and then scaled by Cairo via `transform.apply(cr.scale(zoom))`,
/// so px equal to `font.size * zoom` is exactly what the painted glyphs
/// will look like.
fn apply_inline_editor_style(font: &FontSpec, color: Color, zoom: f64) {
    let alpha = (color.a as f64 / 255.0).clamp(0.0, 1.0);
    let size_px = (font.size * zoom).max(1.0);
    let css = format!(
        "
        .previewer-inline-text,
        .previewer-inline-text text {{
            background: transparent;
            color: rgba({r}, {g}, {b}, {alpha:.3});
            caret-color: rgba({r}, {g}, {b}, 0.85);
            font-family: \"{family}\";
            font-size: {size_px:.2}px;
        }}
        ",
        r = color.r,
        g = color.g,
        b = color.b,
        family = font.family,
    );
    INLINE_EDITOR_PROVIDER.with(|p| {
        if let Some(provider) = p.borrow().as_ref() {
            provider.load_from_string(&css);
        }
    });
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "previewer=info".into()),
        )
        .init();

    let mut argv: Vec<String> = std::env::args().collect();
    // Pop the file path before handing off to GTK — gtk::Application doesn't
    // know we're a file-handling app, so it errors with "This application can
    // not open files" if it sees a positional arg. We parse it ourselves.
    let initial_path = if argv.len() > 1 {
        Some(PathBuf::from(argv.remove(1)))
    } else {
        None
    };
    if let Some(p) = &initial_path {
        tracing::info!(path = %p.display(), "starting Previewer with CLI argument");
    } else {
        tracing::info!("starting Previewer ({APP_ID})");
    }

    let app = RelmApp::new(APP_ID).with_args(argv);
    relm4::main_application().connect_startup(|_| {
        install_icon_search_path();
        install_inline_editor_css();
    });
    app.run::<AppModel>(initial_path);
}
