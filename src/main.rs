//! Mnemo — base de connaissances Markdown native, stockée dans PostgreSQL,
//! avec synchronisation en direct (LISTEN/NOTIFY) et plugins Rhai.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // pas de console sous Windows en release

mod app;
mod board;
mod config;
mod css;
mod db;
mod github;
mod graph;
mod local;
mod markdown;
mod merge;
mod model;
mod plugins;
mod sketch;
#[cfg(test)]
mod sync_tests;
mod theme;

fn main() -> eframe::Result {
    let cfg = config::Config::load();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Mnemo")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 420.0])
            .with_app_id("mnemo") // Wayland: matches mnemo.desktop
            .with_icon(std::sync::Arc::new(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icons/mnemo-256.png")).unwrap_or_default(),
            )),
        persist_window: true,
        // OpenGL 2.0+ manque souvent sous Windows (VM, bureau à distance, pilotes absents) : Direct3D 12 via wgpu
        #[cfg(windows)]
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let result = eframe::run_native("Mnemo", options, Box::new(|cc| Ok(Box::new(app::MnemoApp::new(cc, cfg)))));
    if let Err(e) = &result {
        let msg = e.to_string();
        if cfg!(target_os = "linux")
            && (msg.contains("libX11") || msg.contains("xkbcommon") || msg.contains("Wayland") || msg.contains("EGL"))
        {
            eprintln!(
                "Mnemo n'a pas pu ouvrir de fenêtre : une bibliothèque graphique du système manque.\n\
                 Installez-les puis relancez :\n\
                 \x20 Debian / Ubuntu : sudo apt install libx11-6 libxkbcommon-x11-0 libgl1 libegl1 libwayland-client0 libxcursor1 libxrandr2 libxi6\n\
                 \x20 Arch            : sudo pacman -S libx11 libxkbcommon-x11 libglvnd mesa wayland libxcursor libxrandr libxi\n\
                 \x20 Fedora          : sudo dnf install libX11 libxkbcommon-x11 mesa-libGL mesa-libEGL libwayland-client libXcursor libXrandr libXi\n\
                 Détail : {msg}"
            );
        } else {
            eprintln!("Mnemo n'a pas pu démarrer : {msg}");
        }
    }
    result
}
