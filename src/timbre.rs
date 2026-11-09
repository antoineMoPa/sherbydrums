//! Distinct voice models. Saw and pulse edges use polynomial BLEP correction.
//! References and listening examples are attached to the MIDI synth Moon task.
use crate::keyboard::Sound;
use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy)]
pub struct Timbre {
    phase: [f32; 3],
    modulator: f32,
    sub: f32,
    lfo: f32,
    transient: f32,
    low: f32,
    band: f32,
}
impl Default for Timbre {
    fn default() -> Self {
        Self {
            phase: [0.0, 0.27, 0.61],
            modulator: 0.0,
            sub: 0.0,
            lfo: 0.0,
            transient: 1.0,
            low: 0.0,
            band: 0.0,
        }
    }
}

fn edge(phase: f32, step: f32) -> f32 {
    if phase < step {
        let x = phase / step;
        x + x - x * x - 1.0
    } else if phase > 1.0 - step {
        let x = (phase - 1.0) / step;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}
fn saw(phase: f32, step: f32) -> f32 {
    2.0 * phase - 1.0 - edge(phase, step)
}
fn pulse(phase: f32, step: f32, width: f32) -> f32 {
    let raw = if phase < width { 1.0 } else { -1.0 };
    raw + edge(phase, step) - edge((phase + 1.0 - width).fract(), step) - (2.0 * width - 1.0)
}

impl Timbre {
    pub fn restart_color(&mut self) {
        // Keep oscillator phase continuous, but audition the new patch's attack color.
        self.transient = 1.0;
    }

    pub fn decay(sound: Sound, rate: f32) -> f32 {
        let seconds = match sound {
            Sound::Warm => 0.8,
            Sound::Soft => 2.0,
            Sound::Bass => 0.14,
            Sound::Bright => 0.7,
            Sound::Acid => 0.17,
            Sound::Glass => 0.9,
            Sound::Pixel => 0.08,
            Sound::Organ => 0.045,
        };
        (-1.0 / (rate * seconds)).exp()
    }

    pub fn sample(
        &mut self,
        sound: Sound,
        step: f32,
        rate: f32,
        tone: f32,
        velocity: f32,
        decay: f32,
    ) -> f32 {
        self.transient *= decay;
        let detune = match sound {
            Sound::Bright => 0.011,
            Sound::Soft => 0.004,
            _ => 0.002,
        };
        for (phase, ratio) in self.phase.iter_mut().zip([1.0, 1.0 - detune, 1.0 + detune]) {
            *phase = (*phase + step * ratio).fract();
        }
        self.sub = (self.sub + step * 0.5).fract();
        self.lfo = (self.lfo + 0.37 / rate).fract();
        let motion = (self.lfo * TAU).sin();
        let p = self.phase[0];
        match sound {
            // A decaying harmonic FM tine rather than a static sine mixture.
            Sound::Warm => {
                self.modulator = (self.modulator + step * 3.0).fract();
                let index = ((0.35 + tone * 2.4) * self.transient * velocity)
                    .min((0.45 / step - 1.0).max(0.0) / 3.0);
                (TAU * p + index * (TAU * self.modulator).sin()).sin()
                    * (0.28 + 0.72 * self.transient)
            }
            // Beating oscillators and a slowly moving pulse width add motion to held chords.
            Sound::Soft => {
                let width = 0.5 + motion * 0.17;
                (pulse(p, step, width) * 0.34
                    + saw(self.phase[1], step * (1.0 - detune)) * 0.33
                    + saw(self.phase[2], step * (1.0 + detune)) * 0.33)
                    * (0.85 + motion * 0.15)
            }
            Sound::Bass => {
                self.modulator = (self.modulator + step * 2.0).fract();
                let index =
                    ((0.2 + tone * 4.0) * self.transient).min((0.45 / step - 1.0).max(0.0) / 2.0);
                let body = (TAU * p + index * (TAU * self.modulator).sin()).sin();
                (body * 0.75 + (TAU * self.sub).sin() * 0.25) * (0.45 + 0.55 * self.transient)
            }
            Sound::Bright => {
                (saw(p, step)
                    + saw(self.phase[1], step * (1.0 - detune))
                    + saw(self.phase[2], step * (1.0 + detune)))
                    * 0.48
            }
            // Stable resonant state-variable filter, swept by a per-note envelope.
            // This is an acid-inspired voice, not a circuit emulation of a TB-303.
            Sound::Acid => {
                let cutoff =
                    (150.0 + tone * 1000.0 + self.transient * (900.0 + tone * 4500.0) * velocity)
                        .min(rate * 0.1);
                let f = 2.0 * (PI * cutoff / rate).sin();
                let input = saw(p, step) * 0.55;
                let high = input - self.low - 0.38 * self.band;
                self.band += f * high;
                self.low += f * self.band;
                (self.low * 1.8).tanh() * (0.35 + 0.65 * self.transient)
            }
            Sound::Glass => {
                self.modulator = (self.modulator + step * 2.414).fract();
                let index =
                    ((1.0 + tone * 5.0) * self.transient).min((0.45 / step - 1.0).max(0.0) / 2.414);
                (TAU * p + index * (TAU * self.modulator).sin()).sin()
                    * (0.06 + 0.94 * self.transient)
            }
            Sound::Pixel => {
                pulse(p, step, 0.18 + tone * 0.18) * 0.72 * (0.7 + 0.3 * self.transient)
            }
            Sound::Organ => {
                let mut wave = (TAU * self.sub).sin() * 0.25;
                for (harmonic, amount) in [
                    (1.0, 0.42),
                    (2.0, 0.24),
                    (3.0, 0.16),
                    (4.0, 0.08),
                    (6.0, 0.1 * self.transient),
                ] {
                    if step * harmonic < 0.48 {
                        wave += (TAU * p * harmonic + motion * 0.025 * harmonic).sin() * amount;
                    }
                }
                wave * (0.94 + motion * 0.06)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrected_saw_reduces_aliasing_at_high_pitch() {
        // Integer-cycle FFT bins: compare energy outside the legal harmonics.
        let length = 2048;
        let bin = 197;
        let step = bin as f32 / length as f32;
        let render = |corrected: bool| {
            (0..length)
                .map(|i| {
                    let p = (i as f32 * step).fract();
                    if corrected {
                        saw(p, step)
                    } else {
                        2.0 * p - 1.0
                    }
                })
                .collect::<Vec<_>>()
        };
        let residual = |samples: Vec<f32>| {
            let total = samples.iter().map(|v| v * v).sum::<f32>() / length as f32;
            let mut harmonic_energy = 0.0;
            for harmonic in 1..=5 {
                let b = bin * harmonic;
                let mut re = 0.0;
                let mut im = 0.0;
                for (i, v) in samples.iter().enumerate() {
                    let phase = TAU * b as f32 * i as f32 / length as f32;
                    re += v * phase.cos();
                    im += v * phase.sin();
                }
                harmonic_energy += 2.0 * (re * re + im * im) / (length * length) as f32;
            }
            total - harmonic_energy
        };
        assert!(residual(render(true)) < residual(render(false)) * 0.2);
    }
}
