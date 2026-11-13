//! All timing comes from audio frames, never UI repaint or wall-clock timers.
//! Future cue or microphone controllers can submit the same typed commands as the UI.
use crate::{
    effects::{EffectSettings, Effects},
    groove::{Instrument, Pattern, STEPS, Style, fill_velocity, groove},
    keyboard::{KeyEvent, Keyboard, SynthSettings},
    synth::Mixer,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CustomPattern {
    pub id: u16,
    pub pattern: Pattern,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub custom: Option<CustomPattern>,
    pub style: Style,
    pub bpm: u16,
    /// Sixteenth-note swing, in percent of the straight subdivision.
    pub swing: u8,
    pub volume: u8,
    pub effects: EffectSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            style: Style::Rock,
            custom: None,
            bpm: 100,
            swing: 0,
            volume: 65,
            effects: EffectSettings::default(),
        }
    }
}

impl Settings {
    pub fn validate(self) -> Result<(), &'static str> {
        if !(40..=240).contains(&self.bpm) {
            return Err("Tempo must be between 40 and 240 BPM.");
        }
        if self.swing > 60 {
            return Err("Swing must be between 0 and 60 percent.");
        }
        if self.volume > 100 {
            return Err("Volume must be between 0 and 100 percent.");
        }
        self.effects.validate()
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Command {
    Keyboard(u8, KeyEvent),
    Synth(SynthSettings),
    PanicKeys,
    Hit(Instrument, u8),
    RecordedHit(Instrument, u8, usize),
    Apply(Settings),
    /// Audition an editor draft immediately without restarting the transport.
    Preview(Settings),
    Play,
    PlayOnce,
    Stop,
    /// Finish the current measure before stopping.
    QueueStop,
    CancelStop,
    Fill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub custom_id: u16,
    pub running: bool,
    pub stop_pending: bool,
    pub step: usize,
    pub bar: u64,
    pub style: Style,
    pub fill_pending: bool,
    pub filling: bool,
}

pub struct Engine {
    keyboard: Keyboard,
    skip_recorded: [[bool; STEPS]; crate::groove::INSTRUMENTS],
    live_mixer: Mixer,
    live_effects: Effects,
    sample_rate: u32,
    settings: Settings,
    position: Position,
    mixer: Mixer,
    effects: Effects,
    next_step: usize,
    started: bool,
    play_once: bool,
    ringing_out: bool,
    frames_until_step: f64,
    gain: f32,
    active_pattern: Pattern,
}

impl Engine {
    pub fn new(sample_rate: u32) -> Self {
        assert!(sample_rate > 0, "sample rate must be positive");
        Self {
            keyboard: Keyboard::new(sample_rate),
            skip_recorded: [[false; STEPS]; crate::groove::INSTRUMENTS],
            live_mixer: Mixer::new(sample_rate),
            live_effects: Effects::new(sample_rate),
            sample_rate,
            settings: Settings::default(),
            mixer: Mixer::new(sample_rate),
            effects: Effects::new(sample_rate),
            position: Position {
                custom_id: 0,
                running: false,
                stop_pending: false,
                step: 0,
                bar: 1,
                style: Style::Rock,
                fill_pending: false,
                filling: false,
            },
            next_step: 0,
            started: false,
            play_once: false,
            ringing_out: false,
            frames_until_step: 0.0,
            gain: 0.0,
            active_pattern: *groove(Style::Rock),
        }
    }

    pub fn position(&self) -> Position {
        self.position
    }

    /// Nearest sixteenth boundary on the audio clock, including swing and bar wrap.
    pub fn recording_step(&self) -> Option<usize> {
        if !self.position.running {
            return None;
        }
        if !self.started {
            return Some(0);
        }
        Some(
            if self.frames_until_step <= self.step_frames(self.position.step) / 2.0 {
                self.next_step
            } else {
                self.position.step
            },
        )
    }

    pub fn command(&mut self, command: Command) {
        match command {
            Command::Keyboard(source, event) => self.keyboard.event(source, event),
            Command::Synth(settings) => self.keyboard.configure(settings),
            Command::PanicKeys => self.keyboard.panic(),
            Command::RecordedHit(instrument, velocity, step) => {
                self.live_mixer.trigger(instrument, velocity.min(127));
                // A hit snapped forward has already been heard live. Let it join
                // the loop on the next pass rather than playing twice now.
                if step < STEPS && self.position.running && step == self.next_step {
                    self.skip_recorded[instrument.index()][step] = true;
                }
            }
            Command::Hit(instrument, velocity) => {
                self.live_mixer.trigger(instrument, velocity.min(127))
            }
            Command::PlayOnce => {
                self.command(Command::Stop);
                self.command(Command::Play);
                self.play_once = true;
            }
            Command::Preview(settings) => {
                if settings.validate().is_err() {
                    return;
                }
                self.command(Command::Apply(settings));
                self.position.style = settings.style;
                self.position.custom_id = settings.custom.map_or(0, |p| p.id);
                self.active_pattern = settings
                    .custom
                    .map_or(*groove(settings.style), |p| p.pattern);
            }
            Command::Apply(settings) => {
                if settings.validate().is_err() {
                    return;
                }
                self.frames_until_step *= f64::from(self.settings.bpm) / f64::from(settings.bpm);
                self.effects.configure(settings.effects, settings.bpm);
                self.live_effects.configure(settings.effects, settings.bpm);
                self.keyboard.set_bpm(settings.bpm);
                self.settings = settings;
                if !self.position.running {
                    self.position.style = settings.style;
                    self.position.custom_id = settings.custom.map_or(0, |p| p.id);
                }
            }
            Command::Play if !self.position.running => {
                self.ringing_out = false;
                self.skip_recorded = [[false; STEPS]; crate::groove::INSTRUMENTS];
                self.position = Position {
                    custom_id: self.settings.custom.map_or(0, |p| p.id),
                    running: true,
                    stop_pending: false,
                    step: 0,
                    bar: 1,
                    style: self.settings.style,
                    fill_pending: false,
                    filling: false,
                };
                self.effects.clear();
                self.play_once = false;
                self.started = false;
                self.next_step = 0;
                self.frames_until_step = 0.0;
            }
            Command::QueueStop if self.position.running => {
                self.position.stop_pending = true;
            }
            Command::CancelStop => self.position.stop_pending = false,
            Command::Stop => {
                self.ringing_out = false;
                self.position.stop_pending = false;
                self.play_once = false;
                self.position.running = false;
                self.position.fill_pending = false;
                self.position.filling = false;
                self.position.step = 0;
                self.position.bar = 1;
                self.position.style = self.settings.style;
                self.position.custom_id = self.settings.custom.map_or(0, |p| p.id);
                self.active_pattern = self
                    .settings
                    .custom
                    .map_or(*groove(self.settings.style), |p| p.pattern);
            }
            Command::Fill if self.position.running && !self.position.filling => {
                self.position.fill_pending = true;
            }
            _ => {}
        }
    }

    fn trigger_step(&mut self) {
        if self.next_step == 0 {
            if self.started {
                self.position.bar += 1;
            }
            self.started = true;
            let was_filling = self.position.filling;
            self.position.style = self.settings.style;
            self.position.custom_id = self.settings.custom.map_or(0, |p| p.id);
            self.active_pattern = self
                .settings
                .custom
                .map_or(*groove(self.settings.style), |p| p.pattern);
            self.position.filling = false;
            if was_filling {
                self.mixer.trigger(Instrument::Crash, 95);
            }
        }
        if std::mem::take(&mut self.position.fill_pending) {
            self.position.filling = true;
        }
        let step = self.next_step;
        self.position.step = step;
        for instrument in Instrument::ALL {
            if std::mem::take(&mut self.skip_recorded[instrument.index()][step]) {
                continue;
            }
            let velocity = if self.position.filling {
                // Repeat the eight-step phrase in either half of the bar so
                // every entry point is audible without moving the audio clock.
                fill_velocity(instrument, 8 + step % 8)
            } else {
                self.active_pattern.velocity(instrument, step)
            };
            self.mixer.trigger(instrument, velocity);
        }
        self.frames_until_step += self.step_frames(step);
        self.next_step = (step + 1) % STEPS;
    }

    fn step_frames(&self, step: usize) -> f64 {
        let base = f64::from(self.sample_rate) * 60.0 / f64::from(self.settings.bpm) / 4.0;
        let eighth_swing = self.position.style.eighth_swing();
        let eighth_factor = if step % 4 < 2 {
            1.0 + eighth_swing
        } else {
            1.0 - eighth_swing
        };
        let swing = f64::from(self.settings.swing) / 100.0;
        let sixteenth_factor = if step.is_multiple_of(2) {
            1.0 + swing
        } else {
            1.0 - swing
        };
        base * eighth_factor * sixteenth_factor
    }

    pub fn next_sample(&mut self) -> f32 {
        if self.position.running {
            // Quantize to the nearest frame; fractional remainders carry into the
            // next step, including tiny floating-point errors at bar boundaries.
            if self.frames_until_step < 0.5 {
                if (self.play_once || self.position.stop_pending)
                    && self.started
                    && self.next_step == 0
                {
                    let ring_out = self.position.stop_pending;
                    self.command(Command::Stop);
                    self.ringing_out = ring_out;
                } else {
                    self.trigger_step();
                }
            }
            self.frames_until_step -= 1.0;
        }
        let target = if self.position.running || self.ringing_out {
            f32::from(self.settings.volume) / 100.0
        } else {
            0.0
        };
        // Ten-millisecond gain ramp avoids a discontinuity on transport/volume changes.
        let delta = 1.0 / (self.sample_rate as f32 * 0.01);
        self.gain += (target - self.gain).clamp(-delta, delta);
        let groove = self.effects.process(self.mixer.next_sample()) * self.gain;
        let live_input = self.live_mixer.next_sample() * (f32::from(self.settings.volume) / 100.0)
            + self.keyboard.next_sample();
        let live = self.live_effects.process(live_input);
        ((groove + live) * 0.7).tanh()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn midi_synth_is_independent_of_drum_stop_and_volume_and_uses_all_effects() {
        use super::*;
        let render = |effects| {
            let mut engine = Engine::new(48_000);
            engine.command(Command::Apply(Settings {
                volume: 0,
                effects,
                ..Settings::default()
            }));
            engine.command(Command::Synth(SynthSettings {
                arp: true,
                notes: crate::keyboard::Notes::Minor,
                ..SynthSettings::default()
            }));
            engine.command(Command::Keyboard(
                0,
                KeyEvent::On {
                    note: 72,
                    velocity: 110,
                },
            ));
            engine.command(Command::Stop);
            assert!(!engine.position().running);
            (0..48_000)
                .map(|_| engine.next_sample())
                .collect::<Vec<_>>()
        };
        let dry = render(EffectSettings {
            enabled: true,
            ..EffectSettings::default()
        });
        assert!(dry.iter().map(|s| s.abs()).sum::<f32>() > 100.0);
        for effects in [
            EffectSettings {
                enabled: true,
                drive: 70,
                ..EffectSettings::default()
            },
            EffectSettings {
                enabled: true,
                filter: 10,
                ..EffectSettings::default()
            },
            EffectSettings {
                enabled: true,
                delay: 70,
                ..EffectSettings::default()
            },
            EffectSettings {
                enabled: true,
                reverb: 70,
                ..EffectSettings::default()
            },
        ] {
            let wet = render(effects);
            assert!(wet.iter().all(|s| s.is_finite() && s.abs() < 1.0));
            let difference: f32 = wet.iter().zip(&dry).map(|(w, d)| (w - d).abs()).sum();
            assert!(
                difference > 1.0,
                "Effect did not affect MIDI synth: {effects:?}"
            );
        }
    }

    #[test]
    fn synth_panic_and_test_note_release_stop_arpeggiation() {
        use super::*;
        let mut engine = Engine::new(48_000);
        engine.command(Command::Synth(SynthSettings {
            arp: true,
            hold: true,
            ..SynthSettings::default()
        }));
        engine.command(Command::Keyboard(
            crate::keyboard::TEST_SOURCE,
            KeyEvent::On {
                note: 48,
                velocity: 100,
            },
        ));
        for _ in 0..1000 {
            engine.next_sample();
        }
        engine.command(Command::Keyboard(
            crate::keyboard::TEST_SOURCE,
            KeyEvent::AllOff,
        ));
        for _ in 0..48_000 * 2 {
            engine.next_sample();
        }
        assert_eq!(engine.next_sample(), 0.0);
        engine.command(Command::Keyboard(
            0,
            KeyEvent::On {
                note: 60,
                velocity: 100,
            },
        ));
        for _ in 0..1000 {
            engine.next_sample();
        }
        engine.command(Command::PanicKeys);
        for _ in 0..1000 {
            engine.next_sample();
        }
        assert_eq!(engine.next_sample(), 0.0);
    }

    #[test]
    fn recording_snaps_on_audio_clock_with_swing_and_wrap() {
        use super::*;
        let mut engine = Engine::new(48_000);
        assert_eq!(engine.recording_step(), None);
        engine.command(Command::Apply(Settings {
            swing: 40,
            ..Settings::default()
        }));
        engine.command(Command::Play);
        assert_eq!(engine.recording_step(), Some(0));
        for step in 0..STEPS {
            engine.trigger_step();
            let duration = engine.step_frames(step);
            engine.frames_until_step = duration * 0.75;
            assert_eq!(engine.recording_step(), Some(step));
            engine.frames_until_step = duration * 0.25;
            assert_eq!(engine.recording_step(), Some((step + 1) % STEPS));
        }
        engine.command(Command::Stop);
        assert_eq!(engine.recording_step(), None);
    }

    #[test]
    fn forward_recorded_hit_plays_live_then_joins_the_next_loop() {
        use super::*;
        let mut engine = Engine::new(48_000);
        let mut pattern = Pattern {
            hits: [[0; STEPS]; crate::groove::INSTRUMENTS],
        };
        pattern.hits[Instrument::Snare.index()][1] = 90;
        engine.command(Command::Preview(Settings {
            custom: Some(CustomPattern {
                id: u16::MAX,
                pattern,
            }),
            ..Settings::default()
        }));
        engine.command(Command::Play);
        engine.trigger_step();
        engine.command(Command::RecordedHit(Instrument::Snare, 90, 1));
        engine.trigger_step();
        let groove_energy: f32 = (0..1000).map(|_| engine.mixer.next_sample().abs()).sum();
        let live_energy: f32 = (0..1000)
            .map(|_| engine.live_mixer.next_sample().abs())
            .sum();
        assert_eq!(groove_energy, 0.0);
        assert!(live_energy > 0.0);
        for _ in 0..STEPS {
            engine.trigger_step();
        }
        assert!(
            (0..1000)
                .map(|_| engine.mixer.next_sample().abs())
                .sum::<f32>()
                > 0.0
        );
    }

    #[test]
    fn live_hits_work_after_transport_stop_and_use_effect_settings() {
        use super::*;
        let render = |played_groove, filter| {
            let mut engine = Engine::new(48_000);
            engine.command(Command::Apply(Settings {
                effects: EffectSettings {
                    enabled: true,
                    filter,
                    ..EffectSettings::default()
                },
                ..Settings::default()
            }));
            if played_groove {
                engine.command(Command::Play);
            }
            for _ in 0..2_000 {
                engine.next_sample();
            }
            engine.command(Command::Stop);
            for _ in 0..48_000 {
                engine.next_sample();
            }
            engine.command(Command::Hit(Instrument::Snare, 100));
            let samples: Vec<_> = (0..24_000).map(|_| engine.next_sample()).collect();
            assert!(!engine.position().running);
            samples
        };
        let fresh = render(false, 100);
        assert_eq!(fresh, render(true, 100));
        let dark = render(false, 0);
        let energy = |samples: &[f32]| samples.iter().map(|s| s * s).sum::<f32>();
        assert!(energy(&fresh) > energy(&dark) * 2.0);
    }

    #[test]
    fn live_hits_are_velocity_sensitive_without_starting_a_groove() {
        use super::*;
        for instrument in Instrument::ALL {
            let energy = |velocity| {
                let mut engine = Engine::new(48_000);
                engine.command(Command::Hit(instrument, velocity));
                let energy: f32 = (0..24_000).map(|_| engine.next_sample().powi(2)).sum();
                assert!(!engine.position().running);
                assert_eq!(engine.position().step, 0);
                energy
            };
            assert_eq!(energy(0), 0.0);
            let soft = energy(32);
            let hard = energy(127);
            assert!(soft > 0.0);
            assert!(hard > soft * 8.0, "{instrument:?}: {soft} vs {hard}");
        }
    }
    use super::*;

    fn playing(bpm: u16) -> Engine {
        let mut engine = Engine::new(48_000);
        engine.command(Command::Apply(Settings {
            bpm,
            ..Settings::default()
        }));
        engine.command(Command::Play);
        engine
    }

    fn advance(engine: &mut Engine, frames: usize) {
        for _ in 0..frames {
            engine.next_sample();
        }
    }

    #[test]
    fn queued_stop_finishes_the_measure_without_triggering_next_downbeat() {
        for bpm in [120, 240] {
            for swing in [0, 60] {
                for request_frame in [0, 12_345, 47_999] {
                    let mut engine = playing(bpm);
                    engine.command(Command::Apply(Settings {
                        bpm,
                        swing,
                        ..Default::default()
                    }));
                    advance(&mut engine, request_frame);
                    engine.command(Command::QueueStop);
                    engine.command(Command::QueueStop);
                    assert!(engine.position().stop_pending);
                    advance(&mut engine, 48_000 * 240 / usize::from(bpm) - request_frame);
                    assert!(engine.position().running);
                    assert_eq!(engine.position().step, 15);
                    engine.next_sample();
                    assert!(!engine.position().running);
                    assert!(!engine.position().stop_pending);
                    assert_eq!(engine.position().bar, 1);
                    engine.command(Command::Play);
                    advance(&mut engine, 100_000);
                    assert!(engine.position().running);
                }
            }
        }
    }

    #[test]
    fn queued_stop_preserves_full_drum_and_effect_tails() {
        for enabled in [false, true] {
            let mut pattern = Pattern {
                hits: [[0; STEPS]; crate::groove::INSTRUMENTS],
            };
            pattern.hits[Instrument::Crash.index()][15] = 127;
            let settings = Settings {
                bpm: 120,
                custom: Some(CustomPattern { id: 1, pattern }),
                effects: EffectSettings {
                    enabled,
                    delay: 80,
                    reverb: 80,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut stopped = Engine::new(48_000);
            let mut reference = Engine::new(48_000);
            for engine in [&mut stopped, &mut reference] {
                engine.command(Command::Apply(settings));
                engine.command(Command::Play);
                advance(engine, 96_000);
            }
            stopped.command(Command::QueueStop);
            // The reference keeps running with an empty pattern: identical tails,
            // but no new hits, even after multiple measure boundaries.
            reference.command(Command::Apply(Settings {
                custom: Some(CustomPattern {
                    id: 2,
                    pattern: Pattern {
                        hits: [[0; STEPS]; crate::groove::INSTRUMENTS],
                    },
                }),
                ..settings
            }));
            let mut tail_energy = 0.0;
            for frame in 0..240_000 {
                let sample = stopped.next_sample();
                assert_eq!(
                    sample,
                    reference.next_sample(),
                    "tail differs at frame {frame}"
                );
                if frame > 480 {
                    tail_energy += sample.abs();
                }
            }
            assert!(!stopped.position().running);
            assert!(!stopped.position().stop_pending);
            assert!(tail_energy > 1.0);
        }
    }

    #[test]
    fn queued_stop_can_be_cancelled_and_tracks_tempo_changes() {
        let mut engine = playing(120);
        advance(&mut engine, 24_000);
        engine.command(Command::QueueStop);
        engine.command(Command::CancelStop);
        advance(&mut engine, 72_001);
        assert!(engine.position().running);
        assert_eq!(engine.position().bar, 2);
        assert!(!engine.position().stop_pending);

        let mut engine = playing(120);
        advance(&mut engine, 24_000);
        engine.command(Command::QueueStop);
        engine.command(Command::Apply(Settings {
            bpm: 240,
            ..Default::default()
        }));
        advance(&mut engine, 36_000);
        assert!(engine.position().running);
        engine.next_sample();
        assert!(!engine.position().running);
        engine.command(Command::QueueStop);
        assert!(!engine.position().stop_pending);
    }

    #[test]
    fn play_once_stops_after_exactly_one_groove_bar() {
        for bpm in [120, 240] {
            let mut engine = Engine::new(48_000);
            engine.command(Command::Apply(Settings {
                bpm,
                ..Default::default()
            }));
            engine.command(Command::PlayOnce);
            advance(&mut engine, 48_000 * 240 / usize::from(bpm));
            assert!(engine.position().running);
            engine.next_sample();
            assert!(!engine.position().running);
            advance(&mut engine, 1000);
            assert_eq!(engine.next_sample(), 0.0);
            engine.command(Command::Play);
            advance(&mut engine, 200_000);
            assert!(engine.position().running);
        }
    }

    #[test]
    fn first_hit_is_immediate_and_bar_has_exact_duration() {
        let mut engine = playing(120);
        advance(&mut engine, 96_000);
        assert_eq!(engine.position().bar, 1);
        assert_eq!(engine.position().step, 15);
        engine.next_sample();
        assert_eq!(engine.position().bar, 2);
        assert_eq!(engine.position().step, 0);
    }

    #[test]
    fn fractional_tempo_has_no_accumulated_frame_drift() {
        let mut engine = playing(137);
        let frames = (48_000.0_f64 * 240.0 / 137.0 * 20.0).ceil() as usize;
        advance(&mut engine, frames);
        engine.next_sample();
        assert_eq!(engine.position().bar, 21);
        assert_eq!(engine.position().step, 0);
    }

    #[test]
    fn shuffle_and_swing_preserve_bar_length() {
        for style in Style::ALL {
            let mut engine = playing(120);
            engine.command(Command::Apply(Settings {
                style,
                bpm: 120,
                swing: 60,
                ..Settings::default()
            }));
            advance(&mut engine, 96_001);
            assert_eq!(engine.position().bar, 2, "{style:?}");
            assert_eq!(engine.position().step, 0, "{style:?}");
        }
    }

    #[test]
    fn fill_starts_next_step_while_style_waits_for_bar_boundary() {
        let mut engine = playing(120);
        advance(&mut engine, 24_000);
        engine.command(Command::Apply(Settings {
            style: Style::Funk,
            bpm: 120,
            ..Settings::default()
        }));
        engine.command(Command::Fill);
        assert_eq!(engine.position().style, Style::Rock);
        assert!(engine.position().fill_pending);
        engine.next_sample();
        assert_eq!(engine.position().step, 4);
        assert_eq!(engine.position().style, Style::Rock);
        assert!(engine.position().filling);
        assert!(!engine.position().fill_pending);
        engine.command(Command::Fill);
        assert!(!engine.position().fill_pending);
        advance(&mut engine, 72_000);
        assert_eq!(engine.position().style, Style::Funk);
        assert_eq!(engine.position().bar, 2);
        assert_eq!(engine.position().step, 0);
        assert!(!engine.position().filling);
    }

    #[test]
    fn fill_is_audible_at_every_entry_step_without_restarting_clock() {
        for entry in 0..STEPS {
            let mut engine = playing(120);
            engine.command(Command::Preview(Settings {
                bpm: 120,
                custom: Some(CustomPattern {
                    id: 1,
                    pattern: Pattern {
                        hits: [[0; STEPS]; crate::groove::INSTRUMENTS],
                    },
                }),
                ..Settings::default()
            }));
            // Include entry at the next downbeat, requested during step 15.
            let boundary = if entry == 0 { 96_000 } else { entry * 6000 };
            advance(&mut engine, boundary - 3000);
            engine.command(Command::Fill);
            advance(&mut engine, 3000);
            assert!(engine.position().fill_pending);
            assert!(!engine.position().filling);
            engine.next_sample();
            assert!(engine.position().filling, "entry {entry}");
            assert!(!engine.position().fill_pending);
            assert_eq!(engine.position().step, entry);
            assert_eq!(engine.position().bar, if entry == 0 { 2 } else { 1 });
            let energy: f32 = (0..1000).map(|_| engine.next_sample().abs()).sum();
            assert!(energy > 0.0, "entry {entry} must be audible");
            advance(&mut engine, (STEPS - entry) * 6000 - 1000);
            assert_eq!(engine.position().step, 0);
            assert!(!engine.position().filling);
            let crash_energy: f32 = (0..1000).map(|_| engine.next_sample().abs()).sum();
            assert!(crash_energy > 0.0);
        }
    }

    #[test]
    fn stop_cancels_pending_and_active_fills() {
        let mut engine = playing(120);
        engine.command(Command::Fill);
        engine.command(Command::Stop);
        assert!(!engine.position().fill_pending);
        engine.command(Command::Fill);
        assert!(!engine.position().fill_pending);
        engine.command(Command::Play);
        engine.command(Command::Fill);
        engine.next_sample();
        assert!(engine.position().filling);
        engine.command(Command::Stop);
        assert!(!engine.position().filling);
        engine.command(Command::Play);
        engine.next_sample();
        assert!(!engine.position().filling);
    }

    #[test]
    fn restart_resets_transport_to_first_downbeat() {
        let mut engine = Engine::new(48_000);
        engine.command(Command::Apply(Settings {
            bpm: 120,
            ..Settings::default()
        }));
        engine.command(Command::Play);
        advance(&mut engine, 96_000);
        assert!(engine.position().running);
        engine.next_sample();
        assert_eq!(engine.position().bar, 2);
        engine.command(Command::Stop);
        advance(&mut engine, 1000);
        assert_eq!(engine.next_sample(), 0.0);
        engine.command(Command::Play);
        engine.next_sample();
        assert!(engine.position().running);
        assert_eq!(engine.position().step, 0);
    }

    #[test]
    fn custom_edits_and_factory_return_wait_for_the_next_bar() {
        let mut engine = playing(120);
        advance(&mut engine, 24_000);
        let mut custom = CustomPattern {
            id: 1,
            pattern: *groove(Style::Blues),
        };
        custom.pattern.hits[Instrument::Kick.index()][0] = 0;
        let mut settings = Settings {
            style: Style::Blues,
            custom: Some(custom),
            bpm: 120,
            ..Settings::default()
        };
        engine.command(Command::Apply(settings));
        assert_eq!(engine.active_pattern, *groove(Style::Rock));
        assert_eq!(engine.position().custom_id, 0);
        advance(&mut engine, 72_001);
        assert_eq!(engine.position().custom_id, 1);
        assert_eq!(engine.active_pattern, custom.pattern);
        assert_eq!(engine.step_frames(0), 8000.0);
        custom.pattern.hits[Instrument::Rim.index()][4] = 25;
        settings.custom = Some(custom);
        engine.command(Command::Apply(settings));
        assert_ne!(engine.active_pattern, custom.pattern);
        advance(&mut engine, 96_000);
        assert_eq!(engine.active_pattern, custom.pattern);
        settings.custom = None;
        engine.command(Command::Apply(settings));
        assert_eq!(engine.position().custom_id, 1);
        advance(&mut engine, 96_000);
        assert_eq!(engine.position().custom_id, 0);
        assert_eq!(engine.active_pattern, *groove(Style::Blues));
    }

    #[test]
    fn custom_patterns_drive_audio_and_settings_update_live() {
        let mut engine = Engine::new(48_000);
        let mut settings = Settings {
            custom: Some(CustomPattern {
                id: 3,
                pattern: Pattern {
                    hits: [[0; STEPS]; crate::groove::INSTRUMENTS],
                },
            }),
            bpm: 120,
            ..Settings::default()
        };
        engine.command(Command::Apply(settings));
        engine.command(Command::Play);
        for _ in 0..24_000 {
            assert_eq!(engine.next_sample(), 0.0);
        }
        settings.volume = 35;
        settings.bpm = 60;
        settings.swing = 30;
        engine.command(Command::Apply(settings));
        assert_eq!(engine.settings, settings);
        engine.command(Command::Stop);
        engine.command(Command::Play);
        assert!(engine.position().running);
        assert_eq!(engine.position().custom_id, 3);
    }

    #[test]
    fn editor_preview_updates_live_then_restores_normal_queued_playback() {
        let mut engine = playing(120);
        advance(&mut engine, 24_001);
        let before = engine.position();
        let draft = Settings {
            custom: Some(CustomPattern {
                id: u16::MAX,
                pattern: Pattern {
                    hits: [[0; STEPS]; crate::groove::INSTRUMENTS],
                },
            }),
            bpm: 112,
            volume: 40,
            ..Settings::default()
        };
        engine.command(Command::Preview(draft));
        assert_eq!(engine.position.step, before.step);
        assert_eq!(engine.position.bar, before.bar);
        assert_eq!(engine.active_pattern, draft.custom.unwrap().pattern);
        let mut edited = draft;
        edited.custom.as_mut().unwrap().pattern.hits[Instrument::Kick.index()][8] = 100;
        engine.command(Command::Preview(edited));
        assert_eq!(engine.active_pattern.velocity(Instrument::Kick, 8), 100);
        let mut invalid = edited;
        invalid.bpm = 0;
        engine.command(Command::Preview(invalid));
        assert_eq!(engine.settings, edited);
        engine.command(Command::Stop);
        engine.command(Command::Apply(Settings {
            bpm: 120,
            ..Settings::default()
        }));
        engine.command(Command::Play);
        engine.next_sample();
        assert_eq!(engine.position.custom_id, 0);
        assert_eq!(engine.active_pattern, *groove(Style::Rock));
        engine.command(Command::Apply(Settings {
            style: Style::Samba,
            bpm: 120,
            ..Settings::default()
        }));
        assert_eq!(engine.position.style, Style::Rock);
        advance(&mut engine, 96_000);
        assert_eq!(engine.position.style, Style::Samba);
    }

    #[test]
    fn live_effect_changes_are_bounded_and_stop_still_silences_tails() {
        let mut settings = Settings {
            volume: 100,
            ..Default::default()
        };
        for rate in [44_100, 48_000, 96_000] {
            let mut engine = Engine::new(rate);
            engine.command(Command::Apply(settings));
            engine.command(Command::Play);
            for filter in [0, 50, 100] {
                settings.effects = EffectSettings {
                    enabled: true,
                    drive: 100,
                    filter,
                    delay: 100,
                    reverb: 100,
                };
                settings.bpm = if filter == 50 { 40 } else { 240 };
                engine.command(Command::Apply(settings));
                for _ in 0..rate {
                    let sample = engine.next_sample();
                    assert!(sample.is_finite() && sample.abs() <= 1.0);
                }
            }
            let valid = engine.settings;
            settings.effects.drive = 101;
            engine.command(Command::Apply(settings));
            assert_eq!(engine.settings, valid);
            settings = valid;
            engine.command(Command::Stop);
            advance(&mut engine, rate as usize / 50);
            assert_eq!(engine.next_sample(), 0.0);
        }
    }

    #[test]
    fn every_groove_produces_finite_bounded_audio() {
        for style in Style::ALL {
            let mut engine = playing(240);
            engine.command(Command::Apply(Settings {
                style,
                bpm: 240,
                volume: 100,
                ..Settings::default()
            }));
            engine.command(Command::Fill);
            let mut peak = 0.0_f32;
            for _ in 0..96_000 {
                let sample = engine.next_sample();
                assert!(sample.is_finite() && sample.abs() <= 1.0);
                peak = peak.max(sample.abs());
            }
            assert!(peak > 0.1, "{style:?} must be audible");
        }
    }
}
