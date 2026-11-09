mod app;
mod audio;
#[cfg(target_os = "macos")]
mod media;
mod midi;
mod midi_debug;
mod render;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "midi-debug") {
        if args.len() != 1 {
            return Err("Usage: cargo run -- midi-debug (no additional arguments)".into());
        }
        return midi_debug::run();
    }
    if render::handle_args(args.into_iter())? {
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Sherbydrum")
            .with_inner_size([1120.0, 820.0])
            .with_min_inner_size([880.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "sherbydrum",
        options,
        Box::new(|cc| Ok(Box::new(app::Drummer::new(cc)))),
    )?;
    Ok(())
}
