//! 다중 버전 경고 상세: dependency path·예상 추가 용량·표시 경로별 정보 분리 (XV.8.3).
//!
//! [`DuplicateLibraryVersionWarning`](XV.2.5)에 버전별 cache 크기를 붙여 **예상 추가 용량**을
//! 계산합니다. 같은 content hash의 버전은 공용 cache에서 중복 제거되므로(아키텍처 30.3) 고유 내용만
//! 셉니다. 예상 추가 용량 = 고유 내용 크기 합 − 가장 큰 고유 내용 크기(한 버전만 쓸 때보다 더 드는
//! 최소 용량)입니다. 크기는 privacy cache index(XV.3.5)에서 읽습니다.
//!
//! 출력 경로를 둘로 나눕니다.
//!
//! - 표시 UI([`DuplicateVersionReport::ui_view`]): local source config(XV.3.5)에서 얻은 repository
//!   URL을 버전별로 함께 전달합니다(로컬 UI 전용).
//! - telemetry([`DuplicateVersionReport::telemetry`]): 개수와 byte 수만. URL·경로·이름·library
//!   id·commit은 제외합니다(아키텍처 39장).

use std::collections::BTreeMap;

use crate::cache_privacy::{LocalSourceConfig, PrivateCacheIndex};
use crate::content_hash::ContentHashText;
use crate::declaration::{CommitId, LibraryId, RepositoryUrl};
use crate::graph::{DependencyGraph, DependencyNodeKey, DependencyPath};
use crate::mirror::{RepositoryKey, RepositoryKeyer};

/// 예상 추가 용량.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdditionalSize {
    /// 모든 버전의 크기를 알 때의 값(byte).
    Known(u64),
    /// 일부 버전이 cache index에 없어 아는 버전만으로 계산한 하한.
    LowerBound {
        /// 아는 버전만으로 계산한 byte 수.
        bytes: u64,
        /// 크기를 모르는 버전 수.
        unknown_versions: usize,
    },
}

/// 다중 버전 경고의 버전 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateVersionDetail {
    /// resolved commit.
    pub commit: CommitId,
    /// 이 버전에 도달한 dependency path(정렬).
    pub paths: Vec<DependencyPath>,
    /// 이 버전의 repository keyed identifier(첫 edge 기준).
    pub repository_key: Option<RepositoryKey>,
    /// cache entry content hash(index에 있으면).
    pub content_hash: Option<ContentHashText>,
    /// cache entry 크기(index에 있으면).
    pub size_bytes: Option<u64>,
}

/// 다중 버전 경고 상세.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateVersionReport {
    /// library 식별자.
    pub library_id: LibraryId,
    /// 버전별 상세(commit 순).
    pub versions: Vec<DuplicateVersionDetail>,
    /// 예상 추가 용량.
    pub expected_additional: AdditionalSize,
}

/// 표시 UI용 버전 하나(로컬 source URL 포함).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateVersionUi<'a> {
    /// resolved commit.
    pub commit: &'a CommitId,
    /// dependency path.
    pub paths: &'a [DependencyPath],
    /// 로컬 source config의 repository URL(없으면 `None`).
    pub source: Option<&'a RepositoryUrl>,
    /// 크기.
    pub size_bytes: Option<u64>,
}

/// 표시 UI용 경고.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateWarningUi<'a> {
    /// library 식별자.
    pub library_id: LibraryId,
    /// 버전별 표시 정보.
    pub versions: Vec<DuplicateVersionUi<'a>>,
    /// 예상 추가 용량.
    pub expected_additional: AdditionalSize,
}

/// telemetry용 경고 요약(숫자만).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DuplicateWarningTelemetry {
    /// 정적 이벤트 이름.
    pub event: &'static str,
    /// 버전 수.
    pub version_count: u32,
    /// dependency path 수.
    pub path_count: u32,
    /// 예상 추가 용량(하한이면 하한 값).
    pub expected_additional_bytes: u64,
    /// 크기를 모르는 버전 수.
    pub unknown_size_versions: u32,
}

impl DuplicateVersionReport {
    /// 표시 UI용 view. URL은 local source config에서만 가져옵니다.
    pub fn ui_view<'a>(&'a self, sources: &'a LocalSourceConfig) -> DuplicateWarningUi<'a> {
        DuplicateWarningUi {
            library_id: self.library_id,
            versions: self
                .versions
                .iter()
                .map(|v| DuplicateVersionUi {
                    commit: &v.commit,
                    paths: &v.paths,
                    source: v
                        .repository_key
                        .as_ref()
                        .and_then(|k| sources.source_for(k)),
                    size_bytes: v.size_bytes,
                })
                .collect(),
            expected_additional: self.expected_additional,
        }
    }

    /// telemetry 요약.
    pub fn telemetry(&self) -> DuplicateWarningTelemetry {
        let saturate = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        let (bytes, unknown) = match self.expected_additional {
            AdditionalSize::Known(bytes) => (bytes, 0),
            AdditionalSize::LowerBound {
                bytes,
                unknown_versions,
            } => (bytes, unknown_versions),
        };
        DuplicateWarningTelemetry {
            event: "dependency_duplicate_library_version",
            version_count: saturate(self.versions.len()),
            path_count: saturate(self.versions.iter().map(|v| v.paths.len()).sum()),
            expected_additional_bytes: bytes,
            unknown_size_versions: saturate(unknown),
        }
    }
}

/// 그래프의 다중 버전 경고마다 상세 보고서를 만듭니다(LibraryId 순).
pub fn duplicate_version_reports<K: RepositoryKeyer>(
    graph: &DependencyGraph,
    index: &PrivateCacheIndex,
    keyer: &K,
) -> Vec<DuplicateVersionReport> {
    graph
        .duplicate_library_versions()
        .into_iter()
        .map(|warning| {
            let versions: Vec<DuplicateVersionDetail> = warning
                .versions
                .into_iter()
                .map(|version| {
                    let key = DependencyNodeKey::new(warning.library_id, version.commit.clone());
                    let repository = graph.edges_to(&key).next().map(|e| &e.repository);
                    let cached = repository.and_then(|r| index.get(keyer, r, &version.commit));
                    DuplicateVersionDetail {
                        commit: version.commit,
                        paths: version.paths,
                        repository_key: repository.map(|r| keyer.repository_key(r)),
                        content_hash: cached.map(|c| c.content_hash.clone()),
                        size_bytes: cached.map(|c| c.size_bytes),
                    }
                })
                .collect();
            let expected_additional = additional_size(&versions);
            DuplicateVersionReport {
                library_id: warning.library_id,
                versions,
                expected_additional,
            }
        })
        .collect()
}

/// 고유 content 크기 합 − 최대 고유 content 크기.
fn additional_size(versions: &[DuplicateVersionDetail]) -> AdditionalSize {
    let mut unique: BTreeMap<&ContentHashText, u64> = BTreeMap::new();
    let mut unknown = 0usize;
    for v in versions {
        match (&v.content_hash, v.size_bytes) {
            (Some(hash), Some(size)) => {
                unique.insert(hash, size);
            }
            _ => unknown += 1,
        }
    }
    let total: u64 = unique.values().fold(0u64, |acc, s| acc.saturating_add(*s));
    let largest = unique.values().copied().max().unwrap_or(0);
    let bytes = total - largest;
    if unknown == 0 {
        AdditionalSize::Known(bytes)
    } else {
        AdditionalSize::LowerBound {
            bytes,
            unknown_versions: unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache_index::fixtures::hash;
    use crate::cache_index::{CacheIndex, CacheIndexEntry};
    use crate::declaration::fixtures::{commit, decl, library};
    use crate::declaration::LogicalName;
    use crate::resolver::fake::{resolve, Catalog};
    use crate::test_support::FnvKeyer;

    const A: &str = "https://e.com/a.git";
    const B: &str = "https://e.com/b.git";
    const X: &str = "https://e.com/x.git";
    const C: &str = "git@secret-host.example:team/shared-tokens.git";

    fn path(names: &[&str]) -> DependencyPath {
        DependencyPath::from_names(names.iter().map(|n| LogicalName::new(n).unwrap()).collect())
    }

    /// root → a → c v1, root → b → c v2, root → x → c v3(내용이 v1과 동일).
    fn graph() -> DependencyGraph {
        let mut catalog = Catalog::default();
        catalog.publish(C, 3, "v1", 0xc1, vec![]);
        catalog.publish(C, 3, "v2", 0xc2, vec![]);
        catalog.publish(C, 3, "v3", 0xc3, vec![]);
        catalog.publish(A, 1, "v1", 0xa1, vec![decl("c", 3, C, "v1")]);
        catalog.publish(B, 2, "v1", 0xb1, vec![decl("c", 3, C, "v2")]);
        catalog.publish(X, 4, "v1", 0x41, vec![decl("c", 3, C, "v3")]);
        resolve(
            &catalog,
            &[
                decl("a", 1, A, "v1"),
                decl("b", 2, B, "v1"),
                decl("x", 4, X, "v1"),
            ],
        )
        .unwrap()
    }

    fn index_entry(c: u8, content: u8, size: u64) -> CacheIndexEntry {
        CacheIndexEntry {
            repository: RepositoryUrl::new(C).unwrap(),
            commit: commit(c),
            content_hash: hash(content),
            size_bytes: size,
            last_used_unix_ms: 1,
        }
    }

    fn private(entries: Vec<CacheIndexEntry>) -> (PrivateCacheIndex, LocalSourceConfig) {
        let mut plain = CacheIndex::new();
        for e in entries {
            plain.upsert(e);
        }
        plain.split_private(&FnvKeyer::new(11))
    }

    #[test]
    fn warning_fixture_has_paths_and_expected_additional_size() {
        let (index, sources) = private(vec![
            index_entry(0xc1, 0x01, 1_000),
            index_entry(0xc2, 0x02, 3_000),
            index_entry(0xc3, 0x01, 1_000), // v1과 같은 내용 → cache 공유
        ]);
        let keyer = FnvKeyer::new(11);
        let reports = duplicate_version_reports(&graph(), &index, &keyer);
        assert_eq!(reports.len(), 1);
        let report = &reports[0];
        assert_eq!(report.library_id, library(3));
        let summary: Vec<(CommitId, Vec<DependencyPath>, Option<u64>)> = report
            .versions
            .iter()
            .map(|v| (v.commit.clone(), v.paths.clone(), v.size_bytes))
            .collect();
        assert_eq!(
            summary,
            [
                (commit(0xc1), vec![path(&["a", "c"])], Some(1_000)),
                (commit(0xc2), vec![path(&["b", "c"])], Some(3_000)),
                (commit(0xc3), vec![path(&["x", "c"])], Some(1_000)),
            ]
        );
        // 고유 내용 {1000, 3000} → 4000 − 3000 = 1000.
        assert_eq!(report.expected_additional, AdditionalSize::Known(1_000));

        // UI: local source URL을 버전별로 전달.
        let ui = report.ui_view(&sources);
        assert!(ui
            .versions
            .iter()
            .all(|v| v.source.map(RepositoryUrl::reveal) == Some(C)));
        assert_eq!(ui.versions[1].paths, [path(&["b", "c"])]);

        // telemetry: 숫자만, URL·경로·이름·id·commit 없음.
        let t = report.telemetry();
        assert_eq!(
            t,
            DuplicateWarningTelemetry {
                event: "dependency_duplicate_library_version",
                version_count: 3,
                path_count: 3,
                expected_additional_bytes: 1_000,
                unknown_size_versions: 0,
            }
        );
        let text = format!("{t:?}");
        for needle in [
            "secret-host",
            "shared-tokens",
            "a > c",
            library(3).to_string().as_str(),
            commit(0xc1).as_str(),
        ] {
            assert!(!text.contains(needle), "{needle:?} 노출: {text}");
        }
        // 보고서 Debug도 URL은 담지 않음(keyed identifier만).
        assert!(!format!("{report:?}").contains("secret-host"));
    }

    #[test]
    fn missing_index_entry_gives_lower_bound() {
        let (index, sources) = private(vec![
            index_entry(0xc1, 0x01, 1_000),
            index_entry(0xc2, 0x02, 3_000),
        ]);
        let reports = duplicate_version_reports(&graph(), &index, &FnvKeyer::new(11));
        assert_eq!(
            reports[0].expected_additional,
            AdditionalSize::LowerBound {
                bytes: 1_000,
                unknown_versions: 1
            }
        );
        assert_eq!(reports[0].telemetry().unknown_size_versions, 1);
        // index에 없는 버전도 source는 local config 기준(여기서는 v3의 key가 config에 있음).
        assert!(reports[0].ui_view(&sources).versions[2]
            .size_bytes
            .is_none());
        // 다른 keyer(비밀 key 불일치)로는 크기를 찾지 못함.
        let other = duplicate_version_reports(&graph(), &index, &FnvKeyer::new(12));
        assert_eq!(
            other[0].expected_additional,
            AdditionalSize::LowerBound {
                bytes: 0,
                unknown_versions: 3
            }
        );
    }
}
