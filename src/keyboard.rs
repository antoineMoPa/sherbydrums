//! Polyphonic keyboard voices and arpeggiation, driven exclusively by audio frames.
use crate::timbre::Timbre;
use std::f32::consts::TAU;

pub const MIDI_SOURCES: usize = 16;
pub const TEST_SOURCE: u8 = MIDI_SOURCES as u8;
const SOURCES: usize = MIDI_SOURCES + 1;
const VOICES: usize = 32;

macro_rules! choices {
    ($name:ident { $($variant:ident => $label:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub fn label(self) -> &'static str { match self { $(Self::$variant => $label),+ } }
        }
    };
}
choices!(Sound {
    Warm => "Velvet keys", Soft => "Orbit pad", Bass => "Rubber bass", Bright => "Neon stack",
    Acid => "Acid bite", Glass => "Glass FM", Pixel => "Pixel pulse", Organ => "Tape organ"
});
impl Sound {
    pub fn stepped(self, steps: i16) -> Self {
        let index = Self::ALL.iter().position(|sound| *sound == self).unwrap() as i16;
        Self::ALL[(index + steps).rem_euclid(Self::ALL.len() as i16) as usize]
    }
}
choices!(Notes { Played => "Played", Major => "Major", Minor => "Minor", Minor7 => "Minor 7", Sus2 => "Sus 2" });
choices!(Order { Up => "Up", Down => "Down", UpDown => "Up / down", Random => "Random", AsPlayed => "As played" });
choices!(Rate { Quarter => "1/4", Eighth => "1/8", Triplet => "1/8 triplet", Sixteenth => "1/16", ThirtySecond => "1/32" });
impl Notes {
    pub fn intervals(self) -> &'static [u8] {
        match self {
            Self::Played => &[0],
            Self::Major => &[0, 4, 7],
            Self::Minor => &[0, 3, 7],
            Self::Minor7 => &[0, 3, 7, 10],
            Self::Sus2 => &[0, 2, 7],
        }
    }
}
impl Rate {
    fn per_beat(self) -> f64 {
        match self {
            Self::Quarter => 1.0,
            Self::Eighth => 2.0,
            Self::Triplet => 3.0,
            Self::Sixteenth => 4.0,
            Self::ThirtySecond => 8.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SynthSettings {
    pub enabled: bool,
    pub sound: Sound,
    pub tone: u8,
    pub attack_ms: u16,
    pub release_ms: u16,
    pub volume: u8,
    pub arp: bool,
    pub notes: Notes,
    pub order: Order,
    pub rate: Rate,
    pub octaves: u8,
    pub gate: u8,
    pub hold: bool,
}
impl Default for SynthSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            sound: Sound::Warm,
            tone: 68,
            attack_ms: 6,
            release_ms: 500,
            volume: 65,
            arp: false,
            notes: Notes::Minor,
            order: Order::Up,
            rate: Rate::Sixteenth,
            octaves: 1,
            gate: 65,
            hold: false,
        }
    }
}
impl SynthSettings {
    pub fn validate(self) -> Result<(), &'static str> {
        if self.tone > 100
            || self.volume > 100
            || !(10..=100).contains(&self.gate)
            || self.attack_ms > 1000
            || !(30..=2000).contains(&self.release_ms)
            || !(1..=3).contains(&self.octaves)
        {
            return Err("Invalid keyboard synth settings.");
        }
        Ok(())
    }
    pub fn select_sound(&mut self, sound: Sound) {
        self.sound = sound;
        (self.tone, self.attack_ms, self.release_ms) = match sound {
            Sound::Warm => (68, 6, 500),
            Sound::Soft => (48, 420, 1400),
            Sound::Bass => (65, 4, 140),
            Sound::Bright => (85, 10, 300),
            Sound::Acid => (75, 3, 110),
            Sound::Glass => (90, 3, 1100),
            Sound::Pixel => (82, 3, 80),
            Sound::Organ => (78, 4, 180),
        };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEvent {
    On { note: u8, velocity: u8 },
    Off { note: u8 },
    Sustain(bool),
    AllOff,
}
impl KeyEvent {
    /// Channel 1 keyboard only; channel 10 remains exclusively drum pads.
    pub fn from_midi(bytes: &[u8]) -> Option<Self> {
        match bytes {
            [0x90, note @ 0..=127, velocity @ 1..=127] => Some(Self::On {
                note: *note,
                velocity: *velocity,
            }),
            [0x80, note @ 0..=127, 0..=127] | [0x90, note @ 0..=127, 0] => {
                Some(Self::Off { note: *note })
            }
            [0xb0, 64, value @ 0..=127] => Some(Self::Sustain(*value >= 64)),
            [0xb0, 120 | 123, 0..=127] => Some(Self::AllOff),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Voice {
    note: u8,
    velocity: f32,
    timbre: Timbre,
    increment: f32,
    level: f32,
    release_step: f32,
    active: bool,
    releasing: bool,
    low: f32,
}

pub struct Keyboard {
    settings: SynthSettings,
    sample_rate: f32,
    bpm: u16,
    keys: [[u8; 128]; SOURCES],
    pressed: [[bool; 128]; SOURCES],
    pedal: [bool; SOURCES],
    latched: [u8; 128],
    order: [u64; 128],
    serial: u64,
    sequence: [(u8, u8); 384],
    len: usize,
    step: usize,
    frames: f64,
    gate_frames: f64,
    desired: [u8; 128],
    voices: [Voice; VOICES],
    filter: f32,
    color_decay: f32,
    tone: f32,
    gain: f32,
    random: u32,
    pub sounding_note: Option<u8>,
}
impl Keyboard {
    pub fn new(sample_rate: u32) -> Self {
        let mut keyboard = Self {
            settings: SynthSettings::default(),
            sample_rate: sample_rate as f32,
            bpm: 100,
            keys: [[0; 128]; SOURCES],
            pressed: [[false; 128]; SOURCES],
            pedal: [false; SOURCES],
            latched: [0; 128],
            order: [0; 128],
            serial: 0,
            sequence: [(0, 0); 384],
            len: 0,
            step: 0,
            frames: 0.0,
            gate_frames: 0.0,
            desired: [0; 128],
            voices: [Voice::default(); VOICES],
            filter: 0.0,
            color_decay: 0.0,
            tone: 0.0,
            gain: 0.0,
            random: 0x724a_138b,
            sounding_note: None,
        };
        keyboard.configure(SynthSettings::default());
        keyboard
    }
    pub fn set_bpm(&mut self, bpm: u16) {
        if !(40..=240).contains(&bpm) {
            return;
        }
        let ratio = f64::from(self.bpm) / f64::from(bpm);
        self.frames *= ratio;
        self.gate_frames *= ratio;
        self.bpm = bpm;
    }
    pub fn configure(&mut self, settings: SynthSettings) {
        if settings.validate().is_err() {
            return;
        }
        let reset = self.settings.arp != settings.arp
            || (settings.arp
                && (self.settings.notes != settings.notes
                    || self.settings.order != settings.order
                    || self.settings.octaves != settings.octaves));
        let ratio = self.settings.rate.per_beat() / settings.rate.per_beat();
        self.frames *= ratio;
        self.gate_frames *= ratio;
        if !settings.hold {
            self.latched.fill(0);
        }
        if self.settings.sound != settings.sound {
            for voice in &mut self.voices {
                if voice.active && !voice.releasing {
                    voice.timbre.restart_color();
                }
            }
        }
        self.settings = settings;
        let cutoff =
            (180.0 * 100.0_f32.powf(f32::from(settings.tone) / 100.0)).min(self.sample_rate * 0.45);
        self.filter = 1.0 - (-TAU * cutoff / self.sample_rate).exp();
        self.color_decay = Timbre::decay(settings.sound, self.sample_rate);
        if !settings.enabled {
            self.panic();
            return;
        }
        if reset {
            self.release_all();
            self.desired.fill(0);
            self.step = 0;
            self.frames = 0.0;
        }
        self.rebuild();
    }
    pub fn event(&mut self, source: u8, event: KeyEvent) {
        let source = usize::from(source);
        if source >= SOURCES {
            return;
        }
        match event {
            KeyEvent::On { note, velocity }
                if note < 128 && velocity > 0 && velocity < 128 && self.settings.enabled =>
            {
                if !self.keys.iter().flatten().any(|v| *v > 0) {
                    self.latched.fill(0);
                }
                if !self.settings.arp {
                    self.release_note(note);
                    self.desired[usize::from(note)] = 0;
                }
                self.serial = self.serial.wrapping_add(1);
                self.order[usize::from(note)] = self.serial;
                self.keys[source][usize::from(note)] = velocity;
                self.pressed[source][usize::from(note)] = true;
            }
            KeyEvent::Off { note } if note < 128 => {
                self.pressed[source][usize::from(note)] = false;
                if !self.pedal[source] {
                    self.keys[source][usize::from(note)] = 0;
                }
            }
            KeyEvent::Sustain(down) => {
                self.pedal[source] = down;
                if !down {
                    for note in 0..128 {
                        if !self.pressed[source][note] {
                            self.keys[source][note] = 0;
                        }
                    }
                }
            }
            KeyEvent::AllOff => {
                self.keys[source].fill(0);
                self.pressed[source].fill(false);
                self.pedal[source] = false;
                self.latched.fill(0);
            }
            _ => return,
        }
        self.rebuild();
    }
    pub fn panic(&mut self) {
        self.keys.fill([0; 128]);
        self.pressed.fill([false; 128]);
        self.pedal.fill(false);
        self.latched.fill(0);
        self.desired.fill(0);
        self.len = 0;
        self.frames = 0.0;
        self.step = 0;
        self.sounding_note = None;
        // Short release avoids both stuck notes and a discontinuity.
        for voice in &mut self.voices {
            if voice.active {
                voice.releasing = true;
                voice.release_step = voice.level / (self.sample_rate * 0.01);
            }
        }
    }
    fn rebuild(&mut self) {
        let mut held = [0_u8; 128];
        for source in &self.keys {
            for (value, incoming) in held.iter_mut().zip(source) {
                *value = (*value).max(*incoming);
            }
        }
        if self.settings.arp && self.settings.hold {
            for (value, incoming) in self.latched.iter_mut().zip(held) {
                *value = (*value).max(incoming);
            }
            held = self.latched;
        }
        let mut base = [0_u8; 128];
        if !self.settings.arp || self.settings.notes == Notes::Played {
            base = held;
        } else if let Some(root) = (0..128)
            .filter(|n| held[*n] > 0)
            .max_by_key(|n| self.order[*n])
        {
            for interval in self.settings.notes.intervals() {
                let note = root + usize::from(*interval);
                if note < 128 {
                    base[note] = held[root];
                }
            }
        }
        let previous_len = self.len;
        self.len = 0;
        let octaves = if self.settings.arp {
            self.settings.octaves
        } else {
            1
        };
        for octave in 0..octaves {
            let start = self.len;
            for (note, velocity) in base.iter().enumerate().filter(|(_, v)| **v > 0) {
                let pitch = note + usize::from(octave) * 12;
                if pitch < 128 {
                    self.sequence[self.len] = (pitch as u8, *velocity);
                    self.len += 1;
                }
            }
            if self.settings.order == Order::AsPlayed && self.settings.notes == Notes::Played {
                // Insertion sort uses no allocator on the audio thread.
                for i in start + 1..self.len {
                    let mut j = i;
                    while j > start
                        && self.order
                            [usize::from(self.sequence[j - 1].0) - usize::from(octave) * 12]
                            > self.order[usize::from(self.sequence[j].0) - usize::from(octave) * 12]
                    {
                        self.sequence.swap(j - 1, j);
                        j -= 1;
                    }
                }
            }
        }
        if self.settings.arp {
            if self.len == 0 {
                self.release_all();
                self.sounding_note = None;
            }
            if previous_len == 0 {
                self.frames = 0.0;
                self.step = 0;
            }
        } else {
            for (note, velocity) in base.iter().copied().enumerate() {
                if velocity > 0 && self.desired[note] != velocity {
                    self.release_note(note as u8);
                    self.trigger(note as u8, velocity);
                } else if velocity == 0 && self.desired[note] > 0 {
                    self.release_note(note as u8);
                }
            }
            self.desired = base;
        }
    }
    fn release_note(&mut self, note: u8) {
        for voice in &mut self.voices {
            if voice.active && !voice.releasing && voice.note == note {
                voice.releasing = true;
                voice.release_step =
                    voice.level / (self.sample_rate * f32::from(self.settings.release_ms) / 1000.0);
            }
        }
    }
    fn release_all(&mut self) {
        for voice in &mut self.voices {
            if voice.active && !voice.releasing {
                voice.releasing = true;
                voice.release_step =
                    voice.level / (self.sample_rate * f32::from(self.settings.release_ms) / 1000.0);
            }
        }
    }
    fn trigger(&mut self, note: u8, velocity: u8) {
        let index = self
            .voices
            .iter()
            .position(|v| !v.active)
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| a.level.total_cmp(&b.level))
                    .unwrap()
                    .0
            });
        self.voices[index] = Voice {
            note,
            velocity: f32::from(velocity) / 127.0,
            increment: (440.0 * 2.0_f32.powf((f32::from(note) - 69.0) / 12.0) / self.sample_rate)
                .min(0.45),
            active: true,
            ..Voice::default()
        };
    }
    fn trigger_arp(&mut self) {
        self.release_all();
        let index = match self.settings.order {
            Order::Up | Order::AsPlayed => self.step % self.len,
            Order::Down => self.len - 1 - self.step % self.len,
            Order::UpDown if self.len > 1 => {
                let n = self.step % (2 * self.len - 2);
                if n < self.len {
                    n
                } else {
                    2 * self.len - 2 - n
                }
            }
            Order::UpDown => 0,
            Order::Random => {
                self.random ^= self.random << 13;
                self.random ^= self.random >> 17;
                self.random ^= self.random << 5;
                self.random as usize % self.len
            }
        };
        let (note, velocity) = self.sequence[index];
        self.trigger(note, velocity);
        self.sounding_note = Some(note);
        self.step = self.step.wrapping_add(1);
        let frames = f64::from(self.sample_rate) * 60.0
            / f64::from(self.bpm)
            / self.settings.rate.per_beat();
        self.frames += frames;
        self.gate_frames = frames * f64::from(self.settings.gate) / 100.0;
    }
    pub fn next_sample(&mut self) -> f32 {
        if self.settings.enabled && self.settings.arp && self.len > 0 {
            if self.frames < 0.5 {
                self.trigger_arp();
            }
            if self.gate_frames < 0.5 {
                self.release_all();
                self.sounding_note = None;
            }
            self.frames -= 1.0;
            self.gate_frames -= 1.0;
        }
        let attack =
            1.0 / (self.sample_rate * (f32::from(self.settings.attack_ms) / 1000.0).max(0.003));
        self.tone +=
            (f32::from(self.settings.tone) / 100.0 - self.tone) / (self.sample_rate * 0.01);
        let mut sum = 0.0;
        for voice in &mut self.voices {
            if !voice.active {
                continue;
            }
            if voice.releasing {
                voice.level = (voice.level - voice.release_step).max(0.0);
                if voice.level <= 0.00001 {
                    voice.active = false;
                    continue;
                }
            } else {
                voice.level = (voice.level + attack).min(1.0);
            }
            let wave = voice.timbre.sample(
                self.settings.sound,
                voice.increment,
                self.sample_rate,
                self.tone,
                voice.velocity,
                self.color_decay,
            );
            voice.low += self.filter * (wave - voice.low);
            sum += voice.low * voice.level * voice.velocity * 0.16;
        }
        self.gain +=
            (f32::from(self.settings.volume) / 100.0 - self.gain) / (self.sample_rate * 0.01);
        sum * self.gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn played_settings() -> SynthSettings {
        SynthSettings {
            notes: Notes::Played,
            ..SynthSettings::default()
        }
    }
    fn played_keyboard(rate: u32) -> Keyboard {
        let mut keyboard = Keyboard::new(rate);
        keyboard.configure(played_settings());
        keyboard
    }

    fn on(k: &mut Keyboard, note: u8) {
        k.event(
            0,
            KeyEvent::On {
                note,
                velocity: 100,
            },
        );
    }
    fn pitches(k: &Keyboard) -> Vec<u8> {
        k.sequence[..k.len].iter().map(|n| n.0).collect()
    }
    fn energy(k: &mut Keyboard, frames: usize) -> f32 {
        (0..frames).map(|_| k.next_sample().abs()).sum()
    }

    #[test]
    fn sounds_have_distinct_waveforms_and_tone_changes_held_notes() {
        let render = |sound, tone| {
            let mut k = played_keyboard(48000);
            let mut settings = played_settings();
            settings.select_sound(sound);
            settings.tone = tone;
            settings.attack_ms = 3;
            k.configure(settings);
            on(&mut k, 48);
            (0..8192).map(|_| k.next_sample()).collect::<Vec<_>>()
        };
        let waves: Vec<_> = Sound::ALL.iter().map(|s| render(*s, 75)).collect();
        for i in 0..waves.len() {
            for j in i + 1..waves.len() {
                let a = &waves[i];
                let b = &waves[j];
                let dot = a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
                let energy_a = a.iter().map(|x| x * x).sum::<f32>();
                let energy_b = b.iter().map(|x| x * x).sum::<f32>();
                let similarity = dot.abs() / (energy_a * energy_b).sqrt();
                assert!(
                    similarity < 0.96,
                    "Too similar: {:?} and {:?}: {similarity}",
                    Sound::ALL[i],
                    Sound::ALL[j]
                );
            }
        }
        for sound in Sound::ALL {
            let dark = render(*sound, 5);
            let bright = render(*sound, 95);
            let difference = dark
                .iter()
                .zip(&bright)
                .map(|(a, b)| (a - b).abs())
                .sum::<f32>();
            assert!(difference > 10.0, "Tone has little effect on {sound:?}");
        }
        let mut k = played_keyboard(48000);
        on(&mut k, 60);
        energy(&mut k, 1000);
        k.configure(SynthSettings {
            tone: 5,
            ..played_settings()
        });
        assert_eq!(
            k.voices.iter().filter(|v| v.active && !v.releasing).count(),
            1
        );
        assert!(energy(&mut k, 1000) > 0.1);
    }

    #[test]
    fn preset_changes_restore_attack_color_on_a_held_note() {
        let mut k = played_keyboard(48000);
        on(&mut k, 60);
        energy(&mut k, 48000 * 8);
        let mut settings = played_settings();
        settings.select_sound(Sound::Glass);
        k.configure(settings);
        assert!(energy(&mut k, 4096) > 100.0);
        assert_eq!(
            k.voices.iter().filter(|v| v.active && !v.releasing).count(),
            1
        );
    }

    #[test]
    fn sound_models_stay_finite_across_rates_and_keyboard_range() {
        for rate in [22050, 44100, 96000] {
            for sound in Sound::ALL {
                for note in [24, 60, 96, 127] {
                    let mut k = played_keyboard(rate);
                    let mut settings = played_settings();
                    settings.select_sound(*sound);
                    settings.tone = 100;
                    k.configure(settings);
                    on(&mut k, note);
                    for _ in 0..rate / 10 {
                        let sample = k.next_sample();
                        assert!(
                            sample.is_finite() && sample.abs() < 0.5,
                            "{sound:?} note {note} at {rate}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn midi_notes_and_pedals_are_channel_one_only() {
        assert_eq!(
            KeyEvent::from_midi(&[0x90, 60, 100]),
            Some(KeyEvent::On {
                note: 60,
                velocity: 100
            })
        );
        assert_eq!(
            KeyEvent::from_midi(&[0x90, 60, 0]),
            Some(KeyEvent::Off { note: 60 })
        );
        assert_eq!(
            KeyEvent::from_midi(&[0x80, 60, 64]),
            Some(KeyEvent::Off { note: 60 })
        );
        assert_eq!(
            KeyEvent::from_midi(&[0xb0, 64, 127]),
            Some(KeyEvent::Sustain(true))
        );
        assert_eq!(
            KeyEvent::from_midi(&[0xb0, 64, 0]),
            Some(KeyEvent::Sustain(false))
        );
        for cc in [120, 123] {
            assert_eq!(KeyEvent::from_midi(&[0xb0, cc, 0]), Some(KeyEvent::AllOff));
        }
        for bytes in [
            &[0x99, 60, 100][..],
            &[0x91, 60, 100],
            &[0x90, 128, 100],
            &[0x90, 60, 128],
            &[0x90, 60],
            &[0xb0, 14, 127],
        ] {
            assert_eq!(KeyEvent::from_midi(bytes), None);
        }
    }
    #[test]
    fn disabled_arp_plays_only_held_keys_and_preserves_chord_setting() {
        let mut k = Keyboard::new(48_000);
        let mut settings = SynthSettings::default();
        assert_eq!(settings.notes, Notes::Minor);
        on(&mut k, 60);
        on(&mut k, 64);
        assert_eq!(pitches(&k), vec![60, 64]);
        assert_eq!(
            k.voices.iter().filter(|v| v.active && !v.releasing).count(),
            2
        );
        for notes in Notes::ALL {
            settings.notes = *notes;
            k.configure(settings);
            assert_eq!(pitches(&k), vec![60, 64]);
            assert_eq!(k.voices.iter().filter(|v| v.active).count(), 2);
        }
        settings.notes = Notes::Minor;
        settings.arp = true;
        k.configure(settings);
        assert_eq!(pitches(&k), vec![64, 67, 71]);
        k.next_sample();
        settings.arp = false;
        k.configure(settings);
        assert_eq!(k.settings.notes, Notes::Minor);
        assert_eq!(pitches(&k), vec![60, 64]);
        let sounding: Vec<_> = k
            .voices
            .iter()
            .filter(|v| v.active && !v.releasing)
            .map(|v| v.note)
            .collect();
        assert_eq!(sounding, vec![60, 64]);
        k.event(0, KeyEvent::Off { note: 64 });
        assert_eq!(pitches(&k), vec![60]);
        k.event(0, KeyEvent::Off { note: 60 });
        assert!(pitches(&k).is_empty());
    }

    #[test]
    fn chord_choices_expand_from_latest_key_and_stay_in_midi_range() {
        let mut k = played_keyboard(48_000);
        for (notes, expected) in [
            (Notes::Played, vec![60]),
            (Notes::Major, vec![60, 64, 67]),
            (Notes::Minor, vec![60, 63, 67]),
            (Notes::Minor7, vec![60, 63, 67, 70]),
            (Notes::Sus2, vec![60, 62, 67]),
        ] {
            k.panic();
            k.configure(SynthSettings {
                notes,
                arp: true,
                ..played_settings()
            });
            on(&mut k, 60);
            assert_eq!(pitches(&k), expected);
        }
        on(&mut k, 62);
        assert_eq!(pitches(&k), vec![62, 64, 69]);
        k.event(0, KeyEvent::Off { note: 62 });
        assert_eq!(pitches(&k), vec![60, 62, 67]);
        k.panic();
        k.configure(SynthSettings {
            notes: Notes::Major,
            arp: true,
            octaves: 3,
            ..played_settings()
        });
        on(&mut k, 125);
        assert_eq!(pitches(&k), vec![125]);
    }
    #[test]
    fn sustain_releases_and_identical_notes_from_other_sources_survive() {
        let mut k = played_keyboard(48_000);
        on(&mut k, 60);
        k.event(
            1,
            KeyEvent::On {
                note: 60,
                velocity: 90,
            },
        );
        k.event(0, KeyEvent::Off { note: 60 });
        assert_eq!(pitches(&k), vec![60]);
        k.event(1, KeyEvent::Sustain(true));
        k.event(1, KeyEvent::Off { note: 60 });
        assert_eq!(pitches(&k), vec![60]);
        k.event(1, KeyEvent::Sustain(false));
        assert!(pitches(&k).is_empty());
        energy(&mut k, 48000);
        assert_eq!(energy(&mut k, 1000), 0.0);
    }
    #[test]
    fn hold_keeps_entire_chord_then_replaces_it_and_panic_clears_it() {
        let mut k = played_keyboard(48_000);
        let settings = SynthSettings {
            arp: true,
            hold: true,
            ..played_settings()
        };
        k.configure(settings);
        for n in [60, 64, 67] {
            on(&mut k, n);
        }
        for n in [60, 64, 67] {
            k.event(0, KeyEvent::Off { note: n });
        }
        assert_eq!(pitches(&k), vec![60, 64, 67]);
        on(&mut k, 62);
        assert_eq!(pitches(&k), vec![62]);
        k.event(0, KeyEvent::Off { note: 62 });
        k.configure(SynthSettings {
            hold: false,
            ..settings
        });
        assert_eq!(k.len, 0);
        on(&mut k, 60);
        assert!(energy(&mut k, 1000) > 1.0);
        k.panic();
        energy(&mut k, 1000);
        assert_eq!(energy(&mut k, 1000), 0.0);
    }
    #[test]
    fn order_octaves_gate_and_audio_clock_produce_expected_notes() {
        let mut k = played_keyboard(48_000);
        k.set_bpm(120);
        let settings = SynthSettings {
            arp: true,
            notes: Notes::Minor,
            rate: Rate::Sixteenth,
            octaves: 2,
            gate: 50,
            ..played_settings()
        };
        k.configure(settings);
        on(&mut k, 60);
        assert_eq!(pitches(&k), vec![60, 63, 67, 72, 75, 79]);
        for note in [60, 63, 67, 72, 75, 79, 60] {
            k.next_sample();
            assert_eq!(k.sounding_note, Some(note));
            energy(&mut k, 2999);
            k.next_sample();
            assert_eq!(k.sounding_note, None);
            energy(&mut k, 2999);
        }
        for (order, expected) in [
            (Order::Down, vec![79, 75, 72, 67]),
            (Order::UpDown, vec![60, 63, 67, 72, 75, 79, 75, 72]),
        ] {
            k.configure(SynthSettings { order, ..settings });
            for note in expected {
                k.next_sample();
                assert_eq!(k.sounding_note, Some(note));
                energy(&mut k, 5999);
            }
        }
    }
    #[test]
    fn fractional_timing_and_tempo_changes_preserve_progress() {
        let mut k = played_keyboard(44_100);
        k.set_bpm(137);
        k.configure(SynthSettings {
            arp: true,
            rate: Rate::Triplet,
            ..played_settings()
        });
        on(&mut k, 60);
        let duration = 44_100.0 * 60.0 / 137.0 / 3.0;
        let frames = 500_000;
        energy(&mut k, frames);
        assert_eq!(
            k.step,
            ((frames as f64 - 0.5) / duration).floor() as usize + 1
        );
        let remaining = k.frames;
        k.set_bpm(200);
        assert!((k.frames - remaining * 137.0 / 200.0).abs() < 0.00001);
    }
    #[test]
    fn as_played_order_and_random_stay_in_the_chord() {
        let mut k = played_keyboard(48_000);
        let settings = SynthSettings {
            arp: true,
            order: Order::AsPlayed,
            ..played_settings()
        };
        k.configure(settings);
        for n in [67, 60, 64] {
            on(&mut k, n);
        }
        assert_eq!(pitches(&k), vec![67, 60, 64]);
        k.configure(SynthSettings {
            order: Order::Random,
            ..settings
        });
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..30 {
            k.trigger_arp();
            let note = k.sounding_note.unwrap();
            assert!([60, 64, 67].contains(&note));
            seen.insert(note);
        }
        assert_eq!(seen.len(), 3);
    }
    #[test]
    fn voice_output_is_velocity_sensitive_finite_and_releases() {
        for sound in Sound::ALL {
            let render = |velocity| {
                let mut k = played_keyboard(48_000);
                let mut settings = played_settings();
                settings.select_sound(*sound);
                k.configure(settings);
                k.event(0, KeyEvent::On { note: 60, velocity });
                let energy = energy(&mut k, 48000);
                k.event(0, KeyEvent::Off { note: 60 });
                for _ in 0..96000 {
                    let sample = k.next_sample();
                    assert!(sample.is_finite() && sample.abs() < 1.0);
                }
                assert_eq!(k.next_sample(), 0.0);
                energy
            };
            assert!(render(120) > render(30) * 3.0);
        }
    }
    #[test]
    fn test_source_stop_does_not_release_physical_keys_and_power_clears_latch() {
        let mut k = played_keyboard(48_000);
        on(&mut k, 60);
        k.event(
            TEST_SOURCE,
            KeyEvent::On {
                note: 48,
                velocity: 100,
            },
        );
        k.event(TEST_SOURCE, KeyEvent::AllOff);
        assert_eq!(pitches(&k), vec![60]);
        k.configure(SynthSettings {
            enabled: false,
            ..played_settings()
        });
        k.configure(played_settings());
        assert!(pitches(&k).is_empty());
    }
    #[test]
    fn retrigger_under_sustain_starts_a_fresh_voice() {
        let mut k = played_keyboard(48_000);
        on(&mut k, 60);
        energy(&mut k, 1000);
        k.event(0, KeyEvent::Sustain(true));
        k.event(0, KeyEvent::Off { note: 60 });
        on(&mut k, 60);
        assert_eq!(
            k.voices.iter().filter(|v| v.active && !v.releasing).count(),
            1
        );
        assert!(
            k.voices
                .iter()
                .any(|v| v.active && !v.releasing && v.level == 0.0)
        );
    }
}
