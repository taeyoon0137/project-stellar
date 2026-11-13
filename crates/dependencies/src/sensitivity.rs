//! 의존성 선언 민감값 분류와 redaction (XV.1.4).
//!
//! 기술 아키텍처 39장은 파일 경로와 Git URL·브랜치·커밋 정보의 수집을 금지합니다. 의존성 선언
//! 필드를 세 등급으로 분류합니다.
//!
//! | 등급 | 필드 | 허용 범위 |
//! |---|---|---|
//! | [`DataClass::TelemetrySafe`] | source 종류, 선택자 종류, local override 유무 | 동의 시 telemetry |
//! | [`DataClass::LocalDiagnostic`] | logical name, library id, 선택자 값(tag·branch·commit) | 로컬 진단만 |
//! | [`DataClass::LocalSourceOnly`] | repository URL, local override 경로 | 로컬 UI 표시만, 진단 export에서도 가림 |
//!
//! [`RepositoryUrl`]·[`LocalOverridePath`]는 `Debug`에서 값을 가리고 `Display`를 구현하지 않으므로
//! 형식화 경로로 새지 않습니다. 원문은 `reveal()`로만 얻습니다.
//!
//! ```compile_fail
//! use stellar_dependencies::declaration::RepositoryUrl;
//! let url = RepositoryUrl::new("https://example.com/x.git").unwrap();
//! // Display가 없으므로 문자열 형식화에 쓸 수 없습니다.
//! let _ = format!("{url}");
//! ```

use std::fmt;

use crate::declaration::{
    DependencyDeclaration, LibraryId, LocalOverridePath, LogicalName, RepositoryUrl, SourceKind,
    VersionSelector,
};

/// redaction 표식.
pub const REDACTED: &str = "<redacted>";

/// 데이터 분류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataClass {
    /// 사용자 동의 시 telemetry로 보낼 수 있음(정적 이름·종류·불리언).
    TelemetrySafe,
    /// 로컬 진단에만 기록(telemetry 금지).
    LocalDiagnostic,
    /// 로컬 UI 표시에만 사용(telemetry 금지, 진단 export에서도 가림).
    LocalSourceOnly,
}

/// 의존성 선언 필드.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeclarationField {
    /// logical name.
    LogicalName,
    /// library id.
    LibraryId,
    /// source 종류.
    Source,
    /// repository URL.
    Repository,
    /// 선택자 종류(tag/branch/commit).
    SelectorKind,
    /// 선택자 값(tag·branch 이름, commit hash).
    SelectorValue,
    /// local override 유무.
    HasLocalOverride,
    /// local override 경로.
    LocalOverridePath,
}

impl DeclarationField {
    /// 모든 필드.
    pub const ALL: [DeclarationField; 8] = [
        DeclarationField::LogicalName,
        DeclarationField::LibraryId,
        DeclarationField::Source,
        DeclarationField::Repository,
        DeclarationField::SelectorKind,
        DeclarationField::SelectorValue,
        DeclarationField::HasLocalOverride,
        DeclarationField::LocalOverridePath,
    ];

    /// 데이터 분류.
    pub fn data_class(self) -> DataClass {
        match self {
            DeclarationField::Source
            | DeclarationField::SelectorKind
            | DeclarationField::HasLocalOverride => DataClass::TelemetrySafe,
            DeclarationField::LogicalName
            | DeclarationField::LibraryId
            | DeclarationField::SelectorValue => DataClass::LocalDiagnostic,
            DeclarationField::Repository | DeclarationField::LocalOverridePath => {
                DataClass::LocalSourceOnly
            }
        }
    }
}

impl fmt::Debug for RepositoryUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RepositoryUrl({REDACTED})")
    }
}

impl fmt::Debug for LocalOverridePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LocalOverridePath({REDACTED})")
    }
}

/// 로컬 진단 기록용 선언 요약. URL·경로는 담지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarationDiagnostic {
    /// logical name.
    pub name: LogicalName,
    /// library id.
    pub library_id: LibraryId,
    /// source 종류.
    pub source: SourceKind,
    /// 선택자.
    pub selector: VersionSelector,
    /// local override 유무.
    pub has_local_override: bool,
}

impl fmt::Display for DeclarationDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "name={} library={} source={} version={}:{} repository={REDACTED} local_override={}",
            self.name,
            self.library_id,
            self.source.as_str(),
            self.selector.kind().as_str(),
            self.selector.value(),
            if self.has_local_override {
                REDACTED
            } else {
                "none"
            }
        )
    }
}

/// telemetry로 보낼 수 있는 필드만 담은 요약(정적 문자열·불리언).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclarationTelemetry {
    /// source 종류 이름.
    pub source: &'static str,
    /// 선택자 종류 이름.
    pub selector_kind: &'static str,
    /// local override 유무.
    pub has_local_override: bool,
}

impl DependencyDeclaration {
    /// 로컬 진단 요약.
    pub fn diagnostic(&self) -> DeclarationDiagnostic {
        DeclarationDiagnostic {
            name: self.name.clone(),
            library_id: self.library_id,
            source: self.source,
            selector: self.version.clone(),
            has_local_override: self.local_override.is_some(),
        }
    }

    /// telemetry 요약.
    pub fn telemetry(&self) -> DeclarationTelemetry {
        DeclarationTelemetry {
            source: self.source.as_str(),
            selector_kind: self.version.kind().as_str(),
            has_local_override: self.local_override.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::fixtures::decl;
    use crate::declaration::RefName;

    const URL: &str = "git@private.example:team/secret-ds.git";
    const PATH: &str = "/Users/someone/work/secret-ds";

    fn sample() -> DependencyDeclaration {
        let mut d = decl("ds", 1, URL, "unused");
        d.version = VersionSelector::Branch(RefName::new("wip/private-branch").unwrap());
        d.local_override = Some(LocalOverridePath::new(PATH).unwrap());
        d
    }

    fn assert_no_source(text: &str) {
        for needle in [URL, PATH, "private.example", "secret-ds", "someone"] {
            assert!(!text.contains(needle), "{needle:?} 노출: {text}");
        }
    }

    #[test]
    fn url_and_override_path_are_local_source_only() {
        assert_eq!(
            DeclarationField::Repository.data_class(),
            DataClass::LocalSourceOnly
        );
        assert_eq!(
            DeclarationField::LocalOverridePath.data_class(),
            DataClass::LocalSourceOnly
        );
        // 아키텍처 39장: 브랜치·커밋은 telemetry 금지.
        assert_eq!(
            DeclarationField::SelectorValue.data_class(),
            DataClass::LocalDiagnostic
        );
        let safe: Vec<DeclarationField> = DeclarationField::ALL
            .into_iter()
            .filter(|f| f.data_class() == DataClass::TelemetrySafe)
            .collect();
        assert_eq!(
            safe,
            [
                DeclarationField::Source,
                DeclarationField::SelectorKind,
                DeclarationField::HasLocalOverride
            ]
        );
    }

    #[test]
    fn debug_and_diagnostic_output_are_redacted() {
        let d = sample();
        let debug = format!("{d:?}");
        assert_no_source(&debug);
        assert!(debug.contains("RepositoryUrl(<redacted>)"));
        assert!(debug.contains("LocalOverridePath(<redacted>)"));

        let diagnostic = d.diagnostic().to_string();
        assert_no_source(&diagnostic);
        assert!(diagnostic.contains("name=ds"));
        assert!(diagnostic.contains("version=branch:wip/private-branch"));
        assert!(diagnostic.contains("repository=<redacted>"));
        assert!(diagnostic.contains("local_override=<redacted>"));
        assert_no_source(&format!("{:?}", d.diagnostic()));

        let mut plain = d.clone();
        plain.local_override = None;
        assert!(plain
            .diagnostic()
            .to_string()
            .contains("local_override=none"));
        // 명시적 경로로는 원문 접근 가능.
        assert_eq!(d.repository.reveal(), URL);
        assert_eq!(d.local_override.as_ref().unwrap().reveal(), PATH);
    }

    #[test]
    fn telemetry_carries_only_safe_fields() {
        let t = sample().telemetry();
        assert_eq!(
            t,
            DeclarationTelemetry {
                source: "git",
                selector_kind: "branch",
                has_local_override: true
            }
        );
        let text = format!("{t:?}");
        assert_no_source(&text);
        // 로컬 진단 등급(이름·branch)도 telemetry에는 없음.
        assert!(!text.contains("ds\""));
        assert!(!text.contains("wip/private-branch"));
    }
}
