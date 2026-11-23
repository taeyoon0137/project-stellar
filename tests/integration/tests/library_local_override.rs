//! local override 대상 검증 종단 시험 (XV.6.2, ADR-0119 결정 7).
//!
//! 사용 프로젝트 `stellar.yaml`의 `local_override`가 가리키는 실제 폴더를 제품 검증
//! (`validate_project_local_overrides` → 실제 `probe_project`)으로 판정합니다. 일반 Git 저장소, `.git`이 없는 폴더,
//! 손상 매니페스트, 다른 library id, symlink로 사용 프로젝트 안을 가리키는 대상을 거부하고 정상 Stellar 작업 폴더만
//! 받습니다.

mod e2e_common;
mod library_common;

use e2e_common::{git, trusted_git, TempDir};
use library_common::*;
use stellar_dependencies::local_override::LocalOverrideRejection;
use stellar_project::library_overrides::validate_project_local_overrides;

#[test]
fn plain_repository_or_wrong_library_id_local_override_is_rejected() {
    let Some(gate) = trusted_git() else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };
    let dir = TempDir::new("library-local-override");
    let base = &dir.path;
    let stellar_checkout = |name: &str, id: u64| {
        let path = base.join(name);
        std::fs::create_dir_all(&path).unwrap();
        git(&gate, &path, &["init", "-q", "-b", "main"]);
        std::fs::write(
            path.join("stellar.yaml"),
            manifest_text(library_id(id), name, &[]),
        )
        .unwrap();
        path
    };
    stellar_checkout("ds", 31);
    stellar_checkout("other-lib", 39);
    let broken = stellar_checkout("broken-lib", 31);
    std::fs::write(broken.join("stellar.yaml"), "schema_version: [\n").unwrap();
    // 일반 Git 저장소(stellar.yaml 없음).
    let plain = base.join("plain-repo");
    std::fs::create_dir_all(&plain).unwrap();
    git(&gate, &plain, &["init", "-q", "-b", "main"]);
    // Git 작업 폴더가 아닌 Stellar 폴더(`.git` 없음).
    let loose = base.join("loose");
    std::fs::create_dir_all(&loose).unwrap();
    std::fs::write(
        loose.join("stellar.yaml"),
        manifest_text(library_id(31), "loose", &[]),
    )
    .unwrap();

    let entry = |name: &str, path: &str| {
        format!(
            "  {name}:\n    source: git\n    library_id: \"{}\"\n    repository: \"https://example.com/{name}.git\"\n    version: \"v1\"\n    local_override: \"{path}\"\n",
            library_id(31)
        )
    };
    let mut manifest = format!(
        "schema_version: 4\nproject:\n  id: \"{}\"\n  name: \"app\"\ntarget_profiles: []\ndependencies:\n",
        library_id(97)
    );
    for (name, path) in [
        ("ds", "../ds"),
        ("other", "../other-lib"),
        ("broken", "../broken-lib"),
        ("plain", "../plain-repo"),
        ("loose", "../loose"),
        ("missing", "../missing"),
        ("sneaky", "../sneaky"),
    ] {
        manifest.push_str(&entry(name, path));
    }
    let project = using_project(base, &manifest);
    // lexical 위치 검사는 통과하지만 symlink가 사용 프로젝트 안을 가리킵니다.
    std::fs::create_dir_all(project.join("vendor")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(project.join("vendor"), base.join("sneaky")).unwrap();

    let checks =
        validate_project_local_overrides(&project, &declarations(&manifest)).expect("선언 매핑");
    let result = |name: &str| {
        checks
            .iter()
            .find(|c| c.dependency.as_str() == name)
            .unwrap()
            .result
            .clone()
    };
    let ok = result("ds").expect("정상 Stellar 작업 폴더");
    assert_eq!(ok.library_id(), library_id(31));
    assert_eq!(ok.schema_version(), 4);
    assert_eq!(
        result("other"),
        Err(LocalOverrideRejection::LibraryIdMismatch {
            expected: library_id(31),
            found: library_id(39),
        })
    );
    assert_eq!(
        result("broken"),
        Err(LocalOverrideRejection::CorruptManifest)
    );
    assert_eq!(
        result("plain"),
        Err(LocalOverrideRejection::NotAStellarProject)
    );
    assert_eq!(
        result("loose"),
        Err(LocalOverrideRejection::NotAStellarProject)
    );
    assert!(matches!(
        result("missing"),
        Err(LocalOverrideRejection::Unavailable {
            kind: std::io::ErrorKind::NotFound
        })
    ));
    #[cfg(unix)]
    assert_eq!(result("sneaky"), Err(LocalOverrideRejection::InsideProject));
    // 진단 문장에 경로 원문이 없습니다.
    for check in &checks {
        if let Err(rejection) = &check.result {
            assert!(!rejection.to_string().contains(base.to_str().unwrap()));
        }
    }
}
