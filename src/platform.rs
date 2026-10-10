//! Platform glue: file dialogs, saving/downloading, clipboard images and the
//! "unsaved changes" guard. Native uses rfd + std::fs + arboard; the web uses
//! async rfd pickers, Blob downloads and browser paste events.

use std::sync::mpsc::{Receiver, Sender, channel};

use crate::raster::Image;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenPurpose {
    Document,
    AsLayer,
    Font,
    Script,
    /// Dropped onto the window; the app decides by content.
    Dropped,
}

pub enum FileEvent {
    Opened { name: String, bytes: Vec<u8>, purpose: OpenPurpose, path: Option<std::path::PathBuf> },
    Saved { name: String, path: Option<std::path::PathBuf>, project: bool },
    Error(String),
}

pub struct Platform {
    tx: Sender<FileEvent>,
    rx: Receiver<FileEvent>,
    ctx: egui::Context,
    #[cfg(target_arch = "wasm32")]
    pasted: std::rc::Rc<std::cell::RefCell<Vec<Image>>>,
}

pub struct SaveRequest {
    pub file_name: String,
    pub bytes: Vec<u8>,
    pub ext: &'static str,
    pub mime: &'static str,
    pub filter_name: &'static str,
    /// Save without asking (native only, when we already have a path).
    pub path: Option<std::path::PathBuf>,
    pub project: bool,
}

impl Platform {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        #[cfg(target_arch = "wasm32")]
        let pasted = {
            let pasted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            web::install_paste_listener(ctx.clone(), pasted.clone());
            web::install_unload_guard();
            pasted
        };
        Self {
            tx,
            rx,
            ctx,
            #[cfg(target_arch = "wasm32")]
            pasted,
        }
    }

    pub fn poll(&self) -> Vec<FileEvent> {
        self.rx.try_iter().collect()
    }

    pub fn is_web() -> bool {
        cfg!(target_arch = "wasm32")
    }

    pub fn open(&self, purpose: OpenPurpose, filter_name: &str, exts: &[&str]) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(path) = rfd::FileDialog::new().add_filter(filter_name, exts).pick_file() {
                self.send_file(path, purpose);
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let dialog = rfd::AsyncFileDialog::new().add_filter(filter_name, exts);
            let tx = self.tx.clone();
            let ctx = self.ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                if let Some(file) = dialog.pick_file().await {
                    let bytes = file.read().await;
                    let _ = tx.send(FileEvent::Opened { name: file.file_name(), bytes, purpose, path: None });
                    ctx.request_repaint();
                }
            });
        }
    }

    /// Read a dropped or opened file from disk (native).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn send_file(&self, path: std::path::PathBuf, purpose: OpenPurpose) {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let ev = match std::fs::read(&path) {
            Ok(bytes) => FileEvent::Opened { name, bytes, purpose, path: Some(path) },
            Err(e) => FileEvent::Error(format!("Can't read {name}: {e}")),
        };
        let _ = self.tx.send(ev);
        self.ctx.request_repaint();
    }

    /// Read a file dropped onto the window and deliver it as an `Opened` event.
    pub fn read_dropped(&self, file: egui::DroppedFileHandle) {
        let name = file.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let ev = match file.bytes() {
                Ok(bytes) => FileEvent::Opened { name, bytes, purpose: OpenPurpose::Dropped, path: Some(file.path().to_path_buf()) },
                Err(e) => FileEvent::Error(format!("Can't read {name}: {e}")),
            };
            let _ = self.tx.send(ev);
            self.ctx.request_repaint();
        }
        #[cfg(target_arch = "wasm32")]
        {
            let tx = self.tx.clone();
            let ctx = self.ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let ev = match file.bytes_async().await {
                    Ok(bytes) => FileEvent::Opened { name, bytes, purpose: OpenPurpose::Dropped, path: None },
                    Err(e) => FileEvent::Error(format!("Can't read {name}: {e}")),
                };
                let _ = tx.send(ev);
                ctx.request_repaint();
            });
        }
    }

    pub fn save(&self, req: SaveRequest) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = req.mime;
            let path = match req.path {
                Some(p) => Some(p),
                None => rfd::FileDialog::new()
                    .set_file_name(&req.file_name)
                    .add_filter(req.filter_name, &[req.ext])
                    .save_file()
                    .map(|mut p| {
                        if p.extension().is_none() {
                            p.set_extension(req.ext);
                        }
                        p
                    }),
            };
            let Some(path) = path else { return };
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let ev = match std::fs::write(&path, &req.bytes) {
                Ok(()) => FileEvent::Saved { name, path: Some(path), project: req.project },
                Err(e) => FileEvent::Error(format!("Can't save {name}: {e}")),
            };
            let _ = self.tx.send(ev);
        }
        #[cfg(target_arch = "wasm32")]
        {
            // The browser picks the location; path/filter only matter natively.
            let _ = (req.ext, req.filter_name, &req.path);
            let ev = match web::download(&req.file_name, &req.bytes, req.mime) {
                Ok(()) => FileEvent::Saved { name: req.file_name.clone(), path: None, project: req.project },
                Err(e) => FileEvent::Error(e),
            };
            let _ = self.tx.send(ev);
        }
        self.ctx.request_repaint();
    }

    pub fn copy_image(&self, img: &Image) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let res = arboard::Clipboard::new().and_then(|mut c| {
                c.set_image(arboard::ImageData {
                    width: img.w as usize,
                    height: img.h as usize,
                    bytes: std::borrow::Cow::Borrowed(img.as_bytes()),
                })
            });
            if let Err(e) = res {
                log::warn!("clipboard: {e}");
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.ctx.copy_image(egui::ColorImage::from_rgba_unmultiplied([img.w as usize, img.h as usize], img.as_bytes()));
        }
    }

    /// Image currently on the system clipboard (native).
    pub fn clipboard_image(&self) -> Option<Image> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let data = arboard::Clipboard::new().ok()?.get_image().ok()?;
            Some(Image::from_rgba8(data.width as u32, data.height as u32, &data.bytes))
        }
        #[cfg(target_arch = "wasm32")]
        None
    }

    /// Images pasted into the page (web).
    pub fn take_pasted(&self) -> Vec<Image> {
        #[cfg(target_arch = "wasm32")]
        let pasted = std::mem::take(&mut *self.pasted.borrow_mut());
        #[cfg(not(target_arch = "wasm32"))]
        let pasted = Vec::new();
        pasted
    }

    pub fn set_unsaved(&self, unsaved: bool) {
        #[cfg(target_arch = "wasm32")]
        web::UNSAVED.store(unsaved, std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(target_arch = "wasm32"))]
        let _ = unsaved;
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    use crate::raster::Image;

    pub static UNSAVED: AtomicBool = AtomicBool::new(false);

    pub fn download(name: &str, bytes: &[u8], mime: &str) -> Result<(), String> {
        let err = |e: JsValue| format!("Download failed: {e:?}");
        let window = web_sys::window().ok_or("no window")?;
        let document = window.document().ok_or("no document")?;
        let array = js_sys::Uint8Array::from(bytes);
        let parts = js_sys::Array::of1(&array);
        let opts = web_sys::BlobPropertyBag::new();
        opts.set_type(mime);
        let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(err)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(err)?;
        let a: web_sys::HtmlAnchorElement = document.create_element("a").map_err(err)?.dyn_into().map_err(|_| "anchor")?;
        a.set_href(&url);
        a.set_download(name);
        a.click();
        let _ = web_sys::Url::revoke_object_url(&url);
        Ok(())
    }

    pub fn install_paste_listener(ctx: egui::Context, out: Rc<RefCell<Vec<Image>>>) {
        let Some(window) = web_sys::window() else { return };
        let cb = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(move |ev: web_sys::ClipboardEvent| {
            let Some(data) = ev.clipboard_data() else { return };
            let items = data.items();
            for i in 0..items.length() {
                let Some(item) = items.get(i) else { continue };
                if item.kind() != "file" || !item.type_().starts_with("image/") {
                    continue;
                }
                let Ok(Some(file)) = item.get_as_file() else { continue };
                let out = out.clone();
                let ctx = ctx.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    if let Ok(buf) = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                        let bytes = js_sys::Uint8Array::new(&buf).to_vec();
                        if let Ok(img) = crate::io::decode_image(&bytes) {
                            out.borrow_mut().push(img);
                            ctx.request_repaint();
                        }
                    }
                });
                break;
            }
        });
        let _ = window.add_event_listener_with_callback_and_bool("paste", cb.as_ref().unchecked_ref(), true);
        cb.forget();
    }

    pub fn install_unload_guard() {
        let Some(window) = web_sys::window() else { return };
        let cb = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(move |ev: web_sys::BeforeUnloadEvent| {
            if UNSAVED.load(Ordering::Relaxed) {
                ev.prevent_default();
                ev.set_return_value("You have unsaved changes.");
            }
        });
        let _ = window.add_event_listener_with_callback("beforeunload", cb.as_ref().unchecked_ref());
        cb.forget();
    }
}
