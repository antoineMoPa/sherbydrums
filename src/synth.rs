//! Synthesized one-shots are built once at the device's sample rate.
//! Playback uses a fixed voice pool, with no heap allocation in the audio callback.
use crate::groove::{INSTRUMENTS, Instrument};
use std::f32::consts::TAU;

pub struct Kit {
    samples: [Vec<f32>; INSTRUMENTS],
}

impl Kit {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            samples: Instrument::ALL.map(|instrument| synthesize(instrument, sample_rate)),
        }
    }

    fn sample(&self, instrument: Instrument, position: usize) -> Option<f32> {
        self.samples[instrument.index()].get(position).copied()
    }
}

fn synthesize(instrument: Instrument, sample_rate: u32) -> Vec<f32> {
    if instrument == Instrument::Crash {
        return synthesize_crash(sample_rate);
    }
    let duration = match instrument {
        Instrument::Kick => 0.55,
        Instrument::Snare => 0.28,
        Instrument::ClosedHat => 0.09,
        Instrument::OpenHat => 0.38,
        Instrument::Rim => 0.11,
        Instrument::Tom => 0.5,
        Instrument::Crash => unreachable!("crash has its own synthesis path"),
    };
    let mut seed = 0x1234_abcd_u32;
    let mut phase = 0.0;
    let mut previous_noise = 0.0;
    (0..(sample_rate as f32 * duration) as usize)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
            let high_noise = (noise - previous_noise) * 0.5;
            previous_noise = noise;
            let attack = (t * 2500.0).min(1.0);
            let sample = match instrument {
                Instrument::Kick => {
                    phase += TAU * (48.0 + 115.0 * (-t * 35.0).exp()) / sample_rate as f32;
                    phase.sin() * (-t * 10.0).exp() * 0.95 + noise * (-t * 180.0).exp() * 0.12
                }
                Instrument::Snare => {
                    noise * (-t * 24.0).exp() * 0.48
                        + (TAU * 185.0 * t).sin() * (-t * 35.0).exp() * 0.32
                }
                Instrument::ClosedHat | Instrument::OpenHat => {
                    let decay = if instrument == Instrument::ClosedHat {
                        70.0
                    } else {
                        15.0
                    };
                    let metal = ((TAU * 4133.0 * t).sin() + (TAU * 6737.0 * t).sin()) * 0.08;
                    (high_noise * 0.38 + metal) * (-t * decay).exp()
                }
                Instrument::Rim => {
                    ((TAU * 1720.0 * t).sin() * 0.5 + (TAU * 850.0 * t).sin() * 0.3 + noise * 0.15)
                        * (-t * 65.0).exp()
                }
                Instrument::Tom => {
                    phase += TAU * (95.0 + 75.0 * (-t * 25.0).exp()) / sample_rate as f32;
                    (phase.sin() * 0.7 + noise * 0.08) * (-t * 12.0).exp()
                }
                Instrument::Crash => unreachable!("crash has its own synthesis path"),
            };
            // The final fade makes every one-shot end exactly at zero.
            sample * attack * ((duration - t) * 500.0).clamp(0.0, 1.0)
        })
        .collect()
}

/// A dense inharmonic plate tail, a blooming wash, and a short stick transient.
/// Frequencies/decays are an original approximation, not a measured cymbal model.
/// All resonances are rendered here at kit construction, never in the callback.
fn synthesize_crash(sample_rate: u32) -> Vec<f32> {
    const DURATION: f32 = 2.2;
    let rate = sample_rate as f32;
    let frames = (rate * DURATION) as usize;
    let mut samples = vec![0.0; frames];
    let mut seed = 0x7c41_9e2b_u32;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };

    // Jittered logarithmic spacing avoids a pitched harmonic series. Each mode
    // uses a rotating phasor, so only setup needs trigonometric functions.
    for mode in 0..96 {
        let frequency = 380.0 * 40.0_f32.powf((mode as f32 + random() * 0.8) / 96.0);
        if frequency >= rate * 0.45 {
            continue;
        }
        let angle = TAU * frequency / rate;
        let (rotation_sin, rotation_cos) = angle.sin_cos();
        let (mut sine, mut cosine) = (random() * TAU).sin_cos();
        let decay = (-((0.9 + frequency / 6500.0) / rate)).exp();
        let rise = (-1.0 / (rate * (0.002 + frequency / 900_000.0))).exp();
        let mut amplitude = 0.018 * (0.75 + random() * 0.5);
        let mut onset = 1.0;
        for sample in &mut samples {
            *sample += sine * amplitude * (1.0 - onset);
            let next_sine = sine * rotation_cos + cosine * rotation_sin;
            cosine = cosine * rotation_cos - sine * rotation_sin;
            sine = next_sine;
            amplitude *= decay;
            onset *= rise;
        }
    }

    // Separate envelopes let the bright wash bloom, then give way to the body.
    // Fixed Hz filter coefficients keep the colour consistent across devices.
    let low_coefficient = 1.0 - (-TAU * 700.0 / rate).exp();
    let high_coefficient = 1.0 - (-TAU * 6500.0 / rate).exp();
    let mut low = 0.0;
    let mut high = 0.0;
    for (i, sample) in samples.iter_mut().enumerate() {
        let t = i as f32 / rate;
        let noise = random() * 2.0 - 1.0;
        low += low_coefficient * (noise - low);
        high += high_coefficient * (noise - high);
        let body = high - low;
        let air = noise - high;
        let bloom = 1.0 - (-t * 65.0).exp();
        let wash = body * 0.48 * (-t * 2.8).exp() + air * 0.32 * (-t * 4.5).exp();
        let stick = (noise - low) * 0.24 * (-t * 160.0).exp();
        *sample += wash * bloom + stick;
        // Exact zero endpoints and a gentle final fade, without peak rescaling.
        let attack = (t / 0.0005).min(1.0);
        let release = ((frames - 1 - i) as f32 / (rate * 0.15)).min(1.0);
        // Keep the accent below the drums and damp the lingering plate ring.
        *sample *= 0.55 * (-t * 2.0).exp() * attack * release;
    }
    samples
}

#[derive(Clone, Copy)]
struct Voice {
    instrument: Instrument,
    position: usize,
    gain: f32,
    active: bool,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            instrument: Instrument::Kick,
            position: 0,
            gain: 0.0,
            active: false,
        }
    }
}

pub struct Mixer {
    kit: Kit,
    voices: [Voice; 48],
}

impl Mixer {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            kit: Kit::new(sample_rate),
            voices: [Voice::default(); 48],
        }
    }

    pub fn trigger(&mut self, instrument: Instrument, velocity: u8) {
        if velocity == 0 {
            return;
        }
        if instrument == Instrument::ClosedHat {
            for voice in &mut self.voices {
                if voice.instrument == Instrument::OpenHat {
                    voice.active = false;
                }
            }
        }
        let index = self
            .voices
            .iter()
            .position(|voice| !voice.active)
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, voice)| voice.position)
                    .map(|(index, _)| index)
                    .unwrap_or(0)
            });
        self.voices[index] = Voice {
            instrument,
            position: 0,
            gain: f32::from(velocity) / 127.0,
            active: true,
        };
    }

    pub fn next_sample(&mut self) -> f32 {
        let mut sum = 0.0;
        for voice in &mut self.voices {
            if !voice.active {
                continue;
            }
            if let Some(sample) = self.kit.sample(voice.instrument, voice.position) {
                sum += sample * voice.gain;
                voice.position += 1;
            } else {
                voice.active = false;
            }
        }
        sum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn crash_has_headroom_and_a_decaying_tail_at_device_rates() {
        for rate in [22_050, 44_100, 48_000, 96_000] {
            let samples = synthesize(Instrument::Crash, rate);
            assert_eq!(samples.first(), Some(&0.0));
            assert_eq!(samples.last(), Some(&0.0));
            assert!(samples.iter().all(|s| s.is_finite() && s.abs() < 0.5));
            let window = |start: f32, end: f32| {
                &samples[(start * rate as f32) as usize..(end * rate as f32) as usize]
            };
            let attack = rms(window(0.02, 0.2));
            let tail = rms(window(0.5, 0.8));
            assert!(
                (0.04..0.1).contains(&attack),
                "crash level out of balance at {rate} Hz"
            );
            assert!(
                tail > 0.003 && tail < attack * 0.3,
                "missing/flat tail at {rate} Hz"
            );
            assert!(rms(window(1.5, 2.0)) < 0.001);
            // Difference energy measures brightness independently of level.
            let brightness = |samples: &[f32]| {
                let differences: Vec<_> = samples.windows(2).map(|s| s[1] - s[0]).collect();
                rms(&differences) / rms(samples)
            };
            assert!(
                brightness(window(1.5, 2.0)) < brightness(window(0.02, 0.2)) * 0.8,
                "high frequencies should decay before the body at {rate} Hz"
            );
        }
    }

    #[test]
    fn crash_velocity_and_overlapping_hits_remain_independent() {
        let rate = 48_000;
        let mut loud = Mixer::new(rate);
        let mut soft = Mixer::new(rate);
        loud.trigger(Instrument::Crash, 120);
        soft.trigger(Instrument::Crash, 60);
        for _ in 0..rate / 2 {
            assert!((loud.next_sample() - soft.next_sample() * 2.0).abs() < 1e-6);
        }
        soft.trigger(Instrument::Crash, 60);
        soft.trigger(Instrument::ClosedHat, 80);
        assert_eq!(
            soft.voices
                .iter()
                .filter(|v| v.active && v.instrument == Instrument::Crash)
                .count(),
            2
        );
        for _ in 0..rate * 5 {
            assert!(soft.next_sample().is_finite());
        }
        assert!(!soft.voices.iter().any(|v| v.active));
    }
}
