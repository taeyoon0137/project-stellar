//! token·Target Profile·Git 상태 resource (XIX.2.1 정의, XIX.9.3 headless 연결).
//!
//! 내용 schema는 agent-protocol 정의(`tokens`, `target-profiles`, `git-status`)를 따릅니다. 값은 ops 조회
//! (`query::tokens`, snapshot manifest, `git::GitSession::status`)에서만 옵니다.

use stellar_ops::git::StatusRequest;
use stellar_ops::query::tokens;

use super::operations::read_git_status;
use super::{count, fail, revision, uint, CachedRead, Kind};
use crate::json::JsonValue as J;
use crate::jsonrpc::RpcError;

/// `stellar://tokens`: generation token chunk 수(저장 포맷 미정이라 항목은 비어 있음).
pub(super) fn read_tokens(cached: &CachedRead) -> J {
    let set = tokens(&cached.snapshot);
    J::object(vec![
        ("revision", revision(set.revision)),
        ("chunk_count", count(set.chunk_count)),
        (
            "entries",
            J::Array(set.entries.iter().map(J::string).collect()),
        ),
    ])
}

/// `stellar://target-profiles`: `stellar.yaml`의 Target Profile(저장 순서).
pub(super) fn read_target_profiles(cached: &CachedRead) -> J {
    let profiles = cached
        .snapshot
        .manifest()
        .target_profiles
        .iter()
        .map(|p| {
            J::object(vec![
                ("id", J::string(&p.id)),
                ("platform", J::string(&p.platform)),
                ("minimum_os", J::opt_string(p.minimum_os.as_deref())),
                ("baseline", J::opt_string(p.baseline.as_deref())),
            ])
        })
        .collect();
    J::object(vec![
        ("revision", revision(cached.snapshot.revision())),
        ("profiles", J::Array(profiles)),
    ])
}

/// `stellar://git/status`: 작업 트리 Git 상태 요약(경로·URL 없음). index를 바꾸지 않습니다.
///
/// 정의(XIX.2.1)대로 `staged`·`unstaged`는 semantic 변경 수(`git.status`의 `semantic.*_total`과 같은 값)이고,
/// `conflicted`는 conflict 경로 수, `clean`은 작업 트리 전체(추적 안 함 포함)가 깨끗한지입니다. semantic 상태를
/// 읽지 못하면 경로 수로 대신하지 않고 `unavailable`입니다.
pub(super) fn read_git_summary(cached: &CachedRead) -> Result<J, RpcError> {
    let report = read_git_status(
        cached,
        StatusRequest {
            cursor: None,
            limit: 1,
        },
    )?;
    let counts = &report.repository.counts;
    let branch = match &report.repository.head {
        stellar_ops::git::HeadState::Branch { name, .. }
        | stellar_ops::git::HeadState::Unborn { branch: name } => {
            J::string(String::from_utf8_lossy(name).into_owned())
        }
        stellar_ops::git::HeadState::Detached { .. } => J::Null,
    };
    let semantic = report.semantic.as_ref().map_err(|_| {
        fail(
            Kind::Unavailable,
            "semantic 변경 상태를 읽을 수 없습니다. git.status tool의 진단을 확인하세요",
        )
    })?;
    let clean = counts.staged == 0
        && counts.unstaged == 0
        && counts.untracked == 0
        && counts.conflicted == 0;
    Ok(J::object(vec![
        ("revision", revision(cached.snapshot.revision())),
        ("branch", branch),
        ("clean", J::Bool(clean)),
        ("staged", count(semantic.staged_total)),
        ("unstaged", count(semantic.unstaged_total)),
        ("conflicted", uint(counts.conflicted)),
    ]))
}
