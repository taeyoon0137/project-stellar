//! 텔레메트리 동의 상태 (XVI.8.3, ADR-0109 결정 5).
//!
//! 상태는 [`ConsentState::NotAsked`](기본), [`ConsentState::Declined`], [`ConsentState::Granted`]입니다. 자동 전송은
//! `Granted`에서만 허용합니다([`ConsentState::allows_automatic_upload`]). 전이 규칙은 [`transition`] 하나이고,
//! `consent_transitions.txt` fixture가 모든 (상태, 사건) 쌍을 고정합니다.
//!
//! 동의 상태는 운영체제 사용자 데이터의 진단 디렉터리 아래 [`CONSENT_FILE`]에 둡니다. 프로젝트 파일·Git에는 두지
//! 않습니다. 파일이 없으면 `NotAsked`이고, 손상됐거나 읽지 못하면 `NotAsked`로 읽고 그 사실을 알립니다(닫힘:
//! 동의를 추정하지 않으므로 전송하지 않음).
//!
//! ```text
//! stellar.telemetry-consent v1
//! consent state=granted changed_ms=1763856000000
//! ```

use std::fmt;
use std::io;
use std::path::Path;

use crate::store::{
    checked_text, decode_line, encode_line, read_limited, strip_final_newline, write_private_file,
    FieldSpec, FieldType, FieldValue, RecordSchema, StoreError,
};

/// 동의 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConsentState {
    /// 아직 묻지 않음(기본).
    #[default]
    NotAsked,
    /// 거절(철회 포함).
    Declined,
    /// 동의.
    Granted,
}

impl ConsentState {
    /// 모든 상태.
    pub const ALL: [ConsentState; 3] = [
        ConsentState::NotAsked,
        ConsentState::Declined,
        ConsentState::Granted,
    ];

    /// 저장 이름 전체([`Self::ALL`] 순서).
    pub const NAMES: [&'static str; 3] = ["not_asked", "declined", "granted"];

    /// 저장 이름.
    pub const fn name(self) -> &'static str {
        match self {
            ConsentState::NotAsked => "not_asked",
            ConsentState::Declined => "declined",
            ConsentState::Granted => "granted",
        }
    }

    /// 저장 이름을 해석합니다.
    pub fn from_name(text: &str) -> Option<ConsentState> {
        ConsentState::ALL.into_iter().find(|s| s.name() == text)
    }

    /// 자동 전송을 허용하는지. `Granted`만 허용합니다.
    pub const fn allows_automatic_upload(self) -> bool {
        matches!(self, ConsentState::Granted)
    }
}

/// 사용자가 일으키는 사건.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConsentEvent {
    /// 동의.
    Grant,
    /// 거절.
    Decline,
    /// 동의 철회.
    Revoke,
}

impl ConsentEvent {
    /// 모든 사건.
    pub const ALL: [ConsentEvent; 3] = [
        ConsentEvent::Grant,
        ConsentEvent::Decline,
        ConsentEvent::Revoke,
    ];

    /// 고정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            ConsentEvent::Grant => "grant",
            ConsentEvent::Decline => "decline",
            ConsentEvent::Revoke => "revoke",
        }
    }

    /// 이름을 해석합니다.
    pub fn from_name(text: &str) -> Option<ConsentEvent> {
        ConsentEvent::ALL.into_iter().find(|e| e.name() == text)
    }
}

/// 전이 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionError {
    /// 동의하지 않은 상태에서 철회했습니다.
    NothingToRevoke,
}

impl TransitionError {
    /// 고정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            TransitionError::NothingToRevoke => "nothing_to_revoke",
        }
    }
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransitionError::NothingToRevoke => write!(f, "철회할 동의가 없습니다"),
        }
    }
}

impl std::error::Error for TransitionError {}

/// 전이 규칙(하나뿐인 경로).
pub const fn transition(
    state: ConsentState,
    event: ConsentEvent,
) -> Result<ConsentState, TransitionError> {
    match (state, event) {
        (_, ConsentEvent::Grant) => Ok(ConsentState::Granted),
        (_, ConsentEvent::Decline) => Ok(ConsentState::Declined),
        (ConsentState::Granted, ConsentEvent::Revoke) => Ok(ConsentState::Declined),
        (ConsentState::NotAsked | ConsentState::Declined, ConsentEvent::Revoke) => {
            Err(TransitionError::NothingToRevoke)
        }
    }
}

/// 적용한 전이.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    /// 이전 상태.
    pub from: ConsentState,
    /// 사건.
    pub event: ConsentEvent,
    /// 새 상태.
    pub to: ConsentState,
}

impl Transition {
    /// 상태가 바뀌었는지.
    pub fn changed(&self) -> bool {
        self.from != self.to
    }

    /// `Granted`를 떠났는지(철회·거절). 이때 전송 대기열을 버립니다(XVI.8.5).
    pub fn leaves_granted(&self) -> bool {
        self.from == ConsentState::Granted && self.to != ConsentState::Granted
    }
}

/// 동의 파일 첫 줄.
pub const CONSENT_HEADER: &str = "stellar.telemetry-consent v1";

/// 동의 파일 이름(진단 디렉터리 아래).
pub const CONSENT_FILE: &str = "consent";

/// 동의 record 줄.
pub const CONSENT_SCHEMA: RecordSchema = RecordSchema {
    tag: "consent",
    fields: &[
        FieldSpec {
            name: "state",
            ty: FieldType::Name(&ConsentState::NAMES),
        },
        FieldSpec {
            name: "changed_ms",
            ty: FieldType::U64,
        },
    ],
};

/// 저장하는 동의 상태와 마지막 변경 시각.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConsentRecord {
    /// 상태.
    pub state: ConsentState,
    /// 마지막 변경 시각(Unix epoch ms, 한 번도 바뀌지 않았으면 0).
    pub changed_ms: u64,
}

impl ConsentRecord {
    /// 사건을 적용합니다. 상태가 바뀔 때만 시각을 갱신합니다.
    pub fn apply(
        &mut self,
        event: ConsentEvent,
        at_ms: u64,
    ) -> Result<Transition, TransitionError> {
        let to = transition(self.state, event)?;
        let applied = Transition {
            from: self.state,
            event,
            to,
        };
        if applied.changed() {
            self.state = to;
            self.changed_ms = at_ms;
        }
        Ok(applied)
    }

    /// 파일 bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        format!(
            "{CONSENT_HEADER}\n{}\n",
            encode_line(
                &CONSENT_SCHEMA,
                &[
                    FieldValue::Name(self.state.name()),
                    FieldValue::Int(self.changed_ms)
                ]
            )
        )
        .into_bytes()
    }

    /// 파일 bytes를 해석합니다.
    pub fn from_bytes(bytes: &[u8]) -> Result<ConsentRecord, StoreError> {
        let text = checked_text(bytes)?;
        let pieces: Vec<&str> = text.split('\n').collect();
        let lines = strip_final_newline(&pieces, pieces.len())?;
        let malformed = |line| {
            StoreError::Line(crate::store::LineError {
                line,
                kind: crate::store::LineErrorKind::Header,
            })
        };
        match lines {
            [header, line] if *header == CONSENT_HEADER => {
                let (_, values) = decode_line(&[&CONSENT_SCHEMA], line, 2)?;
                let state = match values[0] {
                    FieldValue::Name(name) => ConsentState::from_name(name),
                    _ => None,
                }
                .ok_or_else(|| malformed(2))?;
                let changed_ms = match values[1] {
                    FieldValue::Int(n) => n,
                    _ => 0,
                };
                Ok(ConsentRecord { state, changed_ms })
            }
            _ => Err(malformed(1)),
        }
    }
}

/// 동의 파일을 읽은 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentLoad {
    /// 파일이 없습니다(`NotAsked`).
    Missing,
    /// 읽었습니다.
    Loaded(ConsentRecord),
    /// 손상됐거나 읽지 못했습니다(`NotAsked`로 닫힘).
    Unreadable,
}

impl ConsentLoad {
    /// 쓸 동의 record. 읽지 못했으면 기본값(`NotAsked`)입니다.
    pub fn record(self) -> ConsentRecord {
        match self {
            ConsentLoad::Loaded(record) => record,
            ConsentLoad::Missing | ConsentLoad::Unreadable => ConsentRecord::default(),
        }
    }
}

/// 동의 파일을 읽습니다.
pub fn read_consent(path: &Path) -> ConsentLoad {
    match read_limited(path) {
        Ok(bytes) => {
            ConsentRecord::from_bytes(&bytes).map_or(ConsentLoad::Unreadable, ConsentLoad::Loaded)
        }
        Err(StoreError::Io(io::ErrorKind::NotFound)) => ConsentLoad::Missing,
        Err(_) => ConsentLoad::Unreadable,
    }
}

/// 동의 파일을 소유자 전용 권한으로 원자적으로 씁니다.
pub fn write_consent(path: &Path, record: &ConsentRecord) -> io::Result<()> {
    write_private_file(path, &record.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("consent_transitions.txt");

    fn expected(rest: &str) -> Result<(ConsentState, bool, bool), String> {
        if let Some(error) = rest.strip_prefix("error:") {
            return Err(error.to_owned());
        }
        let parts: Vec<&str> = rest.split(' ').collect();
        let [state, change, upload] = parts[..] else {
            panic!("fixture 형식: {rest}");
        };
        Ok((
            ConsentState::from_name(state).unwrap(),
            match change {
                "changed" => true,
                "unchanged" => false,
                other => panic!("{other}"),
            },
            match upload {
                "upload:allowed" => true,
                "upload:blocked" => false,
                other => panic!("{other}"),
            },
        ))
    }

    #[test]
    fn default_is_not_asked_and_blocks_upload() {
        assert_eq!(ConsentState::default(), ConsentState::NotAsked);
        assert_eq!(ConsentRecord::default().state, ConsentState::NotAsked);
        for state in ConsentState::ALL {
            assert_eq!(
                state.allows_automatic_upload(),
                state == ConsentState::Granted
            );
        }
    }

    #[test]
    fn transitions_match_the_fixture() {
        // 검증 기준(XVI.8.3): state transition fixture.
        let mut seen = std::collections::BTreeSet::new();
        for line in FIXTURE
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let (pair, rest) = line.split_once(" -> ").unwrap();
            let (state, event) = pair.split_once(' ').unwrap();
            let state = ConsentState::from_name(state).unwrap();
            let event = ConsentEvent::from_name(event).unwrap();
            assert!(seen.insert((state.name(), event.name())), "{line}");
            let mut record = ConsentRecord {
                state,
                changed_ms: 5,
            };
            match (record.apply(event, 9), expected(rest)) {
                (Ok(applied), Ok((to, changed, upload))) => {
                    assert_eq!(applied.to, to, "{line}");
                    assert_eq!(applied.changed(), changed, "{line}");
                    assert_eq!(record.state.allows_automatic_upload(), upload, "{line}");
                    assert_eq!(record.changed_ms, if changed { 9 } else { 5 }, "{line}");
                    assert_eq!(
                        applied.leaves_granted(),
                        state == ConsentState::Granted && to != ConsentState::Granted
                    );
                }
                (Err(e), Err(name)) => {
                    assert_eq!(e.name(), name, "{line}");
                    // 실패한 전이는 상태를 바꾸지 않습니다.
                    assert_eq!(record.state, state);
                    assert_eq!(record.changed_ms, 5);
                }
                (actual, expected) => panic!("{line}: {actual:?} != {expected:?}"),
            }
        }
        assert_eq!(
            seen.len(),
            ConsentState::ALL.len() * ConsentEvent::ALL.len()
        );
    }

    #[test]
    fn upload_is_allowed_only_after_grant_in_any_sequence() {
        // 모든 길이 4 사건 열에서 자동 전송 허용은 마지막 성공 전이가 Grant일 때뿐입니다.
        let events = ConsentEvent::ALL;
        for a in events {
            for b in events {
                for c in events {
                    for d in events {
                        let mut record = ConsentRecord::default();
                        let mut last_ok = None;
                        for event in [a, b, c, d] {
                            if record.apply(event, 1).is_ok() {
                                last_ok = Some(event);
                            }
                        }
                        assert_eq!(
                            record.state.allows_automatic_upload(),
                            last_ok == Some(ConsentEvent::Grant),
                            "{a:?} {b:?} {c:?} {d:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn consent_file_round_trips_and_fails_closed() {
        let dir = std::env::temp_dir().join(format!("stellar-consent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(CONSENT_FILE);
        assert_eq!(read_consent(&path), ConsentLoad::Missing);
        assert_eq!(read_consent(&path).record().state, ConsentState::NotAsked);
        let record = ConsentRecord {
            state: ConsentState::Granted,
            changed_ms: 1_763_856_000_000,
        };
        write_consent(&path, &record).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "stellar.telemetry-consent v1\nconsent state=granted changed_ms=1763856000000\n"
        );
        assert_eq!(read_consent(&path), ConsentLoad::Loaded(record));
        for corrupt in [
            &b"stellar.telemetry-consent v1\nconsent state=yes changed_ms=1\n"[..],
            b"stellar.telemetry-consent v1\nconsent state=granted changed_ms=1",
            b"stellar.telemetry-consent v2\nconsent state=granted changed_ms=1\n",
            b"stellar.telemetry-consent v1\nconsent state=granted changed_ms=1\nconsent state=granted changed_ms=1\n",
            b"granted\n",
            b"",
            &[0xff, 0x00],
        ] {
            std::fs::write(&path, corrupt).unwrap();
            assert_eq!(read_consent(&path), ConsentLoad::Unreadable);
            // 손상된 동의는 전송을 허용하지 않습니다.
            assert!(!read_consent(&path).record().state.allows_automatic_upload());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
