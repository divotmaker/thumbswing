//! FRP end to end in one process: the driver served by flightrelay's
//! `FrpServer` as a launch monitor actor, a peer that publishes a
//! `golfer.turn` with a club, and the shots of a replayed session.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use flightrelay::driver::Stop;
use flightrelay::types::roles;
use flightrelay::{
    BusActor, EndpointConfig, FrpBody, FrpEndpoint, FrpMessage, FrpServer, GolferTurn, Handedness,
    Mode, ServerConfig, ServerEvent, Source, Transport,
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

const SERVER_ID: &str = "7d2f9a41c03be856";
const PEER_ID: &str = "c19e04b7a2d5f318";

/// A session actor whose turn is the driver for a left-hander; collects
/// every message until two shots have finished.
fn peer(addr: std::net::SocketAddr) -> Vec<FrpMessage> {
    let url = format!("ws://{addr}/frp");
    let mut ep = FrpEndpoint::open(EndpointConfig::new(Transport::connect(url))).unwrap();
    let mut conn = ep.establish().unwrap();
    let me = FrpMessage::new(
        PEER_ID,
        BusActor::new("test", 2.0).with_role(roles::SESSION),
    );
    let turn = FrpMessage::new(
        PEER_ID,
        GolferTurn::new(
            None,
            Some(flightrelay::Club::Driver),
            Handedness::Left,
            Mode::Full,
        ),
    );
    conn.greet(&[me], &[turn]).unwrap();
    let mut messages = Vec::new();
    let mut finished = 0;
    while finished < 2 {
        let msg = conn.recv().unwrap();
        if matches!(msg.body, FrpBody::ShotFinished(_)) {
            finished += 1;
        }
        messages.push(msg);
    }
    messages
}

#[test]
fn a_peer_gets_the_shots_with_the_club_of_its_turn() {
    // Two swings: straight at full power, then a pull 20° left and a push 5°
    // right at 75 %.
    let samples = StreamBuilder::new(125.0)
        .pull(0.0, 300.0, 150.0)
        .pause_ms(300.0)
        .push(0.0, 600.0, 100.0)
        .pause_ms(2500.0)
        .pull(20.0, 300.0, 150.0)
        .pause_ms(300.0)
        .push(5.0, 450.0, 100.0)
        .build();
    let rec = synthetic_recording(&samples, &[], 400.0, 125.0);

    let stop = Stop::new();
    let (addr_tx, addr_rx) = mpsc::channel();
    let server = {
        let stop = stop.clone();
        thread::spawn(move || {
            // The swings start once the turn's club has reached the driver.
            let open = Arc::new(AtomicBool::new(false));
            let input = Gated {
                replay: Replay::new(&rec, Pace::Fast),
                open: Arc::clone(&open),
            };
            let mut settings = Settings::default();
            settings.config.grab = false;
            let driver = Thumbswing::with_input(settings, input).unwrap();
            FrpServer::new(
                driver,
                ServerConfig::new(Transport::listen("127.0.0.1:0"), SERVER_ID),
            )
            .on_event(move |event| match event {
                ServerEvent::Listening(addr) => {
                    let _ = addr_tx.send(*addr);
                }
                ServerEvent::Hints(hints) if hints.club.is_some() => {
                    open.store(true, Ordering::SeqCst);
                }
                _ => {}
            })
            .run(&stop)
        })
    };
    let addr = addr_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let messages = peer(addr);
    stop.stop();
    server.join().unwrap().unwrap();
    assert!(messages.iter().all(|m| m.from == SERVER_ID), "{messages:?}");

    // The actor says what the device is, that it is virtual, which club it
    // is using, and that it is ready, before the first shot.
    let first_shot = messages
        .iter()
        .position(|m| matches!(m.body, FrpBody::ShotTrigger(_)))
        .unwrap();
    let actor = messages[..first_shot]
        .iter()
        .rev()
        .find_map(|m| match &m.body {
            FrpBody::BusActor(actor) => Some(actor),
            _ => None,
        })
        .expect("bus.actor before the first shot");
    assert!(actor.has_role(roles::LAUNCH_MONITOR));
    assert!(actor.is_clear(), "ready: {:?}", actor.blockers);
    let device = actor.device.as_ref().expect("device block");
    assert_eq!(device.manufacturer.as_deref(), Some("divotmaker"));
    assert_eq!(device.model.as_deref(), Some("thumbswing trackball"));
    assert_eq!(device.source, Some(Source::Virtual));
    let t = actor.telemetry.as_ref().expect("telemetry");
    assert_eq!(t["profile"], "DR");
    assert_eq!(t["handed"], "lh");
    assert_eq!(t["synthetic"], "true");

    // Each shot is trigger → launch → club → finished under one key, with
    // no flight: a game flies the ball.
    let shots: Vec<Vec<&FrpBody>> = messages[first_shot..]
        .split_inclusive(|m| matches!(m.body, FrpBody::ShotFinished(_)))
        .map(|shot| {
            shot.iter()
                .map(|m| &m.body)
                .filter(|b| !matches!(b, FrpBody::BusActor(_)))
                .collect()
        })
        .collect();
    assert_eq!(shots.len(), 2);
    for (i, shot) in shots.iter().enumerate() {
        let [
            FrpBody::ShotTrigger(trigger),
            FrpBody::ShotLaunch(launch),
            FrpBody::ShotClub(club),
            FrpBody::ShotFinished(finished),
        ] = shot.as_slice()
        else {
            panic!("{shot:?}")
        };
        let key = &trigger.key;
        assert!(&launch.key == key && &club.key == key && &finished.key == key);
        assert_eq!(key.shot_number as usize, i + 1);
        assert!(trigger.timestamp.is_some());
        let speed = launch.launch.launch_speed.unwrap().as_mph();
        let club_speed = club.club.club_speed.unwrap().as_mph();
        if i == 0 {
            // A full-power drive: 100 mph club, 149 mph ball.
            assert!((club_speed - 100.0).abs() < 2.0, "{club_speed}");
            assert!((speed - 149.0).abs() < 3.0, "{speed}");
        } else {
            assert!((club_speed - 75.0).abs() < 2.0, "{club_speed}");
        }
    }
}
