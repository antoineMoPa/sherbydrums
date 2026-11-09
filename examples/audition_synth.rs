//! Render the actual native voices: cargo run --example audition_synth -- /tmp/sounds.wav
use sherbydrum::keyboard::{KeyEvent, Keyboard, Notes, Rate, Sound, SynthSettings};
use std::{error::Error, fs::File, io::BufWriter};
fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("Supply a new output WAV path")?;
    let rate = 48_000;
    let mut writer = hound::WavWriter::new(
        BufWriter::new(File::create_new(path)?),
        hound::WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for (index, sound) in Sound::ALL.iter().enumerate() {
        println!(
            "{:02}:{:02} {}",
            index * 8 / 60,
            index * 8 % 60,
            sound.label()
        );
        let mut synth = Keyboard::new(rate);
        let mut settings = SynthSettings {
            notes: Notes::Played,
            ..SynthSettings::default()
        };
        settings.select_sound(*sound);
        synth.configure(settings);
        synth.set_bpm(120);
        for frame in 0..rate * 8 {
            if frame == 0 {
                synth.event(
                    0,
                    KeyEvent::On {
                        note: 48,
                        velocity: 100,
                    },
                );
            }
            if frame == rate * 2 {
                synth.event(0, KeyEvent::Off { note: 48 });
            }
            if frame == rate * 4 {
                settings.arp = true;
                settings.notes = Notes::Minor;
                settings.rate = Rate::Eighth;
                settings.octaves = 2;
                synth.configure(settings);
                synth.event(
                    0,
                    KeyEvent::On {
                        note: 48,
                        velocity: 100,
                    },
                );
            }
            if frame == rate * 6 {
                synth.event(0, KeyEvent::Off { note: 48 });
            }
            let sample = synth.next_sample() * 0.7;
            assert!(sample.is_finite() && sample.abs() < 1.0, "Audition clipped");
            writer.write_sample((sample * f32::from(i16::MAX)) as i16)?;
        }
    }
    writer.finalize()?;
    Ok(())
}
