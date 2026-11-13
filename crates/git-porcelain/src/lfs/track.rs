//! 사용자 승인 후 Git LFS 추적 적용 — `git lfs track` (XIII.7.3).
//!
//! `.gitattributes`는 Stellar가 직접 쓰지 않습니다. 사용자가 미리보기([`LfsTrackRequest::preview`])를
//! 보고 승인([`LfsTrackApproval::approve`])한 요청만 시스템 Git LFS 명령
//! `git lfs track [--filename] -- <pattern>`으로 적용합니다. 패턴 escape·줄 형식·중복 처리는
//! git-lfs가 맡습니다(파일 문법 재구현 없음).
//!
//! - **승인 없음 → 변경 0**: [`apply_lfs_track`]은 승인 증표를 타입으로 요구하며, 증표는 요청의
//!   argv digest에 묶입니다. 다른 요청의 증표면 프로세스를 하나도 만들지 않고
//!   [`LfsTrackError::ApprovalMismatch`]입니다. 증표는 crate 밖에서 조립할 수 없습니다.
//! - **LFS 설치 확인**: 실행 전에 중립 디렉터리에서 `git lfs version`(XIII.7.1)을 확인합니다. LFS가
//!   없으면 저장소 안에서 `git lfs`를 실행하지 않습니다(저장소 `alias.lfs` 실행 방지).
//! - **결과 검증**: 실행 후 `git check-attr filter`(Git이 해석한 결과)로 검증 경로가 `filter=lfs`인지
//!   확인합니다. 아니면 [`LfsTrackError::NotApplied`]입니다.
//! - **부수 효과**: git-lfs는 `track` 실행 시 저장소에 LFS hook(`pre-push` 등)을 설치할 수 있습니다.
//!   LFS object 업로드에 필요한 표준 동작이므로 막지 않고 미리보기 설명에 포함합니다.
//!
//! 기존 이력의 LFS 변환(`git lfs migrate`)은 지원하지 않습니다(XIII.7.4).

use std::ffi::OsString;
use std::path::Path;

use super::advice::valid_extension;
use super::attr::{check_attribute, AttrError, AttrValue};
use super::{probe_lfs, LfsInstallation};
use crate::decode::GitPath;
use crate::ops::{run_ok, GitOpError};
use crate::runner::{argv, GitRunError};
use crate::snapshot::materialize::validate_snapshot_path;
use crate::trust::{GatedGitRunner, GatedRunError};

/// LFS로 추적할 대상.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LfsTrackTarget {
    /// 저장 확장자 전체(`*.<ext>`). 확장자는 소문자 ASCII 영숫자.
    Extension(String),
    /// 파일 하나(`--filename`: glob 문자를 literal로 escape).
    File(GitPath),
}

/// 승인 대기 중인 LFS 추적 요청(미리보기 가능한 argv).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfsTrackRequest {
    target: LfsTrackTarget,
    verify_path: GitPath,
    args: Vec<OsString>,
}

/// 사용자가 특정 [`LfsTrackRequest`]를 승인했다는 증표(crate 밖에서 조립 불가).
///
/// ```compile_fail
/// use stellar_git_porcelain::lfs::track::LfsTrackApproval;
/// let _forged = LfsTrackApproval { digest: 0 };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LfsTrackApproval {
    digest: u64,
}

/// LFS 추적 적용 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfsTrackOutcome {
    /// 실행 전 검증 경로가 이미 `filter=lfs`였는지.
    pub was_tracked: bool,
    /// 실행 후 Git이 해석한 검증 경로의 `filter` 값(항상 `lfs`).
    pub filter: AttrValue,
}

/// LFS 추적 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LfsTrackError {
    /// 잘못된 대상(Git 실행 전 거부).
    InvalidTarget(&'static str),
    /// 승인 증표가 이 요청의 것이 아님(프로세스 생성 0).
    ApprovalMismatch,
    /// trust gate 거부(프로세스 생성 0).
    Gate(GatedRunError),
    /// `git lfs version` probe 실행 실패.
    Probe(GitRunError),
    /// Git LFS가 설치되어 있지 않음(저장소 안 `git lfs` 실행 0).
    LfsNotInstalled(LfsInstallation),
    /// 속성 확인 실패.
    Attr(AttrError),
    /// `git lfs track` 실행 실패·비영 종료(Git 결과 보존).
    Op(GitOpError),
    /// 실행 후에도 검증 경로가 `filter=lfs`가 아님.
    NotApplied {
        /// Git이 해석한 `filter` 값.
        filter: AttrValue,
    },
}

impl std::fmt::Display for LfsTrackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LfsTrackError::InvalidTarget(what) => write!(f, "잘못된 LFS 추적 대상: {what}"),
            LfsTrackError::ApprovalMismatch => {
                f.write_str("이 LFS 추적 요청에 대한 승인이 아닙니다")
            }
            LfsTrackError::Gate(e) => write!(f, "{e}"),
            LfsTrackError::Probe(e) => write!(f, "{e}"),
            LfsTrackError::LfsNotInstalled(_) => f.write_str("Git LFS가 설치되어 있지 않습니다"),
            LfsTrackError::Attr(e) => write!(f, "{e}"),
            LfsTrackError::Op(e) => write!(f, "{e}"),
            LfsTrackError::NotApplied { .. } => {
                f.write_str("git lfs track 실행 후에도 LFS 추적이 적용되지 않았습니다")
            }
        }
    }
}

impl std::error::Error for LfsTrackError {}

/// `--filename` 대상 경로 검증: snapshot 경로 규칙 + UTF-8 + 제어 문자 없음 + 줄 시작 특수 문자 없음.
fn valid_file_target(path: &GitPath) -> bool {
    if validate_snapshot_path(path.as_bytes()).is_err() {
        return false;
    }
    let Ok(text) = std::str::from_utf8(path.as_bytes()) else {
        return false;
    };
    !text.chars().any(char::is_control) && !text.starts_with(['#', '!', '"', '-'])
}

/// 마지막 component가 `.<ext>`로 끝나는지(대소문자 구분: `*.<ext>` 패턴과 같은 기준).
fn has_extension(path: &GitPath, ext: &str) -> bool {
    let name = path
        .as_bytes()
        .rsplit(|b| *b == b'/')
        .next()
        .unwrap_or_default();
    let suffix = format!(".{ext}");
    name.len() > suffix.len() && name.ends_with(suffix.as_bytes())
}

impl LfsTrackRequest {
    /// 요청을 만듭니다. `verify_path`는 적용 후 `filter=lfs`를 확인할 경로입니다(확장자 대상이면 그
    /// 확장자의 파일, 파일 대상이면 그 파일과 같아야 함).
    pub fn new(
        target: LfsTrackTarget,
        verify_path: GitPath,
    ) -> Result<LfsTrackRequest, LfsTrackError> {
        if validate_snapshot_path(verify_path.as_bytes()).is_err() {
            return Err(LfsTrackError::InvalidTarget("검증 경로"));
        }
        let mut args = argv(["lfs", "track"]);
        match &target {
            LfsTrackTarget::Extension(ext) => {
                if !valid_extension(ext) {
                    return Err(LfsTrackError::InvalidTarget("확장자"));
                }
                if !has_extension(&verify_path, ext) {
                    return Err(LfsTrackError::InvalidTarget("검증 경로가 확장자와 다름"));
                }
                args.push(OsString::from("--"));
                args.push(OsString::from(format!("*.{ext}")));
            }
            LfsTrackTarget::File(path) => {
                if !valid_file_target(path) {
                    return Err(LfsTrackError::InvalidTarget("파일 경로"));
                }
                if path != &verify_path {
                    return Err(LfsTrackError::InvalidTarget("검증 경로가 대상 파일과 다름"));
                }
                let text = String::from_utf8_lossy(path.as_bytes()).into_owned();
                args.extend(argv(["--filename", "--"]));
                args.push(OsString::from(text));
            }
        }
        Ok(LfsTrackRequest {
            target,
            verify_path,
            args,
        })
    }

    /// 대상.
    pub fn target(&self) -> &LfsTrackTarget {
        &self.target
    }

    /// 적용 후 확인할 경로.
    pub fn verify_path(&self) -> &GitPath {
        &self.verify_path
    }

    /// Git에 넘길 argv.
    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    /// 사용자에게 보여 줄 명령 미리보기(`git lfs track -- '*.bin'`).
    pub fn preview(&self) -> String {
        let mut out = String::from("git");
        for arg in &self.args {
            let text = arg.to_string_lossy();
            out.push(' ');
            if text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
            {
                out.push_str(&text);
            } else {
                out.push('\'');
                out.push_str(&text.replace('\'', "'\\''"));
                out.push('\'');
            }
        }
        out
    }

    /// 승인 시 사용자에게 알릴 부수 효과.
    pub fn effects(&self) -> &'static [&'static str] {
        &[
            "저장소 루트 .gitattributes에 LFS 추적 줄을 추가합니다(git-lfs가 작성).",
            "git-lfs가 저장소에 LFS hook(pre-push 등)을 설치할 수 있습니다.",
            "기존 커밋 이력은 바꾸지 않습니다. 이미 커밋된 파일은 다음 커밋부터 LFS로 저장됩니다.",
        ]
    }

    /// argv에 묶인 digest(FNV-1a 64, 승인 대조용).
    fn digest(&self) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |bytes: &[u8]| {
            for b in bytes {
                hash ^= u64::from(*b);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        };
        for arg in &self.args {
            feed(arg.as_encoded_bytes());
            feed(&[0]);
        }
        feed(self.verify_path.as_bytes());
        hash
    }
}

impl LfsTrackApproval {
    /// UI가 미리보기·부수 효과를 보여 주고 사용자가 승인했을 때 만듭니다.
    pub fn approve(request: &LfsTrackRequest) -> LfsTrackApproval {
        LfsTrackApproval {
            digest: request.digest(),
        }
    }
}

fn filter_of(
    git: &GatedGitRunner,
    worktree_root: &Path,
    path: &GitPath,
) -> Result<AttrValue, LfsTrackError> {
    let mut values = check_attribute(git, worktree_root, "filter", std::slice::from_ref(path))
        .map_err(LfsTrackError::Attr)?;
    Ok(values
        .pop()
        .map(|(_, value)| value)
        .unwrap_or(AttrValue::Unspecified))
}

/// 승인된 요청을 `git lfs track`으로 적용하고 Git 결과로 검증합니다.
///
/// `worktree_root`는 저장소 worktree root(`--show-toplevel`)입니다. 승인이 맞지 않거나 신뢰되지 않은
/// 프로젝트면 프로세스를 만들지 않습니다.
pub fn apply_lfs_track(
    git: &GatedGitRunner,
    worktree_root: &Path,
    request: &LfsTrackRequest,
    approval: &LfsTrackApproval,
) -> Result<LfsTrackOutcome, LfsTrackError> {
    if approval.digest != request.digest() {
        return Err(LfsTrackError::ApprovalMismatch);
    }
    let runner = git.runner().map_err(LfsTrackError::Gate)?;
    let installation = probe_lfs(runner).map_err(LfsTrackError::Probe)?;
    if !installation.is_installed() {
        return Err(LfsTrackError::LfsNotInstalled(installation));
    }
    let was_tracked = filter_of(git, worktree_root, &request.verify_path)?.is_value(b"lfs");
    run_ok(git, worktree_root, request.args()).map_err(LfsTrackError::Op)?;
    let filter = filter_of(git, worktree_root, &request.verify_path)?;
    if !filter.is_value(b"lfs") {
        return Err(LfsTrackError::NotApplied { filter });
    }
    Ok(LfsTrackOutcome {
        was_tracked,
        filter,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::GitRunner;
    use crate::trust::GitTrust;

    fn p(s: &str) -> GitPath {
        GitPath::from_bytes(s.as_bytes())
    }

    fn missing_git() -> (GatedGitRunner, GitRunner) {
        let runner = GitRunner::new(
            std::path::PathBuf::from("/nonexistent/git"),
            Vec::new(),
            std::time::Duration::from_secs(5),
        );
        (
            GatedGitRunner::new(runner.clone(), GitTrust::Trusted),
            runner,
        )
    }

    #[test]
    fn requests_build_literal_argv_and_preview() {
        let ext = LfsTrackRequest::new(LfsTrackTarget::Extension("bin".into()), p("assets/a.bin"))
            .unwrap();
        assert_eq!(ext.args(), argv(["lfs", "track", "--", "*.bin"]).as_slice());
        assert_eq!(ext.preview(), "git lfs track -- '*.bin'");
        let file = LfsTrackRequest::new(
            LfsTrackTarget::File(p("assets/project [1].psd")),
            p("assets/project [1].psd"),
        )
        .unwrap();
        assert_eq!(
            file.args(),
            argv(["lfs", "track", "--filename", "--", "assets/project [1].psd"]).as_slice()
        );
        assert_eq!(
            file.preview(),
            "git lfs track --filename -- 'assets/project [1].psd'"
        );
        assert_eq!(file.effects().len(), 3);
    }

    #[test]
    fn invalid_targets_are_rejected() {
        let bad: Vec<(LfsTrackTarget, &str)> = vec![
            (LfsTrackTarget::Extension("BIN".into()), "a.BIN"),
            (LfsTrackTarget::Extension("*".into()), "a.bin"),
            (LfsTrackTarget::Extension("".into()), "a.bin"),
            (LfsTrackTarget::Extension("bin".into()), "a.png"),
            (LfsTrackTarget::Extension("bin".into()), "a.BIN"),
            (LfsTrackTarget::Extension("bin".into()), ".bin"),
            (LfsTrackTarget::Extension("bin".into()), "../a.bin"),
            (LfsTrackTarget::File(p("a\nb.bin")), "a\nb.bin"),
            (LfsTrackTarget::File(p("#a.bin")), "#a.bin"),
            (LfsTrackTarget::File(p("!a.bin")), "!a.bin"),
            (LfsTrackTarget::File(p("-a.bin")), "-a.bin"),
            (LfsTrackTarget::File(p(".git/config")), ".git/config"),
            (LfsTrackTarget::File(p("/abs.bin")), "/abs.bin"),
            (LfsTrackTarget::File(p("a.bin")), "b.bin"),
        ];
        for (target, verify) in bad {
            assert!(
                matches!(
                    LfsTrackRequest::new(target.clone(), p(verify)),
                    Err(LfsTrackError::InvalidTarget(_))
                ),
                "{target:?} {verify:?}"
            );
        }
        let raw = GitPath::from_bytes(b"assets/\xff.bin");
        assert!(LfsTrackRequest::new(LfsTrackTarget::File(raw.clone()), raw).is_err());
    }

    #[test]
    fn approval_of_another_request_spawns_nothing() {
        let (gate, runner) = missing_git();
        let bin = LfsTrackRequest::new(LfsTrackTarget::Extension("bin".into()), p("assets/a.bin"))
            .unwrap();
        let other =
            LfsTrackRequest::new(LfsTrackTarget::Extension("psd".into()), p("assets/a.psd"))
                .unwrap();
        // 같은 argv여도 검증 경로가 다르면 다른 요청입니다.
        let same_argv =
            LfsTrackRequest::new(LfsTrackTarget::Extension("bin".into()), p("assets/b.bin"))
                .unwrap();
        for wrong in [
            LfsTrackApproval::approve(&other),
            LfsTrackApproval::approve(&same_argv),
        ] {
            assert_eq!(
                apply_lfs_track(&gate, &std::env::temp_dir(), &bin, &wrong),
                Err(LfsTrackError::ApprovalMismatch)
            );
        }
        assert_eq!(runner.spawn_count(), 0);
        // 맞는 승인이면 실행을 시도합니다(여기서는 executable이 없어 probe 실패, spawn 1).
        assert!(matches!(
            apply_lfs_track(
                &gate,
                &std::env::temp_dir(),
                &bin,
                &LfsTrackApproval::approve(&bin)
            ),
            Err(LfsTrackError::Probe(GitRunError::Spawn { .. }))
        ));
        assert_eq!(runner.spawn_count(), 1);
    }

    #[test]
    fn untrusted_project_spawns_nothing() {
        let (_, runner) = missing_git();
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted);
        let req = LfsTrackRequest::new(LfsTrackTarget::Extension("bin".into()), p("assets/a.bin"))
            .unwrap();
        assert_eq!(
            apply_lfs_track(
                &gate,
                &std::env::temp_dir(),
                &req,
                &LfsTrackApproval::approve(&req)
            ),
            Err(LfsTrackError::Gate(GatedRunError::ProjectNotTrusted))
        );
        assert_eq!(runner.spawn_count(), 0);
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::lfs::attr::check_attribute;
        use crate::test_support::{
            commit_file, git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use std::ffi::OsString;
        use std::time::Duration;

        /// git-lfs가 설치된 hermetic 실행기(없으면 `None` → 건너뜀).
        fn lfs_runner() -> Option<GitRunner> {
            let runner = hermetic_runner(system_git()?);
            probe_lfs(&runner).ok()?.is_installed().then_some(runner)
        }

        fn filters(gate: &GatedGitRunner, repo: &Path, paths: &[&str]) -> Vec<bool> {
            let paths: Vec<GitPath> = paths.iter().map(|s| p(s)).collect();
            check_attribute(gate, repo, "filter", &paths)
                .unwrap()
                .into_iter()
                .map(|(_, v)| v.is_value(b"lfs"))
                .collect()
        }

        fn hooks(repo: &Path) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(repo.join(".git/hooks"))
                .map(|d| {
                    d.filter_map(Result::ok)
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        }

        #[test]
        fn no_approval_changes_nothing_and_approval_applies_via_git() {
            let Some(runner) = lfs_runner() else {
                return; // git·git-lfs 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("lfs-track");
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            commit_file(&runner, repo, "stellar.yaml", b"v: 1\n");
            let hooks_before = hooks(repo);
            let status_before = git_ok(&runner, repo, ["status", "--porcelain=v2", "-z"]).stdout;
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);

            let bin =
                LfsTrackRequest::new(LfsTrackTarget::Extension("bin".into()), p("assets/a.bin"))
                    .unwrap();
            let psd =
                LfsTrackRequest::new(LfsTrackTarget::Extension("psd".into()), p("assets/a.psd"))
                    .unwrap();

            // 승인 없음(다른 요청의 승인): 프로세스·변경 0.
            let spawns = runner.spawn_count();
            assert_eq!(
                apply_lfs_track(&gate, repo, &bin, &LfsTrackApproval::approve(&psd)),
                Err(LfsTrackError::ApprovalMismatch)
            );
            assert_eq!(runner.spawn_count(), spawns);
            assert!(!repo.join(".gitattributes").exists());
            assert_eq!(hooks(repo), hooks_before);
            assert_eq!(
                git_ok(&runner, repo, ["status", "--porcelain=v2", "-z"]).stdout,
                status_before
            );
            assert_eq!(
                filters(&gate, repo, &["assets/a.bin", "assets/b.bin"]),
                vec![false, false]
            );

            // 승인 후: git lfs track 적용, Git(check-attr)으로 결과 검증.
            let outcome =
                apply_lfs_track(&gate, repo, &bin, &LfsTrackApproval::approve(&bin)).unwrap();
            assert!(!outcome.was_tracked);
            assert!(outcome.filter.is_value(b"lfs"));
            assert!(repo.join(".gitattributes").is_file());
            assert_eq!(
                filters(
                    &gate,
                    repo,
                    &["assets/a.bin", "x/y/z.bin", "assets/a.png", "assets/a.psd"]
                ),
                vec![true, true, false, false]
            );

            // 같은 요청 재적용: 이미 추적 중으로 보고하고 성공.
            let again =
                apply_lfs_track(&gate, repo, &bin, &LfsTrackApproval::approve(&bin)).unwrap();
            assert!(again.was_tracked);
        }

        #[test]
        fn file_target_is_literal_not_a_glob() {
            let Some(runner) = lfs_runner() else {
                return;
            };
            let dir = TempDir::new("lfs-track-file");
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            commit_file(&runner, repo, "stellar.yaml", b"v: 1\n");
            let gate = GatedGitRunner::new(runner, GitTrust::Trusted);
            let target = p("assets/project [1].psd");
            let req = LfsTrackRequest::new(LfsTrackTarget::File(target.clone()), target).unwrap();
            apply_lfs_track(&gate, repo, &req, &LfsTrackApproval::approve(&req)).unwrap();
            assert_eq!(
                filters(
                    &gate,
                    repo,
                    &[
                        "assets/project [1].psd",
                        "assets/project 1.psd",
                        "assets/x.psd"
                    ]
                ),
                vec![true, false, false]
            );
        }

        #[test]
        fn missing_lfs_does_not_run_track_in_repository() {
            let Some(git) = system_git() else {
                return;
            };
            // exec-path에 git-lfs가 있으면 PATH로 미설치를 만들 수 없음.
            let exec = std::process::Command::new(&git)
                .arg("--exec-path")
                .output()
                .unwrap();
            let exec_dir = String::from_utf8_lossy(&exec.stdout).trim().to_string();
            if Path::new(&exec_dir).join("git-lfs").exists() {
                return;
            }
            let dir = TempDir::new("lfs-track-missing");
            let repo = dir.path().join("repo");
            let empty = dir.path().join("empty-bin");
            std::fs::create_dir_all(&repo).unwrap();
            std::fs::create_dir_all(&empty).unwrap();
            init_repo_with_identity(&hermetic_runner(git.clone()), &repo);
            let marker = dir.path().join("PWNED_alias");
            git_ok(
                &hermetic_runner(git.clone()),
                &repo,
                [
                    "config",
                    "alias.lfs",
                    &format!("!/usr/bin/touch '{}'", marker.display()),
                ],
            );
            let runner = GitRunner::new(git, Vec::new(), Duration::from_secs(30))
                .with_env_overrides(vec![
                    (OsString::from("PATH"), empty.as_os_str().to_os_string()),
                    (
                        OsString::from("GIT_CONFIG_GLOBAL"),
                        OsString::from("/dev/null"),
                    ),
                    (
                        OsString::from("GIT_CONFIG_SYSTEM"),
                        OsString::from("/dev/null"),
                    ),
                ]);
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let req =
                LfsTrackRequest::new(LfsTrackTarget::Extension("bin".into()), p("assets/a.bin"))
                    .unwrap();
            assert!(matches!(
                apply_lfs_track(&gate, &repo, &req, &LfsTrackApproval::approve(&req)),
                Err(LfsTrackError::LfsNotInstalled(_))
            ));
            // probe 1회만 실행, 저장소 alias 실행 0, .gitattributes 미생성.
            assert_eq!(runner.spawn_count(), 1);
            assert!(!marker.exists());
            assert!(!repo.join(".gitattributes").exists());
        }
    }
}
