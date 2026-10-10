//! Application state, actions, input handling and the canvas widget.

use std::collections::HashMap;

use egui::{Color32, CursorIcon, Event, Key, KeyboardShortcut, Modifiers, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::doc::{Document, Paint};
use crate::filters::Filter;
use crate::history::History;
use crate::io::{self, ExportFormat, Opened};
use crate::ops::{self, ResampleMethod};
use crate::platform::{FileEvent, OpenPurpose, Platform, SaveRequest};
use crate::raster::{Coverage, IRect, Image, Mask, Rgba};
use crate::text::FontStore;
use crate::tools::{Btn, Ctx, OverlayCx, ToolId, ToolSettings, Tools};
use crate::view::View;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemePref {
    System,
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Background {
    White,
    Transparent,
    Primary,
    Secondary,
}

/// Persisted preferences.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub tools: ToolSettings,
    pub tool: ToolId,
    pub theme: ThemePref,
    pub show_grid: bool,
    pub grid_size: u32,
    pub pixel_grid: bool,
    pub show_rulers: bool,
    pub palette: Vec<Rgba>,
    pub recent: Vec<Rgba>,
    pub export_format: ExportFormat,
    pub jpeg_quality: u8,
    pub new_w: u32,
    pub new_h: u32,
    pub new_bg: Background,
    pub script: String,
    pub show_history: bool,
}

pub const DEFAULT_PALETTE: [Rgba; 24] = [
    [0, 0, 0, 255],
    [64, 64, 64, 255],
    [128, 128, 128, 255],
    [192, 192, 192, 255],
    [255, 255, 255, 255],
    [128, 0, 0, 255],
    [230, 41, 55, 255],
    [255, 128, 128, 255],
    [255, 106, 0, 255],
    [255, 180, 60, 255],
    [255, 221, 0, 255],
    [255, 245, 150, 255],
    [0, 110, 50, 255],
    [30, 180, 80, 255],
    [150, 230, 120, 255],
    [0, 120, 120, 255],
    [0, 200, 220, 255],
    [0, 60, 160, 255],
    [40, 110, 255, 255],
    [140, 190, 255, 255],
    [90, 30, 140, 255],
    [160, 80, 220, 255],
    [255, 110, 200, 255],
    [120, 72, 40, 255],
];

impl Default for Prefs {
    fn default() -> Self {
        Self {
            tools: ToolSettings::default(),
            tool: ToolId::Brush,
            theme: ThemePref::System,
            show_grid: false,
            grid_size: 16,
            pixel_grid: true,
            show_rulers: true,
            palette: DEFAULT_PALETTE.to_vec(),
            recent: Vec::new(),
            export_format: ExportFormat::Png,
            jpeg_quality: 90,
            new_w: 1280,
            new_h: 720,
            new_bg: Background::White,
            script: crate::script::EXAMPLES[0].1.to_string(),
            show_history: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    New,
    Open,
    ImportLayer,
    Save,
    SaveAs,
    Export,
    LoadFont,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    CopyMerged,
    Paste,
    PasteNewImage,
    Clear,
    FillPrimary,
    FillSecondary,
    FreeTransform,
    SelectAll,
    Deselect,
    InvertSelection,
    SelectOpaque,
    GrowSelection,
    ShrinkSelection,
    FeatherSelection,
    CanvasSize,
    ResizeImage,
    CropToSelection,
    Trim,
    RotateCW,
    RotateCCW,
    Rotate180,
    FlipH,
    FlipV,
    Flatten,
    NewLayer,
    DuplicateLayer,
    DeleteLayer,
    MergeDown,
    MergeVisible,
    LayerUp,
    LayerDown,
    FlipLayerH,
    FlipLayerV,
    ClearLayer,
    ZoomIn,
    ZoomOut,
    ZoomFit,
    Zoom100,
    ToggleGrid,
    TogglePixelGrid,
    ToggleRulers,
    ClearGuides,
    Filter(usize),
    RepeatFilter,
    Scripts,
    Shortcuts,
    About,
    Licenses,
    SwapColors,
    ResetColors,
}

const fn sc(m: Modifiers, k: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(m, k)
}
const CMD: Modifiers = Modifiers::COMMAND;
const CMD_SHIFT: Modifiers = Modifiers { shift: true, ..Modifiers::COMMAND };

impl Action {
    pub fn shortcut(self) -> Option<KeyboardShortcut> {
        use Action::*;
        Some(match self {
            New => sc(CMD, Key::N),
            Open => sc(CMD, Key::O),
            Save => sc(CMD, Key::S),
            SaveAs => sc(CMD_SHIFT, Key::S),
            Export => sc(CMD_SHIFT, Key::E),
            Undo => sc(CMD, Key::Z),
            Redo => sc(CMD_SHIFT, Key::Z),
            CopyMerged => sc(CMD_SHIFT, Key::C),
            FreeTransform => sc(CMD, Key::T),
            SelectAll => sc(CMD, Key::A),
            Deselect => sc(CMD, Key::D),
            InvertSelection => sc(CMD_SHIFT, Key::I),
            NewLayer => sc(CMD_SHIFT, Key::N),
            DuplicateLayer => sc(CMD, Key::J),
            MergeDown => sc(CMD, Key::E),
            ZoomIn => sc(CMD, Key::Equals),
            ZoomOut => sc(CMD, Key::Minus),
            ZoomFit => sc(CMD, Key::Num0),
            Zoom100 => sc(CMD, Key::Num1),
            ToggleGrid => sc(CMD, Key::Quote),
            RepeatFilter => sc(CMD, Key::F),
            FillPrimary => sc(Modifiers::ALT, Key::Backspace),
            FillSecondary => sc(CMD, Key::Backspace),
            Clear => sc(Modifiers::NONE, Key::Delete),
            SwapColors => sc(Modifiers::NONE, Key::X),
            ResetColors => sc(Modifiers::NONE, Key::D),
            _ => return None,
        })
    }

    /// Display-only shortcut labels for actions handled through events.
    pub fn shortcut_label(self, ctx: &egui::Context) -> Option<String> {
        let fixed = |m: Modifiers, k: Key| ctx.format_shortcut(&sc(m, k));
        match self {
            Action::Cut => Some(fixed(CMD, Key::X)),
            Action::Copy => Some(fixed(CMD, Key::C)),
            Action::Paste => Some(fixed(CMD, Key::V)),
            _ => self.shortcut().map(|s| ctx.format_shortcut(&s)),
        }
    }
}

pub enum Dialog {
    NewImage { w: u32, h: u32, bg: Background },
    CanvasSize { w: u32, h: u32, anchor: (i32, i32) },
    Resize { w: u32, h: u32, keep: bool, method: ResampleMethod },
    Export { format: ExportFormat, quality: u8, name: String },
    Filter { filter: Filter, session: Option<crate::doc::EditSession>, rect: IRect, dirty: bool },
    SelectionAmount { kind: SelAmount, amount: i32 },
    PasteLarge(Image),
    Confirm(Pending),
    Shortcuts,
    About,
    Licenses,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SelAmount {
    Grow,
    Shrink,
    Feather,
}

/// Action waiting on "discard unsaved changes?".
pub enum Pending {
    New,
    Open,
    OpenBytes { name: String, bytes: Vec<u8>, path: Option<std::path::PathBuf> },
    Close,
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Tool(Btn),
    Pan(Pos2),
    Guide { horizontal: bool, idx: usize },
}

pub struct PaintApp {
    pub doc: Document,
    pub history: History,
    pub view: View,
    pub tools: Tools,
    pub prefs: Prefs,
    pub cov: Coverage,
    pub cov2: Coverage,
    pub fonts: FontStore,
    pub platform: Platform,
    pub status: Option<String>,
    status_time: f64,
    pub dialog: Option<Dialog>,
    pub script_open: bool,
    pub script_output: String,
    pub thumbs: HashMap<u64, (u64, egui::TextureHandle)>,
    thumbs_updated: f64,
    pub icons: Option<egui::TextureHandle>,
    drag: Option<Drag>,
    pub mods: Modifiers,
    space_held: bool,
    pub hover: Option<Pos2>,
    pub guides_h: Vec<f32>,
    pub guides_v: Vec<f32>,
    clipboard: Option<Image>,
    pub last_filter: Option<Filter>,
    allow_close: bool,
    pub renaming: Option<(u64, String)>,
    title: String,
    /// Hue kept separately so it survives desaturated colors.
    pub picker_hsva: Option<(bool, Rgba, egui::ecolor::Hsva)>,
    pub editing_secondary: bool,
    time: f64,
}

impl PaintApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let prefs: Prefs = cc.storage.and_then(|s| eframe::get_value(s, "prefs")).unwrap_or_default();
        apply_theme(&cc.egui_ctx, prefs.theme);
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let (w, h) = (prefs.new_w.clamp(1, 16384), prefs.new_h.clamp(1, 16384));
        let doc = Document::new(w, h, Some([255, 255, 255, 255]));
        let mut app = Self {
            cov: Coverage::new(w, h),
            cov2: Coverage::new(w, h),
            doc,
            history: History::default(),
            view: View::default(),
            tools: Tools::default(),
            prefs,
            fonts: FontStore::default(),
            platform: Platform::new(cc.egui_ctx.clone()),
            status: None,
            status_time: 0.0,
            dialog: None,
            script_open: false,
            script_output: String::new(),
            thumbs: HashMap::new(),
            thumbs_updated: 0.0,
            icons: None,
            drag: None,
            mods: Modifiers::NONE,
            space_held: false,
            hover: None,
            guides_h: Vec::new(),
            guides_v: Vec::new(),
            clipboard: None,
            last_filter: None,
            allow_close: false,
            renaming: None,
            title: String::new(),
            picker_hsva: None,
            editing_secondary: false,
            time: 0.0,
        };
        app.icons = crate::icons::load_atlas(&cc.egui_ctx);
        app
    }

    pub fn tool(&self) -> ToolId {
        self.prefs.tool
    }

    pub fn say(&mut self, msg: impl Into<String>) {
        self.status = Some(msg.into());
        self.status_time = self.time;
    }

    /// Run `f` with the active tool and a tool context.
    pub fn with_tool<R>(&mut self, f: impl FnOnce(&mut dyn crate::tools::Tool, &mut Ctx) -> R) -> R {
        let PaintApp { doc, history, prefs, cov, cov2, fonts, tools, status, view, mods, .. } = self;
        let id = prefs.tool;
        let mut cx = Ctx {
            doc,
            history,
            s: &mut prefs.tools,
            cov,
            cov2,
            fonts,
            zoom: view.zoom,
            mods: *mods,
            status,
        };
        f(tools.get(id), &mut cx)
    }

    pub fn commit_tool(&mut self) {
        self.with_tool(|t, cx| t.commit(cx));
    }

    pub fn cancel_tool(&mut self) {
        self.with_tool(|t, cx| t.cancel(cx));
    }

    pub fn tool_busy(&self) -> bool {
        self.tools.get_ref(self.prefs.tool).busy()
    }

    pub fn set_tool(&mut self, id: ToolId) {
        if id == self.prefs.tool {
            return;
        }
        self.commit_tool();
        self.prefs.tool = id;
    }

    fn replace_document(&mut self, doc: Document) {
        self.cancel_tool();
        self.doc = doc;
        self.doc.mark_all_dirty();
        self.history.clear();
        self.thumbs.clear();
        self.view.fit_pending = true;
        self.guides_h.clear();
        self.guides_v.clear();
        self.cov = Coverage::new(self.doc.w, self.doc.h);
        self.cov2 = Coverage::new(self.doc.w, self.doc.h);
    }

    /// Ask before discarding unsaved work.
    fn guard(&mut self, pending: Pending) {
        if self.doc.modified {
            self.dialog = Some(Dialog::Confirm(pending));
        } else {
            self.run_pending(pending);
        }
    }

    pub fn run_pending(&mut self, p: Pending) {
        match p {
            Pending::New => {
                self.dialog = Some(Dialog::NewImage { w: self.prefs.new_w, h: self.prefs.new_h, bg: self.prefs.new_bg });
            }
            Pending::Open => self.platform.open(OpenPurpose::Document, "Images & projects", io::OPEN_EXTS),
            Pending::OpenBytes { name, bytes, path } => self.open_document(&name, &bytes, path),
            Pending::Close => {
                self.allow_close = true;
                self.platform.set_unsaved(false);
            }
        }
    }

    pub fn create_new(&mut self, w: u32, h: u32, bg: Background) {
        let fill = match bg {
            Background::White => Some([255, 255, 255, 255]),
            Background::Transparent => None,
            Background::Primary => Some(self.prefs.tools.primary),
            Background::Secondary => Some(self.prefs.tools.secondary),
        };
        self.prefs.new_w = w;
        self.prefs.new_h = h;
        self.prefs.new_bg = bg;
        self.replace_document(Document::new(w, h, fill));
    }

    fn open_document(&mut self, name: &str, bytes: &[u8], path: Option<std::path::PathBuf>) {
        match io::open_bytes(name, bytes) {
            Ok(Opened::Document(mut d)) => {
                if io::is_project(bytes) {
                    d.path = path;
                }
                let n = d.layers.len();
                self.replace_document(d);
                self.say(format!("Opened {name} ({n} layers)"));
            }
            Ok(Opened::Image(img)) => {
                let (w, h) = (img.w, img.h);
                let mut d = Document::from_layers(w, h, vec![("Background".into(), img)]);
                d.name = io::stem(name);
                self.replace_document(d);
                self.say(format!("Opened {name} ({w}×{h})"));
            }
            Err(e) => self.say(e),
        }
    }

    fn handle_file_events(&mut self) {
        for ev in self.platform.poll() {
            match ev {
                FileEvent::Opened { name, bytes, purpose, path } => match purpose {
                    // File > Open already asked about unsaved changes before
                    // showing the picker, so don't ask a second time.
                    OpenPurpose::Document => self.run_pending(Pending::OpenBytes { name, bytes, path }),
                    OpenPurpose::Dropped => self.open_dropped(name, bytes, path),
                    OpenPurpose::AsLayer => match io::decode_image(&bytes) {
                        Ok(img) => self.paste_image(img, Some(io::stem(&name))),
                        Err(e) => self.say(e),
                    },
                    OpenPurpose::Font => match self.fonts.add(&name, bytes) {
                        Ok(i) => {
                            self.prefs.tools.text_font = i;
                            self.say(format!("Loaded font {name}"));
                        }
                        Err(e) => self.say(e),
                    },
                    OpenPurpose::Script => {
                        self.prefs.script = String::from_utf8_lossy(&bytes).to_string();
                        self.script_open = true;
                    }
                },
                FileEvent::Saved { name, path, project } => {
                    if project {
                        self.doc.modified = false;
                        self.doc.name = io::stem(&name);
                        if path.is_some() {
                            self.doc.path = path;
                        }
                    }
                    self.say(format!("Saved {name}"));
                }
                FileEvent::Error(e) => self.say(e),
            }
        }
        // Web paste events.
        for img in self.platform.take_pasted() {
            self.paste_image(img, None);
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        // Contents arrive through the platform channel (reading is async on the web).
        for f in ctx.input(|i| i.raw.dropped_files.clone()) {
            self.platform.read_dropped(f);
        }
    }

    /// Open a dropped file according to what it is.
    fn open_dropped(&mut self, name: String, bytes: Vec<u8>, path: Option<std::path::PathBuf>) {
        let lower = name.to_lowercase();
        if lower.ends_with(".ttf") || lower.ends_with(".otf") {
            match self.fonts.add(&name, bytes) {
                Ok(i) => {
                    self.prefs.tools.text_font = i;
                    self.say(format!("Loaded font {name}"));
                }
                Err(e) => self.say(e),
            }
        } else if lower.ends_with(".rhai") {
            self.prefs.script = String::from_utf8_lossy(&bytes).to_string();
            self.script_open = true;
        } else if io::is_project(&bytes) || io::is_qpif(&bytes) {
            self.guard(Pending::OpenBytes { name, bytes, path });
        } else {
            // Dropping an image adds it as a layer; use File > Open to replace.
            match io::decode_image(&bytes) {
                Ok(img) => self.paste_image(img, Some(io::stem(&name))),
                Err(e) => self.say(e),
            }
        }
    }

    // -----------------------------------------------------------------------
    // Clipboard

    fn selection_image(&self, merged: bool) -> Option<Image> {
        let src = if merged { std::borrow::Cow::Owned(self.doc.flatten()) } else { std::borrow::Cow::Borrowed(&*self.doc.active_layer().img) };
        let bounds = match &self.doc.selection {
            Some(m) => m.bounds()?,
            None => self.doc.bounds(),
        };
        let mut img = src.crop(bounds);
        if let Some(m) = &self.doc.selection {
            for y in 0..img.h as i32 {
                for x in 0..img.w as i32 {
                    let i = img.idx(x, y);
                    let k = m.getf(x + bounds.x0, y + bounds.y0);
                    img.px[i][3] = (img.px[i][3] as f32 * k).round() as u8;
                }
            }
        }
        Some(img)
    }

    fn copy(&mut self, merged: bool) {
        self.commit_tool();
        match self.selection_image(merged) {
            Some(img) => {
                self.platform.copy_image(&img);
                self.say(format!("Copied {}×{}", img.w, img.h));
                self.clipboard = Some(img);
            }
            None => self.say("Nothing to copy"),
        }
    }

    fn clear_selection_pixels(&mut self, label: &str) {
        self.commit_tool();
        if self.doc.selection.is_none() {
            self.say("Nothing selected — select an area first (or use Layer ▸ Clear)");
            return;
        }
        let mut s = match self.doc.begin_edit() {
            Ok(s) => s,
            Err(e) => return self.say(e),
        };
        let bounds = self.doc.selection.as_ref().and_then(|m| m.bounds()).unwrap_or_default();
        self.cov.ensure_size(self.doc.w, self.doc.h);
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                self.cov.data[(y as u32 * self.doc.w + x as u32) as usize] = 1.0;
            }
        }
        s.apply_coverage(&mut self.doc, &self.cov, bounds, Paint::Erase { strength: 1.0 });
        self.cov.clear(bounds);
        s.commit(&mut self.doc, &mut self.history, label);
    }

    fn fill_selection(&mut self, color: Rgba) {
        self.commit_tool();
        let mut s = match self.doc.begin_edit() {
            Ok(s) => s,
            Err(e) => return self.say(e),
        };
        let bounds = self.doc.selection.as_ref().and_then(|m| m.bounds()).unwrap_or(self.doc.bounds());
        self.cov.ensure_size(self.doc.w, self.doc.h);
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                self.cov.data[(y as u32 * self.doc.w + x as u32) as usize] = 1.0;
            }
        }
        s.apply_coverage(&mut self.doc, &self.cov, bounds, Paint::Color { color, opacity: 1.0 });
        self.cov.clear(bounds);
        s.commit(&mut self.doc, &mut self.history, "Fill selection");
    }

    fn paste(&mut self) {
        if let Some(img) = self.platform.clipboard_image() {
            self.paste_image(img, None);
        } else if let Some(img) = self.clipboard.clone() {
            self.paste_image(img, None);
        } else {
            self.say("Clipboard has no image");
        }
    }

    /// Paste as a new layer and start transforming it.
    pub fn paste_image(&mut self, img: Image, name: Option<String>) {
        self.commit_tool();
        if img.w > self.doc.w || img.h > self.doc.h {
            self.dialog = Some(Dialog::PasteLarge(img));
            return;
        }
        self.place_image(img, name);
    }

    pub fn place_image(&mut self, img: Image, name: Option<String>) {
        // Center in the visible part of the canvas.
        let c = self.view.to_canvas(self.view.rect.center());
        let x = ((c.x - img.w as f32 / 2.0).round() as i32).clamp(0, (self.doc.w - img.w.min(self.doc.w)) as i32);
        let y = ((c.y - img.h as f32 / 2.0).round() as i32).clamp(0, (self.doc.h - img.h.min(self.doc.h)) as i32);
        let (w, h) = (self.doc.w, self.doc.h);
        let name = name.unwrap_or_else(|| "Pasted".into());
        self.doc.structural(&mut self.history, "Paste", |d| {
            let mut layer_img = Image::new(w, h);
            layer_img.blit(&img, x, y);
            let l = d.make_layer(&name, layer_img);
            d.layers.insert(d.active + 1, l);
            d.active += 1;
            let mut m = Mask::new(w, h);
            let r = IRect::new(x, y, x + img.w as i32, y + img.h as i32).clamp_to(w, h);
            for yy in r.y0..r.y1 {
                for xx in r.x0..r.x1 {
                    m.data[(yy as u32 * w + xx as u32) as usize] = 255;
                }
            }
            d.selection = Some(m);
        });
        self.set_tool(ToolId::Move);
        let PaintApp { doc, history, prefs, cov, cov2, fonts, tools, status, view, mods, .. } = self;
        let mut cx = Ctx { doc, history, s: &mut prefs.tools, cov, cov2, fonts, zoom: view.zoom, mods: *mods, status };
        tools.transform.lift(&mut cx);
    }

    // -----------------------------------------------------------------------
    // Actions

    pub fn act(&mut self, a: Action, ctx: &egui::Context) {
        use Action::*;
        // Most actions finish whatever the tool is doing first.
        if !matches!(a, Undo | Redo | ZoomIn | ZoomOut | ZoomFit | Zoom100 | SwapColors | ResetColors | ToggleGrid | TogglePixelGrid | ToggleRulers | Shortcuts | About | Licenses | Scripts) {
            self.commit_tool();
        }
        let (w, h) = (self.doc.w, self.doc.h);
        match a {
            New => self.guard(Pending::New),
            Open => self.guard(Pending::Open),
            ImportLayer => self.platform.open(OpenPurpose::AsLayer, "Images", &["png", "jpg", "jpeg", "webp", "bmp", "tga", "gif", "ico"]),
            LoadFont => self.platform.open(OpenPurpose::Font, "Fonts", &["ttf", "otf"]),
            Save | SaveAs => match io::save_project(&self.doc) {
                Ok(bytes) => self.platform.save(SaveRequest {
                    file_name: format!("{}.{}", self.doc.name, io::PROJECT_EXT),
                    bytes,
                    ext: io::PROJECT_EXT,
                    mime: "application/octet-stream",
                    filter_name: "paintrs project",
                    path: if a == Save { self.doc.path.clone() } else { None },
                    project: true,
                }),
                Err(e) => self.say(e),
            },
            Export => {
                self.dialog = Some(Dialog::Export {
                    format: self.prefs.export_format,
                    quality: self.prefs.jpeg_quality,
                    name: self.doc.name.clone(),
                })
            }
            Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Undo => {
                if self.tool_busy() {
                    self.cancel_tool();
                } else if let Some(l) = self.history.undo(&mut self.doc) {
                    self.say(format!("Undo: {l}"));
                }
            }
            Redo => {
                self.cancel_tool();
                if let Some(l) = self.history.redo(&mut self.doc) {
                    self.say(format!("Redo: {l}"));
                }
            }
            Cut => {
                self.copy(false);
                self.clear_selection_pixels("Cut");
            }
            Copy => self.copy(false),
            CopyMerged => self.copy(true),
            Paste => self.paste(),
            PasteNewImage => {
                let img = self.platform.clipboard_image().or(self.clipboard.clone());
                match img {
                    Some(img) => self.guard(Pending::OpenBytes {
                        name: "Pasted.png".into(),
                        bytes: io::encode_image(&img, ExportFormat::Png, 100).unwrap_or_default(),
                        path: None,
                    }),
                    None => self.say("Clipboard has no image"),
                }
            }
            Clear => self.clear_selection_pixels("Clear"),
            FillPrimary => self.fill_selection(self.prefs.tools.primary),
            FillSecondary => self.fill_selection(self.prefs.tools.secondary),
            FreeTransform => {
                self.set_tool(ToolId::Move);
                let PaintApp { doc, history, prefs, cov, cov2, fonts, tools, status, view, mods, .. } = self;
                let mut cx = Ctx { doc, history, s: &mut prefs.tools, cov, cov2, fonts, zoom: view.zoom, mods: *mods, status };
                tools.transform.lift(&mut cx);
            }
            SelectAll => self.doc.set_selection(&mut self.history, "Select all", Some(Mask::full(w, h))),
            Deselect => self.doc.set_selection(&mut self.history, "Deselect", None),
            InvertSelection => {
                let mut m = self.doc.selection.clone().unwrap_or_else(|| Mask::new(w, h));
                m.invert();
                self.doc.set_selection(&mut self.history, "Invert selection", Some(m));
            }
            SelectOpaque => {
                let img = &self.doc.active_layer().img;
                let m = Mask { w, h, data: img.px.iter().map(|p| p[3]).collect() };
                self.doc.set_selection(&mut self.history, "Select layer pixels", Some(m));
            }
            GrowSelection => self.dialog = Some(Dialog::SelectionAmount { kind: SelAmount::Grow, amount: 4 }),
            ShrinkSelection => self.dialog = Some(Dialog::SelectionAmount { kind: SelAmount::Shrink, amount: 4 }),
            FeatherSelection => self.dialog = Some(Dialog::SelectionAmount { kind: SelAmount::Feather, amount: 4 }),
            CanvasSize => self.dialog = Some(Dialog::CanvasSize { w, h, anchor: (1, 1) }),
            ResizeImage => self.dialog = Some(Dialog::Resize { w, h, keep: true, method: ResampleMethod::Bicubic }),
            CropToSelection => match self.doc.selection.as_ref().and_then(|m| m.bounds()) {
                Some(b) => self.crop_to(b, "Crop to selection"),
                None => self.say("Nothing selected"),
            },
            Trim => match self.doc.flatten().opaque_bounds() {
                Some(b) if b != self.doc.bounds() => self.crop_to(b, "Trim"),
                Some(_) => self.say("Nothing to trim"),
                None => self.say("Image is empty"),
            },
            RotateCW | RotateCCW | Rotate180 | FlipH | FlipV => {
                let f = move |img: &Image| match a {
                    RotateCW => ops::rotate90(img, true),
                    RotateCCW => ops::rotate90(img, false),
                    Rotate180 => ops::rotate180(img),
                    FlipH => ops::flip(img, true),
                    _ => ops::flip(img, false),
                };
                let label = match a {
                    RotateCW => "Rotate 90° CW",
                    RotateCCW => "Rotate 90° CCW",
                    Rotate180 => "Rotate 180°",
                    FlipH => "Flip horizontal",
                    _ => "Flip vertical",
                };
                self.doc.structural(&mut self.history, label, |d| {
                    for l in &mut d.layers {
                        l.img = std::sync::Arc::new(f(&l.img));
                        l.rev += 1;
                    }
                    d.selection = d.selection.as_ref().map(|m| ops::map_mask(m, f));
                    d.w = d.layers[0].img.w;
                    d.h = d.layers[0].img.h;
                });
                if matches!(a, RotateCW | RotateCCW) {
                    self.view.fit_pending = true;
                }
            }
            Flatten => {
                if self.doc.layers.len() > 1 {
                    self.doc.structural(&mut self.history, "Flatten", |d| {
                        let flat = d.flatten();
                        let l = d.make_layer("Background", flat);
                        d.layers = vec![l];
                        d.active = 0;
                    });
                }
            }
            NewLayer => {
                self.doc.structural(&mut self.history, "New layer", |d| {
                    let name = d.unique_layer_name();
                    let l = d.make_layer(&name, Image::new(w, h));
                    d.layers.insert(d.active + 1, l);
                    d.active += 1;
                });
            }
            DuplicateLayer => {
                self.doc.structural(&mut self.history, "Duplicate layer", |d| {
                    let mut l = d.layers[d.active].clone();
                    l.id = d.next_id;
                    d.next_id += 1;
                    l.name = format!("{} copy", l.name);
                    d.layers.insert(d.active + 1, l);
                    d.active += 1;
                });
            }
            DeleteLayer => {
                if self.doc.layers.len() > 1 {
                    self.doc.structural(&mut self.history, "Delete layer", |d| {
                        d.layers.remove(d.active);
                        d.active = d.active.saturating_sub(1).min(d.layers.len() - 1);
                    });
                } else {
                    self.say("Can't delete the only layer");
                }
            }
            MergeDown => {
                if self.doc.active > 0 {
                    self.doc.structural(&mut self.history, "Merge down", |d| {
                        let i = d.active;
                        let (mut top, mut bottom) = (d.layers[i].clone(), d.layers[i - 1].clone());
                        top.visible = true;
                        bottom.visible = true;
                        let merged = crate::doc::composite_layers(&[&bottom, &top], d.w, d.h);
                        d.layers[i - 1].img = std::sync::Arc::new(merged);
                        d.layers[i - 1].opacity = 1.0;
                        d.layers[i - 1].blend = crate::doc::BlendMode::Normal;
                        d.layers[i - 1].rev += 1;
                        d.layers.remove(i);
                        d.active = i - 1;
                    });
                } else {
                    self.say("No layer below to merge into");
                }
            }
            MergeVisible => {
                self.doc.structural(&mut self.history, "Merge visible", |d| {
                    let flat = d.flatten();
                    let first = d.layers.iter().position(|l| l.visible).unwrap_or(0);
                    let l = d.make_layer("Merged", flat);
                    d.layers.retain(|l| !l.visible);
                    let at = first.min(d.layers.len());
                    d.layers.insert(at, l);
                    d.active = at;
                });
            }
            LayerUp => {
                if self.doc.active + 1 < self.doc.layers.len() {
                    self.doc.structural(&mut self.history, "Move layer up", |d| {
                        d.layers.swap(d.active, d.active + 1);
                        d.active += 1;
                    });
                }
            }
            LayerDown => {
                if self.doc.active > 0 {
                    self.doc.structural(&mut self.history, "Move layer down", |d| {
                        d.layers.swap(d.active, d.active - 1);
                        d.active -= 1;
                    });
                }
            }
            FlipLayerH | FlipLayerV => {
                let horizontal = a == FlipLayerH;
                self.doc.structural(&mut self.history, "Flip layer", |d| {
                    let l = &mut d.layers[d.active];
                    l.img = std::sync::Arc::new(ops::flip(&l.img, horizontal));
                    l.rev += 1;
                });
            }
            ClearLayer => {
                self.doc.structural(&mut self.history, "Clear layer", |d| {
                    let l = &mut d.layers[d.active];
                    l.img = std::sync::Arc::new(Image::new(w, h));
                    l.rev += 1;
                });
            }
            ZoomIn => self.view.zoom_step(1, self.view.rect.center()),
            ZoomOut => self.view.zoom_step(-1, self.view.rect.center()),
            ZoomFit => self.view.fit(w, h),
            Zoom100 => {
                self.view.zoom = 1.0;
                self.view.center(w, h);
            }
            ToggleGrid => self.prefs.show_grid = !self.prefs.show_grid,
            TogglePixelGrid => self.prefs.pixel_grid = !self.prefs.pixel_grid,
            ToggleRulers => self.prefs.show_rulers = !self.prefs.show_rulers,
            ClearGuides => {
                self.guides_h.clear();
                self.guides_v.clear();
            }
            Filter(i) => self.open_filter(crate::filters::MENU[i]),
            RepeatFilter => match self.last_filter {
                Some(f) => self.apply_filter_now(f),
                None => self.say("No filter used yet"),
            },
            Scripts => self.script_open = !self.script_open,
            Shortcuts => self.dialog = Some(Dialog::Shortcuts),
            About => self.dialog = Some(Dialog::About),
            Licenses => self.dialog = Some(Dialog::Licenses),
            SwapColors => {
                let s = &mut self.prefs.tools;
                std::mem::swap(&mut s.primary, &mut s.secondary);
            }
            ResetColors => {
                self.prefs.tools.primary = [0, 0, 0, 255];
                self.prefs.tools.secondary = [255, 255, 255, 255];
            }
        }
    }

    fn crop_to(&mut self, b: IRect, label: &str) {
        self.doc.structural(&mut self.history, label, |d| {
            let (w, h) = (b.width() as u32, b.height() as u32);
            for l in &mut d.layers {
                l.img = std::sync::Arc::new(ops::resize_canvas(&l.img, w, h, -b.x0, -b.y0));
                l.rev += 1;
            }
            d.selection = d.selection.as_ref().map(|m| ops::map_mask(m, |i| ops::resize_canvas(i, w, h, -b.x0, -b.y0)));
            d.w = w;
            d.h = h;
        });
        self.view.fit_pending = true;
    }

    pub fn resize_canvas(&mut self, w: u32, h: u32, anchor: (i32, i32)) {
        let (ox, oy) = (
            (w as i32 - self.doc.w as i32) * anchor.0 / 2,
            (h as i32 - self.doc.h as i32) * anchor.1 / 2,
        );
        self.doc.structural(&mut self.history, "Canvas size", |d| {
            for l in &mut d.layers {
                l.img = std::sync::Arc::new(ops::resize_canvas(&l.img, w, h, ox, oy));
                l.rev += 1;
            }
            d.selection = d.selection.as_ref().map(|m| ops::map_mask(m, |i| ops::resize_canvas(i, w, h, ox, oy)));
            d.w = w;
            d.h = h;
        });
        self.view.fit_pending = true;
    }

    pub fn resize_image(&mut self, w: u32, h: u32, method: ResampleMethod) {
        self.doc.structural(&mut self.history, "Resize image", |d| {
            for l in &mut d.layers {
                l.img = std::sync::Arc::new(ops::resample(&l.img, w, h, method));
                l.rev += 1;
            }
            d.selection = d.selection.as_ref().map(|m| ops::map_mask(m, |i| ops::resample(i, w, h, ResampleMethod::Bilinear)));
            d.w = w;
            d.h = h;
        });
        self.view.fit_pending = true;
    }

    pub fn export(&mut self, format: ExportFormat, quality: u8, name: &str) {
        self.prefs.export_format = format;
        self.prefs.jpeg_quality = quality;
        let flat = self.doc.flatten();
        match io::encode_image(&flat, format, quality) {
            Ok(bytes) => self.platform.save(SaveRequest {
                file_name: format!("{name}.{}", format.ext()),
                bytes,
                ext: format.ext(),
                mime: format.mime(),
                filter_name: format.name(),
                path: None,
                project: false,
            }),
            Err(e) => self.say(e),
        }
    }

    // -----------------------------------------------------------------------
    // Filters

    pub fn open_filter(&mut self, filter: Filter) {
        let session = match self.doc.begin_edit() {
            Ok(s) => s,
            Err(e) => return self.say(e),
        };
        let rect = self.doc.selection.as_ref().and_then(|m| m.bounds()).unwrap_or(self.doc.bounds());
        if !filter.has_params() {
            self.dialog = None;
            self.render_filter(&mut Some(session), rect, filter);
            return;
        }
        let mut s = Some(session);
        self.render_filter(&mut s, rect, filter);
        // Keep the session open while the dialog is up.
        if let Some(s) = s.take() {
            self.dialog = Some(Dialog::Filter { filter, session: Some(s), rect, dirty: false });
        }
    }

    /// Preview (or apply, for parameterless filters) a filter.
    pub fn render_filter(&mut self, session: &mut Option<crate::doc::EditSession>, rect: IRect, filter: Filter) {
        let Some(s) = session else { return };
        let out = crate::filters::apply(s.source(), rect, filter);
        s.apply(&mut self.doc, rect, |x, y, src, sel| {
            let f = out.px[out.idx(x - rect.x0, y - rect.y0)];
            if sel >= 1.0 { f } else { crate::raster::lerp_rgba(src, f, sel) }
        });
        if !filter.has_params()
            && let Some(s) = session.take() {
                s.commit(&mut self.doc, &mut self.history, filter.name());
                self.last_filter = Some(filter);
            }
    }

    fn apply_filter_now(&mut self, filter: Filter) {
        let session = match self.doc.begin_edit() {
            Ok(s) => s,
            Err(e) => return self.say(e),
        };
        let rect = self.doc.selection.as_ref().and_then(|m| m.bounds()).unwrap_or(self.doc.bounds());
        let mut s = Some(session);
        self.render_filter(&mut s, rect, filter);
        if let Some(s) = s.take() {
            s.commit(&mut self.doc, &mut self.history, filter.name());
        }
    }

    // -----------------------------------------------------------------------
    // Input

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            self.space_held = false;
            return;
        }
        use Action::*;
        // egui matches shortcuts leniently on Shift, so the Shift variants
        // must be tried before their plain counterparts.
        let actions = [
            SaveAs, Export, Redo, CopyMerged, InvertSelection, NewLayer, New, Open, Save, Undo, FreeTransform,
            SelectAll, Deselect, DuplicateLayer, MergeDown, ZoomIn, ZoomOut, ZoomFit, Zoom100, ToggleGrid,
            RepeatFilter, FillPrimary, FillSecondary,
        ];
        let mut fired = Vec::new();
        ctx.input_mut(|i| {
            for a in actions {
                if let Some(s) = a.shortcut()
                    && i.consume_shortcut(&s) {
                        fired.push(a);
                    }
            }
            if i.consume_shortcut(&sc(CMD, Key::Y)) {
                fired.push(Redo);
            }
            if i.consume_shortcut(&sc(CMD, Key::Plus)) {
                fired.push(ZoomIn);
            }
        });
        for a in fired {
            self.act(a, ctx);
        }

        let events = ctx.input(|i| i.events.clone());
        self.space_held = ctx.input(|i| i.key_down(Key::Space));
        for ev in events {
            match ev {
                Event::Copy => self.act(Copy, ctx),
                Event::Cut => self.act(Cut, ctx),
                Event::Key { key: Key::V, pressed: false, modifiers, .. } if modifiers.command && !Platform::is_web() => {
                    self.act(Paste, ctx)
                }
                Event::Key { key, pressed: true, modifiers, repeat, .. } => {
                    if modifiers.command {
                        continue;
                    }
                    // Tools get first go at Enter/Esc/arrows/etc.
                    self.mods = modifiers;
                    if self.with_tool(|t, cx| t.key(cx, key)) {
                        continue;
                    }
                    match key {
                        Key::Escape => {
                            if self.doc.selection.is_some() {
                                self.act(Deselect, ctx);
                            }
                        }
                        Key::Delete | Key::Backspace if !modifiers.alt => self.act(Clear, ctx),
                        Key::X => self.act(SwapColors, ctx),
                        Key::D => self.act(ResetColors, ctx),
                        Key::OpenBracket | Key::CloseBracket => {
                            let k = if key == Key::CloseBracket { 1.2 } else { 1.0 / 1.2 };
                            let s = &mut self.prefs.tools;
                            match self.prefs.tool {
                                ToolId::Eraser => s.eraser_size = (s.eraser_size * k).round().clamp(1.0, 1000.0),
                                ToolId::Pencil => {
                                    s.pencil_size = (s.pencil_size + if k > 1.0 { 1 } else { -1 }).clamp(1, 64)
                                }
                                ToolId::Line | ToolId::Rect | ToolId::Ellipse => {
                                    s.line_width = (s.line_width * k).round().clamp(1.0, 200.0)
                                }
                                _ => s.brush_size = (s.brush_size * k).round().clamp(1.0, 1000.0),
                            }
                        }
                        _ if !repeat && !modifiers.alt && !modifiers.shift => {
                            if let Some(t) = ToolId::ALL.iter().find(|t| t.shortcut() == Some(key)) {
                                self.set_tool(*t);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    pub fn add_recent(&mut self, c: Rgba) {
        let r = &mut self.prefs.recent;
        r.retain(|x| *x != c);
        r.insert(0, c);
        r.truncate(16);
    }

    fn canvas_press(&mut self, pos: Pos2, btn: Btn) {
        let p = self.view.to_canvas(pos);
        if matches!(self.prefs.tool, ToolId::Brush | ToolId::Pencil | ToolId::Fill | ToolId::Line | ToolId::Rect | ToolId::Ellipse | ToolId::Text) {
            let c = if btn == Btn::Primary { self.prefs.tools.primary } else { self.prefs.tools.secondary };
            self.add_recent(c);
        }
        self.with_tool(|t, cx| t.press(cx, p, btn));
    }

    /// The central canvas: input, texture sync and drawing.
    pub fn canvas_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        self.view.rect = rect;
        if self.view.fit_pending {
            self.view.fit(self.doc.w, self.doc.h);
        }
        let layer = ui.layer_id();
        let on_canvas = |p: Pos2| rect.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l == layer);
        let ruler = if self.prefs.show_rulers { 18.0 } else { 0.0 };
        let in_top_ruler = |p: Pos2| p.y < rect.top() + ruler && p.x > rect.left() + ruler;
        let in_left_ruler = |p: Pos2| p.x < rect.left() + ruler && p.y > rect.top() + ruler;

        let events = ctx.input(|i| i.events.clone());
        self.mods = ctx.input(|i| i.modifiers);
        let busy_text = ctx.egui_wants_keyboard_input();
        for ev in &events {
            match ev {
                Event::PointerButton { pos, button, pressed: true, modifiers } if self.drag.is_none() && on_canvas(*pos) => {
                    self.mods = *modifiers;
                    if busy_text {
                        // Click on canvas takes focus away from text fields (except the text tool's).
                        if self.prefs.tool != ToolId::Text {
                            ctx.memory_mut(|m| m.stop_text_input());
                        }
                    }
                    let pan = *button == egui::PointerButton::Middle
                        || (*button == egui::PointerButton::Primary && (self.space_held || self.prefs.tool == ToolId::Pan));
                    if pan {
                        self.drag = Some(Drag::Pan(*pos));
                        continue;
                    }
                    if *button == egui::PointerButton::Primary && (in_top_ruler(*pos) || in_left_ruler(*pos)) {
                        let c = self.view.to_canvas(*pos);
                        let horizontal = in_top_ruler(*pos);
                        let idx = if horizontal {
                            self.guides_h.push(c.y.round());
                            self.guides_h.len() - 1
                        } else {
                            self.guides_v.push(c.x.round());
                            self.guides_v.len() - 1
                        };
                        self.drag = Some(Drag::Guide { horizontal, idx });
                        continue;
                    }
                    if *button == egui::PointerButton::Primary && matches!(self.prefs.tool, ToolId::Move | ToolId::Pan)
                        && let Some(g) = self.guide_at(*pos) {
                            self.drag = Some(Drag::Guide { horizontal: g.0, idx: g.1 });
                            continue;
                        }
                    if self.prefs.tool == ToolId::Zoom {
                        let dir = if modifiers.alt || *button == egui::PointerButton::Secondary { -1 } else { 1 };
                        self.view.zoom_step(dir, *pos);
                        continue;
                    }
                    let btn = match button {
                        egui::PointerButton::Primary => Btn::Primary,
                        egui::PointerButton::Secondary => Btn::Secondary,
                        _ => continue,
                    };
                    self.drag = Some(Drag::Tool(btn));
                    self.canvas_press(*pos, btn);
                }
                Event::PointerMoved(pos) => match self.drag {
                    Some(Drag::Tool(_)) => {
                        let p = self.view.to_canvas(*pos);
                        self.with_tool(|t, cx| t.drag(cx, p));
                    }
                    Some(Drag::Pan(last)) => {
                        self.view.offset += *pos - last;
                        self.drag = Some(Drag::Pan(*pos));
                    }
                    Some(Drag::Guide { horizontal, idx }) => {
                        let c = self.view.to_canvas(*pos);
                        if horizontal {
                            self.guides_h[idx] = c.y.round();
                        } else {
                            self.guides_v[idx] = c.x.round();
                        }
                    }
                    None => {}
                },
                Event::PointerButton { pos, button, pressed: false, .. } => match self.drag {
                    Some(Drag::Tool(b)) => {
                        let matches = matches!((b, button), (Btn::Primary, egui::PointerButton::Primary) | (Btn::Secondary, egui::PointerButton::Secondary));
                        if matches {
                            self.drag = None;
                            let p = self.view.to_canvas(*pos);
                            self.with_tool(|t, cx| t.release(cx, p));
                        }
                    }
                    Some(Drag::Pan(_)) => self.drag = None,
                    Some(Drag::Guide { horizontal, idx }) => {
                        // Dropping a guide back on the ruler removes it.
                        if in_top_ruler(*pos) || in_left_ruler(*pos) || !rect.contains(*pos) {
                            if horizontal {
                                self.guides_h.remove(idx);
                            } else {
                                self.guides_v.remove(idx);
                            }
                        }
                        self.drag = None;
                    }
                    None => {}
                },
                Event::MouseWheel { unit, delta, modifiers, .. } => {
                    let Some(hp) = ctx.pointer_hover_pos() else { continue };
                    if !on_canvas(hp) {
                        continue;
                    }
                    let mouse_notch = *unit == egui::MouseWheelUnit::Line
                        || *unit == egui::MouseWheelUnit::Page
                        || (delta.x == 0.0 && delta.y.abs() >= 50.0 && (delta.y.abs() % 1.0) == 0.0);
                    if modifiers.command || (mouse_notch && !modifiers.shift) {
                        // Zoom around the cursor.
                        if mouse_notch {
                            self.view.zoom_step(if delta.y > 0.0 { 1 } else { -1 }, hp);
                        } else {
                            let f = (delta.y * 0.01).exp();
                            self.view.set_zoom_at(self.view.zoom * f, hp);
                        }
                    } else {
                        let d = match unit {
                            egui::MouseWheelUnit::Point => *delta,
                            _ => *delta * 40.0,
                        };
                        let d = if modifiers.shift && d.x == 0.0 { vec2(d.y, 0.0) } else { d };
                        self.view.offset += d;
                    }
                }
                Event::Zoom(f) => {
                    if let Some(hp) = ctx.pointer_hover_pos()
                        && on_canvas(hp) {
                            self.view.set_zoom_at(self.view.zoom * f, hp);
                        }
                }
                _ => {}
            }
        }
        if resp.double_clicked()
            && let Some(p) = resp.interact_pointer_pos() {
                let c = self.view.to_canvas(p);
                self.with_tool(|t, cx| t.double_click(cx, c));
            }

        self.hover = ctx.pointer_hover_pos().filter(|p| on_canvas(*p)).map(|p| self.view.to_canvas(p));

        // Cursor
        if let Some(hp) = ctx.pointer_hover_pos().filter(|p| on_canvas(*p)) {
            let icon = match self.drag {
                Some(Drag::Pan(_)) => CursorIcon::Grabbing,
                Some(Drag::Guide { horizontal: true, .. }) => CursorIcon::ResizeVertical,
                Some(Drag::Guide { .. }) => CursorIcon::ResizeHorizontal,
                _ if self.space_held || self.prefs.tool == ToolId::Pan => CursorIcon::Grab,
                _ if self.prefs.tool == ToolId::Zoom => {
                    if self.mods.alt { CursorIcon::ZoomOut } else { CursorIcon::ZoomIn }
                }
                _ if in_top_ruler(hp) || in_left_ruler(hp) => CursorIcon::Default,
                _ => self.tools.get_ref(self.prefs.tool).cursor(self.hover),
            };
            ctx.set_cursor_icon(icon);
        }

        // Sync GPU textures with document changes.
        if self.doc.dirty.is_some() {
            self.view.sync_textures(&ctx, &mut self.doc);
        }
        if self.doc.selection_changed {
            self.doc.selection_changed = false;
            self.view.update_outline(self.doc.selection.as_ref());
        }

        // Draw
        let painter = ui.painter_at(rect);
        let dark = ui.visuals().dark_mode;
        painter.rect_filled(rect, 0.0, if dark { Color32::from_gray(30) } else { Color32::from_gray(200) });
        self.view.paint_canvas(&ctx, &painter, self.doc.w, self.doc.h, dark);
        if self.prefs.pixel_grid && self.view.zoom >= 8.0 {
            self.view.paint_grid(&painter, self.doc.w, self.doc.h, 1, Color32::from_black_alpha(40));
        }
        if self.prefs.show_grid {
            self.view.paint_grid(&painter, self.doc.w, self.doc.h, self.prefs.grid_size.max(1), Color32::from_rgba_unmultiplied(0, 120, 255, 90));
        }
        let guide_stroke = Stroke::new(1.0, Color32::from_rgb(0, 200, 255));
        for g in &self.guides_h {
            let y = self.view.to_screen(pos2(0.0, *g)).y.round() + 0.5;
            painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], guide_stroke);
        }
        for g in &self.guides_v {
            let x = self.view.to_screen(pos2(*g, 0.0)).x.round() + 0.5;
            painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], guide_stroke);
        }
        self.time = ctx.input(|i| i.time);
        self.view.paint_selection(&painter, self.time);
        let tool = self.prefs.tool;
        let o = OverlayCx {
            view: &self.view,
            painter: &painter,
            hover: if self.space_held { None } else { self.hover },
            s: &self.prefs.tools,
            doc: &self.doc,
            fonts: &self.fonts,
            time: self.time,
        };
        self.tools.get_ref(tool).overlay(&o);
        if self.prefs.show_rulers {
            self.paint_rulers(&painter, rect, ruler, dark);
        }

        // Apply / cancel bar for pending edits.
        if self.tool_busy() {
            let mut apply = false;
            let mut cancel = false;
            egui::Area::new(egui::Id::new("apply_bar"))
                .fixed_pos(pos2(rect.center().x - 90.0, rect.top() + ruler + 8.0))
                .order(egui::Order::Foreground)
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            apply = ui.button("✔ Apply").on_hover_text("Enter").clicked();
                            cancel = ui.button("✖ Cancel").on_hover_text("Esc").clicked();
                        });
                    });
                });
            if apply {
                self.commit_tool();
            }
            if cancel {
                self.cancel_tool();
            }
        }

        // Keep animating while something moves.
        if self.view.has_outline() || self.tool_busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(90));
        }
        if self.doc.dirty.is_some() {
            ctx.request_repaint();
        }
    }

    fn guide_at(&self, pos: Pos2) -> Option<(bool, usize)> {
        for (i, g) in self.guides_h.iter().enumerate() {
            if (self.view.to_screen(pos2(0.0, *g)).y - pos.y).abs() <= 4.0 {
                return Some((true, i));
            }
        }
        for (i, g) in self.guides_v.iter().enumerate() {
            if (self.view.to_screen(pos2(*g, 0.0)).x - pos.x).abs() <= 4.0 {
                return Some((false, i));
            }
        }
        None
    }

    fn paint_rulers(&self, painter: &egui::Painter, rect: Rect, size: f32, dark: bool) {
        let bg = if dark { Color32::from_gray(40) } else { Color32::from_gray(230) };
        let fg = if dark { Color32::from_gray(160) } else { Color32::from_gray(80) };
        let top = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + size));
        let left = Rect::from_min_max(rect.min, pos2(rect.left() + size, rect.bottom()));
        painter.rect_filled(top, 0.0, bg);
        painter.rect_filled(left, 0.0, bg);
        let z = self.view.zoom;
        let step = [1.0f32, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0]
            .into_iter()
            .find(|s| s * z >= 60.0)
            .unwrap_or(10000.0);
        let minor = step / 5.0;
        let font = egui::FontId::monospace(9.0);
        let stroke = Stroke::new(1.0, fg);
        // Top ruler
        let c0 = self.view.to_canvas(pos2(rect.left() + size, 0.0)).x;
        let c1 = self.view.to_canvas(pos2(rect.right(), 0.0)).x;
        let mut v = (c0 / minor).floor() * minor;
        while v <= c1 {
            let x = self.view.to_screen(pos2(v, 0.0)).x.round() + 0.5;
            if x > rect.left() + size {
                let major = (v / step).round() * step == v;
                let len = if major { size } else { size * 0.3 };
                painter.line_segment([pos2(x, top.bottom() - len), pos2(x, top.bottom())], stroke);
                if major {
                    painter.text(pos2(x + 2.0, top.top() + 1.0), egui::Align2::LEFT_TOP, format!("{}", v as i64), font.clone(), fg);
                }
            }
            v += minor;
        }
        // Left ruler
        let c0 = self.view.to_canvas(pos2(0.0, rect.top() + size)).y;
        let c1 = self.view.to_canvas(pos2(0.0, rect.bottom())).y;
        let mut v = (c0 / minor).floor() * minor;
        while v <= c1 {
            let y = self.view.to_screen(pos2(0.0, v)).y.round() + 0.5;
            if y > rect.top() + size {
                let major = (v / step).round() * step == v;
                let len = if major { size } else { size * 0.3 };
                painter.line_segment([pos2(left.right() - len, y), pos2(left.right(), y)], stroke);
                if major {
                    // Vertical numbers read top to bottom, one digit per line.
                    let s = format!("{}", v as i64);
                    for (k, ch) in s.chars().enumerate() {
                        painter.text(pos2(left.left() + 2.0, y + 2.0 + k as f32 * 8.0), egui::Align2::LEFT_TOP, ch, font.clone(), fg);
                    }
                }
            }
            v += minor;
        }
        // Cursor position markers.
        if let Some(h) = self.hover {
            let s = self.view.to_screen(h);
            let mark = Color32::from_rgb(255, 80, 80);
            painter.line_segment([pos2(s.x, top.top()), pos2(s.x, top.bottom())], Stroke::new(1.0, mark));
            painter.line_segment([pos2(left.left(), s.y), pos2(left.right(), s.y)], Stroke::new(1.0, mark));
        }
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(size, size)), 0.0, bg);
        painter.line_segment([pos2(rect.left(), top.bottom()), pos2(rect.right(), top.bottom())], Stroke::new(1.0, fg.gamma_multiply(0.5)));
        painter.line_segment([pos2(left.right(), rect.top()), pos2(left.right(), rect.bottom())], Stroke::new(1.0, fg.gamma_multiply(0.5)));
    }

    /// Per-frame bookkeeping before drawing UI.
    pub fn pre_frame(&mut self, ctx: &egui::Context) {
        self.time = ctx.input(|i| i.time);
        self.handle_file_events();
        self.handle_dropped_files(ctx);
        self.handle_shortcuts(ctx);

        // Close guard (native).
        if ctx.input(|i| i.viewport().close_requested()) && self.doc.modified && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialog = Some(Dialog::Confirm(Pending::Close));
        }
        self.platform.set_unsaved(self.doc.modified);
        // Free thumbnails of layers that no longer exist.
        if self.thumbs.len() > self.doc.layers.len() {
            let doc = &self.doc;
            self.thumbs.retain(|id, _| doc.layers.iter().any(|l| l.id == *id));
        }

        let title = format!("{}{} — paintrs", self.doc.name, if self.doc.modified { " •" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if self.status.is_some() && self.time - self.status_time > 5.0 {
            self.status = None;
        }
    }

    /// Layer thumbnail texture (refreshed at most a few times per second).
    pub fn thumbnail(&mut self, ctx: &egui::Context, idx: usize) -> Option<egui::TextureId> {
        let l = self.doc.layers.get(idx)?;
        let (id, rev) = (l.id, l.rev);
        let stale = self.thumbs.get(&id).is_none_or(|(r, _)| *r != rev);
        let drawing = self.drag.is_some();
        if stale && (!self.thumbs.contains_key(&id) || (!drawing && self.time - self.thumbs_updated > 0.25)) {
            self.thumbs_updated = self.time;
            let img = &l.img;
            let max = 40.0;
            let s = (max / img.w.max(img.h) as f32).min(1.0);
            let (tw, th) = (((img.w as f32 * s).round() as usize).max(1), ((img.h as f32 * s).round() as usize).max(1));
            let mut px = Vec::with_capacity(tw * th);
            for y in 0..th {
                for x in 0..tw {
                    let p = img.get((x as f32 / s) as i32, (y as f32 / s) as i32);
                    px.push(Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]));
                }
            }
            let ci = egui::ColorImage::new([tw, th], px);
            match self.thumbs.get_mut(&id) {
                Some((r, tex)) => {
                    tex.set(ci, egui::TextureOptions::LINEAR);
                    *r = rev;
                }
                None => {
                    let tex = ctx.load_texture(format!("thumb-{id}"), ci, egui::TextureOptions::LINEAR);
                    self.thumbs.insert(id, (rev, tex));
                }
            }
        }
        self.thumbs.get(&id).map(|(_, t)| t.id())
    }
}

pub fn apply_theme(ctx: &egui::Context, t: ThemePref) {
    ctx.set_theme(match t {
        ThemePref::System => egui::ThemePreference::System,
        ThemePref::Dark => egui::ThemePreference::Dark,
        ThemePref::Light => egui::ThemePreference::Light,
    });
}

impl eframe::App for PaintApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.pre_frame(&ctx);
        self.draw_ui(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "prefs", &self.prefs);
    }
}
