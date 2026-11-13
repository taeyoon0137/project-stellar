//! 기존 remote로 push (XIV.9.5).
//!
//! 사용자가 선택한 **이미 등록된 remote**와 refspec으로 시스템 Git `git push`를 실행합니다.
//!
//! - **force push API 없음**: [`PushRequest`]에는 force·force-with-lease·mirror·delete 옵션이 없고,
//!   [`PushRefspec`]은 force 표기(`+`), 삭제 표기(빈 src, `:dst`), wildcard(`*`)를 거부합니다.
//!   non-fast-forward는 Git이 거부하며 그 결과를 그대로 돌려줍니다([`PushError::Rejected`]).
//! - remote 이름은 `git remote -v`에 있는 것만 받습니다(URL 직접 push 불가).
//! - hook(`pre-push`)은 Git이 그대로 실행하며 우회 옵션(`--no-verify`)을 쓰지 않습니다.
//! - 결과는 `--porcelain` 출력(`<flag> TAB <from>:<to> TAB <summary>`)으로 ref별로 해석합니다.
//!   `To <url>` 줄은 민감값(URL)이므로 보존하지 않습니다. 실패 stderr는 자격 증명을 가린 텍스트만 남깁니다.
//! - 인증은 Git(credential helper·SSH agent)에 위임합니다(XIV.9.6).

use std::path::Path;

use super::{list_remotes, redact_credentials, redacted_failure, RemoteError};
use crate::repo_status::{read_repository_status, RepositoryStatus};
use crate::runner::argv;
use crate::trust::GatedGitRunner;

/// push할 ref 쌍(`<src>:<dst>`, force·삭제·wildcard 불가).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushRefspec {
    src: String,
    dst: String,
}

/// refspec 거부 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefspecProblem {
    /// `+`로 시작(force).
    Force,
    /// 빈 src(원격 ref 삭제).
    Delete,
    /// `refs/heads/`·`refs/tags/` 밖의 ref 또는 이름 규칙 위반.
    InvalidRef,
}

fn valid_ref(name: &str) -> bool {
    let body = name
        .strip_prefix("refs/heads/")
        .or_else(|| name.strip_prefix("refs/tags/"));
    let Some(body) = body else {
        return false;
    };
    !body.is_empty()
        && !body.starts_with('/')
        && !body.ends_with('/')
        && !body.ends_with('.')
        && !body.ends_with(".lock")
        && !body.contains("..")
        && !body.contains("//")
        && !body.contains("@{")
        && !body.split('/').any(|c| c.starts_with('.'))
        && !body.chars().any(|c| {
            c.is_control()
                || c.is_whitespace()
                || matches!(c, ':' | '~' | '^' | '?' | '*' | '[' | '\\')
        })
}

impl PushRefspec {
    /// `src:dst`로 만듭니다. 둘 다 `refs/heads/…` 또는 `refs/tags/…` 전체 이름이어야 합니다.
    pub fn new(src: &str, dst: &str) -> Result<PushRefspec, RefspecProblem> {
        if src.starts_with('+') {
            return Err(RefspecProblem::Force);
        }
        if src.is_empty() {
            return Err(RefspecProblem::Delete);
        }
        if !valid_ref(src) || !valid_ref(dst) {
            return Err(RefspecProblem::InvalidRef);
        }
        Ok(PushRefspec {
            src: src.to_string(),
            dst: dst.to_string(),
        })
    }

    /// 같은 이름의 branch를 push하는 refspec(`refs/heads/<b>:refs/heads/<b>`).
    pub fn branch(name: &str) -> Result<PushRefspec, RefspecProblem> {
        let full = format!("refs/heads/{name}");
        PushRefspec::new(&full, &full)
    }

    /// Git 인자 형식(`src:dst`).
    pub fn as_arg(&self) -> String {
        format!("{}:{}", self.src, self.dst)
    }

    /// src ref.
    pub fn src(&self) -> &str {
        &self.src
    }

    /// dst ref.
    pub fn dst(&self) -> &str {
        &self.dst
    }
}

/// push 요청(force 관련 옵션 없음).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushRequest {
    /// 등록된 remote 이름.
    pub remote: String,
    /// refspec.
    pub refspec: PushRefspec,
    /// 성공 시 upstream 설정(`--set-upstream`).
    pub set_upstream: bool,
}

/// ref 갱신 결과 flag(`git push --porcelain`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefUpdateFlag {
    /// ` ` fast-forward.
    FastForward,
    /// `+` 강제 갱신(이 API는 요청하지 않지만 Git 결과를 그대로 표현).
    Forced,
    /// `-` 삭제.
    Deleted,
    /// `*` 새 ref.
    New,
    /// `!` 거부.
    Rejected,
    /// `=` 이미 최신.
    UpToDate,
}

impl RefUpdateFlag {
    fn from_byte(b: u8) -> Option<RefUpdateFlag> {
        Some(match b {
            b' ' => RefUpdateFlag::FastForward,
            b'+' => RefUpdateFlag::Forced,
            b'-' => RefUpdateFlag::Deleted,
            b'*' => RefUpdateFlag::New,
            b'!' => RefUpdateFlag::Rejected,
            b'=' => RefUpdateFlag::UpToDate,
            _ => return None,
        })
    }
}

/// ref별 push 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushedRef {
    /// flag.
    pub flag: RefUpdateFlag,
    /// local ref.
    pub from: String,
    /// remote ref.
    pub to: String,
    /// Git 요약(`[new branch]`, `abc..def`, `[rejected] (non-fast-forward)` 등).
    pub summary: String,
}

/// push 성공 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushOutcome {
    /// ref별 결과.
    pub refs: Vec<PushedRef>,
}

/// push 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushError {
    /// 등록되지 않은 remote(Git push 미실행).
    UnknownRemote {
        /// 요청한 이름.
        name: String,
    },
    /// Git이 ref 갱신을 거부함(non-fast-forward·hook 실패 등, 결과 그대로).
    Rejected {
        /// ref별 결과(거부 포함).
        refs: Vec<PushedRef>,
        /// 자격 증명을 가린 stderr.
        stderr_redacted: String,
    },
    /// Git 실행·해석 실패.
    Remote(RemoteError),
}

impl std::fmt::Display for PushError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PushError::UnknownRemote { name } => write!(f, "등록되지 않은 remote: {name}"),
            PushError::Rejected {
                refs,
                stderr_redacted,
            } => {
                let rejected: Vec<String> = refs
                    .iter()
                    .filter(|r| r.flag == RefUpdateFlag::Rejected)
                    .map(|r| format!("{} {}", r.to, r.summary))
                    .collect();
                write!(
                    f,
                    "push가 거부되었습니다: {} {}",
                    rejected.join(", "),
                    stderr_redacted.trim()
                )
            }
            PushError::Remote(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PushError {}

impl From<RemoteError> for PushError {
    fn from(e: RemoteError) -> PushError {
        PushError::Remote(e)
    }
}

/// `git push --porcelain` stdout에서 ref 결과 줄을 해석합니다(`To <url>`·`Done` 줄은 무시).
pub fn parse_push_porcelain(stdout: &[u8]) -> Result<Vec<PushedRef>, RemoteError> {
    let text = String::from_utf8_lossy(stdout);
    let bad = |detail: &str| RemoteError::Parse {
        command: "push --porcelain",
        detail: detail.to_string(),
    };
    let mut refs = Vec::new();
    for line in text.lines() {
        let bytes = line.as_bytes();
        // ref 결과 줄은 `<flag> TAB …` 형식입니다. `To <url>`, `Done`, upstream 설정 안내 같은
        // 정보 줄은 건너뜁니다(URL이 담길 수 있어 보존하지 않음).
        if bytes.get(1) != Some(&b'\t') {
            continue;
        }
        let flag = RefUpdateFlag::from_byte(bytes[0]).ok_or_else(|| bad("알 수 없는 flag"))?;
        let rest = line[1..]
            .strip_prefix('\t')
            .ok_or_else(|| bad("flag 뒤 TAB 없음"))?;
        let (pair, summary) = rest.split_once('\t').ok_or_else(|| bad("요약 TAB 없음"))?;
        let (from, to) = pair.split_once(':').ok_or_else(|| bad("from:to 아님"))?;
        refs.push(PushedRef {
            flag,
            from: from.to_string(),
            to: to.to_string(),
            summary: summary.to_string(),
        });
    }
    Ok(refs)
}

/// 등록된 remote로 push합니다(force 없음, hook 우회 없음).
pub fn push(
    git: &GatedGitRunner,
    worktree: &Path,
    request: &PushRequest,
) -> Result<PushOutcome, PushError> {
    let remotes = list_remotes(git, worktree).map_err(RemoteError::from)?;
    if !remotes.iter().any(|r| r.name == request.remote) {
        return Err(PushError::UnknownRemote {
            name: request.remote.clone(),
        });
    }
    let mut args = argv(["push", "--porcelain"]);
    if request.set_upstream {
        args.extend(argv(["--set-upstream"]));
    }
    let refspec = request.refspec.as_arg();
    args.extend(argv(["--", request.remote.as_str(), refspec.as_str()]));
    let output = git.run(&args, worktree).map_err(RemoteError::Gate)?;
    let refs = parse_push_porcelain(&output.stdout)?;
    if output.success() {
        return Ok(PushOutcome { refs });
    }
    if refs.is_empty() {
        return Err(PushError::Remote(redacted_failure(&output)));
    }
    Err(PushError::Rejected {
        refs,
        stderr_redacted: redact_credentials(&String::from_utf8_lossy(&output.stderr)),
    })
}

/// push 성공 후 다시 읽은 저장소 상태(XIV.9.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushReport {
    /// ref별 push 결과.
    pub outcome: PushOutcome,
    /// push 직후 status(remote tracking ahead/behind 갱신 반영).
    pub status: RepositoryStatus,
}

/// push 후 remote tracking 상태를 다시 읽습니다(XIV.9.7).
///
/// 성공하면 [`read_repository_status`]로 upstream ahead/behind를 갱신해 돌려줍니다. 실패하면 오류만
/// 돌려주고 **그 밖의 Git 명령을 실행하지 않습니다**(fetch·pull·reset·rollback 없음). 그래서 거부된 push가
/// local commit·index·ref를 바꾸지 않습니다(Git push 자체도 local 상태를 바꾸지 않음).
pub fn push_and_refresh(
    git: &GatedGitRunner,
    worktree: &Path,
    request: &PushRequest,
) -> Result<PushReport, PushError> {
    let outcome = push(git, worktree, request)?;
    let status = read_repository_status(git, worktree).map_err(RemoteError::from)?;
    Ok(PushReport { outcome, status })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refspec_rejects_force_delete_and_wildcards() {
        assert_eq!(
            PushRefspec::new("+refs/heads/main", "refs/heads/main"),
            Err(RefspecProblem::Force)
        );
        assert_eq!(
            PushRefspec::new("", "refs/heads/main"),
            Err(RefspecProblem::Delete)
        );
        for (src, dst) in [
            ("refs/heads/*", "refs/heads/*"),
            ("main", "refs/heads/main"),
            ("refs/heads/main", "main"),
            ("refs/heads/a:b", "refs/heads/a"),
            ("refs/heads/a..b", "refs/heads/a"),
            ("refs/heads/a", "refs/remotes/origin/a"),
            ("refs/heads/.hidden", "refs/heads/x"),
            ("refs/heads/x.lock", "refs/heads/x"),
            ("refs/heads/a b", "refs/heads/a"),
            ("refs/heads/", "refs/heads/"),
            ("-refs/heads/a", "refs/heads/a"),
        ] {
            assert_eq!(
                PushRefspec::new(src, dst),
                Err(RefspecProblem::InvalidRef),
                "{src}:{dst}"
            );
        }
        let ok = PushRefspec::branch("feature/디자인").unwrap();
        assert_eq!(
            ok.as_arg(),
            "refs/heads/feature/디자인:refs/heads/feature/디자인"
        );
        assert!(PushRefspec::new("refs/tags/v1.0", "refs/tags/v1.0").is_ok());
    }

    #[test]
    fn parses_porcelain_output_without_url() {
        let out = b"To /srv/remote.git\n\
                    *\trefs/heads/main:refs/heads/main\t[new branch]\n\
                    \x20\trefs/heads/dev:refs/heads/dev\tabc1234..def5678\n\
                    !\trefs/heads/x:refs/heads/x\t[rejected] (non-fast-forward)\n\
                    =\trefs/tags/v1:refs/tags/v1\t[up to date]\n\
                    Done\n";
        let refs = parse_push_porcelain(out).unwrap();
        let flags: Vec<RefUpdateFlag> = refs.iter().map(|r| r.flag).collect();
        assert_eq!(
            flags,
            [
                RefUpdateFlag::New,
                RefUpdateFlag::FastForward,
                RefUpdateFlag::Rejected,
                RefUpdateFlag::UpToDate
            ]
        );
        assert_eq!(refs[2].summary, "[rejected] (non-fast-forward)");
        assert!(!format!("{refs:?}").contains("/srv/remote.git"));
        assert!(parse_push_porcelain(b"?\ta:b\tx\n").is_err());
        assert!(parse_push_porcelain(b"*\trefs/heads/a\tx\n").is_err());
        assert!(parse_push_porcelain(b"*\trefs/heads/a:refs/heads/a\n").is_err());
        // 정보 줄(upstream 안내 등)은 건너뜀.
        assert_eq!(
            parse_push_porcelain(b"branch 'main' set up to track.\nDone\n").unwrap(),
            vec![]
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
        use std::path::PathBuf;

        struct Fixture {
            _dir: TempDir,
            bare: PathBuf,
            work: PathBuf,
            runner: crate::runner::GitRunner,
        }

        fn fixture() -> Option<Fixture> {
            let git = system_git()?;
            let dir = TempDir::new("push");
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
            commit_file(&runner, &work, "stellar.yaml", b"v: 1\n");
            Some(Fixture {
                _dir: dir,
                bare,
                work,
                runner,
            })
        }

        fn rev(runner: &crate::runner::GitRunner, dir: &Path, r: &str) -> String {
            let out = git_ok(runner, dir, ["rev-parse", r]);
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }

        #[test]
        fn normal_push_fixture() {
            let Some(fx) = fixture() else {
                return; // git 미설치 환경 건너뜀.
            };
            let gate = GatedGitRunner::new(fx.runner.clone(), GitTrust::Trusted);
            let request = PushRequest {
                remote: "origin".into(),
                refspec: PushRefspec::branch("main").unwrap(),
                set_upstream: true,
            };
            let outcome = push(&gate, &fx.work, &request).unwrap();
            assert_eq!(outcome.refs.len(), 1);
            assert_eq!(outcome.refs[0].flag, RefUpdateFlag::New);
            assert_eq!(
                rev(&fx.runner, &fx.bare, "main"),
                rev(&fx.runner, &fx.work, "HEAD")
            );
            let upstream = git_ok(
                &fx.runner,
                &fx.work,
                ["rev-parse", "--abbrev-ref", "@{upstream}"],
            );
            assert_eq!(
                String::from_utf8_lossy(&upstream.stdout).trim(),
                "origin/main"
            );

            commit_file(&fx.runner, &fx.work, "stellar.yaml", b"v: 2\n");
            let outcome = push(
                &gate,
                &fx.work,
                &PushRequest {
                    set_upstream: false,
                    ..request.clone()
                },
            )
            .unwrap();
            assert_eq!(outcome.refs[0].flag, RefUpdateFlag::FastForward);
            assert_eq!(
                rev(&fx.runner, &fx.bare, "main"),
                rev(&fx.runner, &fx.work, "HEAD")
            );
            let again = push(&gate, &fx.work, &request).unwrap();
            assert_eq!(again.refs[0].flag, RefUpdateFlag::UpToDate);
        }

        #[test]
        fn unknown_remote_and_url_are_not_pushed() {
            let Some(fx) = fixture() else {
                return;
            };
            let gate = GatedGitRunner::new(fx.runner.clone(), GitTrust::Trusted);
            for remote in ["upstream", fx.bare.to_str().unwrap()] {
                let request = PushRequest {
                    remote: remote.into(),
                    refspec: PushRefspec::branch("main").unwrap(),
                    set_upstream: false,
                };
                assert!(matches!(
                    push(&gate, &fx.work, &request),
                    Err(PushError::UnknownRemote { .. })
                ));
            }
            // bare에는 아무것도 push되지 않음.
            let refs = git_ok(&fx.runner, &fx.bare, ["for-each-ref"]);
            assert!(refs.stdout.is_empty());
        }

        #[test]
        fn non_fast_forward_is_rejected_as_git_result() {
            let Some(fx) = fixture() else {
                return;
            };
            let gate = GatedGitRunner::new(fx.runner.clone(), GitTrust::Trusted);
            let request = PushRequest {
                remote: "origin".into(),
                refspec: PushRefspec::branch("main").unwrap(),
                set_upstream: false,
            };
            push(&gate, &fx.work, &request).unwrap();
            // 원격에 다른 commit을 올려 이력을 갈라지게 함.
            let other = fx.work.parent().unwrap().join("other");
            git_ok(
                &fx.runner,
                fx.work.parent().unwrap(),
                [
                    "clone",
                    "-q",
                    fx.bare.to_str().unwrap(),
                    other.to_str().unwrap(),
                ],
            );
            init_repo_with_identity(&fx.runner, &other);
            commit_file(&fx.runner, &other, "remote.txt", b"r\n");
            git_ok(&fx.runner, &other, ["push", "-q", "origin", "main"]);
            commit_file(&fx.runner, &fx.work, "local.txt", b"l\n");
            let remote_before = rev(&fx.runner, &fx.bare, "main");
            match push(&gate, &fx.work, &request) {
                Err(PushError::Rejected { refs, .. }) => {
                    assert_eq!(refs[0].flag, RefUpdateFlag::Rejected);
                }
                other => panic!("{other:?}"),
            }
            // 원격은 덮어쓰이지 않음(force 없음).
            assert_eq!(rev(&fx.runner, &fx.bare, "main"), remote_before);
        }

        /// local 상태 지문: HEAD, 모든 ref, index 항목, 작업 트리 status.
        fn local_state(runner: &crate::runner::GitRunner, work: &Path) -> Vec<Vec<u8>> {
            [
                vec!["rev-parse", "HEAD"],
                vec!["for-each-ref", "--format=%(refname) %(objectname)"],
                vec!["ls-files", "--stage", "-z"],
                vec!["status", "--porcelain=v2", "-z", "--branch"],
            ]
            .into_iter()
            .map(|args| git_ok(runner, work, args).stdout)
            .collect()
        }

        #[test]
        fn successful_push_refreshes_tracking_status() {
            use crate::repo_status::UpstreamState;
            let Some(fx) = fixture() else {
                return;
            };
            let gate = GatedGitRunner::new(fx.runner.clone(), GitTrust::Trusted);
            let request = PushRequest {
                remote: "origin".into(),
                refspec: PushRefspec::branch("main").unwrap(),
                set_upstream: true,
            };
            let report = push_and_refresh(&gate, &fx.work, &request).unwrap();
            assert_eq!(
                report.status.upstream,
                UpstreamState::Tracking {
                    name: b"origin/main".to_vec(),
                    ahead: 0,
                    behind: 0
                }
            );
            commit_file(&fx.runner, &fx.work, "stellar.yaml", b"v: 3\n");
            let before = crate::repo_status::read_repository_status(&gate, &fx.work).unwrap();
            assert!(matches!(
                before.upstream,
                UpstreamState::Tracking {
                    ahead: 1,
                    behind: 0,
                    ..
                }
            ));
            let report = push_and_refresh(&gate, &fx.work, &request).unwrap();
            assert_eq!(report.outcome.refs[0].flag, RefUpdateFlag::FastForward);
            assert!(matches!(
                report.status.upstream,
                UpstreamState::Tracking {
                    ahead: 0,
                    behind: 0,
                    ..
                }
            ));
        }

        #[test]
        fn rejected_push_keeps_local_state_identical() {
            let Some(fx) = fixture() else {
                return;
            };
            let gate = GatedGitRunner::new(fx.runner.clone(), GitTrust::Trusted);
            let request = PushRequest {
                remote: "origin".into(),
                refspec: PushRefspec::branch("main").unwrap(),
                set_upstream: true,
            };
            push_and_refresh(&gate, &fx.work, &request).unwrap();
            let other = fx.work.parent().unwrap().join("other");
            git_ok(
                &fx.runner,
                fx.work.parent().unwrap(),
                [
                    "clone",
                    "-q",
                    fx.bare.to_str().unwrap(),
                    other.to_str().unwrap(),
                ],
            );
            init_repo_with_identity(&fx.runner, &other);
            commit_file(&fx.runner, &other, "remote.txt", b"r\n");
            git_ok(&fx.runner, &other, ["push", "-q", "origin", "main"]);
            // local: commit 1개 + staged 변경 + unstaged 변경.
            commit_file(&fx.runner, &fx.work, "local.txt", b"l\n");
            std::fs::write(fx.work.join("staged.txt"), b"s\n").unwrap();
            git_ok(&fx.runner, &fx.work, ["add", "staged.txt"]);
            std::fs::write(fx.work.join("stellar.yaml"), b"unsaved-to-index\n").unwrap();

            let before = local_state(&fx.runner, &fx.work);
            let worktree_bytes = std::fs::read(fx.work.join("stellar.yaml")).unwrap();
            assert!(matches!(
                push_and_refresh(&gate, &fx.work, &request),
                Err(PushError::Rejected { .. })
            ));
            assert_eq!(local_state(&fx.runner, &fx.work), before);
            assert_eq!(
                std::fs::read(fx.work.join("stellar.yaml")).unwrap(),
                worktree_bytes
            );
        }
    }
}
