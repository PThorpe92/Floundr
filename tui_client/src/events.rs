use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyEvent, MouseEvent};
use tokio::time::Instant;

#[derive(Clone, Copy, Debug)]
pub enum AppEvent {
    Tick,
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct AppEventHandler {
    sender: tokio::sync::mpsc::Sender<AppEvent>,
    receiver: tokio::sync::mpsc::Receiver<AppEvent>,
    handler: tokio::task::JoinHandle<()>,
}

impl AppEventHandler {
    pub fn new(tick_rate: u64) -> Self {
        let tick_rate = Duration::from_millis(tick_rate);
        let (sender, receiver) = tokio::sync::mpsc::channel(10);
        let send_event = sender.clone();
        let handler = {
            tokio::spawn(async move {
                let sender = send_event.clone();
                let mut last_tick = Instant::now();
                loop {
                    let timeout = tick_rate
                        .checked_sub(last_tick.elapsed())
                        .unwrap_or(tick_rate);
                    if event::poll(timeout).expect("no events available") {
                        match event::read().expect("unable to read event") {
                            Event::Key(e) => sender.send(AppEvent::Key(e)),
                            Event::Mouse(e) => sender.send(AppEvent::Mouse(e)),
                            Event::Resize(w, h) => sender.send(AppEvent::Resize(w, h)),
                            Event::FocusGained => sender.send(AppEvent::Tick),
                            Event::FocusLost => sender.send(AppEvent::Tick),
                            Event::Paste(_s) => sender.send(AppEvent::Tick),
                        }
                        .await
                        .expect("failed to send terminal event")
                    }

                    if last_tick.elapsed() >= tick_rate {
                        sender
                            .send(AppEvent::Tick)
                            .await
                            .expect("failed to send tick event");
                        last_tick = Instant::now();
                    }
                }
            })
        };
        Self {
            sender,
            receiver,
            handler,
        }
    }

    pub async fn next(&mut self) -> Option<AppEvent> {
        self.receiver.recv().await
    }
}
