//! cargo run --release --example audition_crash -- /tmp/crash.wav
use sherbydrum::{groove::Instrument, synth::Mixer};
use std::{error::Error, fs::File, io::BufWriter};

fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args().nth(1).ok_or("Supply an output WAV path")?;
    let rate = 48_000;
    let mut mixer = Mixer::new(rate);
    let mut writer = hound::WavWriter::new(
        BufWriter::new(File::create_new(path)?),
        hound::WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    // Isolated hit, softer hit, then repeated accents over kick/snare/hat.
    for frame in 0..rate * 18 {
        if frame == 0 {
            mixer.trigger(Instrument::Crash, 110);
        }
        if frame == rate * 6 {
            mixer.trigger(Instrument::Crash, 65);
        }
        if frame >= rate * 12 && frame % (rate / 4) == 0 {
            let step = (frame - rate * 12) / (rate / 4);
            mixer.trigger(Instrument::ClosedHat, 70);
            if step % 4 == 0 {
                mixer.trigger(Instrument::Kick, 110);
            }
            if step % 4 == 2 {
                mixer.trigger(Instrument::Snare, 100);
            }
            if step % 8 == 0 {
                mixer.trigger(Instrument::Crash, 110);
            }
        }
        let sample = mixer.next_sample() * 0.65;
        assert!(sample.is_finite() && sample.abs() < 1.0, "Audition clipped");
        writer.write_sample((sample * f32::from(i16::MAX)) as i16)?;
    }
    writer.finalize()?;
    Ok(())
}
