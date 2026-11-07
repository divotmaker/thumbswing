//! FRP end to end in one process: the driver served by flightrelay's
//! `FrpServer`, a controller that picks a club with `set_detection_mode`, and
//! the shots of a replayed session.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use flightrelay::driver::Stop;
use flightrelay::{
    EndpointConfig, FrpEndpoint, FrpEvent, FrpMessage, FrpProtocolMessage, FrpServer, Role,
    ServerConfig, ServerEvent, Source, Transport,
};
use thumbswing::driver::Thumbswing;
use thumbswing::recording::{Pace, Replay};
use thumbswing::testing::{StreamBuilder, synthetic_recording};
use thumbswing::{DeviceInfo, Input, Sample, Settings};

/// A replay that holds still until `open` is set.
struct Gated {
    replay: Replay,
    open: Arc<AtomicBool>,
}

impl Input for Gated {
    fn try_recv(&mut self) -> Result<Option<Sample>, thumbswing::Error> {
        if self.open.load(Ordering::SeqCst) {
            self.replay.try_recv()
        } else {
            Ok(None)
        }
    }

    fn now_us(&mut self) -> u64 {
        if self.open.load(Ordering::SeqCst) {
            self.replay.now_us()
        } else {
            0
        }
    }

    fn info(&self) -> &DeviceInfo {
        self.replay.info()
    }

    fn paced(&self) -> bool {
        !self.open.load(Ordering::SeqCst) || self.replay.paced()
    }
}

/// A controller that picks the driver for a left-hander and collects every
/// event until two shots have finished.
fn controller(addr: std::net::SocketAddr) -> Vec<FrpEvent> {
    let url = format!("ws://{addr}/frp");
    let mut ep = FrpEndpoint::open(
        EndpointConfig::new(Role::Controller, Transport::connect(url)).with_name("test"),
    )
    .unwrap();
    let mut conn = ep.establish().unwrap();
    conn.send_protocol(&FrpProtocolMessage::SetDetectionMode {
        mode: Some(flightrelay::DetectionMode::Full),
        handed: Some(flightrelay::Handedness::Left),
        club: Some(flightrelay::Club::Driver),
    })
    .unwrap();
    let mut events = Vec::new();
    let mut finished = 0;
    while finished < 2 {
        match conn.recv().unwrap() {
            FrpMessage::Envelope(env) => {
                if matches!(env.event, FrpEvent::ShotFinished { .. }) {
                    finished += 1;
                }
                events.push(env.event);
            }
            FrpMessage::Protocol(_) => {}
        }
    }
    events
}

#[test]
fn a_controller_gets_the_shots_with_the_club_it_chose() {
    // Two swings: straight at full power, then a pull 20° left and a push 5°
    // right at 75 %.
    let samples = StreamBuilder::new(125.0)
        .pull(0.0, 600.0, 150.0)
        .pause_ms(300.0)
        .push(0.0, 1200.0, 100.0)
        .pause_ms(2500.0)
        .pull(20.0, 600.0, 150.0)
        .pause_ms(300.0)
        .push(5.0, 900.0, 100.0)
        .build();
    let rec = synthetic_recording(&samples, &[], 400.0, 125.0);

    let stop = Stop::new();
    let (addr_tx, addr_rx) = mpsc::channel();
    let server = {
        let stop = stop.clone();
        thread::spawn(move || {
            // The swings start once the controller's club choice has reached
            // the driver.
            let open = Arc::new(AtomicBool::new(false));
            let input = Gated {
                replay: Replay::new(&rec, Pace::Fast),
                open: Arc::clone(&open),
            };
            let mut settings = Settings::default();
            settings.config.grab = false;
            let driver = Thumbswing::with_input(settings, input).unwrap();
            FrpServer::new(driver, ServerConfig::new(Transport::listen("127.0.0.1:0")))
                .on_event(move |event| match event {
                    ServerEvent::Listening(addr) => {
                        let _ = addr_tx.send(*addr);
                    }
                    ServerEvent::Command(cmd) if cmd.club.is_some() => {
                        open.store(true, Ordering::SeqCst);
                    }
                    _ => {}
                })
                .run(&stop)
        })
    };
    let addr = addr_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let events = controller(addr);
    stop.stop();
    server.join().unwrap().unwrap();

    // Telemetry says what the device is, that it is virtual, and which club
    // it is using; it comes before the first shot.
    let first_shot = events
        .iter()
        .position(|e| matches!(e, FrpEvent::ShotTrigger { .. }))
        .unwrap();
    let identity = events[..first_shot].iter().find_map(|e| match e {
        FrpEvent::DeviceTelemetry {
            manufacturer: Some(m),
            model,
            source,
            ..
        } => Some((m.clone(), model.clone(), *source)),
        _ => None,
    });
    let (manufacturer, model, source) = identity.expect("identity telemetry before the first shot");
    assert_eq!(manufacturer, "divotmaker");
    assert_eq!(model.as_deref(), Some("thumbswing trackball"));
    assert_eq!(source, Some(Source::Virtual));
    // Telemetry carries what changed: every key at the start, then the club
    // and hand the controller chose.
    let mut t = std::collections::HashMap::new();
    for e in &events[..first_shot] {
        if let FrpEvent::DeviceTelemetry {
            telemetry: Some(keys),
            ..
        } = e
        {
            t.extend(keys.clone());
        }
    }
    assert_eq!(t["profile"], "DR");
    assert_eq!(t["handed"], "lh");
    assert_eq!(t["synthetic"], "true");

    // Each shot is trigger → launch → club_path → finished under one key,
    // with no flight result: the controller simulates the flight.
    let shots: Vec<&[FrpEvent]> = events[first_shot..]
        .split_inclusive(|e| matches!(e, FrpEvent::ShotFinished { .. }))
        .collect();
    assert_eq!(shots.len(), 2);
    for (i, shot) in shots.iter().enumerate() {
        let shot: Vec<&FrpEvent> = shot
            .iter()
            .filter(|e| !matches!(e, FrpEvent::DeviceTelemetry { .. }))
            .collect();
        let [
            FrpEvent::ShotTrigger { key, timestamp },
            FrpEvent::Launch { key: k1, launch },
            FrpEvent::ClubPath { key: k2, club },
            FrpEvent::ShotFinished { key: k3 },
        ] = shot.as_slice()
        else {
            panic!("{shot:?}")
        };
        assert!(k1 == key && k2 == key && k3 == key);
        assert_eq!(key.shot_number as usize, i + 1);
        assert!(timestamp.is_some());
        let speed = launch.launch_speed.unwrap().as_mph();
        let club_speed = club.club_speed.unwrap().as_mph();
        if i == 0 {
            // A full-power drive: 100 mph club, 149 mph ball.
            assert!((club_speed - 100.0).abs() < 2.0, "{club_speed}");
            assert!((speed - 149.0).abs() < 3.0, "{speed}");
        } else {
            assert!((club_speed - 75.0).abs() < 2.0, "{club_speed}");
        }
    }
}
