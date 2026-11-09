use crate::audio::Output;
use eframe::egui::{self, Color32, FontId, RichText, Sense, Stroke, Vec2};
use sherbydrum::{
    engine::{Command, CustomPattern, Position, Settings},
    groove::{INSTRUMENTS, Instrument, Pattern, STEPS, Style, groove},
    keyboard::{KeyEvent, Notes, Order, Rate, Sound, SynthSettings, TEST_SOURCE},
    pattern_store::{self, SavedPattern},
};
use std::{
    collections::VecDeque,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

const BACKGROUND: Color32 = Color32::from_rgb(37, 45, 38);
const SURFACE: Color32 = Color32::from_rgb(52, 62, 50);
const BORDER: Color32 = Color32::from_rgb(70, 80, 65);
const MUTED: Color32 = Color32::from_rgb(185, 197, 178);
const INK: Color32 = Color32::from_rgb(243, 242, 232);
const LIME: Color32 = Color32::from_rgb(217, 243, 108);

// Factory rows first, then the first three saved custom patterns.
const GROOVE_SHORTCUTS: [(egui::Key, &str); 15] = [
    (egui::Key::Num1, "1"),
    (egui::Key::Num2, "2"),
    (egui::Key::Num3, "3"),
    (egui::Key::Num4, "4"),
    (egui::Key::Num5, "5"),
    (egui::Key::Num6, "6"),
    (egui::Key::Q, "Q"),
    (egui::Key::W, "W"),
    (egui::Key::E, "E"),
    (egui::Key::R, "R"),
    (egui::Key::T, "T"),
    (egui::Key::Y, "Y"),
    (egui::Key::Num7, "7"),
    (egui::Key::Num8, "8"),
    (egui::Key::Num9, "9"),
];

struct Editor {
    recording: Option<Instant>,
    undo_take: Option<Pattern>,
    take_hits: [[bool; STEPS]; INSTRUMENTS],
    blank: bool,
    factory_copy: bool,
    draft: SavedPattern,
    target: Option<usize>,
    preview: bool,
    bpm: u16,
    volume: u8,
    sent_preview: Option<Settings>,
}

impl Editor {
    fn toggle_recording(&mut self, now: Instant) {
        if self.recording.is_some() {
            self.recording = None;
        } else {
            self.begin_take(now);
        }
    }

    fn begin_take(&mut self, now: Instant) {
        self.undo_take = Some(self.draft.pattern);
        self.take_hits = [[false; STEPS]; INSTRUMENTS];
        self.recording = Some(now);
    }

    fn recording_step(&self, event: &crate::midi::Event) -> Option<usize> {
        if self.preview
            && self
                .recording
                .is_some_and(|start| event.received_at >= start)
        {
            event.step
        } else {
            None
        }
    }

    fn record_hit(&mut self, event: &crate::midi::Event, instrument: Instrument, velocity: u8) {
        if let Some(step) = self.recording_step(event) {
            self.draft.pattern.hits[instrument.index()][step] = velocity;
            self.take_hits[instrument.index()][step] = true;
        }
    }

    fn undo(&mut self) {
        if self.recording.is_none()
            && let Some(pattern) = self.undo_take.take()
        {
            self.draft.pattern = pattern;
            self.take_hits = [[false; STEPS]; INSTRUMENTS];
        }
    }
}

pub struct Drummer {
    synth: SynthSettings,
    sent_synth: Option<SynthSettings>,
    test_note: bool,
    midi_dropped: usize,
    midi_check: Instant,
    midi: Option<crate::midi::Input>,
    midi_error: Option<String>,
    #[cfg(target_os = "macos")]
    media: Option<crate::media::Media>,
    settings: Settings,
    sent_settings: Option<Settings>,
    output: Option<Output>,
    error: Option<String>,
    playing: bool,
    once_playback: Option<bool>,
    taps: VecDeque<Instant>,
    customs: Vec<SavedPattern>,
    pattern_path: Option<std::path::PathBuf>,
    pattern_error: Option<String>,
    selected: usize,
    editor: Option<Editor>,
}

impl Drummer {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style_of(egui::Theme::Dark)).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = BACKGROUND;
        style.visuals.window_fill = BACKGROUND;
        style.visuals.override_text_color = Some(INK);
        style.visuals.selection.bg_fill = LIME;
        style.visuals.selection.stroke = Stroke::new(1.0, BACKGROUND);
        style.visuals.widgets.inactive.bg_fill = SURFACE;
        style.visuals.widgets.inactive.weak_bg_fill = SURFACE;
        style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(68, 80, 59);
        style.visuals.widgets.active.bg_fill = LIME;
        style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, BACKGROUND);
        style.visuals.slider_trailing_fill = true;
        style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
        style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
        style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
        style.spacing.scroll.fade.strength = 0.0;
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
        style.spacing.button_padding = Vec2::new(14.0, 10.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(15.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(14.0));
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        cc.egui_ctx.set_style_of(egui::Theme::Dark, style);
        let (customs, pattern_path, pattern_error) = match pattern_store::bank_path()
            .and_then(|path| pattern_store::load(&path).map(|bank| (bank, path)))
        {
            Ok((bank, path)) => (bank, Some(path), None),
            Err(error) => (
                Vec::new(),
                None,
                Some(format!(
                    "Could not load patterns: {error}. Saving is disabled to protect your bank. Fix the file or data directory and restart."
                )),
            ),
        };
        let mut app = Self {
            synth: SynthSettings::default(),
            sent_synth: None,
            test_note: false,
            midi_dropped: 0,
            midi_check: Instant::now(),
            midi: None,
            midi_error: None,
            #[cfg(target_os = "macos")]
            media: crate::media::Media::new(cc.egui_ctx.clone())
                .map_err(|e| eprintln!("macOS media controls unavailable: {e}"))
                .ok(),
            settings: Settings::default(),
            sent_settings: None,
            output: None,
            error: None,
            playing: false,
            once_playback: None,
            taps: VecDeque::with_capacity(5),
            customs,
            pattern_path,
            pattern_error,
            selected: 0,
            editor: None,
        };
        app.connect();
        app.settings.effects.enabled = true;
        app.connect_midi(&cc.egui_ctx);
        app
    }
    fn connect_midi(&mut self, ctx: &egui::Context) {
        self.panic_keys();
        self.midi_dropped = 0;
        self.midi = None;
        match crate::midi::Input::open(
            ctx.clone(),
            self.output.as_ref().map(|o| o.telemetry.clone()),
        ) {
            Ok(input) => {
                self.midi = Some(input);
                self.midi_error = None;
            }
            Err(error) => self.midi_error = Some(format!("MIDI unavailable: {error}")),
        }
    }

    fn handle_midi(&mut self) {
        let mut lost_input = false;
        if self.midi_check.elapsed() >= Duration::from_secs(1) {
            self.midi_check = Instant::now();
            lost_input = self.midi.as_mut().is_some_and(|input| input.disconnected());
        }
        let dropped = self.midi.as_ref().map_or(0, |input| input.dropped());
        if lost_input || dropped != self.midi_dropped {
            self.midi_dropped = dropped;
            if let Some(input) = &self.midi {
                input.drain();
            }
            self.panic_keys();
            return;
        }
        let messages = self
            .midi
            .as_ref()
            .map_or_else(Vec::new, |input| input.drain());
        for event in messages {
            let bytes = event.bytes;
            let event_source = event.source;
            if matches!(bytes, [0xb0, 118, 1..=127]) {
                if let Some(editor) = &mut self.editor {
                    editor.toggle_recording(event.received_at);
                }
                continue;
            }
            if self.output.is_some()
                && let Some((instrument, velocity)) = crate::midi::pad_hit(&bytes)
                && let Some(editor) = &mut self.editor
            {
                editor.record_hit(&event, instrument, velocity);
            }
            crate::midi::apply_effect_control(&bytes, &mut self.settings.effects);
            // Preserve the editor's independent tempo, volume and draft when knobs move.
            let ready = if let Some(editor) = &self.editor {
                let settings = preview_settings(editor, self.settings.swing, self.settings.effects);
                if editor.sent_preview == Some(settings) {
                    true
                } else if self.send(Command::Preview(settings)) {
                    self.editor.as_mut().unwrap().sent_preview = Some(settings);
                    true
                } else {
                    false
                }
            } else {
                self.sync_settings()
            };
            if let Some(event) = KeyEvent::from_midi(&bytes)
                && ready
                && self.sync_synth()
            {
                self.send(Command::Keyboard(event_source, event));
            }
            if ready && let Some((instrument, velocity)) = crate::midi::pad_hit(&bytes) {
                let step = self
                    .editor
                    .as_ref()
                    .and_then(|editor| editor.recording_step(&event));
                self.send(if let Some(step) = step {
                    Command::RecordedHit(instrument, velocity, step)
                } else {
                    Command::Hit(instrument, velocity)
                });
            }
        }
    }

    fn connect(&mut self) {
        self.output = None;
        self.playing = false;
        self.sent_settings = None;
        self.sent_synth = None;
        self.test_note = false;
        match Output::open() {
            Ok(output) => {
                self.output = Some(output);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn send(&mut self, command: Command) -> bool {
        let Some(output) = &self.output else {
            return false;
        };
        match output.send(command) {
            Ok(()) => true,
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }

    fn panic_keys(&mut self) {
        self.test_note = false;
        if let Some(output) = &self.output {
            output.telemetry.panic_keys.store(true, Ordering::Release);
        }
    }

    fn sync_synth(&mut self) -> bool {
        if self.sent_synth == Some(self.synth) {
            return true;
        }
        if self.send(Command::Synth(self.synth)) {
            self.sent_synth = Some(self.synth);
            true
        } else {
            false
        }
    }

    fn sync_settings(&mut self) -> bool {
        if self.sent_settings == Some(self.settings) {
            return true;
        }
        if self.send(Command::Apply(self.settings)) {
            self.sent_settings = Some(self.settings);
            true
        } else {
            false
        }
    }

    fn toggle_play(&mut self) {
        self.once_playback = None;
        if !self.sync_settings() {
            return;
        }
        if self.send(if self.playing {
            Command::Stop
        } else {
            Command::Play
        }) {
            self.playing = !self.playing;
        }
    }

    fn tap(&mut self) {
        let now = Instant::now();
        if self
            .taps
            .back()
            .is_some_and(|last| now.duration_since(*last) > Duration::from_millis(1800))
        {
            self.taps.clear();
        }
        self.taps.push_back(now);
        if self.taps.len() > 5 {
            self.taps.pop_front();
        }
        if self.taps.len() >= 2 {
            let seconds =
                now.duration_since(self.taps[0]).as_secs_f64() / (self.taps.len() - 1) as f64;
            self.settings.bpm = (60.0 / seconds).round().clamp(40.0, 240.0) as u16;
        }
    }

    fn position(&self) -> Position {
        self.output
            .as_ref()
            .map(|o| o.telemetry.position())
            .unwrap_or(Position {
                running: false,
                step: 0,
                bar: 1,
                style: self.settings.style,
                custom_id: self.settings.custom.map_or(0, |p| p.id),
                fill_pending: false,
                filling: false,
            })
    }

    fn name(&self, index: usize) -> &str {
        if index < Style::ALL.len() {
            Style::ALL[index].name()
        } else {
            &self.customs[index - Style::ALL.len()].name
        }
    }

    fn select(&mut self, index: usize) {
        self.selected = index;
        if index < Style::ALL.len() {
            self.settings.style = Style::ALL[index];
            self.settings.custom = None;
        } else {
            let pattern = &self.customs[index - Style::ALL.len()];
            self.settings.style = pattern.style;
            self.settings.custom = Some(CustomPattern {
                id: (index - Style::ALL.len() + 1) as u16,
                pattern: pattern.pattern,
            });
        }
    }

    fn open_editor(&mut self, blank: bool) {
        if self.playing && self.send(Command::Stop) {
            self.playing = false;
        }
        let (draft, target) = if blank {
            let mut pattern = Pattern {
                hits: [[0; STEPS]; INSTRUMENTS],
            };
            for step in (0..STEPS).step_by(4) {
                pattern.hits[Instrument::ClosedHat.index()][step] = 100;
            }
            (
                SavedPattern {
                    name: "Untitled groove".into(),
                    style: Style::Rock,
                    pattern,
                },
                None,
            )
        } else if self.selected < Style::ALL.len() {
            let style = Style::ALL[self.selected];
            (
                SavedPattern {
                    name: format!("{} / my take", style.name()),
                    style,
                    pattern: *groove(style),
                },
                None,
            )
        } else {
            let index = self.selected - Style::ALL.len();
            (self.customs[index].clone(), Some(index))
        };
        self.editor = Some(Editor {
            recording: None,
            undo_take: None,
            take_hits: [[false; STEPS]; INSTRUMENTS],
            blank,
            factory_copy: !blank && self.selected < Style::ALL.len(),
            draft,
            target,
            preview: false,
            bpm: self.settings.bpm,
            volume: self.settings.volume,
            sent_preview: None,
        });
    }

    fn practice(&mut self, ui: &mut egui::Ui) {
        let position = self.position();
        ui.add_space(26.0);
        ui.vertical_centered(|ui| {
            let title = self.name(self.selected).to_owned();
            ui.horizontal(|ui| {
                let width = ui
                    .painter()
                    .layout_no_wrap(title.clone(), FontId::proportional(25.0), INK)
                    .size()
                    .x
                    + 145.0;
                ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                ui.add_sized(
                    Vec2::new(
                        (width - 145.0).min((ui.available_width() - 145.0).max(80.0)),
                        0.0,
                    ),
                    egui::Label::new(RichText::new(title).size(25.0)).wrap(),
                );
                let edit_response = ui.add(
                    egui::Button::new(RichText::new("    Edit pattern").size(12.0).color(LIME))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(119, 135, 76))),
                );
                paint_button_icon(
                    ui,
                    &edit_response,
                    ButtonIcon::Edit,
                    "    Edit pattern",
                    12.0,
                    LIME,
                );
                if edit_response.clicked() {
                    self.open_editor(false);
                }
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - 185.0) / 2.0).max(0.0));
                ui.scope(|ui| {
                    ui.style_mut()
                        .text_styles
                        .insert(egui::TextStyle::Button, FontId::proportional(64.0));
                    ui.style_mut()
                        .text_styles
                        .insert(egui::TextStyle::Monospace, FontId::proportional(64.0));
                    ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                    ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
                    ui.add(
                        egui::DragValue::new(&mut self.settings.bpm)
                            .range(40..=240)
                            .speed(0.25),
                    )
                    .on_hover_text("Drag to change tempo, or click to type. Shift+T to tap tempo.");
                });
                ui.label(RichText::new("BPM").monospace().size(11.0).color(MUTED));
            });
            let (rect, _) = ui.allocate_exact_size(Vec2::new(310.0, 9.0), Sense::hover());
            for beat in 0..4 {
                let cell = egui::Rect::from_min_size(
                    rect.min + Vec2::new(beat as f32 * 80.5, 0.0),
                    Vec2::new(68.5, 9.0),
                );
                ui.painter().rect_filled(
                    cell,
                    5.0,
                    if position.running && position.step / 4 == beat {
                        LIME
                    } else {
                        Color32::from_rgb(84, 96, 71)
                    },
                );
            }
            ui.add_space(26.0);
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width().min(600.0), 65.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.columns(2, |cols| {
                        slider(&mut cols[0], "Swing", &mut self.settings.swing, 60);
                        slider(&mut cols[1], "Volume", &mut self.settings.volume, 100);
                    });
                },
            );
            ui.add_space(12.0);
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width().min(750.0), 70.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    let unit = (ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0;
                    let play_response = ui
                        .add_enabled(
                            self.output.is_some(),
                            egui::Button::new(
                                RichText::new(if self.playing { "■ Stop" } else { "    Play" })
                                    .size(17.0)
                                    .color(BACKGROUND),
                            )
                            .fill(LIME)
                            .min_size(Vec2::new(unit * 1.25, 70.0)),
                        )
                        .on_hover_text("Space");
                    if !self.playing {
                        paint_button_icon(
                            ui,
                            &play_response,
                            ButtonIcon::Play,
                            "    Play",
                            17.0,
                            BACKGROUND,
                        );
                    }
                    if play_response.clicked() {
                        self.toggle_play();
                    }
                    if ui
                        .add_enabled(
                            self.output.is_some(),
                            egui::Button::new(RichText::new("Play once").size(13.0))
                                .min_size(Vec2::new(unit * 0.75, 70.0)),
                        )
                        .on_hover_text("Play one bar, then stop.")
                        .clicked()
                        && self.sync_settings()
                        && self.send(Command::PlayOnce)
                    {
                        self.playing = true;
                        self.once_playback = Some(false);
                    }
                    let fill_response = ui
                        .add_enabled(
                            self.playing && !position.fill_pending && !position.filling,
                            egui::Button::new(
                                RichText::new(if position.fill_pending {
                                    "Fill queued\n "
                                } else if position.filling {
                                    "Filling\n "
                                } else {
                                    "Add fill\n "
                                })
                                .size(17.0),
                            )
                            .min_size(Vec2::new(unit, 70.0)),
                        )
                        .on_hover_text("Enter / F · Starts next sixteenth note");
                    let hint_rect = egui::Rect::from_center_size(
                        fill_response.rect.center() + Vec2::new(0.0, 17.0),
                        Vec2::new(82.0, 12.0),
                    );
                    keyboard_hint(ui, hint_rect, "Enter", MUTED);
                    if fill_response.clicked() {
                        self.send(Command::Fill);
                    }
                },
            );
            ui.add_space(12.0);
            ui.add_space(20.0);
        });
    }

    fn keyboard_panel(&mut self, ui: &mut egui::Ui) {
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(28, 22)).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.checkbox(&mut self.synth.enabled, "Synth").changed() && !self.synth.enabled {
                    self.panic_keys();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("All notes off").clicked() { self.panic_keys(); }
                    ui.label(RichText::new("Ch 1").size(11.0).color(MUTED));
                });
            });
            ui.add_enabled_ui(self.synth.enabled, |ui| {
                let old_sound = self.synth.sound;
                choice(ui, "Sound", &mut self.synth.sound, Sound::ALL, Sound::label);
                if self.synth.sound != old_sound { self.synth.select_sound(self.synth.sound); }
                ui.add_space(12.0);
                ui.columns(4, |cols| {
                    slider(&mut cols[0], "Tone", &mut self.synth.tone, 100);
                    milliseconds(&mut cols[1], "Attack", &mut self.synth.attack_ms, 0..=1000);
                    milliseconds(&mut cols[2], "Release", &mut self.synth.release_ms, 30..=2000);
                    slider(&mut cols[3], "Volume", &mut self.synth.volume, 100);
                });
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.synth.arp, "Arpeggiator");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_enabled_ui(self.synth.arp, |ui| {
                            ui.toggle_value(&mut self.synth.hold, "Hold")
                                .on_hover_text("Keep the chord after releasing the keys; a new chord replaces it.");
                        });
                        if ui.add_enabled(self.output.is_some(), egui::Button::new(if self.test_note { "Stop test" } else { "Test C" })).clicked()
                            && self.sync_settings() && self.sync_synth() {
                            let event = if self.test_note { KeyEvent::AllOff } else { KeyEvent::On { note: 48, velocity: 100 } };
                            if self.send(Command::Keyboard(TEST_SOURCE, event)) { self.test_note = !self.test_note; }
                        }
                    });
                });
                ui.add_space(10.0);
                ui.columns(4, |cols| {
                    choice(&mut cols[0], "Notes", &mut self.synth.notes, Notes::ALL, Notes::label)
                        .on_hover_text("Played uses your held notes. Chord modes use your most recently pressed key as the root.");
                    cols[1].add_enabled_ui(self.synth.arp, |ui| { choice(ui, "Pattern", &mut self.synth.order, Order::ALL, Order::label); });
                    cols[2].add_enabled_ui(self.synth.arp, |ui| { choice(ui, "Rate", &mut self.synth.rate, Rate::ALL, Rate::label); });
                    cols[3].add_enabled_ui(self.synth.arp, |ui| { choice(ui, "Octaves", &mut self.synth.octaves, &[1,2,3], |n| match n {1=>"1",2=>"2",_=>"3"}); });
                });
                ui.add_space(10.0);
                ui.add_enabled_ui(self.synth.arp, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Gate").size(12.0));
                        ui.add(egui::Slider::new(&mut self.synth.gate, 10..=100).suffix("%"));
                    });
                });
            });
            ui.add_space(14.0);
            ui.separator();
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Effects");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label(RichText::new("Drums + synth").size(11.0).color(MUTED)); });
            });
            ui.add_space(8.0);
            ui.columns(4, |cols| {
                let effects=&mut self.settings.effects;
                slider(&mut cols[0], "Drive", &mut effects.drive, 100);
                slider(&mut cols[1], "Filter", &mut effects.filter, 100);
                slider(&mut cols[2], "Delay", &mut effects.delay, 100);
                slider(&mut cols[3], "Reverb", &mut effects.reverb, 100);
            });
        });
    }

    fn bank(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("GROOVES").monospace().size(11.0).color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        self.customs.len() < u16::MAX as usize,
                        egui::Button::new("+ New pattern"),
                    )
                    .clicked()
                {
                    self.open_editor(true);
                }
            });
        });
        ui.add_space(6.0);
        let width = (ui.available_width() - ui.spacing().item_spacing.x * 5.0) / 6.0;
        egui::Grid::new("groove-bank")
            .num_columns(6)
            .show(ui, |ui| {
                for index in 0..Style::ALL.len() + self.customs.len() {
                    let selected = self.selected == index;
                    let response = ui.add(
                        egui::Button::new("")
                            .fill(if selected { LIME } else { BACKGROUND })
                            .stroke(Stroke::new(
                                1.0,
                                if selected {
                                    LIME
                                } else {
                                    Color32::from_rgb(81, 91, 76)
                                },
                            ))
                            .min_size(Vec2::new(width, 49.0)),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::SelectableLabel,
                            ui.is_enabled(),
                            selected,
                            self.name(index),
                        )
                    });
                    let color = if selected { BACKGROUND } else { INK };
                    let shortcut = GROOVE_SHORTCUTS.get(index);
                    let text_width = width - if shortcut.is_some() { 46.0 } else { 20.0 };
                    let mut job = egui::text::LayoutJob::simple(
                        self.name(index).to_owned(),
                        FontId::proportional(12.0),
                        color,
                        text_width,
                    );
                    job.wrap.max_rows = 2;
                    let galley = ui.painter().layout_job(job);
                    ui.painter().galley(
                        egui::pos2(
                            response.rect.left() + 10.0,
                            response.rect.center().y - galley.size().y / 2.0,
                        ),
                        galley,
                        color,
                    );
                    if let Some((_, label)) = shortcut {
                        keyboard_hint(ui, response.rect, label, color);
                    }
                    if response.on_hover_text(self.name(index)).clicked() {
                        self.select(index);
                    }
                    if index % 6 == 5 {
                        ui.end_row();
                    }
                }
            });
    }

    fn editor(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        if self.output.is_none() {
            editor.preview = false;
            editor.recording = None;
        }
        let mut toggle_preview = false;
        let mut toggle_record = false;
        let mut restart_preview = false;
        let position = self.position();
        let mut save = false;
        let mut close = false;
        egui::Modal::new(egui::Id::new("pattern-editor"))
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgb(239, 240, 227))
                    .corner_radius(12)
                    .inner_margin(28),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() - 90.0).min(690.0))
                    .min_scrolled_height((ctx.content_rect().height() - 90.0).min(690.0))
                    .show(ui, |ui| {
                        let ink = Color32::from_rgb(41, 56, 43);
                        let green = Color32::from_rgb(71, 103, 60);
                        ui.visuals_mut().override_text_color = Some(ink);
                        ui.visuals_mut().widgets.inactive.weak_bg_fill =
                            Color32::from_rgb(224, 228, 211);
                        ui.visuals_mut().widgets.inactive.bg_fill =
                            Color32::from_rgb(255, 254, 246);
                        ui.visuals_mut().widgets.hovered.weak_bg_fill =
                            Color32::from_rgb(205, 215, 188);
                        ui.visuals_mut().widgets.inactive.bg_stroke =
                            Stroke::new(1.0, Color32::from_rgb(185, 195, 172));
                        ui.visuals_mut().text_edit_bg_color =
                            Some(Color32::from_rgb(255, 254, 246));
                        ui.set_width(780.0_f32.min(ctx.content_rect().width() - 64.0));
                        ui.label(
                            RichText::new("4 / 4 · 1 BAR")
                                .monospace()
                                .size(11.0)
                                .color(Color32::from_rgb(100, 113, 94)),
                        );
                        ui.horizontal(|ui| {
                            ui.heading(if editor.blank {
                                "New pattern"
                            } else {
                                "Edit pattern"
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("×").clicked() {
                                        close = true;
                                    }
                                },
                            );
                        });
                        ui.add_space(12.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut editor.draft.name)
                                .char_limit(48)
                                .desired_width(360.0),
                        );
                        ui.add_space(16.0);
                        ui.separator();
                        ui.horizontal_wrapped(|ui| {
                            let preview_response = ui.add_enabled(
                                self.output.is_some(),
                                egui::Button::new(
                                    RichText::new(if editor.preview {
                                        "■ Stop"
                                    } else {
                                        "    Preview"
                                    })
                                    .color(Color32::WHITE),
                                )
                                .fill(Color32::from_rgb(48, 77, 50))
                                .min_size(Vec2::new(145.0, 40.0)),
                            );
                            if !editor.preview {
                                paint_button_icon(
                                    ui,
                                    &preview_response,
                                    ButtonIcon::Play,
                                    "    Preview",
                                    14.0,
                                    Color32::WHITE,
                                );
                            }
                            if preview_response.clicked() {
                                toggle_preview = true;
                            }
                            let record_response = ui.add_enabled(
                                self.output.is_some(),
                                egui::Button::new(RichText::new("    Record").color(
                                    if editor.recording.is_some() {
                                        Color32::WHITE
                                    } else {
                                        Color32::from_rgb(150, 50, 30)
                                    },
                                ))
                                .fill(if editor.recording.is_some() {
                                    Color32::from_rgb(150, 50, 30)
                                } else {
                                    Color32::from_rgb(245, 231, 221)
                                })
                                .min_size(Vec2::new(115.0, 40.0)),
                            );
                            let icon_center = egui::pos2(
                                record_response.rect.left() + 16.0,
                                record_response.rect.center().y,
                            );
                            let icon_color = if self.output.is_none() {
                                Color32::from_rgb(185, 145, 132)
                            } else if editor.recording.is_some() {
                                Color32::WHITE
                            } else {
                                Color32::from_rgb(150, 50, 30)
                            };
                            ui.painter().circle_filled(icon_center, 4.5, icon_color);
                            if record_response.clicked() {
                                toggle_record = true;
                            }
                            if ui
                                .add_enabled(editor.preview, egui::Button::new("↻"))
                                .on_hover_text("Restart preview")
                                .clicked()
                            {
                                restart_preview = true;
                            }
                            ui.add_enabled(
                                !editor.preview || editor.recording.is_none(),
                                egui::DragValue::new(&mut editor.bpm)
                                    .range(40..=240)
                                    .suffix(" BPM"),
                            );
                            ui.label("Volume");
                            ui.spacing_mut().slider_width = 95.0;
                            ui.add(egui::Slider::new(&mut editor.volume, 0..=100).suffix("%"));
                            ui.label(
                                RichText::new(format!(
                                    "{} / 16",
                                    if editor.preview { position.step + 1 } else { 1 }
                                ))
                                .monospace()
                                .size(12.0),
                            );
                        });
                        if let Some(error) = &self.pattern_error {
                            ui.colored_label(Color32::from_rgb(240, 150, 110), error);
                        }
                        if let Some(error) = &self.error {
                            ui.colored_label(Color32::from_rgb(150, 50, 30), error);
                        }
                        ui.separator();
                        ui.add_space(18.0);
                        egui::ScrollArea::horizontal().show(ui, |ui| {
                            egui::Grid::new("steps")
                                .spacing(Vec2::new(5.0, 6.0))
                                .show(ui, |ui| {
                                    ui.label("");
                                    for step in 0..STEPS {
                                        ui.label(
                                            RichText::new(if step % 4 == 0 {
                                                (step / 4 + 1).to_string()
                                            } else {
                                                ["", "e", "&", "a"][step % 4].into()
                                            })
                                            .monospace()
                                            .size(11.0),
                                        );
                                    }
                                    ui.end_row();
                                    for instrument in [
                                        Instrument::ClosedHat,
                                        Instrument::OpenHat,
                                        Instrument::Snare,
                                        Instrument::Rim,
                                        Instrument::Kick,
                                        Instrument::Tom,
                                        Instrument::Crash,
                                    ] {
                                        ui.label(
                                            RichText::new(instrument.name()).monospace().size(10.0),
                                        );
                                        for step in 0..STEPS {
                                            let velocity = &mut editor.draft.pattern.hits
                                                [instrument.index()][step];
                                            let on = *velocity != 0;
                                            let response = ui
                                                .add(
                                                    egui::Button::new(
                                                        RichText::new(if on { "•" } else { " " })
                                                            .color(Color32::WHITE),
                                                    )
                                                    .min_size(Vec2::new(32.0, 34.0))
                                                    .stroke(Stroke::new(
                                                        if editor.preview && position.step == step {
                                                            2.0
                                                        } else {
                                                            1.0
                                                        },
                                                        if editor.preview && position.step == step {
                                                            Color32::from_rgb(165, 110, 44)
                                                        } else {
                                                            Color32::from_rgb(185, 195, 172)
                                                        },
                                                    ))
                                                    .fill(if on {
                                                        if editor.take_hits[instrument.index()]
                                                            [step]
                                                        {
                                                            Color32::from_rgb(150, 50, 30)
                                                        } else {
                                                            green
                                                        }
                                                    } else {
                                                        Color32::from_rgb(224, 228, 211)
                                                    }),
                                                )
                                                .on_hover_text(format!(
                                                    "{}, beat {}, step {}, velocity {}",
                                                    instrument.name(),
                                                    step / 4 + 1,
                                                    step % 4 + 1,
                                                    velocity
                                                ));
                                            if on {
                                                let rect = response.rect.shrink(2.0);
                                                let height =
                                                    rect.height() * f32::from(*velocity) / 127.0;
                                                ui.painter().rect_filled(
                                                    egui::Rect::from_min_max(
                                                        egui::pos2(
                                                            rect.left(),
                                                            rect.bottom() - height,
                                                        ),
                                                        rect.max,
                                                    ),
                                                    2.0,
                                                    Color32::from_white_alpha(38),
                                                );
                                            }
                                            if response.clicked() {
                                                *velocity = if on { 0 } else { 100 };
                                                editor.take_hits[instrument.index()][step] = false;
                                            }
                                        }
                                        ui.end_row();
                                    }
                                });
                        });
                        ui.add_space(20.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(editor.recording.is_none(), egui::Button::new("Clear"))
                                .clicked()
                            {
                                editor.undo_take = Some(editor.draft.pattern);
                                editor.draft.pattern.hits = [[0; STEPS]; INSTRUMENTS];
                                editor.take_hits = [[false; STEPS]; INSTRUMENTS];
                            }
                            let undo_response = ui.add_enabled(
                                editor.recording.is_none() && editor.undo_take.is_some(),
                                egui::Button::new("    Undo take"),
                            );
                            paint_button_icon(
                                ui,
                                &undo_response,
                                ButtonIcon::Undo,
                                "    Undo take",
                                14.0,
                                ink,
                            );
                            if undo_response.clicked() {
                                editor.undo();
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add_enabled(
                                            self.pattern_path.is_some()
                                                && !editor.draft.name.trim().is_empty(),
                                            egui::Button::new(
                                                RichText::new(if editor.factory_copy {
                                                    "Save as new pattern"
                                                } else {
                                                    "Save pattern"
                                                })
                                                .color(Color32::WHITE),
                                            )
                                            .fill(Color32::from_rgb(48, 77, 50)),
                                        )
                                        .clicked()
                                    {
                                        save = true;
                                    }
                                    if ui.button("Cancel").clicked() {
                                        close = true;
                                    }
                                },
                            );
                        });
                    });
            });
        if !ctx.egui_wants_keyboard_input()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space))
        {
            toggle_preview = true;
        }
        if toggle_preview {
            if editor.preview {
                if self.send(Command::Stop) {
                    editor.preview = false;
                }
            } else {
                let settings =
                    preview_settings(&editor, self.settings.swing, self.settings.effects);
                if self.send(Command::Preview(settings)) && self.send(Command::Play) {
                    editor.preview = true;
                    editor.sent_preview = Some(settings);
                }
            }
        }
        if toggle_record {
            editor.toggle_recording(Instant::now());
        }
        if editor.preview {
            let settings = preview_settings(&editor, self.settings.swing, self.settings.effects);
            if editor.sent_preview != Some(settings) && self.send(Command::Preview(settings)) {
                editor.sent_preview = Some(settings);
            }
            if restart_preview && self.send(Command::Stop) {
                editor.preview = self.send(Command::Play);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if save || close {
            if !self.send(Command::Stop) && self.output.is_some() {
                self.editor = Some(editor);
                return;
            }
            self.sent_settings = None;
        }
        if save {
            editor.preview = false;
            editor.recording = None;
            editor.sent_preview = None;
            let mut bank = self.customs.clone();
            let index = editor.target.unwrap_or(bank.len());
            if editor.target.is_some() {
                bank[index] = editor.draft.clone();
            } else {
                bank.push(editor.draft.clone());
            }
            let Some(path) = &self.pattern_path else {
                self.editor = Some(editor);
                return;
            };
            if let Err(error) = pattern_store::save(path, &bank) {
                self.pattern_error = Some(format!(
                    "Could not save patterns: {error}. Your draft is still open; retry Save."
                ));
                self.editor = Some(editor);
                return;
            }
            self.pattern_error = None;
            self.customs = bank;
            self.select(index + Style::ALL.len());
        } else if !close {
            self.editor = Some(editor);
        }
    }
}

impl eframe::App for Drummer {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(seen_running) = self.once_playback {
            let running = self.position().running;
            if seen_running && !running {
                self.playing = false;
                self.once_playback = None;
            } else if running {
                self.once_playback = Some(true);
            }
        }

        if self
            .output
            .as_ref()
            .is_some_and(|o| o.telemetry.failed.load(Ordering::Relaxed))
        {
            self.output = None;
            self.playing = false;
            self.sent_settings = None;
            if let Some(editor) = &mut self.editor {
                editor.preview = false;
                editor.recording = None;
                editor.sent_preview = None;
            }
            self.error =
                Some("Audio output disconnected. Reconnect your speakers or headphones.".into());
        }
        self.handle_midi();
        #[cfg(target_os = "macos")]
        if let Some(media) = &self.media {
            for action in media.drain() {
                if let Some(mut editor) = self.editor.take() {
                    let desired = action.desired(editor.preview);
                    if desired != editor.preview {
                        if desired {
                            let settings = preview_settings(
                                &editor,
                                self.settings.swing,
                                self.settings.effects,
                            );
                            if self.send(Command::Preview(settings)) && self.send(Command::Play) {
                                editor.preview = true;
                                editor.sent_preview = Some(settings);
                            }
                        } else if self.send(Command::Stop) {
                            editor.preview = false;
                        }
                    }
                    self.editor = Some(editor);
                } else if action.desired(self.playing) != self.playing {
                    self.toggle_play();
                }
            }
        }
        if self.editor.is_none()
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|i| !i.modifiers.command && !i.modifiers.ctrl && !i.modifiers.alt)
        {
            if ui.input(|i| i.key_pressed(egui::Key::Space)) {
                self.toggle_play();
            }
            if ui.input(|i| i.modifiers.shift && i.key_pressed(egui::Key::T)) {
                self.tap();
            }
            if self.playing
                && ui.input_mut(|i| {
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                        || i.key_pressed(egui::Key::F)
                })
            {
                self.send(Command::Fill);
            }
            if !ui.input(|i| i.modifiers.shift) {
                for (index, &(key, _)) in GROOVE_SHORTCUTS.iter().enumerate() {
                    if index < Style::ALL.len() + self.customs.len()
                        && ui.input(|i| i.key_pressed(key))
                    {
                        self.select(index);
                        if !self.playing {
                            self.toggle_play();
                        }
                    }
                }
            }
        }
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::NONE
                    .fill(BACKGROUND)
                    .inner_margin(egui::Margin::symmetric(28, 14))
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let status = self.midi_error.as_deref().unwrap_or_else(||
                        self.midi.as_ref().map_or("MIDI disconnected", |input| input.status.as_str()));
                    ui.label(RichText::new(status).size(11.0).color(MUTED))
                        .on_hover_text("Channel 1 keyboard and sustain. Channel 10 pads 44–51. Channel 1 knobs 14, 15, 30, 31. Reconnect after adding or unplugging a device.");
                    if self.midi.as_ref().is_some_and(|input| input.dropped() > 0) {
                        ui.label("MIDI input overloaded; some events were dropped");
                    }
                    if ui.small_button("Reconnect MIDI").clicked() { self.connect_midi(ui.ctx()); }
                });
                if let Some(error) = &self.pattern_error {
                    ui.label(error);
                }
                if let Some(error) = self.error.clone() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(error);
                        if ui.button("Reconnect audio").clicked() {
                            self.connect();
                            if let Some(editor) = &mut self.editor {
                                editor.preview = false;
                                editor.recording = None;
                                editor.sent_preview = None;
                            }
                            self.connect_midi(ui.ctx());
                        }
                    });
                } else {
                    let p = self.position();
                    let selected_id = self.settings.custom.map_or(0, |p| p.id);
                    let status = if self.editor.is_some() {
                        String::new()
                    } else if p.running
                        && (p.style != self.settings.style || p.custom_id != selected_id)
                    {
                        format!("Next bar · {}", self.name(self.selected))
                    } else if p.filling {
                        "Fill · this bar".into()
                    } else if p.running {
                        format!("Playing · bar {} · beat {}", p.bar, p.step / 4 + 1)
                    } else {
                        String::new()
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(status).monospace().size(11.0).color(MUTED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new("4 / 4").monospace().size(11.0).color(MUTED))
                                .on_hover_text(
                                    self.output.as_ref().map_or("", |o| o.description.as_str()),
                                );
                        });
                    });
                }
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BACKGROUND))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.practice(ui);
                    egui::Frame::NONE
                        .stroke(Stroke::new(1.0, BORDER))
                        .inner_margin(egui::Margin::symmetric(28, 22))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            self.bank(ui);
                        });
                    self.keyboard_panel(ui);
                });
            });
        self.editor(ui.ctx());
        #[cfg(target_os = "macos")]
        {
            let (title, playing) = if let Some(editor) = &self.editor {
                (format!("{} · Preview", editor.draft.name), editor.preview)
            } else {
                (self.name(self.selected).to_owned(), self.playing)
            };
            if let Some(media) = &mut self.media {
                media.publish(&title, playing);
            }
        }

        if self.editor.is_none() {
            self.sync_settings();
        }
        self.sync_synth();
        ui.ctx().request_repaint_after(Duration::from_millis(
            if self.playing || self.editor.as_ref().is_some_and(|e| e.preview) {
                16
            } else {
                250
            },
        ));
    }
}

#[derive(Clone, Copy)]
enum ButtonIcon {
    Play,
    Edit,
    Undo,
}

fn paint_button_icon(
    ui: &egui::Ui,
    response: &egui::Response,
    icon: ButtonIcon,
    label: &str,
    font_size: f32,
    color: Color32,
) {
    let font = FontId::proportional(font_size);
    let width = ui
        .painter()
        .layout_no_wrap(label.into(), font.clone(), color)
        .size()
        .x;
    let padding = ui
        .painter()
        .layout_no_wrap("    ".into(), font, color)
        .size()
        .x;
    let center = egui::pos2(
        response.rect.center().x - width / 2.0 + padding / 2.0,
        response.rect.center().y,
    );
    let color = if response.enabled() {
        color
    } else {
        color.gamma_multiply(0.4)
    };
    let stroke = Stroke::new(1.4, color);
    let point = |x, y| center + Vec2::new(x, y);
    match icon {
        ButtonIcon::Play => {
            ui.painter().add(egui::Shape::convex_polygon(
                vec![point(-3.5, -5.0), point(4.5, 0.0), point(-3.5, 5.0)],
                color,
                Stroke::NONE,
            ));
        }
        ButtonIcon::Edit => {
            ui.painter()
                .line_segment([point(-4.0, 4.0), point(4.0, -4.0)], stroke);
            ui.painter()
                .line_segment([point(-2.0, -4.0), point(4.0, -4.0)], stroke);
            ui.painter()
                .line_segment([point(4.0, -4.0), point(4.0, 2.0)], stroke);
        }
        ButtonIcon::Undo => {
            ui.painter().add(egui::Shape::line(
                vec![
                    point(3.0, 4.0),
                    point(5.0, 1.0),
                    point(4.0, -2.0),
                    point(1.0, -4.0),
                    point(-4.0, -4.0),
                ],
                stroke,
            ));
            ui.painter()
                .line_segment([point(-1.0, -7.0), point(-4.0, -4.0)], stroke);
            ui.painter()
                .line_segment([point(-1.0, -1.0), point(-4.0, -4.0)], stroke);
        }
    }
}

fn choice<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    options: &[T],
    name: impl Fn(T) -> &'static str,
) -> egui::Response {
    ui.label(RichText::new(label).size(11.0).color(MUTED));
    egui::ComboBox::from_id_salt(label)
        .width(ui.available_width().min(260.0))
        .selected_text(name(*value))
        .show_ui(ui, |ui| {
            for option in options {
                ui.selectable_value(value, *option, name(*option));
            }
        })
        .response
}

fn milliseconds(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut u16,
    range: std::ops::RangeInclusive<u16>,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(format!("{value} ms")).size(12.0));
        });
    });
    ui.add_space(8.0);
    ui.spacing_mut().slider_width = ui.available_width();
    ui.add(egui::Slider::new(value, range).show_value(false));
}

fn slider(ui: &mut egui::Ui, label: &str, value: &mut u8, maximum: u8) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(format!("{value}%")).size(12.0));
        });
    });
    ui.add_space(8.0);
    ui.spacing_mut().slider_width = ui.available_width();
    ui.add(egui::Slider::new(value, 0..=maximum).show_value(false));
}

fn keyboard_hint(ui: &egui::Ui, rect: egui::Rect, key: &str, color: Color32) {
    let icon = egui::Rect::from_min_size(
        egui::pos2(
            rect.right() - 28.0 - key.len() as f32 * 6.6,
            rect.center().y - 5.0,
        ),
        Vec2::new(15.0, 10.0),
    );
    let painter = ui.painter();
    painter.rect_stroke(icon, 2.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
    for y in [3.0, 6.0] {
        for x in [3.0, 6.0, 9.0, 12.0] {
            painter.circle_filled(icon.min + Vec2::new(x, y), 0.6, color);
        }
    }
    painter.line_segment(
        [
            icon.min + Vec2::new(5.0, 8.0),
            icon.min + Vec2::new(10.0, 8.0),
        ],
        Stroke::new(0.8, color),
    );
    painter.text(
        egui::pos2(rect.right() - 10.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        key,
        FontId::monospace(11.0),
        color,
    );
}

fn preview_settings(
    editor: &Editor,
    swing: u8,
    effects: sherbydrum::effects::EffectSettings,
) -> Settings {
    Settings {
        style: editor.draft.style,
        custom: Some(CustomPattern {
            id: u16::MAX,
            pattern: editor.draft.pattern,
        }),
        bpm: editor.bpm,
        volume: editor.volume,
        swing,
        effects,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Requires native CoreMIDI and an audio output device"]
    fn native_midi_records_into_editor_and_undo_restores_draft() {
        use midir::os::unix::VirtualOutput;
        let mut source = midir::MidiOutput::new("Recording test")
            .unwrap()
            .create_virtual("Sherbydrum recording test")
            .unwrap();
        let output = Output::open().unwrap();
        let midi =
            crate::midi::Input::open(egui::Context::default(), Some(output.telemetry.clone()))
                .unwrap();
        let mut editor = test_editor();
        editor.draft.pattern = Pattern {
            hits: [[0; STEPS]; INSTRUMENTS],
        };
        let original = editor.draft.pattern;
        let settings = preview_settings(&editor, 0, sherbydrum::effects::EffectSettings::default());
        output.send(Command::Preview(settings)).unwrap();
        output.send(Command::Play).unwrap();
        let mut app = Drummer {
            synth: SynthSettings::default(),
            sent_synth: None,
            test_note: false,
            midi_dropped: 0,
            midi_check: Instant::now(),
            midi: Some(midi),
            midi_error: None,
            media: None,
            settings: Settings::default(),
            sent_settings: None,
            output: Some(output),
            error: None,
            playing: false,
            once_playback: None,
            taps: VecDeque::new(),
            customs: Vec::new(),
            pattern_path: None,
            pattern_error: None,
            selected: 0,
            editor: Some(editor),
        };
        std::thread::sleep(Duration::from_millis(100));
        source.send(&[0xb0, 118, 127]).unwrap();
        source.send(&[0xb0, 118, 127]).unwrap();
        source.send(&[0xb0, 118, 0]).unwrap();
        source.send(&[0x99, 49, 82]).unwrap();
        source.send(&[0x89, 49, 0]).unwrap();
        source.send(&[0xb0, 14, 127]).unwrap();
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(10));
            app.handle_midi();
            if app.settings.effects.drive == 100 {
                break;
            }
        }
        assert!(app.error.is_none(), "{:?}", app.error);
        let editor = app.editor.as_mut().unwrap();
        assert_eq!(
            editor.draft.pattern.hits[Instrument::Tom.index()]
                .iter()
                .filter(|v| **v == 82)
                .count(),
            1
        );
        assert_eq!(editor.sent_preview.unwrap().effects.drive, 100);
        assert!(editor.sent_preview.unwrap().effects.enabled);
        assert!(editor.recording.is_some());
        assert!(editor.preview);
        source.send(&[0xb0, 118, 127]).unwrap();
        source.send(&[0xb0, 118, 0]).unwrap();
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(10));
            app.handle_midi();
            if app.editor.as_ref().unwrap().recording.is_none() {
                break;
            }
        }
        let editor = app.editor.as_mut().unwrap();
        assert!(editor.recording.is_none());
        assert!(editor.preview);
        editor.undo();
        assert_eq!(editor.draft.pattern, original);
        app.send(Command::Stop);
    }

    fn test_editor() -> Editor {
        Editor {
            recording: None,
            undo_take: None,
            take_hits: [[false; STEPS]; INSTRUMENTS],
            blank: false,
            factory_copy: true,
            draft: SavedPattern {
                name: "Test".into(),
                style: Style::Rock,
                pattern: *groove(Style::Rock),
            },
            target: None,
            preview: true,
            bpm: 100,
            volume: 65,
            sent_preview: None,
        }
    }

    #[test]
    fn takes_preserve_velocity_overdub_and_undo_without_touching_factory() {
        let mut editor = test_editor();
        let original = editor.draft.pattern;
        let now = Instant::now();
        editor.begin_take(now);
        for velocity in [66, 117] {
            editor.record_hit(
                &crate::midi::Event {
                    source: 0,
                    bytes: [0x99, 49, velocity],
                    received_at: now,
                    step: Some(15),
                },
                Instrument::Tom,
                velocity,
            );
        }
        assert_eq!(editor.draft.pattern.velocity(Instrument::Tom, 15), 117);
        assert_eq!(
            editor.draft.pattern.hits[Instrument::Kick.index()],
            original.hits[Instrument::Kick.index()]
        );
        assert!(editor.take_hits[Instrument::Tom.index()][15]);
        editor.undo(); // unavailable while recording
        assert_eq!(editor.draft.pattern.velocity(Instrument::Tom, 15), 117);
        editor.recording = None;
        editor.undo();
        assert_eq!(editor.draft.pattern, original);
        assert_eq!(*groove(Style::Rock), original);
        assert!(editor.undo_take.is_none());
    }

    #[test]
    fn recording_ignores_old_events_and_stops_capturing_at_end_take() {
        let mut editor = test_editor();
        let before = editor.draft.pattern;
        let now = Instant::now();
        let event = crate::midi::Event {
            source: 0,
            bytes: [0x99, 49, 100],
            received_at: now,
            step: Some(3),
        };
        editor.record_hit(&event, Instrument::Tom, 100);
        assert_eq!(editor.draft.pattern, before);
        editor.begin_take(now + Duration::from_millis(1));
        editor.record_hit(&event, Instrument::Tom, 100);
        assert_eq!(editor.draft.pattern, before);
        let event = crate::midi::Event {
            source: 0,
            received_at: now + Duration::from_millis(2),
            step: None,
            ..event
        };
        editor.record_hit(&event, Instrument::Tom, 75);
        assert_eq!(editor.draft.pattern, before);
        let event = crate::midi::Event {
            source: 0,
            step: Some(0),
            ..event
        };
        editor.record_hit(&event, Instrument::Tom, 75);
        assert_eq!(editor.draft.pattern.velocity(Instrument::Tom, 0), 75);
        editor.recording = None;
        editor.record_hit(&event, Instrument::Tom, 110);
        assert_eq!(editor.draft.pattern.velocity(Instrument::Tom, 0), 75);
        editor.begin_take(now + Duration::from_millis(3));
        editor.recording = None;
        editor.undo();
        assert_eq!(editor.draft.pattern.velocity(Instrument::Tom, 0), 75);
    }

    #[test]
    fn record_toggle_leaves_transport_and_preview_settings_untouched() {
        let now = Instant::now();
        for playing in [false, true] {
            let mut editor = test_editor();
            editor.preview = playing;
            let before = editor.draft.pattern;
            let settings = preview_settings(&editor, 0, Default::default());
            editor.sent_preview = Some(settings);
            editor.toggle_recording(now);
            assert!(editor.recording.is_some());
            assert_eq!(editor.preview, playing);
            assert_eq!(editor.sent_preview, Some(settings));
            let event = crate::midi::Event {
                source: 0,
                bytes: [0x99, 49, 82],
                received_at: now,
                step: Some(3),
            };
            editor.record_hit(&event, Instrument::Tom, 82);
            if !playing {
                assert_eq!(editor.draft.pattern, before);
            }
            editor.toggle_recording(now);
            assert!(editor.recording.is_none());
            assert_eq!(editor.preview, playing);
            assert_eq!(editor.sent_preview, Some(settings));
        }
    }

    #[test]
    fn groove_shortcuts_are_unique_and_preserve_custom_keys() {
        for (index, &(key, _)) in GROOVE_SHORTCUTS.iter().enumerate() {
            assert!(!GROOVE_SHORTCUTS[..index].iter().any(|entry| entry.0 == key));
            assert!(![egui::Key::Space, egui::Key::Enter, egui::Key::F].contains(&key));
        }
        assert_eq!(GROOVE_SHORTCUTS[Style::Bossa.index()].0, egui::Key::T);
        assert_eq!(GROOVE_SHORTCUTS[Style::ALL.len()].0, egui::Key::Num7);
        assert_eq!(GROOVE_SHORTCUTS[Style::ALL.len() + 2].0, egui::Key::Num9);
    }
}
