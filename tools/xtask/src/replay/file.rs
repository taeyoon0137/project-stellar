//! 입력 재생 파일 형식 (XVI.2.1, XVI.2.6, ADR-0084 결정 9, ADR-0096 결정 4·5).
//!
//! 재생 파일은 프로젝트 밖에 두는 UTF-8 줄 단위 텍스트입니다. 사용자 문서를 가리키지 않고 synthetic fixture
//! 명세와 그 content hash만 담습니다.
//!
//! ```text
//! stellar-replay v2
//! fixture visible-nodes 1 20000
//! fixture-hash <64 hex>
//! viewport 1920 1080
//! scenario undo
//! cache warm
//! measure-from 4
//! event 0 down 950 545
//! event 8333333 move 954 548
//! event 16666666 move 958 551
//! event 25000000 up 958 551
//! event 33333333 key z cmd
//! ```
//!
//! - `fixture <이름> <seed> <규모>`: `stellar_bench_fixtures` 생성기 입력입니다. 재생할 때 다시 만든 fixture의
//!   content hash가 `fixture-hash`와 다르면 거부합니다.
//! - `viewport <너비> <높이>`: 창 logical px.
//! - `scenario <이름>`: §38.1 성능 게이트 시나리오([`ReplayScenario`]) 이름입니다.
//! - `cache warm|cold`(v2): 측정 시작 cache 상태입니다(XVI.2.6). `cold`면 runner가 측정 첫 event 직전에 cache를
//!   비웁니다.
//! - `measure-from <n>`(v2): 앞 `n`개 event는 측정 전 setup(예: undo할 편집 만들기)이고 trace sample이 아닙니다.
//!   이 값은 실행 전에 파일에 정해지므로 측정값을 보고 sample을 고르는 일이 아닙니다(XVI.4.3).
//! - `event <ns> <종류> <값…>`: ManualClock timeline 시각(ns, 감소 금지)과 공통 입력(`InputEvent`, XII.3.1과 같은
//!   의미)입니다. 좌표는 view px이고 f32의 가장 짧은 왕복 표기입니다.
//!   - `down x y`, `move x y`, `up x y`: 주 버튼 pointer. 누르지 않은 `move`는 hover입니다.
//!   - `wheel dx dy`: wheel(view px).
//!   - `key <key> <modifier>`(v2): key 누름. modifier는 `-`(없음) 또는 `cmd`·`ctrl`·`alt`·`shift`를 이 순서로 `+`로
//!     이은 값입니다.
//!   - `text <hex>`(v2): 확정 text 입력 Unicode scalar 하나(소문자 hex, 선행 0 없음).
//!   - `commit <n>`(v2): 패널·View 조작의 확정 단계 `n`. 단계 값의 뜻은 시나리오가 정합니다(예: Variant Change는
//!     `n`번째 Variant). UI 재설계 전이라 패널 입력을 확정 단계로만 적습니다.
//! - v1 파일(`stellar-replay v1`)도 읽습니다. v1에는 `cache`·`measure-from`·v2 event가 없고 각각 warm·0입니다.
//!   쓰기는 항상 v2입니다.
//! - 빈 줄과 `#` 주석 줄은 무시합니다. 같은 값은 같은 텍스트로 씁니다(결정적 직렬화).

use std::fmt;
use std::fmt::Write as _;

use stellar_diagnostics::interaction::CacheState;
use stellar_diagnostics::result::FixtureName;

/// 재생 scenario. §38.1 성능 게이트 시나리오 목록과 같은 값입니다(ADR-0096 결정 3).
pub use stellar_diagnostics::latency_scenario::LatencyScenario as ReplayScenario;

/// 파일 첫 줄(쓰기).
pub const REPLAY_HEADER: &str = "stellar-replay v2";

/// 읽을 수 있는 옛 첫 줄.
pub const REPLAY_HEADER_V1: &str = "stellar-replay v1";

/// event 수 상한(오입력 방지).
pub const MAX_REPLAY_EVENTS: usize = 1_000_000;

/// key modifier 집합.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Modifiers {
    /// command.
    pub cmd: bool,
    /// control.
    pub ctrl: bool,
    /// option.
    pub alt: bool,
    /// shift.
    pub shift: bool,
}

impl Modifiers {
    /// 없음.
    pub const NONE: Modifiers = Modifiers {
        cmd: false,
        ctrl: false,
        alt: false,
        shift: false,
    };

    fn names(self) -> [(bool, &'static str); 4] {
        [
            (self.cmd, "cmd"),
            (self.ctrl, "ctrl"),
            (self.alt, "alt"),
            (self.shift, "shift"),
        ]
    }

    /// 결정적 표기.
    pub fn to_text(self) -> String {
        let parts: Vec<&str> = self
            .names()
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, name)| *name)
            .collect();
        if parts.is_empty() {
            "-".to_owned()
        } else {
            parts.join("+")
        }
    }

    /// 표기를 읽습니다. 순서가 다르거나 중복이면 `None`(표기 하나 = 값 하나).
    pub fn parse(text: &str) -> Option<Modifiers> {
        if text == "-" {
            return Some(Modifiers::NONE);
        }
        let mut out = Modifiers::NONE;
        for part in text.split('+') {
            match part {
                "cmd" => out.cmd = true,
                "ctrl" => out.ctrl = true,
                "alt" => out.alt = true,
                "shift" => out.shift = true,
                _ => return None,
            }
        }
        (out.to_text() == text).then_some(out)
    }
}

/// 재생 입력 event 종류.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplayInput {
    /// 주 버튼 누름(view px).
    Down {
        /// x.
        x: f32,
        /// y.
        y: f32,
    },
    /// 이동(누르지 않았으면 hover).
    Move {
        /// x.
        x: f32,
        /// y.
        y: f32,
    },
    /// 뗌.
    Up {
        /// x.
        x: f32,
        /// y.
        y: f32,
    },
    /// wheel(view px).
    Wheel {
        /// dx.
        dx: f32,
        /// dy.
        dy: f32,
    },
    /// key 누름.
    Key {
        /// key 이름(소문자 영숫자, 예: `z`, `escape`).
        key: String,
        /// modifier.
        modifiers: Modifiers,
    },
    /// 확정 text 입력 scalar 하나.
    Text {
        /// 문자.
        scalar: char,
    },
    /// 패널·View 조작 확정 단계.
    Commit {
        /// 단계.
        step: u64,
    },
}

/// 재생 event.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayEvent {
    /// ManualClock timeline 시각(ns).
    pub at_nanos: u64,
    /// 입력.
    pub input: ReplayInput,
}

/// fixture 명세.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureSpec {
    /// 이름.
    pub name: FixtureName,
    /// seed.
    pub seed: u64,
    /// 규모(가시 node 목표).
    pub size: u64,
}

/// 재생 파일.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayFile {
    /// fixture.
    pub fixture: FixtureSpec,
    /// fixture content hash(32 bytes).
    pub fixture_hash: [u8; 32],
    /// viewport(logical px).
    pub viewport: (f32, f32),
    /// scenario.
    pub scenario: ReplayScenario,
    /// 측정 시작 cache 상태.
    pub cache: CacheState,
    /// 측정 전 setup event 수.
    pub measure_from: usize,
    /// event(시각 순).
    pub events: Vec<ReplayEvent>,
}

/// 재생 파일 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayFileError {
    /// 줄 번호(1부터, 0은 파일 전체).
    pub line: usize,
    /// 이유.
    pub reason: String,
}

impl fmt::Display for ReplayFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "재생 파일 {}행: {}", self.line, self.reason)
    }
}

impl std::error::Error for ReplayFileError {}

fn error(line: usize, reason: impl Into<String>) -> ReplayFileError {
    ReplayFileError {
        line,
        reason: reason.into(),
    }
}

fn parse_f32(line: usize, text: &str) -> Result<f32, ReplayFileError> {
    let value: f32 = text
        .parse()
        .map_err(|_| error(line, format!("수가 아닙니다: {text}")))?;
    if !value.is_finite() {
        return Err(error(line, "유한한 수가 아닙니다"));
    }
    Ok(value)
}

fn parse_u64(line: usize, text: &str) -> Result<u64, ReplayFileError> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(error(line, format!("10진 정수가 아닙니다: {text}")));
    }
    text.parse()
        .map_err(|_| error(line, format!("범위를 벗어났습니다: {text}")))
}

fn parse_hash(line: usize, text: &str) -> Result<[u8; 32], ReplayFileError> {
    let valid = text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !valid {
        return Err(error(line, "fixture-hash는 소문자 hex 64자입니다"));
    }
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&text[2 * i..2 * i + 2], 16)
            .map_err(|_| error(line, "hex가 아닙니다"))?;
    }
    Ok(out)
}

fn parse_cache(line: usize, text: &str) -> Result<CacheState, ReplayFileError> {
    match text {
        "warm" => Ok(CacheState::Warm),
        "cold" => Ok(CacheState::Cold),
        other => Err(error(line, format!("cache는 warm·cold입니다: {other}"))),
    }
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 16
        && key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn parse_scalar(line: usize, text: &str) -> Result<char, ReplayFileError> {
    let canonical = !text.is_empty()
        && text.len() <= 6
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && (text == "0" || !text.starts_with('0'));
    let value = canonical
        .then(|| u32::from_str_radix(text, 16).ok())
        .flatten()
        .and_then(char::from_u32)
        .ok_or_else(|| {
            error(
                line,
                format!("text는 Unicode scalar 소문자 hex입니다: {text}"),
            )
        })?;
    if value.is_control() {
        return Err(error(line, "text에 제어 문자를 넣을 수 없습니다"));
    }
    Ok(value)
}

impl ReplayFile {
    /// 텍스트를 읽습니다.
    pub fn parse(text: &str) -> Result<ReplayFile, ReplayFileError> {
        let mut lines = text
            .lines()
            .enumerate()
            .map(|(n, l)| (n + 1, l.trim_end()))
            .filter(|(_, l)| !l.is_empty() && !l.starts_with('#'));
        let v2 = match lines.next() {
            Some((_, header)) if header == REPLAY_HEADER => true,
            Some((_, header)) if header == REPLAY_HEADER_V1 => false,
            Some((n, _)) => return Err(error(n, format!("첫 줄은 {REPLAY_HEADER}여야 합니다"))),
            None => return Err(error(0, "빈 파일입니다")),
        };
        let mut fixture = None;
        let mut fixture_hash = None;
        let mut viewport = None;
        let mut scenario = None;
        let mut cache = None;
        let mut measure_from = None;
        let mut events: Vec<ReplayEvent> = Vec::new();
        for (n, line) in lines {
            let parts: Vec<&str> = line.split(' ').collect();
            match parts.as_slice() {
                ["fixture", name, seed, size] if fixture.is_none() => {
                    let name = FixtureName::parse(name)
                        .ok_or_else(|| error(n, format!("알 수 없는 fixture: {name}")))?;
                    fixture = Some(FixtureSpec {
                        name,
                        seed: parse_u64(n, seed)?,
                        size: parse_u64(n, size)?,
                    });
                }
                ["fixture-hash", hash] if fixture_hash.is_none() => {
                    fixture_hash = Some(parse_hash(n, hash)?);
                }
                ["viewport", w, h] if viewport.is_none() => {
                    let (w, h) = (parse_f32(n, w)?, parse_f32(n, h)?);
                    if w <= 0.0 || h <= 0.0 {
                        return Err(error(n, "viewport 크기는 양수입니다"));
                    }
                    viewport = Some((w, h));
                }
                ["scenario", name] if scenario.is_none() => {
                    scenario = Some(
                        ReplayScenario::parse(name)
                            .ok_or_else(|| error(n, format!("알 수 없는 scenario: {name}")))?,
                    );
                }
                ["cache", state] if v2 && cache.is_none() => {
                    cache = Some(parse_cache(n, state)?);
                }
                ["measure-from", count] if v2 && measure_from.is_none() => {
                    let count = usize::try_from(parse_u64(n, count)?)
                        .map_err(|_| error(n, "measure-from이 너무 큽니다"))?;
                    measure_from = Some(count);
                }
                ["event", at, rest @ ..] => {
                    let at_nanos = parse_u64(n, at)?;
                    if events.last().is_some_and(|e| e.at_nanos > at_nanos) {
                        return Err(error(n, "event 시각이 감소합니다"));
                    }
                    if events.len() >= MAX_REPLAY_EVENTS {
                        return Err(error(n, "event가 너무 많습니다"));
                    }
                    let input = match rest {
                        [kind @ ("down" | "move" | "up" | "wheel"), a, b] => {
                            let (a, b) = (parse_f32(n, a)?, parse_f32(n, b)?);
                            match *kind {
                                "down" => ReplayInput::Down { x: a, y: b },
                                "move" => ReplayInput::Move { x: a, y: b },
                                "up" => ReplayInput::Up { x: a, y: b },
                                _ => ReplayInput::Wheel { dx: a, dy: b },
                            }
                        }
                        ["key", key, modifiers] if v2 => {
                            if !valid_key(key) {
                                return Err(error(n, format!("key 이름이 잘못되었습니다: {key}")));
                            }
                            let modifiers = Modifiers::parse(modifiers).ok_or_else(|| {
                                error(n, format!("modifier 표기가 잘못되었습니다: {modifiers}"))
                            })?;
                            ReplayInput::Key {
                                key: (*key).to_owned(),
                                modifiers,
                            }
                        }
                        ["text", scalar] if v2 => ReplayInput::Text {
                            scalar: parse_scalar(n, scalar)?,
                        },
                        ["commit", step] if v2 => ReplayInput::Commit {
                            step: parse_u64(n, step)?,
                        },
                        _ => return Err(error(n, format!("알 수 없는 event: {line}"))),
                    };
                    events.push(ReplayEvent { at_nanos, input });
                }
                _ => return Err(error(n, format!("알 수 없거나 중복된 줄: {line}"))),
            }
        }
        let measure_from = measure_from.unwrap_or(0);
        if measure_from > events.len() {
            return Err(error(0, "measure-from이 event 수보다 큽니다"));
        }
        Ok(ReplayFile {
            fixture: fixture.ok_or_else(|| error(0, "fixture 줄이 없습니다"))?,
            fixture_hash: fixture_hash.ok_or_else(|| error(0, "fixture-hash 줄이 없습니다"))?,
            viewport: viewport.ok_or_else(|| error(0, "viewport 줄이 없습니다"))?,
            scenario: scenario.ok_or_else(|| error(0, "scenario 줄이 없습니다"))?,
            cache: if v2 {
                cache.ok_or_else(|| error(0, "cache 줄이 없습니다"))?
            } else {
                CacheState::Warm
            },
            measure_from,
            events,
        })
    }

    /// 결정적 텍스트(v2).
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{REPLAY_HEADER}");
        let _ = writeln!(
            out,
            "fixture {} {} {}",
            self.fixture.name.name(),
            self.fixture.seed,
            self.fixture.size
        );
        let hash: String = self
            .fixture_hash
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let _ = writeln!(out, "fixture-hash {hash}");
        let _ = writeln!(out, "viewport {} {}", self.viewport.0, self.viewport.1);
        let _ = writeln!(out, "scenario {}", self.scenario.name());
        let _ = writeln!(out, "cache {}", self.cache.name());
        let _ = writeln!(out, "measure-from {}", self.measure_from);
        for event in &self.events {
            let _ = write!(out, "event {} ", event.at_nanos);
            let _ = match &event.input {
                ReplayInput::Down { x, y } => writeln!(out, "down {x} {y}"),
                ReplayInput::Move { x, y } => writeln!(out, "move {x} {y}"),
                ReplayInput::Up { x, y } => writeln!(out, "up {x} {y}"),
                ReplayInput::Wheel { dx, dy } => writeln!(out, "wheel {dx} {dy}"),
                ReplayInput::Key { key, modifiers } => {
                    writeln!(out, "key {key} {}", modifiers.to_text())
                }
                ReplayInput::Text { scalar } => writeln!(out, "text {:x}", u32::from(*scalar)),
                ReplayInput::Commit { step } => writeln!(out, "commit {step}"),
            };
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ReplayFile {
        ReplayFile {
            fixture: FixtureSpec {
                name: FixtureName::VisibleNodes,
                seed: 7,
                size: 20_000,
            },
            fixture_hash: [0x5a; 32],
            viewport: (1920.0, 1080.0),
            scenario: ReplayScenario::Drag,
            cache: CacheState::Cold,
            measure_from: 1,
            events: vec![
                ReplayEvent {
                    at_nanos: 0,
                    input: ReplayInput::Down { x: 10.5, y: 20.25 },
                },
                ReplayEvent {
                    at_nanos: 8_333_333,
                    input: ReplayInput::Move {
                        x: 0.1,
                        y: -3.000_001,
                    },
                },
                ReplayEvent {
                    at_nanos: 8_333_333,
                    input: ReplayInput::Wheel { dx: 0.0, dy: 1e-7 },
                },
                ReplayEvent {
                    at_nanos: 16_666_666,
                    input: ReplayInput::Up { x: 12.0, y: 22.0 },
                },
                ReplayEvent {
                    at_nanos: 16_666_667,
                    input: ReplayInput::Key {
                        key: "z".to_owned(),
                        modifiers: Modifiers {
                            cmd: true,
                            shift: true,
                            ..Modifiers::NONE
                        },
                    },
                },
                ReplayEvent {
                    at_nanos: 16_666_668,
                    input: ReplayInput::Text { scalar: '한' },
                },
                ReplayEvent {
                    at_nanos: 16_666_669,
                    input: ReplayInput::Commit { step: 3 },
                },
            ],
        }
    }

    #[test]
    fn replay_file_round_trips() {
        // 검증 기준(XVI.2.1): round-trip.
        let file = sample();
        let text = file.to_text();
        let parsed = ReplayFile::parse(&text).unwrap();
        assert_eq!(parsed, file);
        assert_eq!(parsed.to_text(), text);
        assert!(text.starts_with("stellar-replay v2\nfixture visible-nodes 7 20000\n"));
        assert!(text.contains("cache cold\nmeasure-from 1\n"));
        assert!(text.contains("event 8333333 move 0.1 -3.000001\n"));
        assert!(text.contains("event 16666667 key z cmd+shift\n"));
        assert!(text.contains("event 16666668 text d55c\n"));
        assert!(text.contains("event 16666669 commit 3\n"));
    }

    #[test]
    fn reads_version_one_files_as_warm_without_setup() {
        let v1 = "stellar-replay v1\nfixture visible-nodes 7 20000\nfixture-hash ".to_owned()
            + &"5a".repeat(32)
            + "\nviewport 1920 1080\nscenario pan\nevent 0 down 1 2\nevent 1 up 1 2\n";
        let file = ReplayFile::parse(&v1).unwrap();
        assert_eq!(file.cache, CacheState::Warm);
        assert_eq!(file.measure_from, 0);
        assert_eq!(file.events.len(), 2);
        // v1에는 v2 줄이 없습니다.
        assert!(ReplayFile::parse(&format!("{v1}cache cold\n")).is_err());
        assert!(ReplayFile::parse(&format!("{v1}event 2 key z cmd\n")).is_err());
    }

    #[test]
    fn rejects_malformed_files() {
        let text = sample().to_text();
        for (broken, line) in [
            (text.replace("stellar-replay v2", "stellar-replay v3"), 1),
            (text.replace("event 16666666", "event 1"), 11),
            (text.replace("move 0.1", "move NaN"), 9),
            (text.replace("scenario drag", "scenario spin"), 5),
            (text.replace("visible-nodes", "my-project"), 2),
            (text.replace("cache cold", "cache hot"), 6),
            (text.replace("key z cmd+shift", "key z shift+cmd"), 12),
            (text.replace("key z cmd+shift", "key Z cmd"), 12),
            (text.replace("text d55c", "text 0d55c"), 13),
            (text.replace("text d55c", "text a"), 13),
            (format!("{text}viewport 1 1\n"), 15),
        ] {
            let err = ReplayFile::parse(&broken).unwrap_err();
            assert_eq!(err.line, line, "{err}");
        }
        let no_hash: String = text
            .lines()
            .filter(|l| !l.starts_with("fixture-hash"))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(ReplayFile::parse(&no_hash).unwrap_err().line, 0);
        let no_cache: String = text
            .lines()
            .filter(|l| !l.starts_with("cache"))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(ReplayFile::parse(&no_cache).unwrap_err().line, 0);
        let too_far = text.replace("measure-from 1", "measure-from 8");
        assert_eq!(ReplayFile::parse(&too_far).unwrap_err().line, 0);
    }
}
