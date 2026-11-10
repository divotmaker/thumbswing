# thumbswing

[![CI](https://github.com/divotmaker/thumbswing/actions/workflows/ci.yml/badge.svg)](https://github.com/divotmaker/thumbswing/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/thumbswing.svg)](https://crates.io/crates/thumbswing)
[![docs.rs](https://docs.rs/thumbswing/badge.svg)](https://docs.rs/thumbswing)

A trackball launch monitor. Pull a trackball back, push it forward, and get a
full golf shot — club speed, path, face, ball speed, launch angle and
direction, backspin and sidespin — the way an arcade-style trackball golf
game reads a swing. It works with any trackball or mouse, and serves the
shots to golf simulators over the
[Flight Relay Protocol](https://github.com/flightrelay/spec) (FRP).

It is meant for testing and developing golf games: every number it sends is
synthesized from the swing, not measured from a real ball.

## Status

**Alpha.** The swing model and the defaults are a first fit; expect the feel
to be tuned with real hands on a ball. Input is **Linux only** for now (evdev);
the library builds on every platform, and on others opening a device returns a
"not supported on this platform" error.

## How a swing becomes a shot

- **Pull back.** The direction of the backswing sets how the shot curves; its
  length can add power (off by default).
- **Push forward.** The direction of the push sets the start line; its peak
  speed sets the club speed. A forward roll with no backswing does nothing.
- **The club caps the speed.** Full power is `max_club_speed` (100 mph for the
  driver by default, a typical stiff-flex player) times the club's speed ratio,
  so no swing can hit a 7-iron faster than 80 mph.

Directions are degrees from straight ahead, positive = right. With `f` the
push direction, `b` the backswing line continued forward through the ball (a
pull toward the player's left gives `b > 0`), `δ = b − f` and `p` the power:

| Step | Formula | Default |
|---|---|---|
| Power | `p = clamp(v_peak / full_power_speed, 0, 1)^γ` | `full_power_speed` 15 in/s, `γ` 1 |
| Club speed | `V = p · max_club_speed · speed_ratio` | 100 mph |
| Face | `F = clamp(angle_gain · f, ±20°)` | `angle_gain` 0.25 |
| Path, arcade sense | `P = F − curve_gain · δ` | `curve_gain` 0.20 |
| Path, physical sense | `P = F + curve_gain · δ` | |
| Face-to-path | `A = clamp(F − P, ±25°)` | |

**Curve sense.** In the arcade sense (the default) you push toward where the
ball should start and pull back from where it should finish: the ball curves
toward the backswing line. Pull back to the left and push straight, and the
ball starts straight and bends right. The `club_path` sent downstream is
always physically consistent with the flight, so in this sense it is the
backswing reflected about the push; the raw stroke directions are shown next
to path and face so it is clear what the hands did. The physical sense makes
the backswing line the club path, and the ball curves away from it.

**Launch.** The club delivery goes through a D-plane impact model: start
direction `P + f(Φ)·A` with face weight `f(Φ) = 0.96 − 0.0071·Φ`, vertical
launch `w·D + (1 − w)·K`, ball speed `V · SF · cos Φ / cos L`, spin
`k · V · sin Φ` with the axis from `tan A / tan L` (`Φ` the 3-D spin loft, `L`
the spin loft, `K` attack angle, `D` dynamic loft).

**Putting.** With the putter the backswing has no effect: `V = p ·
max_putt_speed` (11 mph, a 16.5 mph ball), the line follows the push 1:1
(`putt_angle_gain`), no spin, 0.3° launch so the ball rolls.

**Hand.** Every value is absolute (positive = right), so a left-handed player
who pushes left starts the ball left. `mirror_for_left_hand` (off by default)
mirrors the strokes for players who want a mirrored golfer.

### Club table and smash

Smash is one line in spin loft, `SF = 1.49 − 0.0102 · (L − 13.3)`, which
reproduces every Trackman 2023 PGA Tour average smash within 0.015. The speed
ratio, attack angle, dynamic loft, launch weight and spin constant per club
are fitted so that at the tour's average club speed each club returns the
tour-average ball speed (within 2 %), launch angle (within 0.1°) and spin
(within 1 %). GW, SW and LW have no tour row and are extrapolated.

Full power at the defaults; carry and total from
[dimple](https://crates.io/crates/dimple) 0.0.7 (tour ball, sea level,
fairway, straight):

| Club | Speed ratio | Attack | Dyn. loft | Spin loft | Smash | Club mph | Ball mph | Launch | Spin | Carry yd | Total yd |
|---|---|---|---|---|---|---|---|---|---|---|---|
| DR | 1.00 | −0.9 | 12.4 | 13.3 | 1.490 | 100 | 149.0 | 10.4 | 2208 | 230 | 254 |
| 3W | 0.96 | −2.3 | 11.3 | 13.6 | 1.487 | 96 | 142.7 | 9.3 | 3183 | 221 | 241 |
| 5W | 0.92 | −2.5 | 11.9 | 14.4 | 1.479 | 92 | 136.0 | 9.7 | 3752 | 209 | 227 |
| Hybrid | 0.89 | −2.4 | 12.4 | 14.8 | 1.475 | 89 | 131.2 | 10.2 | 4001 | 197 | 212 |
| 3I | 0.87 | −2.5 | 14.6 | 17.1 | 1.451 | 87 | 126.3 | 10.3 | 3837 | 189 | 208 |
| 4I | 0.85 | −2.9 | 15.4 | 18.3 | 1.439 | 85 | 122.3 | 10.8 | 4164 | 179 | 195 |
| 5I | 0.83 | −3.4 | 17.0 | 20.4 | 1.418 | 83 | 117.7 | 11.9 | 4571 | 169 | 183 |
| 6I | 0.82 | −3.7 | 19.9 | 23.6 | 1.385 | 82 | 113.6 | 14.0 | 5417 | 159 | 170 |
| 7I | 0.80 | −3.9 | 22.8 | 26.7 | 1.353 | 80 | 108.3 | 16.1 | 6219 | 148 | 156 |
| 8I | 0.77 | −4.2 | 25.1 | 29.3 | 1.327 | 77 | 102.2 | 17.8 | 6971 | 136 | 142 |
| 9I | 0.76 | −4.3 | 28.1 | 32.4 | 1.295 | 76 | 98.4 | 20.0 | 7697 | 128 | 133 |
| PW | 0.73 | −4.7 | 33.2 | 37.9 | 1.239 | 73 | 90.5 | 23.7 | 8117 | 114 | 117 |
| GW | 0.71 | −4.8 | 36.3 | 41.1 | 1.206 | 71 | 85.7 | 26.0 | 8495 | 105 | 107 |
| SW | 0.70 | −5.0 | 39.7 | 44.7 | 1.170 | 70 | 81.9 | 28.5 | 8912 | 98 | 99 |
| LW | 0.67 | −5.0 | 43.0 | 48.0 | 1.136 | 67 | 76.1 | 31.0 | 9112 | 87 | 88 |
| PT | — | 0 | 3 | — | 1.5 | 11 | 16.5 | 0.3 | 0 | rolls about 56 ft on a green | |

FRP club codes without a row of their own use the nearest one: `MD` → DR,
`2W`/`4W` → 3W, `6W`–`9W` → 5W, every hybrid → the hybrid row, `1I`/`2I` → 3I.

## Linux permissions

Input devices are `/dev/input/event*`, readable by group `input`. Either join
the group (then log out and back in):

```sh
sudo usermod -aG input $USER
```

or give the logged-in user access with a udev rule:

```sh
echo 'KERNEL=="event*", SUBSYSTEM=="input", TAG+="uaccess"' | sudo tee /etc/udev/rules.d/70-thumbswing.rules
sudo udevadm control --reload && sudo udevadm trigger
```

`cargo run --example list` shows every node, whether you can read it, and
prints these fixes when you cannot.

The examples and `thumbswing-frp` take an exclusive grab by default, so the
desktop cursor does not follow the ball (`--no-grab` or `--set grab=false`
turns it off). The grab ends when the program exits.

## Quick start

```sh
# 1. Find the trackball and measure it: report rate, largest per-report
#    value (spin hard to find the device's limit), idle noise.
cargo run --example list
cargo run --example list -- --measure /dev/input/by-id/usb-…-event-mouse --seconds 30

# 2. Swing and watch the numbers; settings are flags (see Settings below).
cargo run --example monitor -- --device usb-…-event-mouse --full-power-speed 40

# 3. Serve shots to a simulator over FRP on port 5881.
cargo run --features frp --bin thumbswing-frp -- --set device=usb-…-event-mouse
#    then point the simulator at ws://<host>:5881/frp, e.g. cyberflight:
#    cyberflight --frp ws://127.0.0.1:5881/frp

# Record a session (type a note + Enter to mark the next swing) and replay it.
cargo run --example record -- /dev/input/by-id/usb-…-event-mouse session.jsonl --cpi 400
cargo run --example replay -- session.jsonl --club DR
```

The `device` setting (`thumbswing-frp`, the library's
`device::open_setting`), `monitor --device`, `list --measure` and `record`
all take a `/dev/input` path, a bare `/dev/input/by-id` name, or the name the
device reports (for devices without a `by-id` link). Without a `device`
setting or `--device`, the only readable trackball or mouse is used; when
there is more than one, opening fails and lists them.

## thumbswing-frp

```text
thumbswing-frp [--config <path>] [--target <listen-addr | ws://host:port/frp>]
               [--set <key>=<value>]... [--log <info|debug|trace>] [--help]
```

Named flags only, in any order:

- `--config <path>`: a TOML file with the settings at the top level (the keys
  under Settings below).
- `--set key=value` (repeatable): one setting over the file, e.g.
  `--set cpi=800 --set tuning.curve_sense=physical`. A key that does not
  exist is refused.
- `--target`: an address to listen on (default `0.0.0.0:5881`, so it runs
  next to a controller on 5880), or a `ws://` URL to dial a host.
- `--log`: the lowest log level to print (default `info`).
- `--help`: usage, then every setting with its default.

A bad flag, file, key or value exits with status 2 before the device is
opened. Ctrl-C closes the device (releasing the grab) and exits 0. Off Linux
the device cannot be opened, and the program says so once and exits 1.

Several controllers can connect at once, and each gets every shot. The club
and the player's hand start as the `club` and `hand` settings (the 7-iron and
right-handed by default, e.g. `--set club=DR --set hand=lh`); a club or hand
from a controller's `set_detection_mode` replaces them, and its putting mode
selects the putter whatever the club. When the trackball is unplugged the
program reports `ready=false`, reopens it when it is back, and goes on; with
`device` unset it chooses the device again, so it is found under a new node
too. When the device cannot be opened, the controllers get an alert saying
what to do (pick one with `device`, fix the permissions, …), without device
paths, and the program keeps trying with a growing delay; a choice only the
user can make is alerted once until the device opens.

What the controllers receive (FRP 0.2.0):

- `device_telemetry` before the first shot: `manufacturer: divotmaker`,
  `model: thumbswing trackball`, the crate version as `firmware`,
  `source: virtual` (every value is synthesized), and the keys `ready`,
  `synthetic = "true"`, `input` (the device name), `cpi`, `profile` (the
  club) and `handed`; after that only the keys that change. The FRP `device`
  id is the device's `/dev/input/by-id` link name (the file name only, e.g.
  `usb-Example_Trackball-event-mouse`), the same across reconnects and
  replugs; a device without a `by-id` link is known by its name. The label
  shows the device name.
- Each shot: `shot_trigger` (with the time of the swing's last report, the
  end of the push) → `launch` (speed,
  launch angle and direction, both spin pairs; a putt has total spin 0 and
  no axis) → `club_path` (club speed, path, face, attack angle, dynamic loft,
  smash) → `shot_finished`. There is no `flight_result`: carry and roll are
  left to the simulator.

Development only: `--replay FILE` serves a recording instead of a device,
starting when the first controller connects (`cpi` and `report_limit`
default to the recording's header); `--fast` replays it as fast as it can be
processed instead of at the recorded timing, and `--exit-after-replay` quits
when it is done.

## Settings

Every setting is a field of `thumbswing::Settings`: a `thumbswing-frp` key
(`--set` or `--config`) and an `examples/monitor` flag. Each starts from the
default below; an invalid value is refused with the key named before the
device is opened. The status line of `monitor` shows the values in
use:

```text
full 15.0 in/s | max 100 mph | angle 0.25 | curve 0.20 arcade | β 0.00 γ 1.0 | 7I RH | last 7I: back +12.0° 1.4 in, fwd −3.0° 13.5 in/s → 90 % 72 mph, path +1.1 face −0.8
```

| Key | `monitor` flag | Default | Unit | What it sets |
|---|---|---|---|---|
| `device` | `--device` | unset | | the input; unset: the only trackball or mouse found |
| `club` | `--club CODE` | 7I | FRP code | the club until a controller names one (`DR`, `3W`, `7I`, `PW`, `PT`, …) |
| `hand` | `--hand RH\|LH` | rh | `rh`/`lh` | the player's hand until a controller names one |
| `cpi` | `--cpi N` | 400 | counts/in | counts per inch of ball surface travel |
| `report_limit` | `--report-limit N` | 127 | counts | largest `\|dx\|`/`\|dy\|` one report carries: 127, 2047 or 32767 |
| `grab` | `--no-grab` | true | | exclusive grab, so the desktop cursor does not follow the ball |
| `tuning.full_power_speed` | `--full-power-speed X` | 15 | in/s | forward peak speed that counts as 100 % power |
| `tuning.max_club_speed` | `--max-club-speed X` | 100 | mph | driver club speed at full power (other clubs scale it) |
| `tuning.max_putt_speed` | `--max-putt-speed X` | 11 | mph | putter club speed at full power |
| `tuning.angle_gain` | `--angle-gain X` | 0.25 | °/° | face angle per degree of push direction |
| `tuning.putt_angle_gain` | `--putt-angle-gain X` | 1 | °/° | putt line per degree of push direction |
| `tuning.curve_gain` | `--curve-gain X` | 0.2 | °/° | face-to-path per degree between backswing line and push |
| `tuning.curve_sense` | `--curve-sense SENSE` | arcade | | `arcade` or `physical` |
| `tuning.backswing_power` | `--backswing-power X` | 0 | 0 to 1 | share of the power set by the backswing length |
| `tuning.power_gamma` | `--power-gamma X` | 1 | | exponent of the power curve |
| `tuning.mirror_for_left_hand` | `--mirror-for-left-hand` | false | | mirror the strokes for a left-handed player |
| `tuning.face_max` | `--face-max X` | 20 | ° | largest face angle |
| `tuning.face_to_path_max` | `--face-to-path-max X` | 25 | ° | largest face-to-path |
| `thresholds.speed_window_ms` | `--speed-window-ms N` | 24 | ms | trailing window the speed is measured over |
| `thresholds.gap_ms` | `--gap-ms N` | 50 | ms | silence that starts a new speed series |
| `thresholds.v_start` | `--v-start X` | 0.067 | × full-power speed | speed that starts a backswing |
| `thresholds.v_stop` | `--v-stop X` | 0.033 | × full-power speed | speed below which the ball counts as stopped (below `v_start`) |
| `thresholds.v_min_swing` | `--v-min-swing X` | 0.133 | × full-power speed | speed a forward stroke must reach to be a swing |
| `thresholds.dwell_min_ms` | `--dwell-min-ms N` | 40 | ms | how long the ball must stay below `v_stop` |
| `thresholds.back_min` | `--back-min X` | 0.13 | × ℓ_full | shortest backswing |
| `thresholds.full_cock` | `--full-cock X` | 0.67 | × ℓ_full | backswing length that counts as fully cocked |
| `thresholds.fwd_min` | `--fwd-min X` | 0.17 | × ℓ_full | forward travel that makes a forward motion a stroke |
| `thresholds.back_max_s` | `--back-max-s X` | 2 | s | longest backswing |
| `thresholds.coil_timeout_s` | `--coil-timeout-s X` | 10 | s | how long a coiled backswing waits for the push |
| `thresholds.fwd_window_ms` | `--fwd-window-ms N` | 300 | ms | longest forward stroke |
| `thresholds.idle_settle_ms` | `--idle-settle-ms N` | 250 | ms | quiet time that ends the cooldown after a shot |
| `thresholds.cooldown_max_s` | `--cooldown-max-s X` | 2 | s | longest cooldown after a shot |
| `thresholds.pull_cone_deg` | `--pull-cone-deg X` | 75 | ° | widest angle from straight back that still counts as a pull |

As a `--config` file:

```toml
device = "usb-Example_Trackball-event-mouse"
club = "DR"
cpi = 800

[tuning]
full_power_speed = 40
curve_sense = "physical"
```

`ℓ_full` is the ball travel of 100 ms at full power (1.5 in at the default
15 in/s). Every detection threshold is a fraction of the full-power speed or
of `ℓ_full`, so setting `full_power_speed` to how hard you want 100 % to be
adapts them all to the device. With `monitor --replay`, `--cpi` and
`--report-limit` default to the recording's header.

The club, the hand, the thresholds and the tuning apply to a running device
at once (the club and hand only while no controller has named one); a change
of `device`, `cpi`, `report_limit` or `grab` reopens it.

## Devices and saturation

Any device with relative X/Y axes works: a large arcade trackball, a
thumb-ball trackball mouse, an ordinary mouse. They differ in counts per inch
(CPI), report rate and per-report width, which is why every setting is a flag
and why `list --measure` exists. Speed is measured over a 24 ms window
rather than per report, so a 125 Hz and a 1000 Hz device give the same
numbers at the same settings.

A classic 8-bit report carries at most ±127 counts. At 125 Hz and 400 CPI
that is 39.7 in/s; at 1600 CPI only 9.9 in/s. A forward stroke with two or
more reports at the limit is flagged (`Event::Saturated`, "SATURATED" on the
status line): its direction comes from the reports before saturation and its
speed is a lower bound. Lower `full_power_speed`, or set `report_limit` to
the device's real limit (2047 for 12-bit, 32767 for 16-bit reports).

## Library

```rust,no_run
use thumbswing::{Client, Club, Config, Event, device::Device};

let config = Config::default();
let input = Device::open("/dev/input/by-id/usb-Example_Trackball-event-mouse", config.grab)?;
let mut client = Client::new(input, config);
client.set_club(Club::Driver);
loop {
    match client.poll()? {
        Some(Event::Shot(shot)) => {
            let l = &shot.launch;
            println!("{:.1} mph, {:.1}° up, {:+.1}°, {:.0}/{:+.0} rpm",
                l.ball_speed_mph, l.launch_angle_deg, l.launch_azimuth_deg,
                l.backspin_rpm, l.sidespin_rpm);
        }
        Some(_) => {}
        None => std::thread::sleep(std::time::Duration::from_millis(2)),
    }
}
# Ok::<(), thumbswing::Error>(())
```

The API is synchronous and poll-driven, with no async runtime. The pipeline
is platform-independent: `Sample`s (one device report) → `Segmenter` (the
clocked swing state machine) → `Swing` → `mapping` → `Delivery` → `launch` →
`Launch`, and each stage is public. The Linux backend reads the device on its
own thread (`poll(2)` with a timeout, kernel timestamps on
`CLOCK_MONOTONIC`); an unplug is `Event::Disconnected`, and the client reopens
the device and sends `Event::Ready` when it is back.

`Settings` holds everything a device runs with, and `Settings::validate`
checks the ranges in the table above.

Features:

- **`config`** — serde on `Settings` (the keys above; a default writes
  nothing) and `Settings` as a [flightrelay](https://crates.io/crates/flightrelay)
  `DriverConfig`. No device stack: a settings form, including one built for
  WebAssembly, can use it.
- **`driver`** — `thumbswing::driver::Thumbswing`, a flightrelay
  `DeviceDriver`, for a host that runs launch monitors (implies `config`).
- **`frp`** — the `thumbswing-frp` binary (implies `driver`).
- **`testing`** — `thumbswing::testing`: synthetic swing streams, a manual
  clock and fixture expectations, for tests of code that uses the crate.

## Recording format

Sessions are JSON Lines, written by `examples/record` and read by
`recording::Recording::load`:

```text
{"kind":"header","version":1,"device":"Some Trackball","cpi":400,"rate_hz":125,"report_limit":127,"grab":true,"started":"2026-11-05T19:02:11Z"}
{"t_us":0,"dx":-3,"dy":12}
{"t_us":8012,"dx":-4,"dy":15}
{"kind":"mark","t_us":912340,"note":"straight drive"}
```

`t_us` is microseconds since the start on the device's monotonic clock, one
line per report with `dx` (positive = right) and `dy` (positive = toward the
player). `mark` lines are notes typed while recording. Lines of unknown kind
(such as the `summary` line `record` appends) are skipped. Replay uses the
recorded timing, so a recording replays exactly as it was swung.

## Testing

```sh
cargo test --features frp,testing
```

- Swing tests from synthetic report streams, each run at 125 Hz and at
  1000 Hz with the same expected result (`tests/segmenter.rs`).
- The impact model against the tour averages, and the worked examples above
  flown with dimple (`tests/worked_examples.rs`).
- Fixtures in `tests/fixtures/`, each a recording plus a `*.expect.json` with
  the expected swings. The two fixtures there now are **generated
  synthetically** by `thumbswing::testing` (their headers say so), not
  recorded from hardware; recordings from real devices are added next to them.
- The driver's session on synthetic streams and the fixtures: telemetry,
  each shot once, the club and hand from the settings and from the host, the
  putting mode, held shots, live tuning, a fast replay without sleeps, an
  unplugged input, choosing the device again after an unplug, alerts without
  device paths and alerted once per outage (`tests/driver.rs`).
- The settings as a driver config: defaults that write nothing, nested and
  optional keys, the club and hand, errors that name the field, and
  `thumbswing-frp --set` refusing a bad club or hand (`tests/settings.rs`).
- FRP end to end in one process: the driver served to a controller that
  picks a club and receives the shots (`tests/frp.rs`).
- The command line of `monitor`: defaults, every settings flag, invalid
  values, the refusal to guess between several devices, and the flag ranges
  against `Settings::validate` (`tests/cli.rs`).
- Through the kernel: a uinput virtual mouse, opened by name, swung, unplugged
  and replugged (`tests/uinput.rs`). It needs a writable `/dev/uinput` and
  read access to the new event node, so it is ignored by default:
  `cargo test --features testing --test uinput -- --ignored` (or
  `make test-uinput`).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
