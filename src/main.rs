//! paintrs — a layered paint program for desktop and web, built on egui.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod doc;
mod filters;
mod history;
mod icons;
mod io;
mod ops;
mod platform;
mod raster;
mod script;
mod text;
mod tools;
mod ui;
mod view;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("paintrs")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([720.0, 480.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("paintrs", options, Box::new(|cc| Ok(Box::new(app::PaintApp::new(cc)))))
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;

    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window().expect("no window").document().expect("no document");
        let canvas = document
            .get_element_by_id("paintrs_canvas")
            .expect("missing #paintrs_canvas")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("#paintrs_canvas is not a canvas");
        let result = eframe::WebRunner::new()
            .start(canvas, eframe::WebOptions::default(), Box::new(|cc| Ok(Box::new(app::PaintApp::new(cc)))))
            .await;
        if let Some(loading) = document.get_element_by_id("loading") {
            match result {
                Ok(()) => loading.remove(),
                Err(e) => loading.set_inner_html(&format!("<p>paintrs failed to start: {e:?}</p>")),
            }
        }
    });
}
