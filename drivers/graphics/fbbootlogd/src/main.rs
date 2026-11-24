// SPDX-License-Identifier: GPL-2.0-only AND MIT
//! Fbbootlogd renders the boot log and presents it on VT1.
//!
//! When the bootloader boots quietly (`QUIET=1` in the kernel environment) it draws a boot splash
//! instead, like plymouth, and keeps the log behind it; any key shows the log.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, IntoRawFd};

use console_draw::alacritty_terminal::grid::Scroll;
use console_draw::alacritty_terminal::term;
use console_draw::{TextScreen, V2DisplayMap};
use event::EventQueue;
use graphics_ipc::DrmHandle;
use inputd::{ConsumerHandle, ConsumerHandleEvent};
use orbclient::{Event, EventOption};

use crate::splash::Splash;

mod splash;

event::user_data! {
    enum Source {
        LogPipe,
        Input,
        DisplayHandle,
    }
}

fn main() {
    daemon::Daemon::new(daemon);
}
fn daemon(daemon: daemon::Daemon) -> ! {
    let event_queue = EventQueue::new().expect("fbbootlogd: failed to create event queue");

    let (mut log_reader, log_writer) = io::pipe().expect("fbbootlogd: failed to create pipe");
    if unsafe { libc::fcntl(log_reader.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) != 0 } {
        panic!(
            "fbbootlogd: failed to set pipe as nonblocking: {}",
            io::Error::last_os_error()
        )
    };

    let input_handle = ConsumerHandle::bootlog_vt().expect("fbbootlogd: Failed to open vt");
    let splash = std::fs::read_to_string("/scheme/sys/env")
        .ok()
        .and_then(|env| Splash::from_env(&env));
    let mut bootlog = Fbbootlog::new(splash);
    bootlog.handle_handoff(&input_handle, &event_queue);

    event_queue
        .subscribe(
            log_reader.as_raw_fd() as usize,
            Source::LogPipe,
            event::EventFlags::READ,
        )
        .expect("fbbootlogd: failed to subscribe to log pipe events");

    event_queue
        .subscribe(
            input_handle.event_handle().as_raw_fd() as usize,
            Source::Input,
            event::EventFlags::READ,
        )
        .expect("fbbootlogd: failed to subscribe to input events");

    {
        // Add ourself as log sink
        let log_file = libredox::Fd::open(
            "/scheme/log/add_sink",
            libredox::flag::O_WRONLY | libredox::flag::O_CLOEXEC,
            0,
        )
        .expect("fbbootlogd: failed to open log/add_sink");
        log_file
            .call_wo(
                &(log_writer.into_raw_fd() as usize).to_ne_bytes(),
                syscall::CallFlags::FD,
                &[],
            )
            .expect("fbbootlogd: failed to send log fd to log scheme.");
    }

    let _ = daemon.ready();

    libredox::call::setrens(0, 0).expect("fbbootlogd: failed to enter null namespace");

    for event in event_queue.iter() {
        match event.expect("fbbootlogd: failed to get event").user_data {
            Source::LogPipe => loop {
                let mut buf = [0; 4096];
                let n = match log_reader.read(&mut buf) {
                    Ok(n) => n,
                    Err(e) if e.raw_os_error() == Some(libc::EAGAIN) => break,
                    Err(e) => panic!("fbbootlogd: failed to read from log pipe: {e}"),
                };
                bootlog.handle_logs(&buf[..n]);
            },
            Source::Input => {
                let mut events = [Event::new(); 16];
                loop {
                    match input_handle
                        .read_events(&mut events)
                        .expect("fbbootlogd: error while reading events")
                    {
                        ConsumerHandleEvent::Events(&[]) => break,
                        ConsumerHandleEvent::Events(events) => {
                            for event in events {
                                bootlog.handle_input(event);
                            }
                        }
                        ConsumerHandleEvent::Handoff => {
                            eprintln!("fbbootlogd: handoff requested");
                            bootlog.handle_handoff(&input_handle, &event_queue);
                        }
                    }
                }
            }
            Source::DisplayHandle => {
                bootlog.handle_display_event();
            }
        }
    }

    std::process::exit(0);
}

struct Fbbootlog {
    display_map: Option<V2DisplayMap>,
    text_screen: TextScreen,
    shift: bool,
    /// Drawn instead of the log until a key is pressed, in a quiet boot.
    splash: Option<Splash>,
}

impl Fbbootlog {
    fn new(splash: Option<Splash>) -> Self {
        let mut config = term::Config::default();
        config.scrolling_history = 1000;

        let mut text_screen = TextScreen::new(None, config);
        if splash.is_some() {
            text_screen.hide();
        }

        Self {
            display_map: None,
            text_screen,
            shift: false,
            splash,
        }
    }

    fn draw_splash(&mut self) {
        if let (Some(splash), Some(map)) = (&self.splash, &mut self.display_map) {
            if let Err(err) = splash.draw(map) {
                eprintln!("fbbootlogd: failed to draw the splash: {err}");
            }
        }
    }

    fn handle_handoff(&mut self, input_handle: &ConsumerHandle, event_queue: &EventQueue<Source>) {
        let new_display_handle = match input_handle.open_display() {
            Ok(display) => DrmHandle::from_file(display).unwrap(),
            Err(err) => {
                eprintln!("fbbootlogd: No display present yet: {err}");
                return;
            }
        };

        match V2DisplayMap::new(new_display_handle) {
            Ok(mut display_map) => {
                self.text_screen.handle_handoff(&mut display_map);
                if let Some(old_display_map) = &self.display_map {
                    event_queue
                        .unsubscribe(old_display_map.event_handle().as_raw_fd() as usize)
                        .expect("fbbootlogd: failed to unsubscribe from old drm events");
                }
                event_queue
                    .subscribe(
                        display_map.event_handle().as_raw_fd() as usize,
                        Source::DisplayHandle,
                        event::EventFlags::READ,
                    )
                    .expect("fbbootlogd: failed to subscribe to drm events");
                self.display_map = Some(display_map);
                self.draw_splash();
            }
            Err(err) => {
                eprintln!("fbbootlogd: failed to open display: {}", err);
                return;
            }
        };

        eprintln!("fbbootlogd: mapped display");
    }

    fn handle_input(&mut self, ev: &Event) {
        // Any key during the splash shows the log instead
        if self.splash.is_some() {
            if let EventOption::Key(key_event) = ev.to_option() {
                if key_event.pressed {
                    self.splash = None;
                    self.text_screen.show(self.display_map.as_mut());
                }
            }
            return;
        }

        match ev.to_option() {
            EventOption::Key(key_event) => {
                if key_event.scancode == 0x2A || key_event.scancode == 0x36 {
                    self.shift = key_event.pressed;
                } else if !key_event.pressed || !self.shift {
                    return;
                }
                match key_event.scancode {
                    0x48 => {
                        // Up
                        self.text_screen.scroll_display(Scroll::Delta(1));
                    }
                    0x49 => {
                        // Page up
                        self.text_screen.scroll_display(Scroll::PageUp);
                    }
                    0x50 => {
                        // Down
                        self.text_screen.scroll_display(Scroll::Delta(-1));
                    }
                    0x51 => {
                        // Page down
                        self.text_screen.scroll_display(Scroll::PageDown);
                    }
                    0x47 => {
                        // Home
                        self.text_screen.scroll_display(Scroll::Bottom);
                    }
                    0x4F => {
                        // End
                        self.text_screen.scroll_display(Scroll::Top);
                    }
                    _ => return,
                }
            }
            _ => return,
        }
        if let Some(map) = &mut self.display_map {
            self.text_screen.write(map, &[], &mut VecDeque::new());
        }
    }

    fn handle_display_event(&mut self) {
        if let Some(map) = &mut self.display_map {
            match self.text_screen.handle_display_event(map) {
                Ok(()) => self.draw_splash(),
                Err(err) => {
                    eprintln!("fbbootlogd: failed to create or map framebuffer: {}", err);
                    return;
                }
            }
        }
    }

    fn handle_logs(&mut self, buf: &[u8]) {
        if self
            .splash
            .as_mut()
            .is_some_and(|splash| splash.take_logs(buf))
        {
            self.draw_splash();
        }
        if let Some(map) = &mut self.display_map {
            self.text_screen.write(map, buf, &mut VecDeque::new());
        }
    }
}
