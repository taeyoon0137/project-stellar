//! Stellar 프로젝트 파일 분류와 raw Git status 분리 (XIV.4.4).
//!
//! Semantic Status는 **Stellar 프로젝트 파일**만 해석합니다(아키텍처 §19 프로젝트 구조):
//!
//! | 경로 | 분류 |
//! |---|---|
//! | `stellar.yaml` | [`StellarFileKind::Manifest`] |
//! | `stellar.lock.yaml` | [`StellarFileKind::Lock`] |
//! | `design/` 아래 | [`StellarFileKind::Design`] |
//! | `assets/` 아래 | [`StellarFileKind::Asset`] |
//! | 그 밖(`README.md`, `.gitignore`, `.gitattributes`, `.stellar/` 등) | non-Stellar |
//!
//! non-Stellar 파일은 **raw Git file status**([`RawFileStatus`], Git `XY` 코드 그대로)로만 표시하고
//! semantic parser에 넘기지 않습니다. 경로는 정확한 바이트로 비교합니다(대소문자만 다른 경로는 §19.2에서
//! 금지되므로 `Design/`은 Stellar 파일이 아님). rename/copy의 두 경로 분류가 다르면
//! [`StatusPartition::cross_boundary`]로 따로 둡니다(raw로 표시, Stellar 쪽 경로의 추가·삭제는 semantic
//! 단계가 snapshot 비교로 처리).

use crate::decode::GitPath;
use crate::status::{StatusEntry, StatusReport};

/// Stellar 프로젝트 파일 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StellarFileKind {
    /// `stellar.yaml`.
    Manifest,
    /// `stellar.lock.yaml`.
    Lock,
    /// `design/` 아래(current pointer·generation·chunk).
    Design,
    /// `assets/` 아래.
    Asset,
}

/// 경로 분류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectFileClass {
    /// Stellar 프로젝트 파일(semantic 해석 대상).
    Stellar(StellarFileKind),
    /// 그 밖의 파일(raw Git status만).
    NonStellar,
}

/// 저장소 root 기준 경로를 분류합니다.
pub fn classify_project_path(path: &[u8]) -> ProjectFileClass {
    match path {
        b"stellar.yaml" => ProjectFileClass::Stellar(StellarFileKind::Manifest),
        b"stellar.lock.yaml" => ProjectFileClass::Stellar(StellarFileKind::Lock),
        _ if path.starts_with(b"design/") && path.len() > b"design/".len() => {
            ProjectFileClass::Stellar(StellarFileKind::Design)
        }
        _ if path.starts_with(b"assets/") && path.len() > b"assets/".len() => {
            ProjectFileClass::Stellar(StellarFileKind::Asset)
        }
        _ => ProjectFileClass::NonStellar,
    }
}

/// raw Git file status(표시용, Git 결과 그대로).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFileStatus {
    /// 경로.
    pub path: GitPath,
    /// rename/copy 원래 경로.
    pub original_path: Option<GitPath>,
    /// Git 2글자 상태(`.M`, `A.`, `R.`, `UU`, `??`, `!!` 등).
    pub xy: String,
}

impl RawFileStatus {
    /// status 항목을 raw 표시로 바꿉니다(해석 없음).
    pub fn from_entry(entry: &StatusEntry) -> RawFileStatus {
        let (xy, original_path) = match entry {
            StatusEntry::Ordinary(e) => (
                format!("{}{}", e.index.as_char(), e.worktree.as_char()),
                None,
            ),
            StatusEntry::Renamed(e) => (
                format!("{}{}", e.entry.index.as_char(), e.entry.worktree.as_char()),
                Some(e.original_path.clone()),
            ),
            StatusEntry::Unmerged(e) => (e.kind.as_str().to_string(), None),
            StatusEntry::Untracked(_) => ("??".to_string(), None),
            StatusEntry::Ignored(_) => ("!!".to_string(), None),
        };
        RawFileStatus {
            path: entry.path().clone(),
            original_path,
            xy,
        }
    }
}

/// Stellar 항목과 raw 항목으로 나눈 status.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusPartition {
    /// Stellar 프로젝트 파일 항목(semantic status 입력).
    pub stellar: Vec<StatusEntry>,
    /// non-Stellar 파일(raw 표시 전용).
    pub raw: Vec<RawFileStatus>,
    /// 두 경로의 분류가 다른 rename/copy(raw 표시, semantic 입력 아님).
    pub cross_boundary: Vec<RawFileStatus>,
}

/// status 항목을 Stellar/raw로 나눕니다(Git 출력 순서 유지).
pub fn partition_status(report: &StatusReport) -> StatusPartition {
    let mut out = StatusPartition::default();
    for entry in &report.entries {
        let class = classify_project_path(entry.path().as_bytes());
        let original_class = match entry {
            StatusEntry::Renamed(e) => Some(classify_project_path(e.original_path.as_bytes())),
            _ => None,
        };
        match (class, original_class) {
            (a, Some(b))
                if a != b
                    && (a == ProjectFileClass::NonStellar || b == ProjectFileClass::NonStellar) =>
            {
                out.cross_boundary.push(RawFileStatus::from_entry(entry));
            }
            (ProjectFileClass::Stellar(_), _) => out.stellar.push(entry.clone()),
            (ProjectFileClass::NonStellar, _) => out.raw.push(RawFileStatus::from_entry(entry)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::parse_status_v2;

    const E1: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";

    #[test]
    fn paths_are_classified_by_exact_bytes() {
        use ProjectFileClass::*;
        use StellarFileKind::*;
        let cases: [(&[u8], ProjectFileClass); 14] = [
            (b"stellar.yaml", Stellar(Manifest)),
            (b"stellar.lock.yaml", Stellar(Lock)),
            (b"design/current.stellarref", Stellar(Design)),
            (b"design/objects/sha256/ab/x.starchunk", Stellar(Design)),
            (b"assets/sha256/ab/x.png", Stellar(Asset)),
            (b"README.md", NonStellar),
            (b".gitignore", NonStellar),
            (b".gitattributes", NonStellar),
            (b".stellar/cache/x", NonStellar),
            (b"Design/x", NonStellar),
            (b"design", NonStellar),
            (b"design/", NonStellar),
            (b"docs/stellar.yaml", NonStellar),
            (b"stellar.yaml.bak", NonStellar),
        ];
        for (path, class) in cases {
            assert_eq!(classify_project_path(path), class, "{path:?}");
        }
    }

    #[test]
    fn partition_keeps_raw_codes_and_excludes_non_stellar_from_semantic_input() {
        let z = "0".repeat(40);
        let bytes = format!(
            "# branch.oid {E1}\0# branch.head main\0\
             1 .M N... 100644 100644 100644 {E1} {E1} README.md\0\
             1 M. N... 100644 100644 100644 {E1} {E1} design/current.stellarref\0\
             2 R. N... 100644 100644 100644 {E1} {E1} R100 docs/guide.md\0notes.md\0\
             2 R. N... 100644 100644 100644 {E1} {E1} R100 design/moved.starchunk\0old.bin\0\
             u UU N... 100644 100644 100644 100644 {E1} {E1} {E1} LICENSE\0\
             1 A. N... 000000 100644 100644 {z} {E1} assets/sha256/aa/a.png\0\
             ? notes.txt\0? stellar.lock.yaml\0! build.log\0"
        );
        let report = parse_status_v2(bytes.as_bytes()).unwrap();
        let p = partition_status(&report);
        let stellar: Vec<&[u8]> = p.stellar.iter().map(|e| e.path().as_bytes()).collect();
        assert_eq!(
            stellar,
            vec![
                &b"design/current.stellarref"[..],
                b"assets/sha256/aa/a.png",
                b"stellar.lock.yaml"
            ]
        );
        let raw: Vec<(&[u8], &str)> = p
            .raw
            .iter()
            .map(|r| (r.path.as_bytes(), r.xy.as_str()))
            .collect();
        assert_eq!(
            raw,
            vec![
                (&b"README.md"[..], ".M"),
                (b"docs/guide.md", "R."),
                (b"LICENSE", "UU"),
                (b"notes.txt", "??"),
                (b"build.log", "!!"),
            ]
        );
        assert_eq!(
            p.raw[1].original_path,
            Some(GitPath::from_bytes(b"notes.md"))
        );
        assert_eq!(p.cross_boundary.len(), 1);
        assert_eq!(
            p.cross_boundary[0].path.as_bytes(),
            b"design/moved.starchunk"
        );
        assert_eq!(
            p.cross_boundary[0].original_path,
            Some(GitPath::from_bytes(b"old.bin"))
        );
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::status::{read_status, StatusOptions};
        use crate::test_support::{
            git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use crate::trust::{GatedGitRunner, GitTrust};

        #[test]
        fn readme_change_keeps_raw_status() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("classify-readme");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            std::fs::create_dir_all(repo.join("design")).unwrap();
            std::fs::write(repo.join("README.md"), "# Project\n").unwrap();
            std::fs::write(repo.join("stellar.yaml"), "schema_version: 1\n").unwrap();
            std::fs::write(repo.join("design/current.stellarref"), "gen-1\n").unwrap();
            git_ok(&runner, repo, ["add", "-A"]);
            git_ok(&runner, repo, ["commit", "-q", "-m", "base"]);
            std::fs::write(repo.join("README.md"), "# Project\n\nmore\n").unwrap();
            std::fs::write(repo.join("design/current.stellarref"), "gen-2\n").unwrap();
            git_ok(&runner, repo, ["add", "design/current.stellarref"]);

            let gate = GatedGitRunner::new(runner, GitTrust::Trusted);
            let report = read_status(&gate, repo, StatusOptions::default()).unwrap();
            let p = partition_status(&report);
            assert_eq!(
                p.raw,
                vec![RawFileStatus {
                    path: GitPath::from_bytes(b"README.md"),
                    original_path: None,
                    xy: ".M".to_string(),
                }]
            );
            assert_eq!(p.stellar.len(), 1);
            assert_eq!(p.stellar[0].path().as_bytes(), b"design/current.stellarref");
            assert!(p.cross_boundary.is_empty());
        }
    }
}
