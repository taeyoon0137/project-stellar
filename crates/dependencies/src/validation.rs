//! 의존성 선언 충돌 validation (XV.1.3).
//!
//! 세 가지 충돌을 구분합니다.
//!
//! | 충돌 | 심각도 | 근거 |
//! |---|---|---|
//! | 같은 logical name 중복 | 오류 | logical name은 dependency path·lock key의 식별자 |
//! | 같은 LibraryId, 다른 선택자 | 경고 | 여러 버전 동시 로드 허용(아키텍처 30.3), 자동 통합 금지 |
//! | 같은 LibraryId, 같은 선택자, 다른 이름 | 경고 | 같은 node로 dedup되는 중복 선언 |
//! | 다른 LibraryId, 같은 repository | 오류 | 한 저장소 root는 한 Stellar 프로젝트(아키텍처 19.1) |
//!
//! repository 비교는 [`RepositoryUrl::comparison_key`]로 정규화합니다. 충돌 결과와 메시지에는
//! URL을 담지 않습니다(XV.1.4).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::declaration::{DependencyDeclaration, LibraryId, LogicalName, RepositoryUrl};

impl RepositoryUrl {
    /// 같은 저장소 판정용 비교 키.
    ///
    /// 끝의 `/`와 `.git`을 제거하고, `scheme://[user@]host/...`와 scp 형식 `[user@]host:path`의
    /// scheme·host만 소문자로 바꿉니다. 경로 대소문자는 보존합니다(서버가 구분할 수 있음).
    pub fn comparison_key(&self) -> String {
        let mut text = self.as_str().trim_end_matches('/');
        if let Some(stripped) = text.strip_suffix(".git") {
            text = stripped.trim_end_matches('/');
        }
        if let Some((scheme, rest)) = text.split_once("://") {
            let (authority, path) = match rest.find('/') {
                Some(i) => rest.split_at(i),
                None => (rest, ""),
            };
            return format!(
                "{}://{}{path}",
                scheme.to_ascii_lowercase(),
                lowercase_host(authority)
            );
        }
        // scp 형식: 첫 ':'이 첫 '/'보다 앞에 있고, host가 한 글자(Windows drive)가 아님.
        if let Some(colon) = text.find(':') {
            let slash = text.find('/').unwrap_or(usize::MAX);
            let authority = &text[..colon];
            if colon < slash && authority.len() > 1 {
                return format!("{}:{}", lowercase_host(authority), &text[colon + 1..]);
            }
        }
        text.to_string()
    }
}

/// `[user@]host[:port]`에서 host 부분만 소문자로 바꿉니다(user는 보존).
fn lowercase_host(authority: &str) -> String {
    match authority.rsplit_once('@') {
        Some((user, host)) => format!("{user}@{}", host.to_ascii_lowercase()),
        None => authority.to_ascii_lowercase(),
    }
}

/// 충돌 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConflictKind {
    /// 같은 logical name이 두 번 이상 선언됨.
    DuplicateLogicalName,
    /// 서로 다른 repository를 같은 LibraryId로 선언함.
    DifferentLibrarySameRepository,
    /// 같은 LibraryId를 서로 다른 선택자로 선언함(다중 버전).
    SameLibraryDifferentSelector,
    /// 같은 LibraryId·같은 선택자를 서로 다른 이름으로 선언함.
    RedundantLibraryDeclaration,
}

impl ConflictKind {
    /// 심각도.
    pub fn severity(self) -> ConflictSeverity {
        match self {
            ConflictKind::DuplicateLogicalName | ConflictKind::DifferentLibrarySameRepository => {
                ConflictSeverity::Error
            }
            ConflictKind::SameLibraryDifferentSelector
            | ConflictKind::RedundantLibraryDeclaration => ConflictSeverity::Warning,
        }
    }

    /// 안정 코드(진단·telemetry 카운트용).
    pub fn code(self) -> &'static str {
        match self {
            ConflictKind::DuplicateLogicalName => "dependency-duplicate-name",
            ConflictKind::DifferentLibrarySameRepository => {
                "dependency-different-library-same-repository"
            }
            ConflictKind::SameLibraryDifferentSelector => {
                "dependency-same-library-different-selector"
            }
            ConflictKind::RedundantLibraryDeclaration => "dependency-redundant-library",
        }
    }
}

/// 심각도.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConflictSeverity {
    /// 해석을 진행할 수 없음.
    Error,
    /// 해석은 진행하되 사용자에게 알림.
    Warning,
}

/// 선언 충돌 한 건.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarationConflict {
    /// 종류.
    pub kind: ConflictKind,
    /// 관련 logical name(정렬, 중복 제거).
    pub names: Vec<LogicalName>,
    /// 관련 LibraryId(정렬, 중복 제거).
    pub library_ids: Vec<LibraryId>,
}

impl fmt::Display for DeclarationConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.names.iter().map(LogicalName::as_str).collect();
        let names = names.join(", ");
        match self.kind {
            ConflictKind::DuplicateLogicalName => {
                write!(f, "같은 logical name이 중복 선언되었습니다: {names}")
            }
            ConflictKind::DifferentLibrarySameRepository => write!(
                f,
                "서로 다른 library id가 같은 repository를 가리킵니다: {names}"
            ),
            ConflictKind::SameLibraryDifferentSelector => write!(
                f,
                "같은 library를 서로 다른 버전으로 선언했습니다(다중 버전): {names}"
            ),
            ConflictKind::RedundantLibraryDeclaration => write!(
                f,
                "같은 library·같은 버전이 여러 이름으로 선언되었습니다: {names}"
            ),
        }
    }
}

/// validation 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeclarationReport {
    /// 충돌 목록(종류 → 첫 이름 순으로 정렬).
    pub conflicts: Vec<DeclarationConflict>,
}

impl DeclarationReport {
    /// 오류 심각도 충돌이 있는지.
    pub fn has_errors(&self) -> bool {
        self.errors().next().is_some()
    }

    /// 오류 심각도 충돌.
    pub fn errors(&self) -> impl Iterator<Item = &DeclarationConflict> {
        self.conflicts
            .iter()
            .filter(|c| c.kind.severity() == ConflictSeverity::Error)
    }

    /// 경고 심각도 충돌.
    pub fn warnings(&self) -> impl Iterator<Item = &DeclarationConflict> {
        self.conflicts
            .iter()
            .filter(|c| c.kind.severity() == ConflictSeverity::Warning)
    }
}

fn conflict(
    kind: ConflictKind,
    names: BTreeSet<&LogicalName>,
    libraries: BTreeSet<LibraryId>,
) -> DeclarationConflict {
    DeclarationConflict {
        kind,
        names: names.into_iter().cloned().collect(),
        library_ids: libraries.into_iter().collect(),
    }
}

/// 선언 목록의 충돌을 판정합니다. 결과는 선언 순서와 무관하게 결정적입니다.
pub fn validate_declarations(declarations: &[DependencyDeclaration]) -> DeclarationReport {
    let mut conflicts = Vec::new();

    // 1. 같은 logical name.
    let mut by_name: BTreeMap<&LogicalName, BTreeSet<LibraryId>> = BTreeMap::new();
    let mut name_counts: BTreeMap<&LogicalName, usize> = BTreeMap::new();
    for d in declarations {
        by_name.entry(&d.name).or_default().insert(d.library_id);
        *name_counts.entry(&d.name).or_default() += 1;
    }
    for (name, count) in &name_counts {
        if *count > 1 {
            conflicts.push(conflict(
                ConflictKind::DuplicateLogicalName,
                BTreeSet::from([*name]),
                by_name[name].clone(),
            ));
        }
    }

    // 2. 같은 LibraryId: 선택자별로 묶습니다.
    let mut by_library: BTreeMap<LibraryId, BTreeMap<_, BTreeSet<&LogicalName>>> = BTreeMap::new();
    for d in declarations {
        by_library
            .entry(d.library_id)
            .or_default()
            .entry(&d.version)
            .or_default()
            .insert(&d.name);
    }
    for (library, selectors) in &by_library {
        if selectors.len() > 1 {
            let names: BTreeSet<&LogicalName> = selectors.values().flatten().copied().collect();
            conflicts.push(conflict(
                ConflictKind::SameLibraryDifferentSelector,
                names,
                BTreeSet::from([*library]),
            ));
        }
        for names in selectors.values() {
            if names.len() > 1 {
                conflicts.push(conflict(
                    ConflictKind::RedundantLibraryDeclaration,
                    names.clone(),
                    BTreeSet::from([*library]),
                ));
            }
        }
    }

    // 3. 같은 repository(정규화)를 서로 다른 LibraryId가 가리킴.
    let mut by_repository: BTreeMap<String, (BTreeSet<LibraryId>, BTreeSet<&LogicalName>)> =
        BTreeMap::new();
    for d in declarations {
        let slot = by_repository
            .entry(d.repository.comparison_key())
            .or_default();
        slot.0.insert(d.library_id);
        slot.1.insert(&d.name);
    }
    for (libraries, names) in by_repository.into_values() {
        if libraries.len() > 1 {
            conflicts.push(conflict(
                ConflictKind::DifferentLibrarySameRepository,
                names,
                libraries,
            ));
        }
    }

    conflicts.sort_by(|a, b| (a.kind, &a.names).cmp(&(b.kind, &b.names)));
    DeclarationReport { conflicts }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::fixtures::decl;

    fn url(text: &str) -> RepositoryUrl {
        RepositoryUrl::new(text).unwrap()
    }

    #[test]
    fn repository_comparison_key_normalization() {
        let same = [
            (
                "https://Example.COM/org/x.git/",
                "https://example.com/org/x",
            ),
            ("HTTPS://example.com/org/x", "https://example.com/org/x.git"),
            ("git@GitHub.com:org/x.git", "git@github.com:org/x"),
            ("ssh://Git@Host:22/org/x.git", "ssh://Git@host:22/org/x"),
            ("/srv/repos/x.git", "/srv/repos/x"),
        ];
        for (a, b) in same {
            assert_eq!(
                url(a).comparison_key(),
                url(b).comparison_key(),
                "{a} == {b}"
            );
        }
        let different = [
            ("https://e.com/Org/x", "https://e.com/org/x"),
            ("git@h:Org/x", "git@h:org/x"),
            ("ssh://alice@h/x", "ssh://bob@h/x"),
            ("/srv/X", "/srv/x"),
        ];
        for (a, b) in different {
            assert_ne!(
                url(a).comparison_key(),
                url(b).comparison_key(),
                "{a} != {b}"
            );
        }
    }

    type Row = (&'static str, u8, &'static str, &'static str);
    type Expected = Vec<(ConflictKind, Vec<String>)>;

    fn run(rows: &[Row]) -> Expected {
        let declarations: Vec<_> = rows
            .iter()
            .map(|(name, lib, url, tag)| decl(name, *lib, url, tag))
            .collect();
        let report = validate_declarations(&declarations);
        // 선언 순서를 뒤집어도 같은 결과.
        let mut reversed = declarations.clone();
        reversed.reverse();
        assert_eq!(validate_declarations(&reversed), report);
        report
            .conflicts
            .iter()
            .map(|c| {
                (
                    c.kind,
                    c.names.iter().map(|n| n.as_str().to_string()).collect(),
                )
            })
            .collect()
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn conflict_matrix_fixture() {
        use ConflictKind::*;
        const A: &str = "https://e.com/a.git";
        const B: &str = "https://e.com/b.git";

        // (이름, lib, url, tag) 조합 → 기대 충돌.
        let matrix: Vec<(&str, Vec<Row>, Expected)> = vec![
            ("clean", vec![("a", 1, A, "v1"), ("b", 2, B, "v1")], vec![]),
            (
                "같은 이름 중복",
                vec![("a", 1, A, "v1"), ("a", 2, B, "v2")],
                vec![(DuplicateLogicalName, names(&["a"]))],
            ),
            (
                "같은 lib 다른 선택자",
                vec![("a", 1, A, "v1"), ("b", 1, A, "v2")],
                vec![(SameLibraryDifferentSelector, names(&["a", "b"]))],
            ),
            (
                "같은 lib 같은 선택자",
                vec![("a", 1, A, "v1"), ("b", 1, A, "v1")],
                vec![(RedundantLibraryDeclaration, names(&["a", "b"]))],
            ),
            (
                "다른 lib 같은 URL",
                vec![("a", 1, A, "v1"), ("b", 2, A, "v1")],
                vec![(DifferentLibrarySameRepository, names(&["a", "b"]))],
            ),
            (
                "다른 lib 정규화 후 같은 URL",
                vec![
                    ("a", 1, "https://E.com/a.git/", "v1"),
                    ("b", 2, "https://e.com/a", "v1"),
                ],
                vec![(DifferentLibrarySameRepository, names(&["a", "b"]))],
            ),
            (
                "같은 lib 다른 URL 다른 선택자",
                vec![("a", 1, A, "v1"), ("b", 1, B, "v2")],
                vec![(SameLibraryDifferentSelector, names(&["a", "b"]))],
            ),
            (
                "복합",
                vec![("a", 1, A, "v1"), ("b", 1, A, "v2"), ("c", 2, A, "v1")],
                vec![
                    (DifferentLibrarySameRepository, names(&["a", "b", "c"])),
                    (SameLibraryDifferentSelector, names(&["a", "b"])),
                ],
            ),
        ];
        for (label, rows, expected) in matrix {
            assert_eq!(run(&rows), expected, "matrix 행: {label}");
        }
    }

    #[test]
    fn severities_and_messages_without_urls() {
        let declarations = vec![
            decl("a", 1, "https://secret.example/a.git", "v1"),
            decl("a", 2, "https://secret.example/b.git", "v1"),
            decl("b", 3, "https://secret.example/a", "v1"),
            decl("c", 3, "https://secret.example/a", "v2"),
        ];
        let report = validate_declarations(&declarations);
        assert!(report.has_errors());
        let errors: Vec<ConflictKind> = report.errors().map(|c| c.kind).collect();
        assert_eq!(
            errors,
            [
                ConflictKind::DuplicateLogicalName,
                ConflictKind::DifferentLibrarySameRepository
            ]
        );
        let warnings: Vec<ConflictKind> = report.warnings().map(|c| c.kind).collect();
        assert_eq!(warnings, [ConflictKind::SameLibraryDifferentSelector]);
        for c in &report.conflicts {
            let text = c.to_string();
            assert!(
                !text.contains("secret.example"),
                "메시지에 URL 노출: {text}"
            );
        }
        assert!(!validate_declarations(&declarations[2..]).has_errors());
    }
}
