//! macOS Now Playing commands are handed back to the UI thread.
use eframe::egui;
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig};
use std::sync::mpsc::{Receiver, channel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Play,
    Pause,
    Toggle,
}
impl Action {
    pub fn desired(self, playing: bool) -> bool {
        match self {
            Self::Play => true,
            Self::Pause => false,
            Self::Toggle => !playing,
        }
    }
}
pub struct Media {
    controls: MediaControls,
    events: Receiver<Action>,
    published: Option<(String, bool)>,
}
impl Media {
    pub fn new(ctx: egui::Context) -> Result<Self, String> {
        let mut controls = MediaControls::new(PlatformConfig {
            display_name: "Sherbydrum",
            dbus_name: "sherbydrum",
            hwnd: None,
        })
        .map_err(|e| format!("{e:?}"))?;
        let (sender, events) = channel();
        controls
            .attach(move |event| {
                let action = match event {
                    MediaControlEvent::Play => Action::Play,
                    MediaControlEvent::Pause | MediaControlEvent::Stop => Action::Pause,
                    MediaControlEvent::Toggle => Action::Toggle,
                    _ => return,
                };
                let _ = sender.send(action);
                ctx.request_repaint();
            })
            .map_err(|e| format!("{e:?}"))?;
        Ok(Self {
            controls,
            events,
            published: None,
        })
    }
    pub fn drain(&self) -> Vec<Action> {
        self.events.try_iter().collect()
    }
    pub fn publish(&mut self, title: &str, playing: bool) {
        let state = (title.to_owned(), playing);
        if self.published.as_ref() == Some(&state) {
            return;
        }
        let metadata = self.controls.set_metadata(MediaMetadata {
            title: Some(title),
            artist: Some("Sherbydrum"),
            ..Default::default()
        });
        let playback = self.controls.set_playback(if playing {
            MediaPlayback::Playing { progress: None }
        } else {
            MediaPlayback::Paused { progress: None }
        });
        if metadata.is_ok() && playback.is_ok() {
            self.published = Some(state);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_are_idempotent_except_toggle() {
        for playing in [false, true] {
            assert!(Action::Play.desired(playing));
            assert!(!Action::Pause.desired(playing));
            assert_eq!(Action::Toggle.desired(playing), !playing);
        }
    }
}
