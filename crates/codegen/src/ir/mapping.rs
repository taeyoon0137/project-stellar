//! node별 mapping 진단 (XXII.1.2, ADR-0106 결정 2).
//!
//! IR node가 출력에 쓰는 대상마다 Native를 포함한 capability 상태를 남깁니다. 출처는 IX.10.8 validator 진단과
//! CSS 기반 속성 사용을 [`crate::matrix`]로 판정한 값입니다. 순서는 (출처, subject, path) 오름차순입니다.

use stellar_core_types::property::PropertyPath;
use stellar_target_profile::capability::{CapabilityStatus, CapabilitySubject};
use stellar_target_profile::css_matrix::{MappingInput, RegistryInput};
use stellar_target_profile::profile::TargetProfileKind;
use stellar_target_profile::validate::{CapabilityDiagnosticCode, CapabilityFinding};

use crate::matrix::css_entry;

/// node가 쓰는 CSS 기반 속성 registry 항목 하나(문서 registry를 읽는 상위가 채움).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssPropertyUse {
    /// registry 범위 이름(`node`, `frame`, …, `modifier`).
    pub scope: &'static str,
    /// registry 이름(CSS longhand 또는 Stellar 이름).
    pub name: &'static str,
    /// registry CSS 매핑 상태.
    pub status: MappingInput,
    /// 값이 있는 속성 경로(layout longhand처럼 경로가 없는 항목은 `None`).
    pub path: Option<PropertyPath>,
}

/// mapping 진단 출처.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MappingSource {
    /// IX.10.8 capability validator.
    Validator,
    /// 코드 생성 matrix(CSS 기반 속성).
    Matrix,
}

impl MappingSource {
    /// 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            MappingSource::Validator => "validator",
            MappingSource::Matrix => "matrix",
        }
    }
}

/// mapping 대상.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MappingSubject {
    /// capability registry subject(element·contract·속성).
    Capability(CapabilitySubject),
    /// CSS 기반 속성 registry 항목.
    Css {
        /// 범위.
        scope: &'static str,
        /// 이름.
        name: &'static str,
    },
}

/// node 하나의 mapping 진단.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrMapping {
    /// 출처.
    pub source: MappingSource,
    /// 대상.
    pub subject: MappingSubject,
    /// 속성 경로(있으면).
    pub path: Option<PropertyPath>,
    /// 상태(`None` = 등록되지 않음).
    pub status: Option<CapabilityStatus>,
    /// lowering 규칙 id.
    pub rule: &'static str,
    /// 짝 진단 code(1:1이면 `None`).
    pub diagnostic: Option<&'static str>,
    /// Deprecated의 대체 subject.
    pub replacement: Option<CapabilitySubject>,
}

/// validator 진단 code의 안정 이름.
pub const fn diagnostic_name(code: CapabilityDiagnosticCode) -> &'static str {
    match code {
        CapabilityDiagnosticCode::Native => "capability-native",
        CapabilityDiagnosticCode::Approximated => "capability-approximated",
        CapabilityDiagnosticCode::Unsupported => "capability-unsupported",
        CapabilityDiagnosticCode::Deprecated => "capability-deprecated",
        CapabilityDiagnosticCode::NotRegistered => "capability-not-registered",
    }
}

/// validator 진단 → mapping.
pub(crate) fn from_finding(finding: &CapabilityFinding) -> IrMapping {
    IrMapping {
        source: MappingSource::Validator,
        subject: MappingSubject::Capability(finding.subject),
        path: finding.path.clone(),
        status: finding.status,
        rule: "capability-registry",
        diagnostic: Some(diagnostic_name(finding.code)),
        replacement: finding.replacement,
    }
}

/// CSS 기반 속성 사용 → mapping(target profile kind로 판정).
pub(crate) fn from_css_use(css: &CssPropertyUse, kind: TargetProfileKind) -> IrMapping {
    let entry = css_entry(
        RegistryInput {
            scope: css.scope,
            name: css.name,
            status: css.status,
        },
        kind,
    );
    IrMapping {
        source: MappingSource::Matrix,
        subject: MappingSubject::Css {
            scope: css.scope,
            name: css.name,
        },
        path: css.path.clone(),
        status: entry.status,
        rule: entry.rule,
        diagnostic: entry.diagnostic,
        replacement: None,
    }
}

/// 결정론 순서로 정렬하고 같은 (출처, subject, path)를 하나만 남깁니다.
pub(crate) fn normalize(mappings: &mut Vec<IrMapping>) {
    mappings.sort_by(|a, b| (a.source, a.subject, &a.path).cmp(&(b.source, b.subject, &b.path)));
    mappings.dedup_by(|a, b| a.source == b.source && a.subject == b.subject && a.path == b.path);
}
