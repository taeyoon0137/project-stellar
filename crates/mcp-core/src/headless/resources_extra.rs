//! token·Target Profile·Git 상태 resource (XIX.2.1 정의, XIX.9.3 headless 연결).
//!
//! 내용 schema는 agent-protocol 정의(`tokens`, `target-profiles`, `git-status`)를 따릅니다. 값은 ops 조회
//! (`query::tokens`, snapshot manifest, `git::GitSession::status`)에서만 옵니다.

use stellar_ops::git::StatusRequest;
use stellar_ops::query::tokens;

use super::operations::read_git_status;
use super::{count, revision, uint, CachedRead};
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
    let clean = counts.staged == 0
        && counts.unstaged == 0
        && counts.untracked == 0
        && counts.conflicted == 0;
    Ok(J::object(vec![
        ("revision", revision(cached.snapshot.revision())),
        ("branch", branch),
        ("clean", J::Bool(clean)),
        ("staged", uint(counts.staged)),
        ("unstaged", uint(counts.unstaged)),
        ("conflicted", uint(counts.conflicted)),
    ]))
}
