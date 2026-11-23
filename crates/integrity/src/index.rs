//! 공통 DiagnosticIndex: 집계·필터·pagination (XXV.2.1, XXV.3.1 코어 API).
//!
//! 모든 생산자의 진단을 [`DiagnosticKey`] 순서의 map 하나에 둡니다. key가 문서 → 종류 순으로 시작하므로 문서·종류
//! 필터는 map 범위 탐색으로 풀고(page 비용은 page 크기에 비례), 심각도·source 필터는 그 범위 안의 조건 검사로 풉니다.
//! 종류×심각도 개수와 문서별 개수는 삽입·삭제 때 갱신하므로 요약과 종류·심각도 개수 조회는 진단 수와 무관합니다.
//!
//! pagination cursor는 마지막으로 돌려준 key와 index revision입니다. index가 바뀌면 revision이 올라가고 이전 cursor는
//! [`PageError::Stale`]로 거부합니다(같은 cursor로 다른 상태를 섞어 읽지 않음).
//!
//! 같은 (문서, 생산자) 묶음을 scope라 부르고 [`DiagnosticIndex::replace_scope`]가 scope 단위로 교체합니다(증분 재검사,
//! [`crate::incremental`]).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Bound;

use stellar_core_types::ids::StableId;

use crate::diagnostic::{Diagnostic, DiagnosticKey, DiagnosticKind, Severity};
use crate::remediation::RemediationActionId;

/// page 크기 상한.
pub const MAX_PAGE_LIMIT: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Stored {
    severity: Severity,
    message: String,
    remediation: Vec<RemediationActionId>,
}

/// 진단 필터. 모든 조건은 AND입니다. 빈 필터는 전체입니다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticFilter {
    /// 종류(비어 있으면 전체).
    pub kinds: BTreeSet<DiagnosticKind>,
    /// 최소 심각도.
    pub min_severity: Option<Severity>,
    /// 문서(`Some(None)`은 프로젝트 단위 진단만).
    pub document: Option<Option<StableId>>,
    /// source StableId.
    pub source: Option<StableId>,
}

impl DiagnosticFilter {
    fn accepts(&self, key: &DiagnosticKey, stored: &Stored) -> bool {
        (self.kinds.is_empty() || self.kinds.contains(&key.kind))
            && self.min_severity.is_none_or(|min| stored.severity >= min)
            && self.document.is_none_or(|d| d == key.document)
            && self.source.is_none_or(|s| key.source == Some(s))
    }

    /// 종류·심각도 조건만 있는지(개수 표로 셀 수 있음).
    fn counts_only(&self) -> bool {
        self.document.is_none() && self.source.is_none()
    }
}

/// pagination cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    revision: u64,
    after: DiagnosticKey,
}

impl Cursor {
    /// 문자열 token(`<revision hex>.<key hex>`).
    pub fn token(&self) -> String {
        let bytes = self.after.to_bytes();
        let mut out = format!("{:x}.", self.revision);
        for b in bytes {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }

    /// token을 읽습니다.
    pub fn parse(token: &str) -> Option<Cursor> {
        let (revision, key) = token.split_once('.')?;
        let revision = u64::from_str_radix(revision, 16).ok()?;
        if key.len() % 2 != 0 || !key.is_ascii() {
            return None;
        }
        let bytes = (0..key.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&key[i..i + 2], 16).ok())
            .collect::<Option<Vec<u8>>>()?;
        let after = crate::codec::read_key(&bytes).ok()?;
        Some(Cursor { revision, after })
    }
}

/// page 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageError {
    /// limit이 0이거나 상한을 넘음.
    InvalidLimit(usize),
    /// cursor 이후 index가 바뀜.
    Stale,
}

impl fmt::Display for PageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PageError::InvalidLimit(n) => {
                write!(f, "page 크기 {n}은 1..={MAX_PAGE_LIMIT} 범위여야 합니다")
            }
            PageError::Stale => {
                f.write_str("cursor 이후 진단이 바뀌었습니다. 처음부터 다시 읽으세요")
            }
        }
    }
}

impl std::error::Error for PageError {}

/// 진단 한 page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticPage {
    /// 진단(key 순).
    pub items: Vec<Diagnostic>,
    /// 다음 page cursor(마지막 page면 `None`).
    pub next: Option<Cursor>,
    /// 읽은 index revision.
    pub revision: u64,
}

/// 종류×심각도 개수 요약.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticSummary {
    /// (종류, 심각도) → 개수(0은 없음).
    pub counts: BTreeMap<(DiagnosticKind, Severity), usize>,
}

impl DiagnosticSummary {
    /// 전체 개수.
    pub fn total(&self) -> usize {
        self.counts.values().sum()
    }

    /// 종류 하나의 개수.
    pub fn kind(&self, kind: DiagnosticKind) -> usize {
        self.counts
            .iter()
            .filter(|((k, _), _)| *k == kind)
            .map(|(_, n)| n)
            .sum()
    }

    /// 심각도 하나의 개수.
    pub fn severity(&self, severity: Severity) -> usize {
        self.counts
            .iter()
            .filter(|((_, s), _)| *s == severity)
            .map(|(_, n)| n)
            .sum()
    }
}

/// 공통 진단 index.
#[derive(Debug, Clone, Default)]
pub struct DiagnosticIndex {
    entries: BTreeMap<DiagnosticKey, Stored>,
    summary: DiagnosticSummary,
    documents: BTreeMap<Option<StableId>, usize>,
    revision: u64,
}

/// (문서, 종류)의 가장 작은 key(범위 시작).
fn floor(document: Option<StableId>, kind: DiagnosticKind) -> DiagnosticKey {
    DiagnosticKey {
        document,
        kind,
        source: None,
        property: None,
        code: String::new(),
        subject: String::new(),
        origin: String::new(),
    }
}

impl DiagnosticIndex {
    /// 빈 index.
    pub fn new() -> DiagnosticIndex {
        DiagnosticIndex::default()
    }

    /// 진단 수.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 비었는지.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 변경 revision(삽입·삭제·교체마다 1 증가).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// 종류×심각도 요약.
    pub fn summary(&self) -> &DiagnosticSummary {
        &self.summary
    }

    fn count_add(&mut self, key: &DiagnosticKey, severity: Severity) {
        *self.summary.counts.entry((key.kind, severity)).or_default() += 1;
        *self.documents.entry(key.document).or_default() += 1;
    }

    fn count_remove(&mut self, key: &DiagnosticKey, severity: Severity) {
        let slot = (key.kind, severity);
        if let Some(n) = self.summary.counts.get_mut(&slot) {
            *n -= 1;
            if *n == 0 {
                self.summary.counts.remove(&slot);
            }
        }
        if let Some(n) = self.documents.get_mut(&key.document) {
            *n -= 1;
            if *n == 0 {
                self.documents.remove(&key.document);
            }
        }
    }

    fn put(&mut self, diagnostic: Diagnostic) -> bool {
        let Diagnostic {
            key,
            severity,
            message,
            remediation,
        } = diagnostic;
        let stored = Stored {
            severity,
            message,
            remediation,
        };
        if self.entries.get(&key) == Some(&stored) {
            return false;
        }
        if let Some(old) = self.entries.get(&key) {
            let old_severity = old.severity;
            self.count_remove(&key, old_severity);
        }
        self.count_add(&key, severity);
        self.entries.insert(key, stored);
        true
    }

    fn take(&mut self, key: &DiagnosticKey) -> bool {
        match self.entries.remove(key) {
            Some(stored) => {
                self.count_remove(key, stored.severity);
                true
            }
            None => false,
        }
    }

    /// 진단 하나를 넣습니다(같은 key는 교체). 바뀌었으면 `true`.
    pub fn insert(&mut self, diagnostic: Diagnostic) -> bool {
        let changed = self.put(diagnostic);
        if changed {
            self.revision += 1;
        }
        changed
    }

    /// key의 진단을 뺍니다.
    pub fn remove(&mut self, key: &DiagnosticKey) -> bool {
        let changed = self.take(key);
        if changed {
            self.revision += 1;
        }
        changed
    }

    /// (문서, 생산자) scope의 진단을 `diagnostics`로 교체합니다. `diagnostics`의 문서·origin은 scope 값으로 맞춥니다.
    /// 실제로 바뀐 것이 있으면 revision을 한 번 올리고 `true`.
    pub fn replace_scope(
        &mut self,
        document: Option<StableId>,
        origin: &str,
        diagnostics: Vec<Diagnostic>,
    ) -> bool {
        let mut fresh: BTreeMap<DiagnosticKey, Diagnostic> = BTreeMap::new();
        for mut d in diagnostics {
            d.key.document = document;
            d.key.origin = origin.to_string();
            fresh.insert(d.key.clone(), d);
        }
        let stale: Vec<DiagnosticKey> = self
            .document_range(document)
            .filter(|(k, _)| k.origin == origin && !fresh.contains_key(*k))
            .map(|(k, _)| k.clone())
            .collect();
        let mut changed = false;
        for key in &stale {
            changed |= self.take(key);
        }
        for (_, d) in fresh {
            changed |= self.put(d);
        }
        if changed {
            self.revision += 1;
        }
        changed
    }

    /// (문서, 생산자) scope의 진단 수.
    pub fn scope_len(&self, document: Option<StableId>, origin: &str) -> usize {
        self.document_range(document)
            .filter(|(k, _)| k.origin == origin)
            .count()
    }

    fn document_range(
        &self,
        document: Option<StableId>,
    ) -> impl Iterator<Item = (&DiagnosticKey, &Stored)> + '_ {
        self.entries
            .range((
                Bound::Included(floor(document, DiagnosticKind::ALL[0])),
                Bound::Unbounded,
            ))
            .take_while(move |(k, _)| k.document == document)
    }

    fn materialize(key: &DiagnosticKey, stored: &Stored) -> Diagnostic {
        Diagnostic {
            key: key.clone(),
            severity: stored.severity,
            message: stored.message.clone(),
            remediation: stored.remediation.clone(),
        }
    }

    /// key의 진단.
    pub fn get(&self, key: &DiagnosticKey) -> Option<Diagnostic> {
        self.entries.get(key).map(|s| Self::materialize(key, s))
    }

    /// 안정 ID로 찾습니다(전체 탐색).
    pub fn find(&self, id: crate::diagnostic::DiagnosticId) -> Option<Diagnostic> {
        self.entries
            .iter()
            .find(|(k, _)| k.id() == id)
            .map(|(k, s)| Self::materialize(k, s))
    }

    /// 필터에 맞는 진단을 key 순으로 돌려주는 반복자. `after`가 있으면 그 key 다음부터입니다.
    fn matching<'a>(
        &'a self,
        filter: &'a DiagnosticFilter,
        after: Option<&'a DiagnosticKey>,
    ) -> impl Iterator<Item = (&'a DiagnosticKey, &'a Stored)> + 'a {
        let documents: Vec<Option<StableId>> = match filter.document {
            Some(document) => vec![document],
            None => self
                .documents
                .keys()
                .copied()
                .filter(|d| after.is_none_or(|a| *d >= a.document))
                .collect(),
        };
        let kinds: Vec<Option<DiagnosticKind>> = if filter.kinds.is_empty() {
            vec![None]
        } else {
            filter.kinds.iter().copied().map(Some).collect()
        };
        documents
            .into_iter()
            .flat_map(move |document| {
                kinds.clone().into_iter().flat_map(move |kind| {
                    let start = floor(document, kind.unwrap_or(DiagnosticKind::ALL[0]));
                    let lower = match after {
                        Some(a) if *a >= start => Bound::Excluded(a.clone()),
                        _ => Bound::Included(start),
                    };
                    self.entries
                        .range((lower, Bound::Unbounded))
                        .take_while(move |(k, _)| {
                            k.document == document && kind.is_none_or(|kind| k.kind == kind)
                        })
                })
            })
            .filter(move |(k, s)| filter.accepts(k, s))
    }

    /// 필터에 맞는 진단 수. 종류·심각도 조건만 있으면 개수 표로, 아니면 범위를 세어 계산합니다.
    pub fn count(&self, filter: &DiagnosticFilter) -> usize {
        if filter.counts_only() {
            return self
                .summary
                .counts
                .iter()
                .filter(|((kind, severity), _)| {
                    (filter.kinds.is_empty() || filter.kinds.contains(kind))
                        && filter.min_severity.is_none_or(|min| *severity >= min)
                })
                .map(|(_, n)| n)
                .sum();
        }
        self.matching(filter, None).count()
    }

    /// 필터에 맞는 진단 한 page(XXV.3.1). `cursor`는 이전 page의 `next`입니다.
    pub fn page(
        &self,
        filter: &DiagnosticFilter,
        cursor: Option<&Cursor>,
        limit: usize,
    ) -> Result<DiagnosticPage, PageError> {
        if limit == 0 || limit > MAX_PAGE_LIMIT {
            return Err(PageError::InvalidLimit(limit));
        }
        if cursor.is_some_and(|c| c.revision != self.revision) {
            return Err(PageError::Stale);
        }
        let mut items: Vec<Diagnostic> = Vec::with_capacity(limit);
        let mut more = false;
        for (key, stored) in self.matching(filter, cursor.map(|c| &c.after)) {
            if items.len() == limit {
                more = true;
                break;
            }
            items.push(Self::materialize(key, stored));
        }
        let next = if more {
            items.last().map(|d| Cursor {
                revision: self.revision,
                after: d.key.clone(),
            })
        } else {
            None
        };
        Ok(DiagnosticPage {
            items,
            next,
            revision: self.revision,
        })
    }

    /// 전체 진단(key 순).
    pub fn iter(&self) -> impl Iterator<Item = Diagnostic> + '_ {
        self.entries.iter().map(|(k, s)| Self::materialize(k, s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticKey;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn diag(
        document: Option<u64>,
        kind: DiagnosticKind,
        source: u64,
        severity: Severity,
    ) -> Diagnostic {
        Diagnostic {
            key: DiagnosticKey {
                document: document.map(sid),
                kind,
                source: Some(sid(source)),
                property: None,
                code: kind.name().into(),
                subject: String::new(),
                origin: "test".into(),
            },
            severity,
            message: format!("{source}"),
            remediation: Vec::new(),
        }
    }

    fn read_all(
        index: &DiagnosticIndex,
        filter: &DiagnosticFilter,
        limit: usize,
    ) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let mut cursor = None;
        loop {
            let page = index.page(filter, cursor.as_ref(), limit).expect("page");
            assert!(page.items.len() <= limit);
            out.extend(page.items);
            match page.next {
                Some(next) => {
                    let parsed = Cursor::parse(&next.token()).expect("token 왕복");
                    assert_eq!(parsed, next);
                    cursor = Some(parsed);
                }
                None => return out,
            }
        }
    }

    fn fixture() -> DiagnosticIndex {
        let mut index = DiagnosticIndex::new();
        for n in 0..200u64 {
            let kind = DiagnosticKind::ALL[(n % 10) as usize];
            let severity = Severity::ALL[(n % 3) as usize];
            let document = if n % 7 == 0 { None } else { Some(n % 4) };
            index.insert(diag(document, kind, n, severity));
        }
        index
    }

    #[test]
    fn filtered_pages_equal_filtered_scan_in_key_order() {
        let index = fixture();
        let all: Vec<Diagnostic> = index.iter().collect();
        let filters = [
            DiagnosticFilter::default(),
            DiagnosticFilter {
                kinds: [DiagnosticKind::MissingFont, DiagnosticKind::OrphanedBinding].into(),
                ..Default::default()
            },
            DiagnosticFilter {
                min_severity: Some(Severity::Warning),
                document: Some(Some(sid(2))),
                ..Default::default()
            },
            DiagnosticFilter {
                document: Some(None),
                ..Default::default()
            },
            DiagnosticFilter {
                kinds: [DiagnosticKind::MigrationWarning].into(),
                min_severity: Some(Severity::Error),
                ..Default::default()
            },
            DiagnosticFilter {
                source: Some(sid(17)),
                ..Default::default()
            },
        ];
        for filter in &filters {
            let expected: Vec<Diagnostic> = all
                .iter()
                .filter(|d| {
                    (filter.kinds.is_empty() || filter.kinds.contains(&d.kind()))
                        && filter.min_severity.is_none_or(|m| d.severity >= m)
                        && filter.document.is_none_or(|doc| doc == d.document())
                        && filter.source.is_none_or(|s| d.source() == Some(s))
                })
                .cloned()
                .collect();
            for limit in [1, 3, 64, 1000] {
                assert_eq!(
                    read_all(&index, filter, limit),
                    expected,
                    "{filter:?} {limit}"
                );
            }
            assert_eq!(index.count(filter), expected.len(), "{filter:?}");
        }
    }

    #[test]
    fn summary_tracks_insert_replace_remove() {
        let mut index = DiagnosticIndex::new();
        let a = diag(Some(1), DiagnosticKind::MissingFont, 1, Severity::Warning);
        assert!(index.insert(a.clone()));
        assert!(!index.insert(a.clone()), "같은 진단은 변경 없음");
        let mut b = a.clone();
        b.severity = Severity::Error;
        assert!(index.insert(b.clone()));
        assert_eq!(index.len(), 1);
        assert_eq!(index.summary().severity(Severity::Warning), 0);
        assert_eq!(index.summary().severity(Severity::Error), 1);
        assert!(index.remove(&b.key));
        assert!(index.is_empty());
        assert_eq!(index.summary().total(), 0);
    }

    #[test]
    fn replace_scope_touches_only_that_scope_and_stale_cursor_is_rejected() {
        let mut index = DiagnosticIndex::new();
        let mut other = diag(Some(1), DiagnosticKind::MissingFont, 9, Severity::Warning);
        other.key.origin = "fonts".into();
        index.insert(other.clone());
        assert!(index.replace_scope(
            Some(sid(1)),
            "references",
            vec![
                diag(Some(1), DiagnosticKind::MissingNode, 1, Severity::Error),
                diag(Some(1), DiagnosticKind::MissingNode, 2, Severity::Error),
            ],
        ));
        assert_eq!(index.scope_len(Some(sid(1)), "references"), 2);
        let first = index.page(&DiagnosticFilter::default(), None, 1).unwrap();
        let cursor = first.next.clone().unwrap();
        assert!(index.replace_scope(
            Some(sid(1)),
            "references",
            vec![diag(
                Some(1),
                DiagnosticKind::MissingNode,
                2,
                Severity::Error
            )],
        ));
        assert!(
            !index.replace_scope(
                Some(sid(1)),
                "references",
                vec![diag(
                    Some(1),
                    DiagnosticKind::MissingNode,
                    2,
                    Severity::Error
                )],
            ),
            "같은 내용 교체는 변경 없음"
        );
        assert_eq!(index.scope_len(Some(sid(1)), "references"), 1);
        assert_eq!(index.get(&other.key), Some(other));
        assert_eq!(
            index.page(&DiagnosticFilter::default(), Some(&cursor), 1),
            Err(PageError::Stale)
        );
        assert_eq!(
            index.page(&DiagnosticFilter::default(), None, 0),
            Err(PageError::InvalidLimit(0))
        );
        assert!(Cursor::parse("zz").is_none());
        assert!(Cursor::parse("1.0").is_none());
    }

    #[test]
    fn million_diagnostics_paginate_with_filters() {
        // 검증 기준(XXV.3.1): 100만 diagnostic pagination. 전체를 page로 읽어 개수·순서·중복 없음을 확인하고,
        // 문서·종류 필터 page는 범위 탐색이라 앞 page를 다시 훑지 않습니다.
        let mut index = DiagnosticIndex::new();
        let documents: Vec<StableId> = (1..=8).map(sid).collect();
        for n in 0..1_000_000u64 {
            let kind = DiagnosticKind::ALL[(n % 10) as usize];
            let severity = Severity::ALL[(n % 3) as usize];
            index.put(Diagnostic {
                key: DiagnosticKey {
                    document: Some(documents[(n % 8) as usize]),
                    kind,
                    source: Some(StableId::from_bytes((n as u128).to_be_bytes())),
                    property: None,
                    code: String::new(),
                    subject: String::new(),
                    origin: "bulk".into(),
                },
                severity,
                message: String::new(),
                remediation: Vec::new(),
            });
        }
        assert_eq!(index.len(), 1_000_000);
        assert_eq!(index.count(&DiagnosticFilter::default()), 1_000_000);
        let mut cursor: Option<Cursor> = None;
        let mut seen = 0usize;
        let mut last: Option<DiagnosticKey> = None;
        loop {
            let page = index
                .page(
                    &DiagnosticFilter::default(),
                    cursor.as_ref(),
                    MAX_PAGE_LIMIT,
                )
                .unwrap();
            for item in &page.items {
                if let Some(prev) = &last {
                    assert!(prev < &item.key);
                }
                last = Some(item.key.clone());
            }
            seen += page.items.len();
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(seen, 1_000_000);
        let filter = DiagnosticFilter {
            kinds: [DiagnosticKind::MissingAsset].into(),
            document: Some(Some(documents[3])),
            min_severity: Some(Severity::Warning),
            ..Default::default()
        };
        // 기대 개수는 생성 규칙에서 따로 셉니다(문서 3번, 종류 MissingAsset, 심각도 warning 이상).
        let expected = (0..1_000_000u64)
            .filter(|n| n % 8 == 3 && n % 10 == 3 && n % 3 != 0)
            .count();
        assert!(expected > 0);
        assert_eq!(index.count(&filter), expected);
        let mut cursor: Option<Cursor> = None;
        let mut total = 0;
        loop {
            let page = index.page(&filter, cursor.as_ref(), 500).unwrap();
            assert!(page
                .items
                .iter()
                .all(|d| d.kind() == DiagnosticKind::MissingAsset
                    && d.document() == Some(documents[3])
                    && d.severity >= Severity::Warning));
            total += page.items.len();
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(total, expected);
    }
}
