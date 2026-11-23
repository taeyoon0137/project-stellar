//! XVI.11.2 종단 시나리오: 객체 속성 일부 stage → commit → 빈 GitHub 저장소 fixture를 remote로 연결 → push.
//!
//! 실제 GitHub 네트워크는 쓰지 않습니다(ADR-0111 결정 4). 둘로 나눕니다.
//!
//! - **URL 검증 단위**: GitHub URL 형식 판정(`classify_github_url`)과 연결 전 거부(GitHub가 아님·형식 오류·이름 규칙)는
//!   Git을 실행하지 않는 순수 판정이라 따로 확인합니다.
//! - **local bare remote**: 빈 bare 저장소를 만들고 테스트 HOME의 전역 Git 설정 `url.<bare>.insteadOf =
//!   https://github.com/<owner>/<repo>.git`으로 GitHub URL을 그 경로로 바꿉니다. 그래서 제품 연결 함수
//!   (`connect_empty_github_remote`: URL 검증 → 이름 → 기존 remote → `ls-remote` 빈 저장소 확인 → `remote add`)와 push
//!   (`push_and_refresh`)가 GitHub URL 그대로 끝까지 실행되고, 전송은 로컬 파일로만 일어납니다.
//!
//! 편집은 앱 runtime 속성 패널(같은 Frame의 `width`와 `background-color`)이고, stage·commit은 `stellar git`과 같은 ops
//! Git 엔진(semantic 변경 ID 하나만 stage, 재검증 뒤 commit)입니다.

mod e2e_common;

use e2e_common::{add_global_git_config, git, git_session, porcelain, trusted_git, App, TempDir};
use stellar_git_porcelain::remote::connect::{
    connect_empty_github_remote, ConnectError, ConnectRemoteRequest,
};
use stellar_git_porcelain::remote::github::{
    classify_github_url, GitHubRepo, GitHubUrlCheck, GitHubUrlForm, GitHubUrlProblem,
};
use stellar_git_porcelain::remote::push::{
    push_and_refresh, PushRefspec, PushRequest, RefUpdateFlag,
};
use stellar_git_porcelain::remote::RemoteUrl;
use stellar_git_porcelain::repo_status::UpstreamState;
use stellar_ops::git::{GitItem, StatusArea, StatusRequest};
use stellar_ops::query::{project_structure, property_path_label};
use stellar_ops::snapshot::open_snapshot;
use stellar_runtime::app::inspector::InspectorProperty;

const OWNER: &str = "stellar-e2e";

#[test]
fn github_url_validation_is_a_pure_unit_without_git() {
    // XIV.9.2 형식 판정: 네트워크·Git 없이 문자열만 봅니다.
    let repo = |form| {
        GitHubUrlCheck::GitHub(GitHubRepo {
            form,
            owner: OWNER.into(),
            repo: "design".into(),
        })
    };
    assert_eq!(
        classify_github_url("https://github.com/stellar-e2e/design.git"),
        repo(GitHubUrlForm::Https)
    );
    assert_eq!(
        classify_github_url("https://github.com/stellar-e2e/design"),
        repo(GitHubUrlForm::Https)
    );
    assert_eq!(
        classify_github_url("git@github.com:stellar-e2e/design.git"),
        repo(GitHubUrlForm::SshScp)
    );
    assert_eq!(
        classify_github_url("ssh://git@github.com/stellar-e2e/design.git"),
        repo(GitHubUrlForm::SshUrl)
    );
    for (url, problem) in [
        (
            "https://user:token@github.com/stellar-e2e/design.git",
            GitHubUrlProblem::CredentialsInUrl,
        ),
        (
            "http://github.com/stellar-e2e/design.git",
            GitHubUrlProblem::InsecureScheme,
        ),
        (
            "https://github.com:443/stellar-e2e/design.git",
            GitHubUrlProblem::PortNotAllowed,
        ),
        (
            "https://github.com/stellar-e2e/design.git?x=1",
            GitHubUrlProblem::QueryOrFragment,
        ),
        (
            "https://github.com/stellar-e2e/design/tree/main",
            GitHubUrlProblem::UnexpectedPath,
        ),
        (
            "https://github.com/-bad/design.git",
            GitHubUrlProblem::InvalidOwner,
        ),
        (
            "https://github.com/stellar-e2e/..",
            GitHubUrlProblem::InvalidRepo,
        ),
        (
            "root@github.com:stellar-e2e/design.git",
            GitHubUrlProblem::UnexpectedSshUser,
        ),
    ] {
        assert_eq!(
            classify_github_url(url),
            GitHubUrlCheck::Invalid(problem),
            "{url}"
        );
    }
    for url in [
        "https://gitlab.com/a/b.git",
        "/srv/git/design.git",
        "file:///srv/design.git",
    ] {
        assert_eq!(classify_github_url(url), GitHubUrlCheck::NotGitHub, "{url}");
    }
}

/// 빈 bare 저장소를 만들고 `https://github.com/stellar-e2e/<name>.git`을 그 경로로 바꾸는 전역 설정을 더합니다.
fn github_fixture(
    gate: &stellar_git_porcelain::trust::GatedGitRunner,
    parent: &std::path::Path,
    name: &str,
) -> (std::path::PathBuf, String) {
    let bare = parent.join(format!("{name}.git"));
    std::fs::create_dir_all(&bare).unwrap();
    git(gate, &bare, &["init", "-q", "--bare", "-b", "main"]);
    let url = format!("https://github.com/{OWNER}/{name}.git");
    add_global_git_config(&format!("url \"{}\"", bare.display()), "insteadOf", &url);
    (bare, url)
}

#[test]
fn partial_property_stage_commit_connect_empty_github_remote_and_push() {
    let Some(gate) = trusted_git() else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };
    let dir = TempDir::new("stage-push");
    let root = dir.path.join("업로드 프로젝트");

    // ── 1. 새 프로젝트와 같은 객체의 두 속성 편집(앱 runtime 속성 패널).
    let app = App::start();
    assert_eq!(app.create_project(&root), 0);
    let first_commit = git(&gate, &root, &["rev-parse", "HEAD"]).trim().to_string();
    let structure = project_structure(&open_snapshot(&root).unwrap());
    let frame = structure.documents[0].pages[0].views[0].root_frame;
    let project = app.open_project(&root);
    let (window, _, _) = app.open_window(project);
    app.select(window, &[frame]);
    app.edit_property(window, InspectorProperty::Width, "1024px");
    app.edit_property(window, InspectorProperty::BackgroundColor, "#102030");
    assert!(app.save(window).1);
    app.close_window(window);
    app.close_project(project);
    app.shutdown();

    // ── 2. semantic 상태: 같은 Frame의 두 속성 변경이 따로 보입니다.
    let session = git_session(&gate, &root);
    let status = session.status(&StatusRequest::default()).expect("Git 상태");
    let page = status.semantic.as_ref().expect("semantic 상태");
    assert_eq!(page.staged_total, 0);
    let unstaged: Vec<_> = page
        .entries
        .iter()
        .filter(|e| e.area == StatusArea::Unstaged && e.object == frame)
        .collect();
    let labels: Vec<String> = unstaged
        .iter()
        .map(|e| {
            e.property
                .as_ref()
                .map(property_path_label)
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(
        unstaged.len(),
        2,
        "width·background 두 속성 변경: {labels:?}"
    );
    let mut sorted = labels.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        ["background-color", "width"],
        "CSS 속성 이름으로 보임"
    );
    let width = unstaged
        .iter()
        .find(|e| {
            e.property
                .as_ref()
                .is_some_and(|p| property_path_label(p) == "width")
        })
        .unwrap_or_else(|| panic!("width 변경 없음: {labels:?}"));
    let other = unstaged.iter().find(|e| e.id != width.id).unwrap();

    // ── 3. 속성 하나만 stage: 필수 동반 변경이 없으므로 그 변경 하나만 index에 들어갑니다.
    let staged = session
        .stage(vec![GitItem::Change(width.id.clone())], Vec::new())
        .expect("속성 하나 stage");
    assert_eq!(staged.applied_changes, 1);
    let page = session
        .status(&StatusRequest::default())
        .unwrap()
        .semantic
        .unwrap();
    let staged_ids: Vec<_> = page
        .entries
        .iter()
        .filter(|e| e.area == StatusArea::Staged)
        .map(|e| e.id.clone())
        .collect();
    assert_eq!(
        staged_ids,
        vec![width.id.clone()],
        "staged = 고른 속성 하나"
    );
    assert!(
        page.entries
            .iter()
            .any(|e| e.area == StatusArea::Unstaged && e.id == other.id),
        "나머지 속성은 unstaged로 남음"
    );
    assert!(
        session.validate_staged().unwrap().is_valid(),
        "staged snapshot 재검증 통과"
    );

    // ── 4. commit: 재검증 뒤 시스템 Git commit. 작업 트리는 그대로이고 나머지 속성은 여전히 unstaged입니다.
    let before_commit = e2e_common::tree(&root);
    let commit = session.commit("너비만 commit").expect("commit");
    assert_eq!(
        commit
            .previous_head
            .as_ref()
            .map(|h| h.as_str().to_string()),
        Some(first_commit.clone())
    );
    assert_eq!(commit.staged_remaining, 0);
    assert!(commit.tree_matches_validated);
    assert_eq!(
        e2e_common::tree(&root),
        before_commit,
        "commit은 작업 트리를 바꾸지 않음"
    );
    let page = session
        .status(&StatusRequest::default())
        .unwrap()
        .semantic
        .unwrap();
    assert_eq!(page.staged_total, 0);
    let remaining: Vec<_> = page
        .entries
        .iter()
        .filter(|e| e.object == frame)
        .map(|e| e.id.clone())
        .collect();
    assert_eq!(
        remaining,
        vec![other.id.clone()],
        "HEAD에는 width만, 작업 트리에는 background가 더 있음"
    );
    assert_eq!(
        git(&gate, &root, &["rev-list", "--count", "HEAD"]).trim(),
        "2",
        "최초 commit + 속성 commit"
    );

    // ── 5. 빈 GitHub 저장소 연결. 연결 전 거부는 Git을 실행하지 않고, 이력이 있는 저장소는 최초 연결을 거부합니다.
    let remotes = dir.path.join("remotes");
    std::fs::create_dir_all(&remotes).unwrap();
    let (bare, url) = github_fixture(&gate, &remotes, "empty-design");
    let (used, used_url) = github_fixture(&gate, &remotes, "used-design");
    let seed = dir.path.join("seed");
    git(
        &gate,
        &dir.path,
        &[
            "clone",
            "-q",
            used.to_str().unwrap(),
            seed.to_str().unwrap(),
        ],
    );
    std::fs::write(seed.join("README.md"), "이미 있는 이력\n").unwrap();
    git(&gate, &seed, &["add", "README.md"]);
    git(&gate, &seed, &["commit", "-q", "-m", "seed"]);
    git(
        &gate,
        &seed,
        &["push", "-q", "origin", "HEAD:refs/heads/main"],
    );

    let request = |name: Option<&str>, url: &str| ConnectRemoteRequest {
        name: name.map(str::to_owned),
        url: RemoteUrl::new(url),
    };
    assert_eq!(
        connect_empty_github_remote(&gate, &root, &request(None, "https://gitlab.com/a/b.git")),
        Err(ConnectError::NotGitHub)
    );
    assert_eq!(
        connect_empty_github_remote(
            &gate,
            &root,
            &request(None, "http://github.com/stellar-e2e/empty-design.git")
        ),
        Err(ConnectError::InvalidGitHubUrl(
            GitHubUrlProblem::InsecureScheme
        ))
    );
    assert_eq!(
        connect_empty_github_remote(&gate, &root, &request(Some("-x"), &url)),
        Err(ConnectError::InvalidRemoteName)
    );
    assert!(matches!(
        connect_empty_github_remote(&gate, &root, &request(Some("used"), &used_url)),
        Err(ConnectError::RemoteHasHistory { ref_count }) if ref_count >= 1
    ));
    assert_eq!(
        git(&gate, &root, &["remote"]).trim(),
        "",
        "거부된 연결은 remote를 만들지 않음"
    );

    let connected =
        connect_empty_github_remote(&gate, &root, &request(None, &url)).expect("빈 저장소 연결");
    assert_eq!(connected.name, "origin");
    assert_eq!(connected.repo.repo, "empty-design");
    assert_eq!(
        git(
            &gate,
            &root,
            &["config", "--local", "--get", "remote.origin.url"]
        )
        .trim(),
        url,
        "저장소 config에는 GitHub URL 그대로(전송 경로 바꾸기는 전역 insteadOf)"
    );
    assert!(matches!(
        connect_empty_github_remote(&gate, &root, &request(None, &url)),
        Err(ConnectError::RemoteNameTaken { .. })
    ));
    let report = session.status(&StatusRequest::default()).unwrap();
    assert_eq!(report.remotes.len(), 1);
    // 상태의 remote URL은 Git이 보고한 값(`remote -v`, insteadOf 적용 뒤)이라 fixture에서는 bare 경로입니다.
    assert_eq!(report.remotes[0].name, "origin");
    assert!(report.remotes[0].fetch_url.is_some());

    // ── 6. push(force 없음)와 remote refs 확인.
    let head = git(&gate, &root, &["rev-parse", "HEAD"]).trim().to_string();
    let pushed = push_and_refresh(
        &gate,
        &root,
        &PushRequest {
            remote: "origin".into(),
            refspec: PushRefspec::branch("main").unwrap(),
            set_upstream: true,
        },
    )
    .expect("push");
    assert_eq!(pushed.outcome.refs.len(), 1);
    let only = &pushed.outcome.refs[0];
    assert_eq!(only.flag, RefUpdateFlag::New);
    assert_eq!(
        (only.from.as_str(), only.to.as_str()),
        ("refs/heads/main", "refs/heads/main")
    );
    assert!(matches!(
        &pushed.status.upstream,
        UpstreamState::Tracking { name, ahead: 0, behind: 0 } if name == b"origin/main"
    ));
    let refs = git(
        &gate,
        &bare,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    );
    assert_eq!(
        refs.trim(),
        format!("refs/heads/main {head}"),
        "remote ref는 main 하나 = local HEAD"
    );
    assert_eq!(
        git(&gate, &bare, &["rev-list", "--count", "refs/heads/main"]).trim(),
        "2"
    );
    // 원격의 commit tree가 local HEAD tree와 같습니다(속성 하나만 담긴 commit).
    assert_eq!(
        git(&gate, &bare, &["rev-parse", "refs/heads/main^{tree}"]),
        git(&gate, &root, &["rev-parse", "HEAD^{tree}"])
    );
    // push는 작업 트리·index를 바꾸지 않습니다. 남은 속성 변경은 그대로 unstaged입니다.
    assert_eq!(e2e_common::tree(&root), before_commit);
    assert!(!porcelain(&gate, &root).is_empty());
}
