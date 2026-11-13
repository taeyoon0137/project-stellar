//! 두 source의 비교 기준 — `git merge-base` (XXIII.1.2).
//!
//! 두 [`ResolvedSource`]의 이력 기준 commit에 공통 이력이 있으면 시스템 Git
//! `git merge-base --all <a> <b>`로 base를 구합니다. Stellar가 commit graph를 직접 순회하지 않습니다.
//!
//! | 결과 | 의미 |
//! |---|---|
//! | [`ComparisonMode::ThreeWay`] | merge base가 하나 — 3-way 비교 |
//! | [`ComparisonMode::CrissCross`] | merge base가 둘 이상(criss-cross) — 단일 base를 고르지 않고 명시 |
//! | [`ComparisonMode::TwoWay`] | 공통 이력 없음(`merge-base` 종료 코드 1) 또는 한쪽에 commit 없음 |
//!
//! base 후보는 정렬·중복 제거한 object ID 목록입니다. 종료 코드 0·1 외는 Git 실패로 보존합니다.

use std::path::Path;

use super::source::ResolvedSource;
use crate::object::ObjectId;
use crate::ops::{failure_of, GitOpError, ParseFailure};
use crate::runner::argv;
use crate::trust::GatedGitRunner;

const COMMAND: &str = "merge-base --all";

/// 2-way 비교인 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TwoWayReason {
    /// 두 commit에 공통 조상이 없음.
    NoCommonHistory,
    /// 한쪽 또는 양쪽 source에 이력 기준 commit이 없음(unborn HEAD의 index·작업 트리).
    MissingHistory,
}

/// 두 source의 비교 방식.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComparisonMode {
    /// merge base 하나로 3-way 비교.
    ThreeWay {
        /// merge base commit.
        base: ObjectId,
    },
    /// merge base가 여럿(criss-cross). 단일 base를 고르지 않습니다.
    CrissCross {
        /// base 후보(정렬).
        bases: Vec<ObjectId>,
    },
    /// 공통 base 없이 2-way 비교.
    TwoWay {
        /// 이유.
        reason: TwoWayReason,
    },
}

impl ComparisonMode {
    /// 단일 base(3-way일 때만).
    pub fn base(&self) -> Option<&ObjectId> {
        match self {
            ComparisonMode::ThreeWay { base } => Some(base),
            _ => None,
        }
    }
}

/// `merge-base --all` stdout(줄마다 object ID)을 해석합니다(정렬·중복 제거).
pub fn parse_merge_bases(stdout: &[u8]) -> Result<Vec<ObjectId>, ParseFailure> {
    let body = stdout
        .strip_suffix(b"\n")
        .ok_or_else(|| ParseFailure::new(COMMAND, "LF로 끝나지 않음", stdout))?;
    let mut bases = Vec::new();
    for line in body.split(|b| *b == b'\n') {
        bases.push(
            ObjectId::from_hex_bytes(line)
                .ok_or_else(|| ParseFailure::new(COMMAND, "object ID 아님", line))?,
        );
    }
    bases.sort();
    bases.dedup();
    Ok(bases)
}

/// 두 commit의 비교 방식을 구합니다(`cwd`: 같은 저장소의 worktree).
pub fn comparison_mode_for_commits(
    git: &GatedGitRunner,
    cwd: &Path,
    a: &ObjectId,
    b: &ObjectId,
) -> Result<ComparisonMode, GitOpError> {
    let output = git.run(
        &argv([
            "merge-base",
            "--all",
            "--end-of-options",
            a.as_str(),
            b.as_str(),
        ]),
        cwd,
    )?;
    match output.exit_code {
        Some(0) => {
            let mut bases = parse_merge_bases(&output.stdout)?;
            if bases.len() == 1 {
                Ok(ComparisonMode::ThreeWay {
                    base: bases.remove(0),
                })
            } else {
                Ok(ComparisonMode::CrissCross { bases })
            }
        }
        Some(1) if output.stdout.is_empty() => Ok(ComparisonMode::TwoWay {
            reason: TwoWayReason::NoCommonHistory,
        }),
        _ => Err(GitOpError::Git(failure_of(output))),
    }
}

/// 두 source의 비교 방식을 구합니다. 한쪽이라도 이력 기준 commit이 없으면 Git을 실행하지 않고
/// [`TwoWayReason::MissingHistory`]입니다.
pub fn comparison_mode(
    git: &GatedGitRunner,
    cwd: &Path,
    a: &ResolvedSource,
    b: &ResolvedSource,
) -> Result<ComparisonMode, GitOpError> {
    match (&a.commit, &b.commit) {
        (Some(a), Some(b)) => comparison_mode_for_commits(git, cwd, a, b),
        _ => Ok(ComparisonMode::TwoWay {
            reason: TwoWayReason::MissingHistory,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composer::source::{resolve_source, SourceRef};
    use crate::fixture::{build_standard_scenario, GitFixture};

    #[test]
    fn parses_sorted_unique_bases() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let out = format!("{b}\n{a}\n{b}\n");
        assert_eq!(
            parse_merge_bases(out.as_bytes()).unwrap(),
            vec![ObjectId::parse(&a).unwrap(), ObjectId::parse(&b).unwrap()]
        );
        for bad in [&b""[..], b"\n", b"xyz\n", b"aaaa\n"] {
            assert!(parse_merge_bases(bad).is_err(), "{bad:?}");
        }
        assert!(parse_merge_bases(a.as_bytes()).is_err());
    }

    #[test]
    fn missing_history_is_two_way_without_spawn() {
        use crate::runner::GitRunner;
        use crate::snapshot::SnapshotSource;
        use crate::trust::GitTrust;
        let runner = GitRunner::new(
            std::path::PathBuf::from("/nonexistent/git"),
            Vec::new(),
            std::time::Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
        let unborn = ResolvedSource {
            source: SourceRef::Index,
            worktree: std::env::temp_dir(),
            snapshot: SnapshotSource::Index,
            commit: None,
        };
        let other = ResolvedSource {
            commit: ObjectId::parse(&"a".repeat(40)),
            ..unborn.clone()
        };
        for (a, b) in [(&unborn, &other), (&other, &unborn), (&unborn, &unborn)] {
            assert_eq!(
                comparison_mode(&gate, &std::env::temp_dir(), a, b),
                Ok(ComparisonMode::TwoWay {
                    reason: TwoWayReason::MissingHistory
                })
            );
        }
        assert_eq!(runner.spawn_count(), 0);
    }

    /// related/unrelated fixture.
    #[test]
    fn related_and_unrelated_sources() {
        let Some(fx) = GitFixture::with_system_git("composer-base").unwrap() else {
            return; // git 미설치 환경 건너뜀.
        };
        let s = build_standard_scenario(&fx).unwrap();
        // 공통 이력이 없는 orphan branch.
        // `switch --orphan`은 추적 파일을 작업 트리·index에서 지웁니다.
        fx.git(["switch", "-q", "--orphan", "unrelated"]).unwrap();
        fx.write("other.txt", b"other\n").unwrap();
        let orphan = fx.commit_all("orphan").unwrap();
        fx.git(["switch", "-q", "-f", "main"]).unwrap();
        // 작업 트리 source(이력 기준 = HEAD = main).
        let gate = fx.gated();
        let work = fx.work();
        let src = |r: SourceRef| resolve_source(&gate, work, &r).unwrap();
        let mode = |a: SourceRef, b: SourceRef| comparison_mode(&gate, work, &src(a), &src(b));
        let three = |base: &ObjectId| Ok(ComparisonMode::ThreeWay { base: base.clone() });

        // related: 분기한 branch → 분기점.
        assert_eq!(
            mode(
                SourceRef::Branch("main".into()),
                SourceRef::Branch("feature".into())
            ),
            three(&s.initial)
        );
        assert_eq!(
            mode(
                SourceRef::Branch("feature".into()),
                SourceRef::Tag("v1".into())
            ),
            three(&s.initial)
        );
        // 조상 관계 → 조상 자신, 같은 commit → 그 commit.
        assert_eq!(
            mode(SourceRef::Commit(s.initial.clone()), SourceRef::Head),
            three(&s.initial)
        );
        assert_eq!(
            mode(SourceRef::Head, SourceRef::Tag("v1".into())),
            three(&s.main)
        );
        // 작업 트리·index source는 그 worktree HEAD 기준.
        assert_eq!(
            mode(SourceRef::WorkingTree, SourceRef::Branch("feature".into())),
            three(&s.initial)
        );
        assert_eq!(
            mode(SourceRef::Index, SourceRef::WorkingTree),
            three(&s.main)
        );
        // unrelated: 공통 이력 없음 → 2-way 명시.
        let two_way = ComparisonMode::TwoWay {
            reason: TwoWayReason::NoCommonHistory,
        };
        assert_eq!(
            mode(
                SourceRef::Branch("unrelated".into()),
                SourceRef::Branch("main".into())
            ),
            Ok(two_way.clone())
        );
        assert_eq!(
            comparison_mode_for_commits(&gate, work, &orphan, &s.feature),
            Ok(two_way.clone())
        );
        let three_way = ComparisonMode::ThreeWay {
            base: s.initial.clone(),
        };
        assert_eq!(three_way.base(), Some(&s.initial));
        assert_eq!(two_way.base(), None);
    }

    #[test]
    fn criss_cross_history_reports_every_base() {
        let Some(fx) = GitFixture::with_system_git("composer-criss").unwrap() else {
            return;
        };
        let s = build_standard_scenario(&fx).unwrap();
        fx.git(["switch", "-q", "-c", "left", s.initial.as_str()])
            .unwrap();
        fx.write("left.txt", b"l\n").unwrap();
        let a1 = fx.commit_all("left 1").unwrap();
        fx.git(["switch", "-q", "-c", "right", s.initial.as_str()])
            .unwrap();
        fx.write("right.txt", b"r\n").unwrap();
        let b1 = fx.commit_all("right 1").unwrap();
        // left ← right 1, right ← left 1을 서로 병합(criss-cross).
        fx.git(["switch", "-q", "left"]).unwrap();
        fx.git(["merge", "-q", "--no-ff", "-m", "m1", b1.as_str()])
            .unwrap();
        let m1 = fx.rev_parse("HEAD").unwrap();
        fx.git(["switch", "-q", "right"]).unwrap();
        fx.git(["merge", "-q", "--no-ff", "-m", "m2", a1.as_str()])
            .unwrap();
        let m2 = fx.rev_parse("HEAD").unwrap();

        let mut expected = vec![a1, b1];
        expected.sort();
        assert_eq!(
            comparison_mode_for_commits(&fx.gated(), fx.work(), &m1, &m2),
            Ok(ComparisonMode::CrissCross { bases: expected })
        );
    }

    #[test]
    fn git_failure_is_preserved() {
        let Some(fx) = GitFixture::with_system_git("composer-base-fail").unwrap() else {
            return;
        };
        fx.write("a.txt", b"a\n").unwrap();
        let head = fx.commit_all("a").unwrap();
        let missing = ObjectId::parse(&"1".repeat(40)).unwrap();
        assert!(matches!(
            comparison_mode_for_commits(&fx.gated(), fx.work(), &head, &missing),
            Err(GitOpError::Git(_))
        ));
    }
}
