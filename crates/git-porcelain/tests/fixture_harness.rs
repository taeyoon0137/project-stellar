//! Git fixture harness(XIV.14.1)를 crate 밖 소비자처럼 `test-support` feature로 씁니다.
//!
//! 같은 scenario가 플랫폼과 무관하게 같은 commit ID를 만들 조건을 고정합니다(Windows 재개 때 CI-WIN에서 같은
//! golden을 확인하는 근거, ADR-0006·ADR-0120 결정 5).
//!
//! - commit에 들어간 항목은 모두 일반 파일(mode `100644`)입니다. 실행 비트·symlink·submodule이 없어 파일 시스템
//!   차이가 tree에 드러나지 않습니다.
//! - blob은 LF 줄바꿈만 쓰고 CR이 없습니다(`core.autocrlf=false`와 함께 checkout·commit 변환이 없음).
//! - 경로는 ASCII 소문자·숫자·`/`·`.`만 씁니다(대소문자 구분 없는 파일 시스템과 경로 구분자 차이 없음).
//! - harness root가 달라도(다른 tag) 같은 golden이 나옵니다.

use stellar_git_porcelain::fixture::{build_standard_scenario, GitFixture};

const GOLDEN_MAIN: &str = "6073a623e79df89edcf0ec0a8ae2b34c5415bf4e";
const GOLDEN_FEATURE: &str = "1cf4a59f0e548d662e0ab18e7092a0008a285203";

fn stdout(fixture: &GitFixture, args: &[&str]) -> String {
    String::from_utf8(fixture.git(args.iter().copied()).unwrap().stdout).unwrap()
}

#[test]
fn standard_scenario_tree_has_only_platform_neutral_entries() {
    let Some(fixture) = GitFixture::with_system_git("harness-neutral").unwrap() else {
        return; // 시스템 Git 없음.
    };
    let scenario = build_standard_scenario(&fixture).unwrap();
    assert_eq!(scenario.main.as_str(), GOLDEN_MAIN);
    assert_eq!(scenario.feature.as_str(), GOLDEN_FEATURE);
    for commit in [&scenario.initial, &scenario.main, &scenario.feature] {
        let listing = stdout(&fixture, &["ls-tree", "-r", "--full-tree", commit.as_str()]);
        assert!(!listing.is_empty());
        for line in listing.lines() {
            let (meta, path) = line.split_once('\t').expect("ls-tree 줄");
            let mut parts = meta.split(' ');
            let (mode, kind, blob) = (
                parts.next().unwrap(),
                parts.next().unwrap(),
                parts.next().unwrap(),
            );
            assert_eq!((mode, kind), ("100644", "blob"), "{path}");
            assert!(
                path.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"/._-".contains(&b)),
                "{path}"
            );
            let content = fixture.git(["cat-file", "blob", blob]).unwrap().stdout;
            assert!(!content.contains(&b'\r'), "{path}: CR 없음");
            assert!(content.ends_with(b"\n"), "{path}: LF로 끝남");
        }
    }
    // checkout한 작업 트리 bytes도 blob과 같습니다(줄바꿈 변환 없음).
    let home = std::fs::read(fixture.work().join("design/pages/home.yaml")).unwrap();
    assert_eq!(home, b"id: home\ntitle: Home\n");
}

#[test]
fn scenario_ids_do_not_depend_on_the_fixture_location() {
    let (Some(a), Some(b)) = (
        GitFixture::with_system_git("harness-loc-a").unwrap(),
        GitFixture::with_system_git("harness-location-b-longer").unwrap(),
    ) else {
        return;
    };
    assert_ne!(a.root(), b.root());
    assert_eq!(
        build_standard_scenario(&a).unwrap(),
        build_standard_scenario(&b).unwrap()
    );
}
