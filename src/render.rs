//! Offline rendering exercises the same engine without a window or audio device.
use sherbydrum::{
    engine::{Command, Engine, Settings},
    groove::Style,
};
use std::{fs::File, io::BufWriter, path::PathBuf};

const HELP: &str = "Sherbydrum — a drum partner for guitar practice

  sherbydrum                              Open the desktop app
  sherbydrum midi-debug                   Log all MIDI inputs to stdout (Ctrl+C to stop)
  sherbydrum --render beat.wav [options]   Render a groove without an audio device

Render options:
  --style rock|blues|funk|pop|samba|half-time|boom-bap|garage|breakbeat|deep-house|bossa|trip-hop   Default: rock
  --bpm 40..240                                Default: 100
  --bars 1..128                                Default: 4
  --swing 0..60                                Default: 0

Output is 48 kHz / 16-bit mono WAV, with a 10 ms stop fade.
Existing files are never overwritten.
Desktop shortcuts: Space = play/stop, Shift+T = tap tempo, QWERTY = second groove row, Enter / F = fill next bar.";

struct RenderArgs {
    path: PathBuf,
    settings: Settings,
    bars: u16,
}

fn parse(args: Vec<String>) -> Result<RenderArgs, String> {
    let mut path = None;
    let mut settings = Settings {
        ..Settings::default()
    };
    let mut bars = 4;
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {flag}. Use --help for usage."))?;
        match flag.as_str() {
            "--render" if path.is_none() => path = Some(PathBuf::from(value)),
            "--style" => {
                settings.style = match value.as_str() {
                    "rock" => Style::Rock,
                    "blues" => Style::Blues,
                    "funk" => Style::Funk,
                    "pop" => Style::Pop,
                    "samba" => Style::Samba,
                    "half-time" => Style::HalfTime,
                    "boom-bap" => Style::BoomBap,
                    "garage" => Style::Garage,
                    "breakbeat" => Style::Breakbeat,
                    "deep-house" => Style::DeepHouse,
                    "bossa" => Style::Bossa,
                    "trip-hop" => Style::TripHop,
                    _ => {
                        return Err(format!(
                            "Unknown style: {value}. Use --help for supported styles."
                        ));
                    }
                }
            }
            "--bpm" => {
                settings.bpm = value
                    .parse()
                    .map_err(|_| "BPM must be an integer.".to_owned())?
            }
            "--swing" => {
                settings.swing = value
                    .parse()
                    .map_err(|_| "Swing must be an integer.".to_owned())?
            }
            "--bars" => {
                bars = value
                    .parse()
                    .map_err(|_| "Bars must be an integer.".to_owned())?
            }
            _ => {
                return Err(format!(
                    "Unknown or repeated option: {flag}. Use --help for usage."
                ));
            }
        }
    }
    settings.validate()?;
    if !(1..=128).contains(&bars) {
        return Err("Bars must be between 1 and 128.".into());
    }
    Ok(RenderArgs {
        path: path
            .ok_or("Supply --render <path.wav>, or run without arguments for the desktop app.")?,
        settings,
        bars,
    })
}

pub fn handle_args(args: impl Iterator<Item = String>) -> Result<bool, Box<dyn std::error::Error>> {
    let args: Vec<_> = args.collect();
    if args.is_empty() {
        return Ok(false);
    }
    if args == ["--help"] || args == ["-h"] {
        println!("{HELP}");
        return Ok(true);
    }
    let args = parse(args)?;
    let sample_rate = 48_000;
    let mut engine = Engine::new(sample_rate);
    engine.command(Command::Apply(args.settings));
    engine.command(Command::Play);
    let frames = (f64::from(sample_rate) * 240.0 / f64::from(args.settings.bpm)
        * f64::from(args.bars))
    .ceil() as u64;
    let file = File::create_new(&args.path)?;
    let mut writer = hound::WavWriter::new(
        BufWriter::new(file),
        hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for _ in 0..frames {
        writer.write_sample((engine.next_sample() * f32::from(i16::MAX)) as i16)?;
    }
    engine.command(Command::Stop);
    for _ in 0..sample_rate / 100 {
        writer.write_sample((engine.next_sample() * f32::from(i16::MAX)) as i16)?;
    }
    writer.finalize()?;
    println!(
        "Rendered {} bars of {} at {} BPM to {}",
        args.bars,
        args.settings.style.name(),
        args.settings.bpm,
        args.path.display()
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_row_styles_are_available_for_rendering() {
        for (name, expected) in [
            ("boom-bap", Style::BoomBap),
            ("garage", Style::Garage),
            ("breakbeat", Style::Breakbeat),
            ("deep-house", Style::DeepHouse),
            ("bossa", Style::Bossa),
            ("trip-hop", Style::TripHop),
        ] {
            let parsed = parse(
                ["--render", "unused.wav", "--style", name]
                    .into_iter()
                    .map(String::from)
                    .collect(),
            )
            .unwrap();
            assert_eq!(parsed.settings.style, expected);
        }
    }

    #[test]
    fn invalid_render_requests_fail_before_creating_a_file() {
        for args in [
            vec!["--render"],
            vec!["--render", "test.wav", "--bpm", "0"],
            vec!["--render", "test.wav", "--style", "Rock"],
            vec!["--render", "test.wav", "--bars", "129"],
            vec!["--render", "test.wav", "--swing", "61"],
            vec!["--bpm", "120"],
            vec!["--unknown", "value"],
        ] {
            assert!(parse(args.into_iter().map(String::from).collect()).is_err());
        }
    }
}
