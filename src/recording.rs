//! Recorded sessions: the JSON Lines fixture format, a writer, and replay as
//! an [`Input`].
//!
//! ```text
//! {"kind":"header","version":1,"device":"Some Trackball","cpi":400,"rate_hz":125,"report_limit":127,"grab":true,"started":"2026-11-05T19:02:11Z"}
//! {"t_us":0,"dx":-3,"dy":12}
//! {"t_us":8012,"dx":-4,"dy":15}
//! {"kind":"mark","t_us":912340,"note":"straight drive"}
//! ```
//!
//! - The first line is the header. `cpi`, `rate_hz` and `report_limit` are
//!   what the recorder knew or measured; extra header keys are kept in
//!   [`Header::extra`].
//! - One line per report: `t_us` is microseconds since the start of the
//!   recording on the device's monotonic clock, `dx`/`dy` the report's summed
//!   `REL_X`/`REL_Y` (positive = right, positive = toward the player).
//! - `mark` lines are annotations typed while recording.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufRead, BufReader, LineWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::error::Error;
use crate::input::{DeviceInfo, Input};
use crate::json::{self, Value};
use crate::sample::Sample;

/// Format version written by [`Recorder`].
pub const FORMAT_VERSION: u32 = 1;

/// The first line of a recording.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// Format version.
    pub version: u32,
    /// Device name.
    pub device: String,
    /// Counts per inch the recorder assumed.
    pub cpi: f64,
    /// Report rate, Hz (nominal or measured).
    pub rate_hz: f64,
    /// Per-report limit.
    pub report_limit: i32,
    /// Whether the device was grabbed.
    pub grab: bool,
    /// UTC start time, RFC 3339.
    pub started: String,
    /// Other header keys, as raw JSON values.
    pub extra: Vec<(String, String)>,
}

impl Default for Header {
    fn default() -> Self {
        Self {
            version: FORMAT_VERSION,
            device: String::new(),
            cpi: 400.0,
            rate_hz: 125.0,
            report_limit: 127,
            grab: true,
            started: utc_now(),
            extra: Vec::new(),
        }
    }
}

/// One line after the header.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// A report.
    Sample(Sample),
    /// An annotation.
    Mark {
        /// When it was typed, µs since the start.
        t_us: u64,
        /// The text.
        note: String,
    },
}

/// A loaded recording.
#[derive(Debug, Clone, PartialEq)]
pub struct Recording {
    /// The header.
    pub header: Header,
    /// Reports and marks in file order.
    pub entries: Vec<Entry>,
    /// Where it was loaded from, if from a file.
    pub path: Option<PathBuf>,
}

fn parse_err(file: &str, line: usize, message: impl Into<String>) -> Error {
    Error::Parse {
        file: file.to_owned(),
        line,
        message: message.into(),
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn as_u64(v: Option<&Value>) -> Option<u64> {
    v.and_then(Value::as_f64)
        .filter(|n| *n >= 0.0 && n.fract() == 0.0)
        .map(|n| n as u64)
}

#[allow(clippy::cast_possible_truncation)]
fn as_i32(v: Option<&Value>) -> Option<i32> {
    v.and_then(Value::as_f64)
        .filter(|n| n.fract() == 0.0 && n.abs() <= f64::from(i32::MAX))
        .map(|n| n as i32)
}

impl Recording {
    /// Load a recording from a file.
    ///
    /// # Errors
    /// I/O errors and [`Error::Parse`] for malformed lines.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let name = path.display().to_string();
        let reader = BufReader::new(File::open(path)?);
        let mut rec = Self::read(reader, &name)?;
        rec.path = Some(path.to_owned());
        Ok(rec)
    }

    /// Parse a recording from text.
    ///
    /// # Errors
    /// [`Error::Parse`] for malformed lines.
    pub fn parse(text: &str) -> Result<Self, Error> {
        Self::read(text.as_bytes(), "<text>")
    }

    fn read(reader: impl BufRead, name: &str) -> Result<Self, Error> {
        let mut header: Option<Header> = None;
        let mut entries = Vec::new();
        for (i, line) in reader.lines().enumerate() {
            let n = i + 1;
            let line = line?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let v = json::parse(line).map_err(|e| parse_err(name, n, e))?;
            match v.get("kind").and_then(Value::as_str) {
                Some("header") => header = Some(parse_header(&v)),
                Some("mark") => entries.push(Entry::Mark {
                    t_us: as_u64(v.get("t_us"))
                        .ok_or_else(|| parse_err(name, n, "mark without t_us"))?,
                    note: v
                        .get("note")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                }),
                // Unknown kinds are skipped, so newer files stay readable.
                Some(_) => {}
                None => {
                    let t_us = as_u64(v.get("t_us"));
                    let dx = as_i32(v.get("dx"));
                    let dy = as_i32(v.get("dy"));
                    match (t_us, dx, dy) {
                        (Some(t_us), Some(dx), Some(dy)) => {
                            entries.push(Entry::Sample(Sample::new(t_us, dx, dy)));
                        }
                        _ => {
                            return Err(parse_err(name, n, "report needs integer t_us, dx and dy"));
                        }
                    }
                }
            }
        }
        let header = header.ok_or_else(|| parse_err(name, 1, "no header line"))?;
        Ok(Self {
            header,
            entries,
            path: None,
        })
    }

    /// The reports, in order.
    pub fn samples(&self) -> impl Iterator<Item = Sample> + '_ {
        self.entries.iter().filter_map(|e| match e {
            Entry::Sample(s) => Some(*s),
            Entry::Mark { .. } => None,
        })
    }

    /// The marks, in order.
    pub fn marks(&self) -> impl Iterator<Item = (u64, &str)> + '_ {
        self.entries.iter().filter_map(|e| match e {
            Entry::Mark { t_us, note } => Some((*t_us, note.as_str())),
            Entry::Sample(_) => None,
        })
    }

    /// Write the recording as JSON Lines.
    ///
    /// # Errors
    /// I/O errors.
    pub fn write(&self, mut w: impl Write) -> io::Result<()> {
        writeln!(w, "{}", header_line(&self.header))?;
        for e in &self.entries {
            match e {
                Entry::Sample(s) => writeln!(w, "{}", sample_line(s))?,
                Entry::Mark { t_us, note } => writeln!(w, "{}", mark_line(*t_us, note))?,
            }
        }
        Ok(())
    }
}

fn parse_header(v: &Value) -> Header {
    let mut h = Header {
        started: String::new(),
        ..Header::default()
    };
    if let Value::Obj(fields) = v {
        for (k, val) in fields {
            match k.as_str() {
                "kind" => {}
                "version" => {
                    h.version = as_u64(Some(val)).map_or(0, |n| u32::try_from(n).unwrap_or(0));
                }
                "device" => val.as_str().unwrap_or("").clone_into(&mut h.device),
                "cpi" => h.cpi = val.as_f64().unwrap_or(h.cpi),
                "rate_hz" => h.rate_hz = val.as_f64().unwrap_or(h.rate_hz),
                "report_limit" => h.report_limit = as_i32(Some(val)).unwrap_or(h.report_limit),
                "grab" => h.grab = val.as_bool().unwrap_or(h.grab),
                "started" => val.as_str().unwrap_or("").clone_into(&mut h.started),
                _ => h.extra.push((k.clone(), value_json(val))),
            }
        }
    }
    h
}

fn value_json(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Num(n) => fmt_num(*n),
        Value::Str(s) => json::quote(s),
        Value::Arr(a) => format!(
            "[{}]",
            a.iter().map(value_json).collect::<Vec<_>>().join(",")
        ),
        Value::Obj(o) => format!(
            "{{{}}}",
            o.iter()
                .map(|(k, v)| format!("{}:{}", json::quote(k), value_json(v)))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{n:.0}")
    } else {
        format!("{n}")
    }
}

fn header_line(h: &Header) -> String {
    let mut s = format!(
        "{{\"kind\":\"header\",\"version\":{},\"device\":{},\"cpi\":{},\"rate_hz\":{},\"report_limit\":{},\"grab\":{},\"started\":{}",
        h.version,
        json::quote(&h.device),
        fmt_num(h.cpi),
        fmt_num(h.rate_hz),
        h.report_limit,
        h.grab,
        json::quote(&h.started),
    );
    for (k, v) in &h.extra {
        let _ = write!(s, ",{}:{v}", json::quote(k));
    }
    s.push('}');
    s
}

fn sample_line(s: &Sample) -> String {
    format!("{{\"t_us\":{},\"dx\":{},\"dy\":{}}}", s.t_us, s.dx, s.dy)
}

fn mark_line(t_us: u64, note: &str) -> String {
    format!(
        "{{\"kind\":\"mark\",\"t_us\":{t_us},\"note\":{}}}",
        json::quote(note)
    )
}

/// Writes a recording line by line, so a killed recorder loses at most the
/// line being written.
#[derive(Debug)]
pub struct Recorder<W: Write> {
    out: W,
    t0: Option<u64>,
}

impl Recorder<LineWriter<File>> {
    /// Create (truncate) `path` and write the header.
    ///
    /// # Errors
    /// I/O errors.
    pub fn create(path: impl AsRef<Path>, header: &Header) -> io::Result<Self> {
        Self::new(LineWriter::new(File::create(path)?), header)
    }
}

impl<W: Write> Recorder<W> {
    /// Write the header to `out`.
    ///
    /// # Errors
    /// I/O errors.
    pub fn new(mut out: W, header: &Header) -> io::Result<Self> {
        writeln!(out, "{}", header_line(header))?;
        Ok(Self { out, t0: None })
    }

    /// Fix time zero on the input's clock (otherwise the first sample is
    /// time zero).
    pub fn start_at(&mut self, now_us: u64) {
        self.t0.get_or_insert(now_us);
    }

    /// Write a report, re-based to the recording's time zero.
    ///
    /// # Errors
    /// I/O errors.
    pub fn sample(&mut self, s: &Sample) -> io::Result<()> {
        let t0 = *self.t0.get_or_insert(s.t_us);
        let rel = Sample::new(s.t_us.saturating_sub(t0), s.dx, s.dy);
        writeln!(self.out, "{}", sample_line(&rel))
    }

    /// Write a mark at `now_us` on the input's clock.
    ///
    /// # Errors
    /// I/O errors.
    pub fn mark(&mut self, now_us: u64, note: &str) -> io::Result<()> {
        let t0 = *self.t0.get_or_insert(now_us);
        writeln!(self.out, "{}", mark_line(now_us.saturating_sub(t0), note))
    }

    /// Flush and return the writer.
    ///
    /// # Errors
    /// I/O errors.
    pub fn finish(mut self) -> io::Result<W> {
        self.out.flush()?;
        Ok(self.out)
    }
}

/// How fast a [`Replay`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pace {
    /// As fast as polled: every poll advances the replay clock by 1 ms, and
    /// the input is not [`paced`](Input::paced) until it has been played, so
    /// a polling loop runs it without sleeping.
    Fast,
    /// At the recorded speed, on the wall clock.
    RealTime,
}

/// Replay of a [`Recording`] as an [`Input`], with the recording's own
/// timing: its pauses are pauses on the replay clock.
#[derive(Debug, Clone)]
pub struct Replay {
    samples: Vec<Sample>,
    next: usize,
    pace: Pace,
    now: u64,
    started: Option<Instant>,
    info: DeviceInfo,
}

/// Fast-pace clock step per poll, µs.
const FAST_STEP_US: u64 = 1000;
/// Clock time after the last report until [`Replay::finished`], µs: enough
/// for every timer of the segmenter to run out.
const TAIL_US: u64 = 3_000_000;

impl Replay {
    /// Replay `recording` at `pace`.
    #[must_use]
    pub fn new(recording: &Recording, pace: Pace) -> Self {
        let samples: Vec<Sample> = recording.samples().collect();
        let now = samples.first().map_or(0, |s| s.t_us);
        Self {
            samples,
            next: 0,
            pace,
            now,
            started: None,
            info: DeviceInfo {
                path: recording.path.clone().unwrap_or_default(),
                name: recording.header.device.clone(),
                relative_xy: true,
                readable: true,
                kernel_timestamps: true,
                ..DeviceInfo::default()
            },
        }
    }

    /// Whether every report has been delivered and the clock has run on long
    /// enough for the last swing to finish.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.next >= self.samples.len()
            && self.now >= self.samples.last().map_or(0, |s| s.t_us) + TAIL_US
    }

    /// Reports not yet delivered.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.samples.len() - self.next
    }
}

impl Input for Replay {
    fn try_recv(&mut self) -> Result<Option<Sample>, Error> {
        if self.pace == Pace::RealTime {
            self.now_us();
        }
        match self.samples.get(self.next) {
            Some(s) if s.t_us <= self.now => {
                self.next += 1;
                Ok(Some(*s))
            }
            _ => Ok(None),
        }
    }

    fn now_us(&mut self) -> u64 {
        let base = self.samples.first().map_or(0, |s| s.t_us);
        match self.pace {
            Pace::Fast => {
                let n = self.now;
                self.now += FAST_STEP_US;
                n
            }
            Pace::RealTime => {
                let started = *self.started.get_or_insert_with(Instant::now);
                let elapsed = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
                self.now = base.saturating_add(elapsed);
                self.now
            }
        }
    }

    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn paced(&self) -> bool {
        self.pace == Pace::RealTime || self.finished()
    }
}

/// The current UTC time as RFC 3339, to the second.
#[must_use]
pub fn utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    utc_from_unix(secs)
}

/// A Unix time as RFC 3339 UTC, to the second.
#[must_use]
pub fn utc_from_unix(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = r#"{"kind":"header","version":1,"device":"Test Ball","cpi":400,"rate_hz":125,"report_limit":127,"grab":true,"started":"2026-11-05T19:02:11Z","synthetic":true}
{"t_us":0,"dx":-3,"dy":12}
{"t_us":8012,"dx":-4,"dy":15}
{"kind":"mark","t_us":912340,"note":"straight drive"}
"#;

    #[test]
    fn parses_the_documented_format() {
        let r = Recording::parse(TEXT).unwrap();
        assert_eq!(r.header.device, "Test Ball");
        assert_eq!(r.header.report_limit, 127);
        assert_eq!(
            r.header.extra,
            vec![("synthetic".to_owned(), "true".to_owned())]
        );
        assert_eq!(
            r.samples().collect::<Vec<_>>(),
            vec![Sample::new(0, -3, 12), Sample::new(8012, -4, 15)]
        );
        assert_eq!(
            r.marks().collect::<Vec<_>>(),
            vec![(912_340, "straight drive")]
        );
    }

    #[test]
    fn write_then_parse_round_trips() {
        let r = Recording::parse(TEXT).unwrap();
        let mut out = Vec::new();
        r.write(&mut out).unwrap();
        let again = Recording::parse(std::str::from_utf8(&out).unwrap()).unwrap();
        assert_eq!(again, r);
    }

    #[test]
    fn recorder_rebases_time() {
        let mut rec = Recorder::new(Vec::new(), &Header::default()).unwrap();
        rec.sample(&Sample::new(5_000_000, 1, 2)).unwrap();
        rec.mark(5_010_000, "x").unwrap();
        rec.sample(&Sample::new(5_020_000, 3, 4)).unwrap();
        let text = String::from_utf8(rec.finish().unwrap()).unwrap();
        let r = Recording::parse(&text).unwrap();
        assert_eq!(
            r.samples().collect::<Vec<_>>(),
            vec![Sample::new(0, 1, 2), Sample::new(20_000, 3, 4)]
        );
        assert_eq!(r.marks().collect::<Vec<_>>(), vec![(10_000, "x")]);
    }

    #[test]
    fn missing_header_or_bad_report_is_an_error() {
        assert!(Recording::parse(r#"{"t_us":0,"dx":1,"dy":1}"#).is_err());
        let bad = TEXT.replace(r#""dx":-4"#, r#""dx":"x""#);
        assert!(matches!(
            Recording::parse(&bad),
            Err(Error::Parse { line: 3, .. })
        ));
    }

    #[test]
    fn fast_replay_delivers_on_its_own_clock() {
        let r = Recording::parse(TEXT).unwrap();
        let mut replay = Replay::new(&r, Pace::Fast);
        assert_eq!(replay.try_recv().unwrap(), Some(Sample::new(0, -3, 12)));
        assert_eq!(replay.try_recv().unwrap(), None);
        for _ in 0..8 {
            replay.now_us();
        }
        assert_eq!(replay.try_recv().unwrap(), None);
        replay.now_us();
        assert_eq!(replay.try_recv().unwrap(), Some(Sample::new(8012, -4, 15)));
        assert!(!replay.finished());
    }

    #[test]
    fn utc_formatting() {
        assert_eq!(utc_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_from_unix(1_793_646_131), "2026-11-02T19:02:11Z");
    }
}
