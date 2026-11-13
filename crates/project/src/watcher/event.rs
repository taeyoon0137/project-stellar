//! 플랫폼 독립 파일 감시 event 모델 (XIII.2.1).
//!
//! 플랫폼 adapter(macOS FSEvents 등)는 OS 알림을 다섯 종류의 [`FileWatchEvent`]로 normalize합니다:
//! 생성(`Created`), 수정(`Modified`), 삭제(`Removed`), 이름 변경(`Renamed`), 넘침(`Overflow`).
//!
//! - 경로는 [`WatchPath`] = (감시 root [`WatchRootId`], root 기준 상대 경로)입니다. root 0은 프로젝트
//!   root이고 1 이상은 외부 symlink target 감시 root입니다. 상대 경로는 프로젝트 경로 규칙
//!   (`/` 구분, 빈·`.`·`..` 컴포넌트 금지)을 따르며 빈 문자열은 root 자신입니다.
//! - `Overflow`는 OS가 알림을 잃었거나(커널·사용자 큐 넘침) 정확한 변경 목록을 줄 수 없음을
//!   뜻합니다. 수신자는 상태를 clean으로 가정하지 않고 다시 검사해야 합니다(XIII.2.6).
//!
//! canonical 인코딩(로그·trace·FFI 전달용, storage canonical 규칙 R1~R3 사용):
//!
//! ```text
//! batch   = u8 version(=1) · u32 LE event 수 · event*
//! event   = u8 tag · body
//!   tag 1 Created  / 2 Modified / 3 Removed : path
//!   tag 4 Renamed                            : path(from) · path(to)
//!   tag 5 Overflow                           : u8 reason
//! path    = u32 LE root · u32 LE 길이 · UTF-8 상대 경로
//! ```
//!
//! decoder는 미지원 version·알 수 없는 tag/reason·잘린 입력·남는 bytes·UTF-8 오류·규칙 위반 경로를
//! 모두 거부합니다.

use std::fmt;

use stellar_storage::canonical::decode::{CanonicalDecoder, DecodeError};
use stellar_storage::canonical::encode::{CanonicalEncoder, EncodeError};

use crate::path_resolver::{split_project_path, InvalidPathReason};

/// event batch 인코딩 version.
pub const WATCH_EVENT_FORMAT_VERSION: u8 = 1;

/// 감시 root 식별자. [`WatchRootId::PROJECT`](0)는 프로젝트 root, 1 이상은 외부 symlink target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WatchRootId(pub u32);

impl WatchRootId {
    /// 프로젝트 root.
    pub const PROJECT: WatchRootId = WatchRootId(0);

    /// 프로젝트 root인지.
    pub fn is_project(self) -> bool {
        self == WatchRootId::PROJECT
    }
}

/// 감시 root 기준 경로.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WatchPath {
    root: WatchRootId,
    relative: String,
}

impl WatchPath {
    /// 경로를 만듭니다. `relative`는 빈 문자열(root 자신)이거나 프로젝트 경로 규칙을 따라야 합니다.
    pub fn new(
        root: WatchRootId,
        relative: impl Into<String>,
    ) -> Result<WatchPath, InvalidPathReason> {
        let relative = relative.into();
        if !relative.is_empty() {
            split_project_path(&relative)?;
        }
        Ok(WatchPath { root, relative })
    }

    /// 프로젝트 root 기준 경로.
    pub fn project(relative: impl Into<String>) -> Result<WatchPath, InvalidPathReason> {
        WatchPath::new(WatchRootId::PROJECT, relative)
    }

    /// 감시 root.
    pub fn root(&self) -> WatchRootId {
        self.root
    }

    /// root 기준 상대 경로(`/` 구분, 빈 문자열은 root 자신).
    pub fn relative(&self) -> &str {
        &self.relative
    }

    /// root 자신을 가리키는지.
    pub fn is_root(&self) -> bool {
        self.relative.is_empty()
    }

    /// 이 경로가 `prefix`와 같거나 그 아래에 있는지(같은 root, 컴포넌트 단위 비교).
    pub fn starts_with(&self, prefix: &WatchPath) -> bool {
        if self.root != prefix.root {
            return false;
        }
        if prefix.relative.is_empty() || self.relative == prefix.relative {
            return true;
        }
        self.relative
            .strip_prefix(&prefix.relative)
            .is_some_and(|rest| rest.starts_with('/'))
    }
}

impl fmt::Display for WatchPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}:{}", self.root.0, self.relative)
    }
}

/// 넘침 사유.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverflowReason {
    /// OS(커널·감시 서비스)가 알림을 버렸음.
    EventsDropped,
    /// OS가 정확한 변경 대신 하위 트리 재검사를 요구함.
    RescanRequired,
    /// 감시 root 자체가 이동·삭제·교체됨.
    RootChanged,
    /// adapter 내부 대기열 상한을 넘어 event를 버렸음.
    QueueFull,
    /// 경로를 감시 root 기준 상대 경로로 표현할 수 없음(UTF-8 아님·root 밖 등).
    UnrepresentablePath,
}

impl OverflowReason {
    /// 모든 사유(인코딩 순서).
    pub const ALL: [OverflowReason; 5] = [
        OverflowReason::EventsDropped,
        OverflowReason::RescanRequired,
        OverflowReason::RootChanged,
        OverflowReason::QueueFull,
        OverflowReason::UnrepresentablePath,
    ];

    /// 인코딩 코드.
    pub fn code(self) -> u8 {
        match self {
            OverflowReason::EventsDropped => 1,
            OverflowReason::RescanRequired => 2,
            OverflowReason::RootChanged => 3,
            OverflowReason::QueueFull => 4,
            OverflowReason::UnrepresentablePath => 5,
        }
    }

    /// 코드에서 사유를 찾습니다.
    pub fn from_code(code: u8) -> Option<OverflowReason> {
        OverflowReason::ALL.into_iter().find(|r| r.code() == code)
    }
}

/// event 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WatchEventKind {
    /// 생성.
    Created,
    /// 내용·metadata 수정.
    Modified,
    /// 삭제.
    Removed,
    /// 이름 변경·이동.
    Renamed,
    /// 넘침(변경을 정확히 알 수 없음).
    Overflow,
}

impl WatchEventKind {
    /// 인코딩 tag.
    pub fn tag(self) -> u8 {
        match self {
            WatchEventKind::Created => 1,
            WatchEventKind::Modified => 2,
            WatchEventKind::Removed => 3,
            WatchEventKind::Renamed => 4,
            WatchEventKind::Overflow => 5,
        }
    }
}

/// 플랫폼 독립 파일 감시 event.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FileWatchEvent {
    /// 항목 생성.
    Created(WatchPath),
    /// 항목 내용·metadata 수정.
    Modified(WatchPath),
    /// 항목 삭제.
    Removed(WatchPath),
    /// 이름 변경·이동(감시 root 안에서 짝을 알 수 있을 때).
    Renamed {
        /// 이전 경로.
        from: WatchPath,
        /// 새 경로.
        to: WatchPath,
    },
    /// 넘침: 정확한 변경 목록을 알 수 없음.
    Overflow(OverflowReason),
}

impl FileWatchEvent {
    /// event 종류.
    pub fn kind(&self) -> WatchEventKind {
        match self {
            FileWatchEvent::Created(_) => WatchEventKind::Created,
            FileWatchEvent::Modified(_) => WatchEventKind::Modified,
            FileWatchEvent::Removed(_) => WatchEventKind::Removed,
            FileWatchEvent::Renamed { .. } => WatchEventKind::Renamed,
            FileWatchEvent::Overflow(_) => WatchEventKind::Overflow,
        }
    }

    /// event가 언급하는 경로(이름 변경은 이전·새 경로, 넘침은 없음).
    pub fn paths(&self) -> Vec<&WatchPath> {
        match self {
            FileWatchEvent::Created(p)
            | FileWatchEvent::Modified(p)
            | FileWatchEvent::Removed(p) => {
                vec![p]
            }
            FileWatchEvent::Renamed { from, to } => vec![from, to],
            FileWatchEvent::Overflow(_) => Vec::new(),
        }
    }
}

/// event 인코딩·디코딩 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEventCodecError {
    /// canonical 인코딩 실패(길이 상한 등).
    Encode(EncodeError),
    /// canonical 디코딩 실패(잘린 입력·남는 bytes·UTF-8 오류 등).
    Decode(DecodeError),
    /// 지원하지 않는 batch version.
    UnsupportedVersion(u8),
    /// 알 수 없는 event tag.
    UnknownEventTag(u8),
    /// 알 수 없는 넘침 사유 코드.
    UnknownOverflowReason(u8),
    /// 경로 규칙 위반.
    InvalidPath {
        /// 디코딩한 경로 문자열.
        path: String,
        /// 사유.
        reason: InvalidPathReason,
    },
}

impl fmt::Display for WatchEventCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WatchEventCodecError::Encode(e) => write!(f, "감시 event 인코딩 실패: {e}"),
            WatchEventCodecError::Decode(e) => write!(f, "감시 event 디코딩 실패: {e}"),
            WatchEventCodecError::UnsupportedVersion(v) => {
                write!(f, "지원하지 않는 감시 event version: {v}")
            }
            WatchEventCodecError::UnknownEventTag(t) => write!(f, "알 수 없는 감시 event tag: {t}"),
            WatchEventCodecError::UnknownOverflowReason(r) => {
                write!(f, "알 수 없는 넘침 사유 코드: {r}")
            }
            WatchEventCodecError::InvalidPath { path, reason } => {
                write!(f, "잘못된 감시 경로 {path:?}: {reason}")
            }
        }
    }
}

impl std::error::Error for WatchEventCodecError {}

impl From<EncodeError> for WatchEventCodecError {
    fn from(e: EncodeError) -> WatchEventCodecError {
        WatchEventCodecError::Encode(e)
    }
}

impl From<DecodeError> for WatchEventCodecError {
    fn from(e: DecodeError) -> WatchEventCodecError {
        WatchEventCodecError::Decode(e)
    }
}

/// event batch를 canonical bytes로 인코딩합니다(입력 순서 보존).
pub fn encode_events(events: &[FileWatchEvent]) -> Result<Vec<u8>, WatchEventCodecError> {
    let mut enc = CanonicalEncoder::new();
    enc.put_u8(WATCH_EVENT_FORMAT_VERSION);
    let count = u32::try_from(events.len())
        .map_err(|_| EncodeError::LengthOverflow { len: events.len() })?;
    enc.put_u32(count);
    for event in events {
        enc.put_u8(event.kind().tag());
        match event {
            FileWatchEvent::Created(p)
            | FileWatchEvent::Modified(p)
            | FileWatchEvent::Removed(p) => {
                put_path(&mut enc, p)?;
            }
            FileWatchEvent::Renamed { from, to } => {
                put_path(&mut enc, from)?;
                put_path(&mut enc, to)?;
            }
            FileWatchEvent::Overflow(reason) => enc.put_u8(reason.code()),
        }
    }
    Ok(enc.into_bytes())
}

/// canonical bytes에서 event batch를 디코딩합니다.
pub fn decode_events(bytes: &[u8]) -> Result<Vec<FileWatchEvent>, WatchEventCodecError> {
    let mut dec = CanonicalDecoder::new(bytes);
    let version = dec.take_u8()?;
    if version != WATCH_EVENT_FORMAT_VERSION {
        return Err(WatchEventCodecError::UnsupportedVersion(version));
    }
    let count = dec.take_u32()?;
    // event 하나는 최소 2 bytes(tag + 사유)이므로 남은 입력으로 상한을 잡아 과할당을 막습니다.
    let mut events = Vec::with_capacity((count as usize).min(dec.remaining() / 2));
    for _ in 0..count {
        let tag = dec.take_u8()?;
        let event = match tag {
            1 => FileWatchEvent::Created(take_path(&mut dec)?),
            2 => FileWatchEvent::Modified(take_path(&mut dec)?),
            3 => FileWatchEvent::Removed(take_path(&mut dec)?),
            4 => FileWatchEvent::Renamed {
                from: take_path(&mut dec)?,
                to: take_path(&mut dec)?,
            },
            5 => {
                let code = dec.take_u8()?;
                FileWatchEvent::Overflow(
                    OverflowReason::from_code(code)
                        .ok_or(WatchEventCodecError::UnknownOverflowReason(code))?,
                )
            }
            other => return Err(WatchEventCodecError::UnknownEventTag(other)),
        };
        events.push(event);
    }
    dec.expect_end()?;
    Ok(events)
}

fn put_path(enc: &mut CanonicalEncoder, path: &WatchPath) -> Result<(), WatchEventCodecError> {
    enc.put_u32(path.root.0);
    enc.put_string(&path.relative)?;
    Ok(())
}

fn take_path(dec: &mut CanonicalDecoder<'_>) -> Result<WatchPath, WatchEventCodecError> {
    let root = WatchRootId(dec.take_u32()?);
    let relative = dec.take_string()?;
    WatchPath::new(root, relative).map_err(|reason| WatchEventCodecError::InvalidPath {
        path: relative.to_string(),
        reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(root: u32, relative: &str) -> WatchPath {
        WatchPath::new(WatchRootId(root), relative).unwrap()
    }

    /// 모든 종류·사유와 까다로운 경로를 포함한 batch.
    fn sample_events() -> Vec<FileWatchEvent> {
        let mut events = vec![
            FileWatchEvent::Created(path(0, "design/generations/a.starmanifest")),
            FileWatchEvent::Modified(path(0, "design/current")),
            FileWatchEvent::Removed(path(0, "assets/old.png")),
            FileWatchEvent::Renamed {
                from: path(0, "design/.current.tmp-1-0"),
                to: path(0, "design/current"),
            },
            FileWatchEvent::Modified(path(0, "")),
            FileWatchEvent::Created(path(3, "외부/한글 이름.txt")),
            FileWatchEvent::Renamed {
                from: path(1, "a"),
                to: path(2, "b/c"),
            },
        ];
        events.extend(OverflowReason::ALL.map(FileWatchEvent::Overflow));
        events
    }

    #[test]
    fn event_batch_round_trips() {
        let events = sample_events();
        let bytes = encode_events(&events).unwrap();
        assert_eq!(decode_events(&bytes).unwrap(), events);
        // 결정론: 같은 입력은 같은 bytes.
        assert_eq!(encode_events(&events).unwrap(), bytes);
        // 빈 batch.
        let empty = encode_events(&[]).unwrap();
        assert_eq!(empty, vec![1, 0, 0, 0, 0]);
        assert_eq!(decode_events(&empty).unwrap(), Vec::new());
    }

    #[test]
    fn encoding_matches_golden_bytes() {
        let events = vec![
            FileWatchEvent::Modified(path(0, "a/b")),
            FileWatchEvent::Renamed {
                from: path(1, "x"),
                to: path(1, ""),
            },
            FileWatchEvent::Overflow(OverflowReason::QueueFull),
        ];
        let expected: Vec<u8> = [
            &[1u8][..],    // version
            &[3, 0, 0, 0], // event 수
            &[2],          // Modified
            &[0, 0, 0, 0], // root 0
            &[3, 0, 0, 0], // 길이 3
            b"a/b",
            &[4],          // Renamed
            &[1, 0, 0, 0], // root 1
            &[1, 0, 0, 0],
            b"x",
            &[1, 0, 0, 0], // root 1
            &[0, 0, 0, 0], // 빈 경로(root 자신)
            &[5, 4],       // Overflow(QueueFull)
        ]
        .concat();
        assert_eq!(encode_events(&events).unwrap(), expected);
        assert_eq!(decode_events(&expected).unwrap(), events);
    }

    #[test]
    fn malformed_batches_are_rejected() {
        let bytes = encode_events(&sample_events()).unwrap();
        // 모든 잘린 접두사는 거부(panic 없음).
        for len in 0..bytes.len() {
            assert!(decode_events(&bytes[..len]).is_err(), "len {len}");
        }
        // 남는 bytes.
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(matches!(
            decode_events(&trailing),
            Err(WatchEventCodecError::Decode(
                DecodeError::TrailingBytes { .. }
            ))
        ));
        // 미지원 version, 알 수 없는 tag·사유.
        assert_eq!(
            decode_events(&[2, 0, 0, 0, 0]),
            Err(WatchEventCodecError::UnsupportedVersion(2))
        );
        assert_eq!(
            decode_events(&[1, 1, 0, 0, 0, 9]),
            Err(WatchEventCodecError::UnknownEventTag(9))
        );
        assert_eq!(
            decode_events(&[1, 1, 0, 0, 0, 5, 0]),
            Err(WatchEventCodecError::UnknownOverflowReason(0))
        );
        // 규칙 위반 경로와 UTF-8 오류.
        for (relative, reason) in [
            ("../x", InvalidPathReason::DotComponent),
            ("/abs", InvalidPathReason::Absolute),
            ("a//b", InvalidPathReason::EmptyComponent),
            ("a\\b", InvalidPathReason::Backslash),
        ] {
            let mut raw = vec![1, 1, 0, 0, 0, 1, 0, 0, 0, 0];
            raw.extend((relative.len() as u32).to_le_bytes());
            raw.extend(relative.as_bytes());
            assert_eq!(
                decode_events(&raw),
                Err(WatchEventCodecError::InvalidPath {
                    path: relative.to_string(),
                    reason
                })
            );
        }
        let bad_utf8 = [1, 1, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0xff];
        assert_eq!(
            decode_events(&bad_utf8),
            Err(WatchEventCodecError::Decode(DecodeError::InvalidUtf8))
        );
        // 큰 event 수를 주장하는 짧은 입력도 과할당 없이 거부.
        assert!(decode_events(&[1, 0xff, 0xff, 0xff, 0xff]).is_err());
    }

    #[test]
    fn watch_paths_follow_project_path_rules() {
        assert!(WatchPath::project("").unwrap().is_root());
        assert_eq!(
            WatchPath::project("a/../b"),
            Err(InvalidPathReason::DotComponent)
        );
        let dir = path(0, "design");
        assert!(path(0, "design/current").starts_with(&dir));
        assert!(path(0, "design").starts_with(&dir));
        assert!(!path(0, "designer/x").starts_with(&dir));
        assert!(!path(1, "design/x").starts_with(&dir));
        assert!(path(0, "anything").starts_with(&path(0, "")));
        assert_eq!(path(2, "a/b").to_string(), "#2:a/b");
    }

    #[test]
    fn event_kinds_and_paths() {
        let events = sample_events();
        let kinds: Vec<u8> = events.iter().map(|e| e.kind().tag()).collect();
        assert_eq!(kinds, vec![1, 2, 3, 4, 2, 1, 4, 5, 5, 5, 5, 5]);
        assert_eq!(events[3].paths().len(), 2);
        assert!(events[7].paths().is_empty());
        for reason in OverflowReason::ALL {
            assert_eq!(OverflowReason::from_code(reason.code()), Some(reason));
        }
        assert_eq!(OverflowReason::from_code(0), None);
    }
}
