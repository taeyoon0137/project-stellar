//! 무결성 진단 한 건의 공통 형태 (XXV.2.1, XXV.2.2).
//!
//! 생산자마다 진단 타입이 다르므로([`crate::sources`]가 변환), 집계·필터·pagination·remediation은 이 형태 하나로
//! 다룹니다. 진단에는 사용자 텍스트 내용·파일 경로·repository URL을 넣지 않습니다(II.3.2 비민감 진단). 문자열 필드는
//! 생산자의 안정 code, 식별자, 생산자가 이미 비민감으로 보장한 설명만 담습니다.
//!
//! # 정체성과 순서
//!
//! [`DiagnosticKey`]가 진단의 정체성이자 전역 순서입니다: 문서(프로젝트 단위 진단은 문서 없음이 먼저) → 종류 →
//! source StableId → property path → 생산자 code → subject(생산자가 정한 구분자) → origin(생산자 이름). 같은 key의
//! 진단은 같은 진단으로 보고 하나만 둡니다. pagination cursor와 [`DiagnosticId`]는 이 key에서 나옵니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;

use crate::remediation::RemediationActionId;

/// 진단 종류(XXV.2.1 목록). 숫자 값은 저장 schema([`crate::codec`])의 일부라 바꾸지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum DiagnosticKind {
    /// target 속성이나 source 참조가 사라져 끊어진 binding(VIII.8.1).
    OrphanedBinding = 1,
    /// 없는 token을 가리키는 참조.
    MissingToken = 2,
    /// 등록되지 않은 font face(X.8.4).
    MissingFont = 3,
    /// asset store에 없는 asset을 가리키는 Image(VI.8.2).
    MissingAsset = 4,
    /// 공용 cache에서 읽을 수 없는 library 또는 그 resource를 가리키는 참조(XV.5.5).
    MissingLibrary = 5,
    /// 문서 안에 없는 node를 가리키는 참조(clip·mask·boolean operand, III.2.4).
    MissingNode = 6,
    /// 없는 컴포넌트 정의를 가리키는 참조.
    MissingComponent = 7,
    /// 접근성 의미 문제(IX.8.5).
    InvalidAccessibility = 8,
    /// Target Profile이 지원하지 않거나 폐기·미등록인 element·contract·속성(IX.10.8).
    TargetUnsupported = 9,
    /// 낮은 schema generation을 메모리에서 migration해 읽기 전용으로 연 상태(IX.11.21).
    MigrationWarning = 10,
}

impl DiagnosticKind {
    /// 모든 종류(숫자 순).
    pub const ALL: [DiagnosticKind; 10] = [
        DiagnosticKind::OrphanedBinding,
        DiagnosticKind::MissingToken,
        DiagnosticKind::MissingFont,
        DiagnosticKind::MissingAsset,
        DiagnosticKind::MissingLibrary,
        DiagnosticKind::MissingNode,
        DiagnosticKind::MissingComponent,
        DiagnosticKind::InvalidAccessibility,
        DiagnosticKind::TargetUnsupported,
        DiagnosticKind::MigrationWarning,
    ];

    /// 안정 이름(CLI·MCP 필터와 출력).
    pub const fn name(self) -> &'static str {
        match self {
            DiagnosticKind::OrphanedBinding => "orphaned-binding",
            DiagnosticKind::MissingToken => "missing-token",
            DiagnosticKind::MissingFont => "missing-font",
            DiagnosticKind::MissingAsset => "missing-asset",
            DiagnosticKind::MissingLibrary => "missing-library",
            DiagnosticKind::MissingNode => "missing-node",
            DiagnosticKind::MissingComponent => "missing-component",
            DiagnosticKind::InvalidAccessibility => "invalid-accessibility",
            DiagnosticKind::TargetUnsupported => "target-unsupported",
            DiagnosticKind::MigrationWarning => "migration-warning",
        }
    }

    /// 이름에서 종류를 찾습니다.
    pub fn from_name(name: &str) -> Option<DiagnosticKind> {
        DiagnosticKind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// 저장 schema 숫자 값.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// 숫자 값에서 종류를 찾습니다.
    pub fn from_code(code: u8) -> Option<DiagnosticKind> {
        DiagnosticKind::ALL.into_iter().find(|k| k.code() == code)
    }
}

impl fmt::Display for DiagnosticKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 심각도. 순서는 Info < Warning < Error입니다(최소 심각도 필터).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Severity {
    /// 정보(문제는 아니지만 알릴 것).
    Info = 1,
    /// 경고(문서는 열리고 동작하지만 결과가 의도와 다를 수 있음).
    Warning = 2,
    /// 오류(결과가 틀리거나 target에서 표현할 수 없음).
    Error = 3,
}

impl Severity {
    /// 모든 심각도(낮은 순).
    pub const ALL: [Severity; 3] = [Severity::Info, Severity::Warning, Severity::Error];

    /// 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }

    /// 이름에서 심각도를 찾습니다.
    pub fn from_name(name: &str) -> Option<Severity> {
        Severity::ALL.into_iter().find(|s| s.name() == name)
    }

    /// 저장 schema 숫자 값.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// 숫자 값에서 심각도를 찾습니다.
    pub fn from_code(code: u8) -> Option<Severity> {
        Severity::ALL.into_iter().find(|s| s.code() == code)
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 진단 정체성과 전역 순서(모듈 문서 참조).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DiagnosticKey {
    /// 대상 문서(프로젝트 단위 진단은 `None`).
    pub document: Option<StableId>,
    /// 종류.
    pub kind: DiagnosticKind,
    /// 원인 위치의 영속 객체(node·modifier·binding 등).
    pub source: Option<StableId>,
    /// 원인 속성 경로.
    pub property: Option<PropertyPath>,
    /// 생산자 안정 code.
    pub code: String,
    /// 생산자가 정한 구분자(같은 source·property의 서로 다른 진단을 나눔, 비민감).
    pub subject: String,
    /// 생산자 이름([`crate::incremental::SourceId`]).
    pub origin: String,
}

impl DiagnosticKey {
    /// key의 canonical byte 표현(순서와 무관한 정체성 hash·cursor용).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_key(&mut out, self);
        out
    }

    /// 안정 진단 ID.
    pub fn id(&self) -> DiagnosticId {
        DiagnosticId::of(&self.to_bytes())
    }
}

pub(crate) fn write_str(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u32).to_be_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn write_opt_id(out: &mut Vec<u8>, id: Option<StableId>) {
    match id {
        None => out.push(0),
        Some(id) => {
            out.push(1);
            out.extend_from_slice(&id.to_bytes());
        }
    }
}

/// key의 canonical bytes(진단 저장 schema v1의 key 부분과 같음, [`crate::codec`]).
pub(crate) fn write_key(out: &mut Vec<u8>, key: &DiagnosticKey) {
    write_opt_id(out, key.document);
    out.push(key.kind.code());
    write_opt_id(out, key.source);
    match &key.property {
        None => out.push(0),
        Some(path) => {
            out.push(1);
            out.extend_from_slice(&path.to_canonical_bytes());
        }
    }
    write_str(out, &key.code);
    write_str(out, &key.subject);
    write_str(out, &key.origin);
}

/// 진단의 안정 ID: [`DiagnosticKey`] canonical bytes의 FNV-1a 64bit hash. 같은 진단은 실행·프로세스와 무관하게
/// 같은 ID입니다. 표시·remediation 지정용이며 순서에는 쓰지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DiagnosticId(pub u64);

impl DiagnosticId {
    fn of(bytes: &[u8]) -> DiagnosticId {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        DiagnosticId(hash)
    }

    /// 16자리 소문자 hex에서 읽습니다.
    pub fn parse(text: &str) -> Option<DiagnosticId> {
        if text.len() != 16 {
            return None;
        }
        u64::from_str_radix(text, 16).ok().map(DiagnosticId)
    }
}

impl fmt::Display for DiagnosticId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// 무결성 진단 한 건(XXV.2.2: severity, source StableId, property path, remediation action ID 포함).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// 정체성.
    pub key: DiagnosticKey,
    /// 심각도.
    pub severity: Severity,
    /// 사람이 읽는 설명(비민감: 식별자·code·생산자 설명만).
    pub message: String,
    /// 이 진단에 제공하는 remediation action(중복 없음, inventory 순).
    pub remediation: Vec<RemediationActionId>,
}

impl Diagnostic {
    /// 안정 진단 ID.
    pub fn id(&self) -> DiagnosticId {
        self.key.id()
    }

    /// 종류.
    pub fn kind(&self) -> DiagnosticKind {
        self.key.kind
    }

    /// 대상 문서.
    pub fn document(&self) -> Option<StableId> {
        self.key.document
    }

    /// source StableId.
    pub fn source(&self) -> Option<StableId> {
        self.key.source
    }

    /// property path.
    pub fn property(&self) -> Option<&PropertyPath> {
        self.key.property.as_ref()
    }

    /// 생산자 code.
    pub fn code(&self) -> &str {
        &self.key.code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_and_severity_names_and_codes_are_unique_and_round_trip() {
        let mut names = std::collections::BTreeSet::new();
        for kind in DiagnosticKind::ALL {
            assert!(names.insert(kind.name()));
            assert_eq!(DiagnosticKind::from_name(kind.name()), Some(kind));
            assert_eq!(DiagnosticKind::from_code(kind.code()), Some(kind));
        }
        for severity in Severity::ALL {
            assert_eq!(Severity::from_name(severity.name()), Some(severity));
            assert_eq!(Severity::from_code(severity.code()), Some(severity));
        }
        assert!(Severity::Info < Severity::Warning && Severity::Warning < Severity::Error);
        assert_eq!(DiagnosticKind::from_code(0), None);
        assert_eq!(DiagnosticKind::from_name("missing"), None);
    }

    #[test]
    fn id_is_stable_hex() {
        let key = DiagnosticKey {
            document: None,
            kind: DiagnosticKind::MigrationWarning,
            source: None,
            property: None,
            code: "schema-migrated-in-memory".into(),
            subject: "3->4".into(),
            origin: "migration".into(),
        };
        let id = key.id();
        assert_eq!(id, key.clone().id());
        assert_eq!(DiagnosticId::parse(&id.to_string()), Some(id));
        assert_eq!(DiagnosticId::parse("xyz"), None);
    }
}
