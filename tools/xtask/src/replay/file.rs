//! 입력 재생 파일 형식 (XVI.2.1, ADR-0084 결정 9).
//!
//! 재생 파일은 프로젝트 밖에 두는 UTF-8 줄 단위 텍스트입니다. 사용자 문서를 가리키지 않고 synthetic fixture
//! 명세와 그 content hash만 담습니다.
//!
//! ```text
//! stellar-replay v1
//! fixture visible-nodes 1 20000
//! fixture-hash <64 hex>
//! viewport 1920 1080
//! scenario drag
//! event 0 down 950 545
//! event 8333333 move 954 548
//! event 16666666 up 958 551
//! ```
//!
//! - `fixture <이름> <seed> <규모>`: `stellar_bench_fixtures` 생성기 입력입니다. 재생할 때 다시 만든 fixture의
//!   content hash가 `fixture-hash`와 다르면 거부합니다.
//! - `viewport <너비> <높이>`: 창 logical px.
//! - `event <ns> <종류> <값…>`: ManualClock timeline 시각(ns, 감소 금지)과 공통 입력의 pointer·wheel 부분
//!   (`InputEvent`, XII.3.1과 같은 의미)입니다. 좌표는 view px이고 f32의 가장 짧은 왕복 표기입니다.
//!   종류는 `down x y`, `move x y`, `up x y`, `wheel dx dy`입니다.
//! - 빈 줄과 `#` 주석 줄은 무시합니다. 같은 값은 같은 텍스트로 씁니다(결정적 직렬화).

use std::fmt;
use std::fmt::Write as _;

use stellar_diagnostics::interaction::InteractionKind;
use stellar_diagnostics::result::FixtureName;

/// 파일 첫 줄.
pub const REPLAY_HEADER: &str = "stellar-replay v1";

/// event 수 상한(오입력 방지).
pub const MAX_REPLAY_EVENTS: usize = 1_000_000;

/// 재생 scenario. 각 scenario의 상호작용 종류와 도구가 정해집니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReplayScenario {
    /// Hand 도구 pan.
    Pan,
    /// 선택한 Frame 끌기.
    Drag,
    /// 선택한 Frame 오른쪽 아래 모서리 크기 조절.
    Resize,
}

impl ReplayScenario {
    /// 모든 표준 scenario.
    pub const ALL: [ReplayScenario; 3] = [
        ReplayScenario::Pan,
        ReplayScenario::Drag,
        ReplayScenario::Resize,
    ];

    /// 고정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            ReplayScenario::Pan => "pan",
            ReplayScenario::Drag => "drag",
            ReplayScenario::Resize => "resize",
        }
    }

    /// 이름에서 찾습니다.
    pub fn parse(text: &str) -> Option<ReplayScenario> {
        ReplayScenario::ALL.into_iter().find(|s| s.name() == text)
    }

    /// trace dimension의 상호작용 종류.
    pub const fn kind(self) -> InteractionKind {
        match self {
            ReplayScenario::Pan => InteractionKind::Pan,
            ReplayScenario::Drag => InteractionKind::Drag,
            ReplayScenario::Resize => InteractionKind::Resize,
        }
    }
}

/// 재생 입력 event 종류.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReplayInput {
    /// 주 버튼 누름(view px).
    Down {
        /// x.
        x: f32,
        /// y.
        y: f32,
    },
    /// 이동.
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
}

/// 재생 event.
#[derive(Debug, Clone, Copy, PartialEq)]
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

impl ReplayFile {
    /// 텍스트를 읽습니다.
    pub fn parse(text: &str) -> Result<ReplayFile, ReplayFileError> {
        let mut lines = text
            .lines()
            .enumerate()
            .map(|(n, l)| (n + 1, l.trim_end()))
            .filter(|(_, l)| !l.is_empty() && !l.starts_with('#'));
        match lines.next() {
            Some((_, header)) if header == REPLAY_HEADER => {}
            Some((n, _)) => return Err(error(n, format!("첫 줄은 {REPLAY_HEADER}여야 합니다"))),
            None => return Err(error(0, "빈 파일입니다")),
        }
        let mut fixture = None;
        let mut fixture_hash = None;
        let mut viewport = None;
        let mut scenario = None;
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
                ["event", at, kind, a, b] => {
                    let at_nanos = parse_u64(n, at)?;
                    if events.last().is_some_and(|e| e.at_nanos > at_nanos) {
                        return Err(error(n, "event 시각이 감소합니다"));
                    }
                    if events.len() >= MAX_REPLAY_EVENTS {
                        return Err(error(n, "event가 너무 많습니다"));
                    }
                    let (a, b) = (parse_f32(n, a)?, parse_f32(n, b)?);
                    let input = match *kind {
                        "down" => ReplayInput::Down { x: a, y: b },
                        "move" => ReplayInput::Move { x: a, y: b },
                        "up" => ReplayInput::Up { x: a, y: b },
                        "wheel" => ReplayInput::Wheel { dx: a, dy: b },
                        other => return Err(error(n, format!("알 수 없는 event: {other}"))),
                    };
                    events.push(ReplayEvent { at_nanos, input });
                }
                _ => return Err(error(n, format!("알 수 없거나 중복된 줄: {line}"))),
            }
        }
        Ok(ReplayFile {
            fixture: fixture.ok_or_else(|| error(0, "fixture 줄이 없습니다"))?,
            fixture_hash: fixture_hash.ok_or_else(|| error(0, "fixture-hash 줄이 없습니다"))?,
            viewport: viewport.ok_or_else(|| error(0, "viewport 줄이 없습니다"))?,
            scenario: scenario.ok_or_else(|| error(0, "scenario 줄이 없습니다"))?,
            events,
        })
    }

    /// 결정적 텍스트.
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
        for event in &self.events {
            let (kind, a, b) = match event.input {
                ReplayInput::Down { x, y } => ("down", x, y),
                ReplayInput::Move { x, y } => ("move", x, y),
                ReplayInput::Up { x, y } => ("up", x, y),
                ReplayInput::Wheel { dx, dy } => ("wheel", dx, dy),
            };
            let _ = writeln!(out, "event {} {kind} {a} {b}", event.at_nanos);
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
        assert!(text.starts_with("stellar-replay v1\nfixture visible-nodes 7 20000\n"));
        assert!(text.contains("event 8333333 move 0.1 -3.000001\n"));
    }

    #[test]
    fn rejects_malformed_files() {
        let text = sample().to_text();
        for (broken, line) in [
            (text.replace("stellar-replay v1", "stellar-replay v2"), 1),
            (text.replace("event 16666666", "event 1"), 9),
            (text.replace("move 0.1", "move NaN"), 7),
            (text.replace("scenario drag", "scenario spin"), 5),
            (text.replace("visible-nodes", "my-project"), 2),
            (format!("{text}viewport 1 1\n"), 10),
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
    }
}
