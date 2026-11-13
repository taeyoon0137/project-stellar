//! fetch와 진행률 (XXVI.1.2).
//!
//! 등록된 remote만 `git fetch --progress --no-recurse-submodules -- <remote>`로 가져옵니다. Git은 진행률을
//! stderr에 `\r`로 덮어쓰는 줄(`Receiving objects:  50% (2/4)`, `remote: Counting objects: 100% (3/3), done.`)로
//! 쓰므로, stderr를 도착하는 대로 받아([`GatedGitRunner::run_streaming_stderr`]) [`ProgressParser`]로
//! [`FetchProgress`] 이벤트를 만들어 호출자 콜백에 넘깁니다. 진행률 문구는 해석 실패해도 fetch 결과에
//! 영향을 주지 않습니다(표시 전용).
//!
//! 결과는 fetch 전후의 `refs/remotes/<remote>/`·`refs/tags/` 비교([`FetchOutcome::updated`])로 만듭니다.
//! remote-tracking ref를 지우는 `--prune`은 제공하지 않습니다. 취소는 [`GitCancelToken`]으로 하며, 취소된
//! fetch는 성공으로 보고하지 않습니다. 인증은 Git에 위임합니다(XIV.9.6).

use std::collections::BTreeMap;
use std::path::Path;

use crate::cancel::GitCancelToken;
use crate::object::ObjectId;
use crate::ops::{run_ok, GitOpError};
use crate::remote::{list_remotes, redacted_failure, RemoteError};
use crate::runner::argv;
use crate::trust::GatedGitRunner;

/// 진행률 한 갱신.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchProgress {
    /// 원격 쪽(`remote: `) 진행률인지.
    pub remote_side: bool,
    /// 단계 이름(Git 문구 그대로, 예: `Receiving objects`).
    pub phase: String,
    /// 백분율.
    pub percent: Option<u8>,
    /// 처리한 수.
    pub done: Option<u64>,
    /// 전체 수.
    pub total: Option<u64>,
}

/// 진행률 segment 하나(`\r`·`\n` 사이)를 해석합니다.
pub fn parse_progress_segment(segment: &str) -> Option<FetchProgress> {
    let text = segment.trim();
    let (remote_side, text) = match text.strip_prefix("remote: ") {
        Some(rest) => (true, rest.trim()),
        None => (false, text),
    };
    let (phase, rest) = text.split_once(": ")?;
    if phase.is_empty() {
        return None;
    }
    let rest = rest.trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let after = &rest[digits.len()..];
    let (percent, done, total) = if let Some(after) = after.strip_prefix('%') {
        let percent: u8 = digits.parse().ok().filter(|p: &u8| *p <= 100)?;
        let counts = after
            .split_once('(')
            .and_then(|(_, r)| r.split_once(')'))
            .and_then(|(inner, _)| inner.split_once('/'))
            .and_then(|(d, t)| Some((d.trim().parse().ok()?, t.trim().parse().ok()?)));
        (Some(percent), counts.map(|c| c.0), counts.map(|c| c.1))
    } else {
        (None, digits.parse().ok(), None)
    };
    Some(FetchProgress {
        remote_side,
        phase: phase.to_string(),
        percent,
        done,
        total,
    })
}

/// stderr 조각을 받아 진행률 이벤트로 바꾸는 상태 기계(조각 경계를 넘는 줄 보존).
#[derive(Debug, Default)]
pub struct ProgressParser {
    pending: Vec<u8>,
}

impl ProgressParser {
    /// 새 parser.
    pub fn new() -> ProgressParser {
        ProgressParser::default()
    }

    /// 조각을 넣고 완결된 segment의 진행률을 돌려줍니다.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<FetchProgress> {
        self.pending.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(i) = self.pending.iter().position(|b| *b == b'\r' || *b == b'\n') {
            let segment: Vec<u8> = self.pending.drain(..=i).collect();
            if let Some(p) = parse_progress_segment(&String::from_utf8_lossy(&segment)) {
                out.push(p);
            }
        }
        out
    }

    /// 남은 segment를 마무리합니다.
    pub fn finish(&mut self) -> Vec<FetchProgress> {
        let rest = std::mem::take(&mut self.pending);
        parse_progress_segment(&String::from_utf8_lossy(&rest))
            .into_iter()
            .collect()
    }
}

/// ref 하나의 변화.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefUpdate {
    /// 전체 ref 이름.
    pub name: String,
    /// 이전 값(새 ref면 `None`).
    pub old: Option<ObjectId>,
    /// 새 값(사라졌으면 `None`).
    pub new: Option<ObjectId>,
}

/// fetch 결과.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FetchOutcome {
    /// 바뀐 remote-tracking·tag ref(이름 순).
    pub updated: Vec<RefUpdate>,
}

/// fetch 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// 등록되지 않은 remote(fetch 미실행).
    UnknownRemote {
        /// 요청한 이름.
        name: String,
    },
    /// Git 실행·취소·비영 종료(자격 증명을 가린 stderr).
    Remote(RemoteError),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::UnknownRemote { name } => write!(f, "등록되지 않은 remote: {name}"),
            FetchError::Remote(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for FetchError {}

impl From<RemoteError> for FetchError {
    fn from(e: RemoteError) -> FetchError {
        FetchError::Remote(e)
    }
}

/// remote-tracking·tag ref 값(이름 → ID).
pub(crate) fn tracking_refs(
    git: &GatedGitRunner,
    worktree: &Path,
    remote: &str,
) -> Result<BTreeMap<String, ObjectId>, GitOpError> {
    let prefix = format!("refs/remotes/{remote}/");
    let out = run_ok(
        git,
        worktree,
        &argv([
            "for-each-ref",
            "--format=%(refname)%00%(objectname)",
            prefix.as_str(),
            "refs/tags/",
        ]),
    )?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut refs = BTreeMap::new();
    for line in text.lines() {
        if let Some((name, oid)) = line.split_once('\0') {
            if let Some(oid) = ObjectId::parse(oid) {
                refs.insert(name.to_string(), oid);
            }
        }
    }
    Ok(refs)
}

fn diff_refs(
    before: &BTreeMap<String, ObjectId>,
    after: &BTreeMap<String, ObjectId>,
) -> Vec<RefUpdate> {
    let mut names: Vec<&String> = before.keys().chain(after.keys()).collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter_map(|name| {
            let (old, new) = (before.get(name), after.get(name));
            (old != new).then(|| RefUpdate {
                name: name.clone(),
                old: old.cloned(),
                new: new.cloned(),
            })
        })
        .collect()
}

/// 등록된 remote에서 fetch하며 진행률을 콜백으로 전달합니다.
pub fn fetch_remote(
    git: &GatedGitRunner,
    worktree: &Path,
    remote: &str,
    cancel: &GitCancelToken,
    on_progress: &mut dyn FnMut(&FetchProgress),
) -> Result<FetchOutcome, FetchError> {
    let remotes = list_remotes(git, worktree).map_err(RemoteError::from)?;
    if !remotes.iter().any(|r| r.name == remote) {
        return Err(FetchError::UnknownRemote {
            name: remote.to_string(),
        });
    }
    let before = tracking_refs(git, worktree, remote).map_err(RemoteError::from)?;
    let mut parser = ProgressParser::new();
    let output = git
        .run_streaming_stderr(
            &argv([
                "fetch",
                "--progress",
                "--no-recurse-submodules",
                "--",
                remote,
            ]),
            worktree,
            cancel,
            &mut |chunk| {
                for p in parser.feed(chunk) {
                    on_progress(&p);
                }
            },
        )
        .map_err(RemoteError::Gate)?;
    for p in parser.finish() {
        on_progress(&p);
    }
    if !output.success() {
        return Err(FetchError::Remote(redacted_failure(&output)));
    }
    let after = tracking_refs(git, worktree, remote).map_err(RemoteError::from)?;
    Ok(FetchOutcome {
        updated: diff_refs(&before, &after),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_segments_parse() {
        let p =
            parse_progress_segment("Receiving objects:  50% (2/4), 1.00 KiB | 1.00 MiB/s").unwrap();
        assert_eq!(
            p,
            FetchProgress {
                remote_side: false,
                phase: "Receiving objects".into(),
                percent: Some(50),
                done: Some(2),
                total: Some(4),
            }
        );
        let p = parse_progress_segment("remote: Counting objects: 100% (3/3), done.").unwrap();
        assert!(p.remote_side);
        assert_eq!((p.percent, p.done, p.total), (Some(100), Some(3), Some(3)));
        let p = parse_progress_segment("remote: Enumerating objects: 5, done.").unwrap();
        assert_eq!((p.percent, p.done, p.total), (None, Some(5), None));
        for not_progress in [
            "From /srv/remote",
            " * [new branch]      main       -> origin/main",
            "fatal: could not read from remote",
            "",
            "x: 101% (1/1)",
        ] {
            assert_eq!(
                parse_progress_segment(not_progress),
                None,
                "{not_progress:?}"
            );
        }
    }

    #[test]
    fn parser_handles_carriage_returns_across_chunks() {
        let mut parser = ProgressParser::new();
        let mut events = parser.feed(b"Receiving objects:  33% (1/3)\rReceiving obj");
        events.extend(parser.feed(b"ects:  66% (2/3)\rReceiving objects: 100% (3/3), done.\n"));
        events.extend(parser.feed(b"Resolving deltas: 100% (1/1)"));
        events.extend(parser.finish());
        let percents: Vec<(String, Option<u8>)> =
            events.into_iter().map(|e| (e.phase, e.percent)).collect();
        assert_eq!(
            percents,
            vec![
                ("Receiving objects".to_string(), Some(33)),
                ("Receiving objects".to_string(), Some(66)),
                ("Receiving objects".to_string(), Some(100)),
                ("Resolving deltas".to_string(), Some(100)),
            ]
        );
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::test_support::{
            commit_file, git_ok, hermetic_runner, init_bare, init_repo_with_identity, system_git,
            TempDir,
        };
        use crate::trust::GitTrust;

        #[test]
        fn local_remote_fetch_reports_progress_and_ref_updates() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("adv-fetch");
            let runner = hermetic_runner(git);
            let bare = dir.path().join("remote.git");
            init_bare(&runner, &bare);
            let upstream = dir.path().join("upstream");
            std::fs::create_dir_all(&upstream).unwrap();
            init_repo_with_identity(&runner, &upstream);
            for i in 0..5 {
                commit_file(
                    &runner,
                    &upstream,
                    &format!("design/c{i}.starchunk"),
                    &vec![i; 4096],
                );
            }
            git_ok(
                &runner,
                &upstream,
                ["push", "-q", bare.to_str().unwrap(), "main"],
            );
            git_ok(&runner, &upstream, ["tag", "v1"]);
            git_ok(
                &runner,
                &upstream,
                ["push", "-q", bare.to_str().unwrap(), "v1"],
            );

            let work = dir.path().join("work");
            std::fs::create_dir_all(&work).unwrap();
            init_repo_with_identity(&runner, &work);
            git_ok(
                &runner,
                &work,
                ["remote", "add", "origin", bare.to_str().unwrap()],
            );
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);

            let mut events = Vec::new();
            let outcome = fetch_remote(&gate, &work, "origin", &GitCancelToken::new(), &mut |p| {
                events.push(p.clone())
            })
            .unwrap();
            let names: Vec<&str> = outcome.updated.iter().map(|u| u.name.as_str()).collect();
            // Git 버전에 따라 `refs/remotes/origin/HEAD`(remote HEAD 추적)도 생깁니다.
            assert!(names.contains(&"refs/remotes/origin/main"), "{names:?}");
            assert!(names.contains(&"refs/tags/v1"), "{names:?}");
            assert!(
                names
                    .iter()
                    .all(|n| n.starts_with("refs/remotes/origin/") || n.starts_with("refs/tags/")),
                "{names:?}"
            );
            assert!(outcome
                .updated
                .iter()
                .all(|u| u.old.is_none() && u.new.is_some()));
            assert!(!events.is_empty(), "진행률 이벤트 없음");
            assert!(
                events
                    .iter()
                    .any(|e| e.percent == Some(100) || e.done.is_some()),
                "{events:?}"
            );

            // 변화 없으면 갱신 0.
            let again =
                fetch_remote(&gate, &work, "origin", &GitCancelToken::new(), &mut |_| {}).unwrap();
            assert!(again.updated.is_empty());

            // 새 commit → old→new.
            commit_file(&runner, &upstream, "design/c9.starchunk", b"new");
            git_ok(
                &runner,
                &upstream,
                ["push", "-q", bare.to_str().unwrap(), "main"],
            );
            let next =
                fetch_remote(&gate, &work, "origin", &GitCancelToken::new(), &mut |_| {}).unwrap();
            let main = next
                .updated
                .iter()
                .find(|u| u.name == "refs/remotes/origin/main")
                .unwrap();
            assert!(main.old.is_some() && main.new.is_some() && main.new != main.old);
            // 바뀐 것은 main을 가리키는 ref뿐(tag 등 그대로).
            assert!(next
                .updated
                .iter()
                .all(|u| u.name.starts_with("refs/remotes/origin/")));
        }

        #[test]
        fn unknown_remote_and_cancelled_fetch_are_not_success() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("adv-fetch-cancel");
            let runner = hermetic_runner(git);
            let bare = dir.path().join("remote.git");
            init_bare(&runner, &bare);
            let work = dir.path().join("work");
            std::fs::create_dir_all(&work).unwrap();
            init_repo_with_identity(&runner, &work);
            git_ok(
                &runner,
                &work,
                ["remote", "add", "origin", bare.to_str().unwrap()],
            );
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            assert!(matches!(
                fetch_remote(&gate, &work, "nope", &GitCancelToken::new(), &mut |_| {}),
                Err(FetchError::UnknownRemote { .. })
            ));
            let cancel = GitCancelToken::new();
            cancel.cancel();
            assert!(matches!(
                fetch_remote(&gate, &work, "origin", &cancel, &mut |_| {}),
                Err(FetchError::Remote(RemoteError::Gate(
                    crate::trust::GatedRunError::Run(crate::runner::GitRunError::Cancelled)
                )))
            ));
        }
    }
}
