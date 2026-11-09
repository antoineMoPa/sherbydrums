//! Desktop MIDI input and explicit controller mappings.
use midir::{Ignore, MidiInput, MidiInputConnection};
use sherbydrum::{
    effects::EffectSettings,
    groove::Instrument,
    keyboard::{KeyEvent, MIDI_SOURCES},
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
                        && !matches!(message, [0xb0, 14 | 15 | 30 | 31, 0..=127])
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

pub fn apply_effect_control(bytes: &[u8], effects: &mut EffectSettings) -> bool {
    let [0xb0, control, value @ 0..=127] = bytes else {
        return false;
    };
    let amount = ((u16::from(*value) * 100 + 63) / 127) as u8;
    match control {
        14 => effects.drive = amount,
        15 => effects.filter = amount,
        30 => effects.delay = amount,
        31 => effects.reverb = amount,
        _ => return false,
    }
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
        for control in [14, 15, 30, 31] {
            for (value, amount) in [(0, 0), (64, 50), (127, 100)] {
                let mut effects = EffectSettings::default();
                assert!(apply_effect_control(&[0xb0, control, value], &mut effects));
                let mut expected = EffectSettings {
                    enabled: true,
                    ..EffectSettings::default()
                };
                match control {
                    14 => expected.drive = amount,
                    15 => expected.filter = amount,
                    30 => expected.delay = amount,
                    31 => expected.reverb = amount,
                    _ => unreachable!(),
                }
                assert_eq!(effects, expected);
            }
        }
        for bytes in [
            &[0xb1, 14, 127][..],
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
