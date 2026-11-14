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
    // une bibliothèque graphique absente fait paniquer la couche de fenêtrage au lieu de renvoyer une erreur :
    // les deux cas donnent le même message d'aide
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eframe::run_native("Mnemo", options, Box::new(|cc| Ok(Box::new(app::MnemoApp::new(cc, cfg)))))
    }));
    let (result, panic_msg) = match run {
        Ok(r) => (r, None),
        Err(p) => {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
            (Ok(()), Some(msg.unwrap_or_default()))
        }
    };
    let failure = panic_msg.clone().or_else(|| result.as_ref().err().map(|e| e.to_string()));
    if let Some(msg) = failure {
        let lib = ["libX11", "xkbcommon", "Wayland", "wayland", "EGL", "libGL", "could not be loaded"];
        if cfg!(target_os = "linux") && lib.iter().any(|l| msg.contains(l)) {
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
    if panic_msg.is_some() {
        std::process::exit(1);
    }
    result
}
