//! Allocation-free mono effects; all buffers are allocated when the engine opens.
use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectSettings {
    pub enabled: bool,
    pub drive: u8,
    /// 100 is fully open; lower values darken the drums.
    pub filter: u8,
    pub delay: u8,
    pub reverb: u8,
}

impl Default for EffectSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            drive: 0,
            filter: 100,
            delay: 0,
            reverb: 0,
        }
    }
}

impl EffectSettings {
    pub fn validate(self) -> Result<(), &'static str> {
        if [self.drive, self.filter, self.delay, self.reverb]
            .iter()
            .any(|v| *v > 100)
        {
            return Err("Effect amounts must be between 0 and 100 percent.");
        }
        Ok(())
    }
}

struct DelayLine {
    samples: Vec<f32>,
    cursor: usize,
}
impl DelayLine {
    fn new(frames: usize) -> Self {
        Self {
            samples: vec![0.0; frames.max(2)],
            cursor: 0,
        }
    }
    fn read(&self, frames: f32) -> f32 {
        let position = (self.cursor as f32 - frames).rem_euclid(self.samples.len() as f32);
        let index = position as usize;
        let fraction = position - index as f32;
        self.samples[index] * (1.0 - fraction)
            + self.samples[(index + 1) % self.samples.len()] * fraction
    }
    fn push(&mut self, sample: f32) {
        self.samples[self.cursor] = sample;
        self.cursor = (self.cursor + 1) % self.samples.len();
    }
    fn clear(&mut self) {
        self.samples.fill(0.0);
        self.cursor = 0;
    }
}

pub struct Effects {
    sample_rate: f32,
    target: [f32; 7],
    current: [f32; 7],
    smoothing: f32,
    low: [f32; 2],
    delay: DelayLine,
    room: [DelayLine; 4],
    damp: [f32; 4],
    diffuser: DelayLine,
}

impl Effects {
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate as f32;
        let mut effects = Self {
            sample_rate: rate,
            target: [0.0; 7],
            current: [0.0; 7],
            smoothing: 1.0 - (-1.0 / (rate * 0.02)).exp(),
            low: [0.0; 2],
            delay: DelayLine::new((rate * 0.8).ceil() as usize + 2),
            room: [0.0297, 0.0371, 0.0411, 0.0437]
                .map(|seconds| DelayLine::new((rate * seconds) as usize)),
            damp: [0.0; 4],
            diffuser: DelayLine::new((rate * 0.005).round() as usize),
        };
        effects.configure(EffectSettings::default(), 100);
        effects.current = effects.target;
        effects
    }

    pub fn configure(&mut self, settings: EffectSettings, bpm: u16) {
        // Exponential cutoff mapping gives useful travel across the audible range.
        let cutoff = (180.0 * (100.0_f32).powf(f32::from(settings.filter) / 100.0))
            .min(self.sample_rate * 0.45);
        self.target = [
            if settings.enabled { 1.0 } else { 0.0 },
            f32::from(settings.drive) / 100.0,
            if settings.filter == 100 { 0.0 } else { 1.0 },
            1.0 - (-TAU * cutoff / self.sample_rate).exp(),
            f32::from(settings.delay) / 100.0,
            f32::from(settings.reverb) / 100.0,
            (self.sample_rate * 30.0 / f32::from(bpm))
                .clamp(1.0, (self.delay.samples.len() - 1) as f32),
        ];
    }

    pub fn clear(&mut self) {
        self.delay.clear();
        for line in &mut self.room {
            line.clear();
        }
        self.diffuser.clear();
        self.damp = [0.0; 4];
        self.low = [0.0; 2];
    }

    pub fn process(&mut self, dry: f32) -> f32 {
        let was_enabled = self.current[0] > 0.0;
        for (index, (current, target)) in self.current.iter_mut().zip(self.target).enumerate() {
            // Slew the read head by at most one frame per sample. Unlike a
            // floating-point exponential at large offsets this reaches the exact beat.
            *current += if index == 6 {
                (target - *current).clamp(-1.0, 1.0)
            } else {
                (target - *current) * self.smoothing
            };
            if (*current - target).abs() < 0.00001 {
                *current = target;
            }
        }
        let [
            enabled,
            drive,
            filtered,
            coefficient,
            delay_mix,
            room_mix,
            frames,
        ] = self.current;
        if enabled == 0.0 && self.target[0] == 0.0 {
            if was_enabled {
                self.clear();
            }
            return dry;
        }
        let driven = (dry * (1.0 + 8.0 * drive)).tanh() / (1.0 + 2.0 * drive);
        let mut sample = dry + drive * (driven - dry);
        self.low[0] += coefficient * (sample - self.low[0]);
        self.low[1] += coefficient * (self.low[0] - self.low[1]);
        sample += filtered * (self.low[1] - sample);
        let echo = self.delay.read(frames);
        self.delay.push(sample * delay_mix * enabled + echo * 0.38);
        sample += echo * delay_mix * 0.6;
        let mut room = 0.0;
        for (line, damp) in self.room.iter_mut().zip(&mut self.damp) {
            let tap = line.samples[line.cursor];
            *damp += 0.3 * (tap - *damp);
            line.push(sample * room_mix * enabled + *damp * 0.72);
            room += tap * 0.25;
        }
        let tap = self.diffuser.samples[self.diffuser.cursor];
        let diffused = tap - room * 0.5;
        self.diffuser.push(room + diffused * 0.5);
        sample += diffused * room_mix * 0.6;
        dry + enabled * (sample - dry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn configured(settings: EffectSettings, rate: u32) -> Effects {
        let mut fx = Effects::new(rate);
        fx.configure(settings, 120);
        for _ in 0..rate {
            fx.process(0.0);
        }
        fx
    }
    #[test]
    fn neutral_and_disabled_are_exactly_dry() {
        for enabled in [false, true] {
            let mut fx = configured(
                EffectSettings {
                    enabled,
                    ..Default::default()
                },
                48_000,
            );
            for sample in [-0.8, 0.0, 0.1, 0.6] {
                assert_eq!(fx.process(sample), sample);
            }
        }
    }
    #[test]
    fn delay_echo_lands_on_eighth_note() {
        for rate in [44_100, 48_000, 96_000] {
            let mut fx = configured(
                EffectSettings {
                    enabled: true,
                    delay: 100,
                    ..Default::default()
                },
                rate,
            );
            fx.process(1.0);
            for _ in 1..rate / 4 {
                assert!(fx.process(0.0).abs() < 0.0001);
            }
            assert!(fx.process(0.0) > 0.5);
        }
    }
    #[test]
    fn filter_attenuates_high_frequencies_and_drive_changes_signal() {
        let mut filter = configured(
            EffectSettings {
                enabled: true,
                filter: 0,
                ..Default::default()
            },
            48_000,
        );
        let mut energy = 0.0;
        for n in 0..4800 {
            energy += filter.process(if n % 2 == 0 { 0.5 } else { -0.5 }).powi(2);
        }
        assert!(energy < 0.1);
        let mut drive = configured(
            EffectSettings {
                enabled: true,
                drive: 80,
                ..Default::default()
            },
            48_000,
        );
        assert!((drive.process(0.1) - 0.1).abs() > 0.05);
    }
    #[test]
    fn room_tail_decays_and_clear_removes_it() {
        let mut fx = configured(
            EffectSettings {
                enabled: true,
                reverb: 100,
                ..Default::default()
            },
            48_000,
        );
        fx.process(1.0);
        let mut early = 0.0;
        for _ in 0..24_000 {
            early += fx.process(0.0).powi(2);
        }
        let mut late = 0.0;
        for _ in 0..96_000 {
            late += fx.process(0.0).powi(2);
        }
        assert!(early > 0.001 && late < early * 0.01);
        fx.clear();
        for _ in 0..48_000 {
            assert_eq!(fx.process(0.0), 0.0);
        }
    }
    #[test]
    fn bypass_settles_to_dry_and_discards_old_tails() {
        let settings = EffectSettings {
            enabled: true,
            delay: 100,
            reverb: 100,
            ..Default::default()
        };
        let mut fx = configured(settings, 48_000);
        for _ in 0..48_000 {
            fx.process(0.2);
        }
        fx.configure(
            EffectSettings {
                enabled: false,
                ..settings
            },
            120,
        );
        for _ in 0..48_000 {
            fx.process(0.0);
        }
        assert_eq!(fx.process(0.2), 0.2);
        fx.configure(settings, 120);
        for _ in 0..48_000 {
            assert_eq!(fx.process(0.0), 0.0);
        }
    }
}
