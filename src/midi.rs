//! Desktop MIDI input and explicit controller mappings.
use midir::{Ignore, MidiInput, MidiInputConnection};
use sherbydrum::{
    effects::EffectSettings,
    groove::Instrument,
    keyboard::{KeyEvent, MIDI_SOURCES, Notes, Order, Rate, SynthSettings},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc::{self, Receiver},
};
use std::time::Instant;

pub struct Event {
    pub source: u8,
    pub bytes: [u8; 3],
    pub received_at: Instant,
    pub step: Option<usize>,
}

pub struct Input {
    _connections: Vec<MidiInputConnection<()>>,
    receiver: Receiver<Event>,
    pub status: String,
    dropped: Arc<AtomicUsize>,
    names: Vec<String>,
}

impl Input {
    pub fn open(
        ctx: eframe::egui::Context,
        telemetry: Option<Arc<crate::audio::Telemetry>>,
    ) -> Result<Self, String> {
        let input = MidiInput::new("Sherbydrum MIDI").map_err(|e| e.to_string())?;
        let (sender, receiver) = mpsc::sync_channel(256);
        let dropped = Arc::new(AtomicUsize::new(0));
        let mut connections = Vec::new();
        let mut names = Vec::new();
        let mut errors = Vec::new();
        for (source, port) in input.ports().into_iter().take(MIDI_SOURCES).enumerate() {
            let name = input
                .port_name(&port)
                .unwrap_or_else(|_| "Unnamed MIDI input".into());
            let mut client = MidiInput::new("Sherbydrum MIDI").map_err(|e| e.to_string())?;
            client.ignore(Ignore::None);
            let sender = sender.clone();
            let ctx = ctx.clone();
            let dropped = Arc::clone(&dropped);
            let telemetry = telemetry.clone();
            let mut record_down = false;
            match client.connect(
                &port,
                "sherbydrum-midi",
                move |_, bytes, _| {
                    let Ok(message) = <[u8; 3]>::try_from(bytes) else {
                        return;
                    };
                    let toggle_record = record_button_pressed(&message, &mut record_down);
                    if !toggle_record
                        && pad_hit(&message).is_none()
                        && KeyEvent::from_midi(&message).is_none()
                        && SynthControl::from_midi(&message).is_none()
                        && EffectControl::from_midi(&message).is_none()
                    {
                        return;
                    }
                    let event = Event {
                        source: source as u8,
                        bytes: message,
                        received_at: Instant::now(),
                        step: telemetry.as_ref().and_then(|clock| clock.recording_step()),
                    };
                    if sender.try_send(event).is_err() {
                        dropped.fetch_add(1, Ordering::Relaxed);
                    }
                    ctx.request_repaint();
                },
                (),
            ) {
                Ok(connection) => {
                    connections.push(connection);
                    names.push(name);
                }
                Err(error) => errors.push(format!("{name}: {error}")),
            }
        }
        let mut status = if names.is_empty() {
            "No MIDI inputs".into()
        } else {
            format!("MIDI: {}", names.join(", "))
        };
        if !errors.is_empty() {
            status.push_str(&format!(" · {}", errors.join("; ")));
        }
        Ok(Self {
            _connections: connections,
            receiver,
            status,
            dropped,
            names,
        })
    }

    /// Called infrequently by the UI; removing an input must release its notes.
    pub fn disconnected(&mut self) -> bool {
        let Ok(input) = MidiInput::new("Sherbydrum port check") else {
            return false;
        };
        let names: Vec<_> = input
            .ports()
            .iter()
            .filter_map(|port| input.port_name(port).ok())
            .collect();
        let before = self.names.len();
        self.names.retain(|name| names.contains(name));
        before != self.names.len()
    }

    pub fn drain(&self) -> Vec<Event> {
        self.receiver.try_iter().take(256).collect()
    }
    pub fn dropped(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }
}

fn pad_instrument(note: u8) -> Option<Instrument> {
    match note {
        44 => Some(Instrument::Kick),
        45 | 51 => Some(Instrument::Snare),
        46 => Some(Instrument::ClosedHat),
        47 => Some(Instrument::OpenHat),
        48 => Some(Instrument::Rim),
        49 => Some(Instrument::Tom),
        50 => Some(Instrument::Crash),
        _ => None,
    }
}

/// Track each input independently: a held button emits one toggle, release emits none.
fn record_button_pressed(bytes: &[u8], down: &mut bool) -> bool {
    let [0xb0, 118, value @ 0..=127] = bytes else {
        return false;
    };
    let pressed = *value != 0;
    let rising = pressed && !*down;
    *down = pressed;
    rising
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SynthControl {
    Tone(u8),
    Attack(u8),
    Release(u8),
    Volume(u8),
    SoundStep(i16),
    ArpNotes(u8),
    ArpPattern(u8),
    ArpRate(u8),
    ArpOctaves(u8),
}

impl SynthControl {
    fn from_midi(bytes: &[u8]) -> Option<Self> {
        match bytes {
            [0xb0, 74, value @ 0..=127] => Some(Self::Tone(*value)),
            [0xb0, 71, value @ 0..=127] => Some(Self::Attack(*value)),
            [0xb0, 76, value @ 0..=127] => Some(Self::Release(*value)),
            [0xb0, 77, value @ 0..=127] => Some(Self::Volume(*value)),
            [0xb0, 93, value @ 0..=127] => Some(Self::ArpNotes(*value)),
            [0xb0, 18, value @ 0..=127] => Some(Self::ArpPattern(*value)),
            [0xb0, 19, value @ 0..=127] => Some(Self::ArpRate(*value)),
            [0xb0, 16, value @ 0..=127] => Some(Self::ArpOctaves(*value)),
            [0xb0, 114, value @ 0..=127] => Some(Self::SoundStep(i16::from(*value) - 64)),
            _ => None,
        }
    }
}

pub fn apply_synth_control(bytes: &[u8], synth: &mut SynthSettings) -> bool {
    let Some(control) = SynthControl::from_midi(bytes) else {
        return false;
    };
    match control {
        SynthControl::ArpNotes(value) => {
            synth.notes = Notes::ALL[usize::from(value) * Notes::ALL.len() / 128]
        }
        SynthControl::ArpPattern(value) => {
            synth.order = Order::ALL[usize::from(value) * Order::ALL.len() / 128]
        }
        SynthControl::ArpRate(value) => {
            synth.rate = Rate::ALL[usize::from(value) * Rate::ALL.len() / 128]
        }
        SynthControl::ArpOctaves(value) => synth.octaves = 1 + (u16::from(value) * 3 / 128) as u8,
        SynthControl::SoundStep(0) => return false,
        SynthControl::SoundStep(steps) => synth.select_sound(synth.sound.stepped(steps)),
        SynthControl::Tone(value) => synth.tone = ((u16::from(value) * 100 + 63) / 127) as u8,
        SynthControl::Attack(value) => {
            synth.attack_ms = ((u32::from(value) * 1000 + 63) / 127) as u16
        }
        SynthControl::Release(value) => {
            synth.release_ms = 30 + ((u32::from(value) * 1970 + 63) / 127) as u16
        }
        SynthControl::Volume(value) => synth.volume = ((u16::from(value) * 100 + 63) / 127) as u8,
    }
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EffectControl {
    Drive(u8),
    Filter(u8),
    Delay(u8),
    Reverb(u8),
}
impl EffectControl {
    fn from_midi(bytes: &[u8]) -> Option<Self> {
        match bytes {
            [0xb0, 14 | 82, value @ 0..=127] => Some(Self::Drive(*value)),
            [0xb0, 15 | 83, value @ 0..=127] => Some(Self::Filter(*value)),
            [0xb0, 30 | 85, value @ 0..=127] => Some(Self::Delay(*value)),
            [0xb0, 31 | 17, value @ 0..=127] => Some(Self::Reverb(*value)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlSection {
    Effects,
    Sound,
    Synth,
    Arpeggiator,
}

pub fn control_section(bytes: &[u8]) -> Option<ControlSection> {
    if EffectControl::from_midi(bytes).is_some() {
        return Some(ControlSection::Effects);
    }
    match SynthControl::from_midi(bytes)? {
        SynthControl::SoundStep(0) => None,
        SynthControl::SoundStep(_) => Some(ControlSection::Sound),
        SynthControl::Tone(_)
        | SynthControl::Attack(_)
        | SynthControl::Release(_)
        | SynthControl::Volume(_) => Some(ControlSection::Synth),
        SynthControl::ArpNotes(_)
        | SynthControl::ArpPattern(_)
        | SynthControl::ArpRate(_)
        | SynthControl::ArpOctaves(_) => Some(ControlSection::Arpeggiator),
    }
}

pub fn apply_effect_control(bytes: &[u8], effects: &mut EffectSettings) -> bool {
    let Some(control) = EffectControl::from_midi(bytes) else {
        return false;
    };
    let (value, target) = match control {
        EffectControl::Drive(value) => (value, &mut effects.drive),
        EffectControl::Filter(value) => (value, &mut effects.filter),
        EffectControl::Delay(value) => (value, &mut effects.delay),
        EffectControl::Reverb(value) => (value, &mut effects.reverb),
    };
    *target = ((u16::from(value) * 100 + 63) / 127) as u8;
    effects.enabled = true;
    true
}

pub fn pad_hit(bytes: &[u8]) -> Option<(Instrument, u8)> {
    match bytes {
        [0x99, note, velocity @ 1..=127] => {
            pad_instrument(*note).map(|instrument| (instrument, *velocity))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn scroll_targets_follow_supported_controls_only() {
        use super::*;
        for (section, controls) in [
            (
                ControlSection::Effects,
                &[14, 15, 30, 31, 82, 83, 85, 17][..],
            ),
            (ControlSection::Sound, &[114][..]),
            (ControlSection::Synth, &[74, 71, 76, 77][..]),
            (ControlSection::Arpeggiator, &[93, 18, 19, 16][..]),
        ] {
            for control in controls {
                assert_eq!(control_section(&[0xb0, *control, 65]), Some(section));
                assert_eq!(control_section(&[0xb1, *control, 65]), None);
            }
        }
        for bytes in [
            &[0xb0, 114, 64][..],
            &[0xb0, 64, 127],
            &[0xb0, 118, 127],
            &[0x90, 60, 100],
            &[0x99, 44, 100],
            &[0xb0, 74, 128],
        ] {
            assert_eq!(control_section(bytes), None);
        }
    }

    #[test]
    fn arp_knobs_cover_choices_without_changing_other_settings() {
        for value in 0..=127 {
            for control in [93, 18, 19, 16] {
                let mut settings = SynthSettings::default();
                let mut expected = settings;
                match control {
                    93 => {
                        expected.notes = match value {
                            0..=25 => Notes::Played,
                            26..=51 => Notes::Major,
                            52..=76 => Notes::Minor,
                            77..=102 => Notes::Minor7,
                            _ => Notes::Sus2,
                        }
                    }
                    18 => {
                        expected.order = match value {
                            0..=25 => Order::Up,
                            26..=51 => Order::Down,
                            52..=76 => Order::UpDown,
                            77..=102 => Order::Random,
                            _ => Order::AsPlayed,
                        }
                    }
                    19 => {
                        expected.rate = match value {
                            0..=25 => Rate::Quarter,
                            26..=51 => Rate::Eighth,
                            52..=76 => Rate::Triplet,
                            77..=102 => Rate::Sixteenth,
                            _ => Rate::ThirtySecond,
                        }
                    }
                    16 => {
                        expected.octaves = match value {
                            0..=42 => 1,
                            43..=85 => 2,
                            _ => 3,
                        }
                    }
                    _ => unreachable!(),
                }
                assert!(apply_synth_control(&[0xb0, control, value], &mut settings));
                assert_eq!(settings, expected);
                assert!(settings.validate().is_ok());
                let before = settings;
                assert!(!apply_synth_control(&[0xb1, control, value], &mut settings));
                assert_eq!(settings, before);
            }
        }
    }

    #[test]
    fn sound_encoder_uses_relative_steps_and_ignores_neutral() {
        use super::*;
        use sherbydrum::keyboard::Sound;
        let mut settings = SynthSettings {
            tone: 23,
            volume: 42,
            ..SynthSettings::default()
        };
        assert!(!apply_synth_control(&[0xb0, 114, 64], &mut settings));
        assert_eq!(settings.tone, 23);
        for (value, expected) in [
            (65, Sound::Soft),
            (64, Sound::Soft),
            (66, Sound::Bright),
            (64, Sound::Bright),
            (61, Sound::Warm),
            (63, Sound::Organ),
            (65, Sound::Warm),
        ] {
            apply_synth_control(&[0xb0, 114, value], &mut settings);
            assert_eq!(settings.sound, expected);
            assert_eq!(settings.volume, 42);
            assert!(settings.validate().is_ok());
        }
        let before = settings;
        assert!(!apply_synth_control(&[0xb1, 114, 65], &mut settings));
        assert!(!apply_synth_control(&[0xb0, 114, 128], &mut settings));
        assert_eq!(settings, before);
    }

    #[test]
    fn synth_knobs_map_endpoints_midpoint_and_preserve_other_controls() {
        use super::*;
        for (control, expected_values) in [
            (74, [0, 50, 100]),
            (71, [0, 504, 1000]),
            (76, [30, 1023, 2000]),
            (77, [0, 50, 100]),
        ] {
            for (value, expected) in [0, 64, 127].into_iter().zip(expected_values) {
                let initial = SynthSettings {
                    enabled: false,
                    arp: true,
                    hold: true,
                    ..SynthSettings::default()
                };
                let mut synth = initial;
                assert!(apply_synth_control(&[0xb0, control, value], &mut synth));
                let mut wanted = initial;
                match control {
                    74 => wanted.tone = expected as u8,
                    71 => wanted.attack_ms = expected,
                    76 => wanted.release_ms = expected,
                    77 => wanted.volume = expected as u8,
                    _ => unreachable!(),
                }
                assert_eq!(synth, wanted);
                assert!(synth.validate().is_ok());
            }
        }
    }

    #[test]
    fn synth_knobs_reject_other_channels_and_invalid_messages() {
        use super::*;
        for bytes in [
            &[0xb1, 74, 127][..],
            &[0xb9, 71, 127],
            &[0xb0, 74, 128],
            &[0xb0, 74],
            &[0x90, 74, 100],
            &[0xb0, 14, 127],
        ] {
            let mut synth = SynthSettings::default();
            assert!(!apply_synth_control(bytes, &mut synth));
            assert_eq!(synth, SynthSettings::default());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Requires native CoreMIDI"]
    fn native_keyboard_receives_notes_sustain_and_disconnect() {
        use midir::os::unix::VirtualOutput;
        use sherbydrum::engine::{Command, Engine};
        use std::time::Duration;
        let mut source = midir::MidiOutput::new("Synth integration test")
            .unwrap()
            .create_virtual("Sherbydrum synth integration test")
            .unwrap();
        let mut input = Input::open(eframe::egui::Context::default(), None).unwrap();
        let mut engine = Engine::new(48_000);
        source.send(&[0x90, 60, 100]).unwrap();
        source.send(&[0xb0, 64, 127]).unwrap();
        source.send(&[0x90, 60, 0]).unwrap();
        let mut received = Vec::new();
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(10));
            received.extend(input.drain());
            if received.len() >= 3 {
                break;
            }
        }
        assert_eq!(received.len(), 3);
        for event in received {
            engine.command(Command::Keyboard(
                event.source,
                KeyEvent::from_midi(&event.bytes).unwrap(),
            ));
        }
        assert!((0..24000).map(|_| engine.next_sample().abs()).sum::<f32>() > 10.0);
        source.send(&[0xb0, 64, 0]).unwrap();
        let mut released = false;
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(10));
            for event in input.drain() {
                engine.command(Command::Keyboard(
                    event.source,
                    KeyEvent::from_midi(&event.bytes).unwrap(),
                ));
                released = true;
            }
            if released {
                break;
            }
        }
        assert!(released);
        for _ in 0..48000 {
            engine.next_sample();
        }
        assert_eq!(engine.next_sample(), 0.0);
        for control in [74, 71, 76, 77] {
            source.send(&[0xb0, control, 127]).unwrap();
        }
        let mut synth = SynthSettings::default();
        let mut controls = 0;
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(10));
            for event in input.drain() {
                assert!(apply_synth_control(&event.bytes, &mut synth));
                controls += 1;
            }
            if controls == 4 {
                break;
            }
        }
        assert_eq!(controls, 4);
        assert_eq!(
            (synth.tone, synth.attack_ms, synth.release_ms, synth.volume),
            (100, 1000, 2000, 100)
        );
        drop(source);
        std::thread::sleep(Duration::from_millis(100));
        assert!(input.disconnected());
        assert!(!input.disconnected());
    }

    use super::*;
    #[test]
    fn record_button_toggles_once_per_press_on_channel_one() {
        let mut down = false;
        assert!(!record_button_pressed(&[0xb0, 118, 0], &mut down));
        assert!(!record_button_pressed(&[0xb1, 118, 127], &mut down));
        assert!(record_button_pressed(&[0xb0, 118, 127], &mut down));
        assert!(!record_button_pressed(&[0xb0, 118, 127], &mut down));
        assert!(!record_button_pressed(&[0xb0, 14, 0], &mut down));
        assert!(down);
        assert!(!record_button_pressed(&[0xb0, 118, 0], &mut down));
        assert!(record_button_pressed(&[0xb0, 118, 127], &mut down));
    }
    #[test]
    fn pads_map_to_kit_and_preserve_hit_strength() {
        let expected = [
            Instrument::Kick,
            Instrument::Snare,
            Instrument::ClosedHat,
            Instrument::OpenHat,
            Instrument::Rim,
            Instrument::Tom,
            Instrument::Crash,
            Instrument::Snare,
        ];
        for (note, instrument) in (44..=51).zip(expected) {
            for velocity in [1, 66, 117, 127] {
                assert_eq!(
                    pad_hit(&[0x99, note, velocity]),
                    Some((instrument, velocity))
                );
            }
        }
        for bytes in [
            &[0x89, 44, 64][..],
            &[0x99, 44, 0],
            &[0x90, 44, 100],
            &[0x99, 43, 100],
            &[0x99, 52, 100],
            &[0x99, 44],
            &[0xa9, 44, 100],
            &[0x99, 44, 128],
        ] {
            assert_eq!(pad_hit(bytes), None);
        }
    }

    #[test]
    fn effect_knobs_map_full_range_and_keep_other_settings() {
        for control in [14, 15, 30, 31, 82, 83, 85, 17] {
            for (value, amount) in [(0, 0), (64, 50), (127, 100)] {
                let mut effects = EffectSettings::default();
                assert!(apply_effect_control(&[0xb0, control, value], &mut effects));
                let mut expected = EffectSettings {
                    enabled: true,
                    ..EffectSettings::default()
                };
                match control {
                    14 | 82 => expected.drive = amount,
                    15 | 83 => expected.filter = amount,
                    30 | 85 => expected.delay = amount,
                    31 | 17 => expected.reverb = amount,
                    _ => unreachable!(),
                }
                assert_eq!(effects, expected);
            }
        }
        for bytes in [
            &[0xb1, 14, 127][..],
            &[0xb1, 82, 127],
            &[0xb9, 83, 127],
            &[0xb0, 85, 128],
            &[0xb0, 17],
            &[0xb0, 16, 127],
            &[0xb0, 14, 128],
            &[0xb0, 14],
        ] {
            let mut effects = EffectSettings::default();
            assert!(!apply_effect_control(bytes, &mut effects));
            assert_eq!(effects, EffectSettings::default());
        }
    }
}
