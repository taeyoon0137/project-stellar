//! 선택 staging의 필수 동반 원본 단위 계산 (UX.5.4, XIV.14.3, ADR-0080 결정 4, ADR-0127 결정 4).
//!
//! 선택한 원본 단위만 index에 넣으면 끊기는 참조가 있을 때, 그 참조를 잇는 데 필요한 원본 단위를 고정점까지 더합니다.
//! 더한 단위는 자동으로 포함하지 않고 호출자 승인을 받습니다(`PrepareStageError::CompanionRequired`).
//!
//! | 의존 | 끊기는 경우 | 필요한 단위 |
//! |---|---|---|
//! | View instance → 컴포넌트 정의 | 후보 문서의 instance가 가리키는 정의가 후보 컴포넌트 원본에 없음 | 원천(stage: 작업 트리, unstage: HEAD)에서 그 정의를 담은 컴포넌트 파일. 원천에도 없으면 View generation 묶음 |
//! | 컴포넌트 정의 → token | 후보 컴포넌트 원본의 `local-tokens`가 가리키는 token이 후보 token set 원본에 없음 | 원천에서 그 token을 담은 token set 파일(`design/tokens/<slug>.stellar.yaml`). 원천 token set에도 없으면 원천 컴포넌트 파일(원천이 참조를 뺐을 때) |
//!
//! 후보는 "단위에 속한 경로는 원천, 나머지는 기준 index"입니다. 기준 index에서 이미 끊겨 있던 참조는 이 선택이 만든 문제가
//! 아니므로 동반을 요구하지 않습니다. token 참조가 원천에서도 끊겨 있으면(외부 library token 등 프로젝트 token set 밖의
//! token, 또는 작업 트리 자체가 끊긴 상태) 어느 단위로도 이을 수 없으므로 막지 않습니다. 이는 semantic dependency
//! graph(XIV.5.2)가 같은 diff 안에서 추가·제거되는 대상만 의존으로 보는 것과 같은 범위입니다.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use stellar_document::records::{DocumentRecords, NodePayload};
use stellar_git_porcelain::decode::GitPath;
use stellar_project::document_io::component_source::read_project_library;
use stellar_project::document_io::source_v4::token;
use stellar_project::path_resolver::ExternalAccess;
use stellar_project::yaml::loader::MAX_INPUT_BYTES;
use stellar_storage::source_file;

use super::{CompanionReason, PrepareStageError, SourceUnit, COMPONENTS_DIR};
use crate::core_types::ids::StableId;
use crate::core_types::reference::ComponentRef;

/// token set 원본 폴더.
pub(super) const TOKENS_DIR: &str = "design/tokens/";
/// 텍스트 원본 파일 확장자.
const SOURCE_EXT: &str = ".stellar.yaml";

/// 컴포넌트 원본 파일 하나의 정의와 참조 token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ComponentFile {
    /// 정의 ID.
    pub id: StableId,
    /// `local-tokens` token ID.
    pub tokens: BTreeSet<StableId>,
}

/// 저장소 경로 → 컴포넌트 원본.
pub(super) type ComponentFiles = BTreeMap<GitPath, ComponentFile>;
/// 저장소 경로 → token set 원본의 token ID.
pub(super) type TokenFiles = BTreeMap<GitPath, BTreeSet<StableId>>;

/// 문서들이 참조하는 컴포넌트 정의 ID.
pub(super) fn component_refs<'a>(
    records: impl IntoIterator<Item = &'a DocumentRecords>,
) -> BTreeSet<StableId> {
    records
        .into_iter()
        .flat_map(|r| r.nodes.iter())
        .filter_map(|n| match &n.payload {
            NodePayload::ComponentInstance(i) => Some(i.component().id()),
            _ => None,
        })
        .collect()
}

/// 프로젝트 root의 컴포넌트 원본. 폴더가 없으면 빈 목록입니다.
pub(super) fn component_files(root: &Path) -> Result<ComponentFiles, PrepareStageError> {
    let (library, files) = read_project_library(root, ExternalAccess::MetadataOnly)
        .map_err(|e| PrepareStageError::ComponentSource(e.to_string()))?;
    Ok(files
        .into_iter()
        .filter_map(|(id, (path, _))| {
            let name = path.file_name()?.to_str()?;
            let tokens = library
                .get(ComponentRef::new(id))
                .map(|entry| {
                    entry
                        .definition()
                        .local_tokens()
                        .iter()
                        .map(|t| t.id())
                        .collect()
                })
                .unwrap_or_default();
            Some((
                GitPath::from_bytes(format!("{COMPONENTS_DIR}{name}").as_bytes()),
                ComponentFile { id, tokens },
            ))
        })
        .collect())
}

/// 프로젝트 root의 token set 원본(`design/tokens/*.stellar.yaml`). 폴더가 없으면 빈 목록입니다.
///
/// 정규 파일만 읽고(symlink는 따르지 않음) YAML loader 상한을 넘거나 해석하지 못하면 실패합니다(fail closed).
pub(super) fn token_files(root: &Path) -> Result<TokenFiles, PrepareStageError> {
    let invalid = |reason: String| PrepareStageError::TokenSource(reason);
    let dir = root.join(TOKENS_DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(TokenFiles::new()),
        Err(e) => {
            return Err(invalid(format!(
                "token 폴더를 읽지 못했습니다: {:?}",
                e.kind()
            )))
        }
    };
    let mut out = TokenFiles::new();
    for entry in entries {
        let entry = entry.map_err(|e| invalid(format!("token 폴더 항목: {:?}", e.kind())))?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.ends_with(SOURCE_EXT) || name.len() == SOURCE_EXT.len() {
            continue;
        }
        let meta = std::fs::symlink_metadata(entry.path())
            .map_err(|e| invalid(format!("{name}: {:?}", e.kind())))?;
        if !meta.is_file() {
            return Err(invalid(format!("{name}: 정규 파일이 아닙니다")));
        }
        // 텍스트 원본 reader(`source_v4::project::read_dir`)와 같은 제한 읽기: 열린 handle의 종류·크기를 상한까지 검사합니다.
        let bytes = source_file::read(&entry.path(), MAX_INPUT_BYTES as u64)
            .map_err(|e| invalid(format!("{name}: {:?}", e.kind())))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| invalid(format!("{name}: UTF-8이 아닌 텍스트 원본")))?;
        let set = token::decode(&text).map_err(|e| invalid(format!("{name}: {e}")))?;
        out.insert(
            GitPath::from_bytes(format!("{TOKENS_DIR}{name}").as_bytes()),
            set.tokens.iter().map(|t| t.id).collect(),
        );
    }
    Ok(out)
}

/// 후보의 파일 집합: 단위에 속한 경로는 원천, 나머지는 기준 index.
fn candidate<'a, T>(
    index: &'a BTreeMap<GitPath, T>,
    source: &'a BTreeMap<GitPath, T>,
    units: &BTreeSet<SourceUnit>,
) -> impl Iterator<Item = (&'a GitPath, &'a T)> + 'a {
    let covered: BTreeSet<GitPath> = index
        .keys()
        .chain(source.keys())
        .filter(|path| units.iter().any(|u| u.contains(path.as_bytes())))
        .cloned()
        .collect();
    let keep = covered.clone();
    index
        .iter()
        .filter(move |(path, _)| !covered.contains(*path))
        .chain(source.iter().filter(move |(path, _)| keep.contains(*path)))
}

/// 동반 계산 입력.
pub(super) struct CompanionInputs<'a> {
    /// 변경 ID 선택이 있는지(있으면 generation 묶음을 동반으로 더할 수 없음).
    pub has_changes: bool,
    /// 기준 index 문서의 정의 참조.
    pub base_refs: &'a BTreeSet<StableId>,
    /// 선택 적용 문서의 정의 참조.
    pub selection_refs: &'a BTreeSet<StableId>,
    /// 원천 문서의 정의 참조.
    pub source_refs: &'a BTreeSet<StableId>,
    /// 기준 index의 컴포넌트 원본.
    pub index_defs: &'a ComponentFiles,
    /// 원천의 컴포넌트 원본.
    pub source_defs: &'a ComponentFiles,
    /// 기준 index의 token set 원본.
    pub index_tokens: &'a TokenFiles,
    /// 원천의 token set 원본.
    pub source_tokens: &'a TokenFiles,
}

/// 새로 끊기는 참조가 없어질 때까지 필요한 단위를 더한 집합(고정점)과 이유(모듈 문서 표).
pub(super) fn companion_closure(
    inputs: &CompanionInputs<'_>,
    selected: &BTreeSet<SourceUnit>,
) -> Result<(BTreeSet<SourceUnit>, Vec<CompanionReason>), PrepareStageError> {
    let index_def_ids: BTreeSet<StableId> = inputs.index_defs.values().map(|f| f.id).collect();
    let base_dangling: BTreeSet<StableId> = inputs
        .base_refs
        .difference(&index_def_ids)
        .copied()
        .collect();
    let index_token_ids: BTreeSet<StableId> =
        inputs.index_tokens.values().flatten().copied().collect();
    let base_dangling_tokens: BTreeSet<StableId> = inputs
        .index_defs
        .values()
        .flat_map(|f| f.tokens.iter())
        .filter(|t| !index_token_ids.contains(t))
        .copied()
        .collect();
    let mut units = selected.clone();
    let mut reasons = Vec::new();
    loop {
        let added = component_step(inputs, &base_dangling, &mut units, &mut reasons)?
            || token_step(inputs, &base_dangling_tokens, &mut units, &mut reasons);
        if !added {
            break;
        }
    }
    reasons.sort();
    Ok((units, reasons))
}

/// View instance → 컴포넌트 정의. 단위를 더했으면 `true`.
fn component_step(
    inputs: &CompanionInputs<'_>,
    base_dangling: &BTreeSet<StableId>,
    units: &mut BTreeSet<SourceUnit>,
    reasons: &mut Vec<CompanionReason>,
) -> Result<bool, PrepareStageError> {
    let refs = if units.contains(&SourceUnit::Generation) {
        inputs.source_refs
    } else {
        inputs.selection_refs
    };
    let defs: BTreeSet<StableId> = candidate(inputs.index_defs, inputs.source_defs, units)
        .map(|(_, f)| f.id)
        .collect();
    let mut added = false;
    for id in refs.difference(&defs) {
        if base_dangling.contains(id) {
            continue;
        }
        let needed = match inputs.source_defs.iter().find(|(_, def)| def.id == *id) {
            Some((path, _)) => SourceUnit::File(path.clone()),
            // 정의 원본이 원천에 없으면 인스턴스 쪽(View generation 묶음)을 원천과 맞춰야 합니다.
            // stage(작업 트리에서 정의와 인스턴스를 함께 지움)와 unstage 모두 같습니다. 원천에도
            // 인스턴스가 남아 있으면 다음 반복에서 해결할 수 없는 오류가 됩니다.
            None if !inputs.has_changes && !units.contains(&SourceUnit::Generation) => {
                SourceUnit::Generation
            }
            None => return Err(PrepareStageError::DanglingComponent(*id)),
        };
        if units.contains(&needed) {
            return Err(PrepareStageError::DanglingComponent(*id));
        }
        reasons.push(CompanionReason {
            unit: needed.clone(),
            component: *id,
            token: None,
        });
        units.insert(needed);
        added = true;
    }
    Ok(added)
}

/// 컴포넌트 정의 → token. 단위를 하나 더했으면 `true`(후보를 다시 계산하도록 하나씩 더함).
fn token_step(
    inputs: &CompanionInputs<'_>,
    base_dangling: &BTreeSet<StableId>,
    units: &mut BTreeSet<SourceUnit>,
    reasons: &mut Vec<CompanionReason>,
) -> bool {
    let tokens: BTreeSet<StableId> = candidate(inputs.index_tokens, inputs.source_tokens, units)
        .flat_map(|(_, ids)| ids.iter().copied())
        .collect();
    let components: Vec<(GitPath, ComponentFile)> =
        candidate(inputs.index_defs, inputs.source_defs, units)
            .map(|(path, file)| (path.clone(), file.clone()))
            .collect();
    for (path, file) in components {
        for token in &file.tokens {
            if tokens.contains(token) || base_dangling.contains(token) {
                continue;
            }
            let covered = units.iter().any(|u| u.contains(path.as_bytes()));
            let needed = match inputs
                .source_tokens
                .iter()
                .find(|(_, ids)| ids.contains(token))
            {
                Some((token_path, _)) => SourceUnit::File(token_path.clone()),
                // 원천이 그 참조를 뺐으면(token set 삭제와 함께 정의를 고침) 정의 파일을 원천과 맞춥니다.
                None if !covered
                    && inputs
                        .source_defs
                        .get(&path)
                        .is_none_or(|source| !source.tokens.contains(token)) =>
                {
                    SourceUnit::File(path.clone())
                }
                // 원천에서도 끊긴 참조(프로젝트 token set 밖의 token): 어느 단위로도 이을 수 없어 막지 않습니다.
                None => continue,
            };
            if units.contains(&needed) {
                continue;
            }
            reasons.push(CompanionReason {
                unit: needed.clone(),
                component: file.id,
                token: Some(*token),
            });
            units.insert(needed);
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(1_700_000_000_000 + n, 0x14e, n)
    }

    fn path(text: &str) -> GitPath {
        GitPath::from_bytes(text.as_bytes())
    }

    fn file(id: u64, tokens: &[u64]) -> ComponentFile {
        ComponentFile {
            id: sid(id),
            tokens: tokens.iter().map(|t| sid(*t)).collect(),
        }
    }

    fn set(ids: &[u64]) -> BTreeSet<StableId> {
        ids.iter().map(|t| sid(*t)).collect()
    }

    const CARD: &str = "design/components/Card.stellar.yaml";
    const BRAND: &str = "design/tokens/brand.stellar.yaml";

    struct Case {
        index_defs: ComponentFiles,
        source_defs: ComponentFiles,
        index_tokens: TokenFiles,
        source_tokens: TokenFiles,
    }

    impl Case {
        fn run(
            &self,
            selected: &[&str],
        ) -> Result<(BTreeSet<SourceUnit>, Vec<CompanionReason>), PrepareStageError> {
            let empty = BTreeSet::new();
            let inputs = CompanionInputs {
                has_changes: false,
                base_refs: &empty,
                selection_refs: &empty,
                source_refs: &empty,
                index_defs: &self.index_defs,
                source_defs: &self.source_defs,
                index_tokens: &self.index_tokens,
                source_tokens: &self.source_tokens,
            };
            let selected = selected.iter().map(|p| SourceUnit::File(path(p))).collect();
            companion_closure(&inputs, &selected)
        }
    }

    #[test]
    fn new_component_needs_its_new_token_set_and_unstage_needs_the_component() {
        // 작업 트리: Card(token 1 참조)와 brand(token 1)를 새로 만듦. index에는 둘 다 없음.
        let case = Case {
            index_defs: ComponentFiles::new(),
            source_defs: [(path(CARD), file(10, &[1]))].into(),
            index_tokens: TokenFiles::new(),
            source_tokens: [(path(BRAND), set(&[1]))].into(),
        };
        let (units, reasons) = case.run(&[CARD]).unwrap();
        assert_eq!(
            units,
            BTreeSet::from([SourceUnit::File(path(CARD)), SourceUnit::File(path(BRAND))])
        );
        assert_eq!(
            reasons,
            [CompanionReason {
                unit: SourceUnit::File(path(BRAND)),
                component: sid(10),
                token: Some(sid(1)),
            }]
        );
        // token set만 고르면 동반이 없습니다(token은 참조하지 않음).
        assert_eq!(case.run(&[BRAND]).unwrap().1, []);

        // index에 둘 다 있고 HEAD(원천)에는 없음: token set만 빼면 index Card가 끊기므로 Card를 함께 빼야 합니다.
        let unstage = Case {
            index_defs: [(path(CARD), file(10, &[1]))].into(),
            source_defs: ComponentFiles::new(),
            index_tokens: [(path(BRAND), set(&[1]))].into(),
            source_tokens: TokenFiles::new(),
        };
        let (units, reasons) = unstage.run(&[BRAND]).unwrap();
        assert!(units.contains(&SourceUnit::File(path(CARD))));
        assert_eq!(reasons[0].token, Some(sid(1)));
        assert_eq!(reasons[0].unit, SourceUnit::File(path(CARD)));
    }

    #[test]
    fn token_set_removal_needs_component_that_dropped_the_reference() {
        // index: Card(token 1)·brand(token 1). 작업 트리: brand 삭제, Card는 참조를 뺌.
        let case = Case {
            index_defs: [(path(CARD), file(10, &[1]))].into(),
            source_defs: [(path(CARD), file(10, &[]))].into(),
            index_tokens: [(path(BRAND), set(&[1]))].into(),
            source_tokens: TokenFiles::new(),
        };
        let (units, reasons) = case.run(&[BRAND]).unwrap();
        assert!(units.contains(&SourceUnit::File(path(CARD))));
        assert_eq!(reasons.len(), 1);
        // 작업 트리 Card가 아직 참조하면(작업 트리 자체가 끊김) 이을 단위가 없어 막지 않습니다.
        let dangling = Case {
            source_defs: [(path(CARD), file(10, &[1]))].into(),
            ..case
        };
        assert_eq!(dangling.run(&[BRAND]).unwrap().1, []);
    }

    #[test]
    fn tokens_outside_project_sets_and_already_dangling_do_not_block() {
        // Card가 프로젝트 token set 어디에도 없는 token 9(외부 library token)를 참조합니다.
        let external = Case {
            index_defs: ComponentFiles::new(),
            source_defs: [(path(CARD), file(10, &[9]))].into(),
            index_tokens: TokenFiles::new(),
            source_tokens: [(path(BRAND), set(&[1]))].into(),
        };
        assert_eq!(external.run(&[CARD]).unwrap().1, []);
        // index에서 이미 끊겨 있던 token 참조는 이 선택이 만든 문제가 아닙니다.
        let already = Case {
            index_defs: [(path(CARD), file(10, &[1]))].into(),
            source_defs: [(path(CARD), file(10, &[1]))].into(),
            index_tokens: TokenFiles::new(),
            source_tokens: [(path(BRAND), set(&[1]))].into(),
        };
        assert_eq!(already.run(&[CARD]).unwrap().1, []);
    }
}
