//! 프로젝트 내부 원본 경로 경계 — `Update from Source`가 record의 프로젝트 상대 경로로 파일을
//! 읽어 `assets/`로 복사하기 전에 하는 판정 (XIII.5.5 후속 수정).
//!
//! record는 Git으로 들어오는 **신뢰할 수 없는 입력**이라 공격자가 상대 경로를 마음대로 적을 수
//! 있습니다. "canonical project root 안"은 "commit되는 asset으로 복사해도 안전"과 같지 않습니다.
//! `.git/config`의 credential URL, Git이 무시하는 `.env` 같은 로컬 비밀도 root 안에 있습니다.
//! 그래서 두 단계로 판정하고, 둘 다 통과해야 원본을 엽니다.
//!
//! 1. **고정 거부(이 crate)** — [`protected_project_path`]. 기록된 상대 경로와 canonicalize한
//!    경로(project root 기준) **양쪽**에 적용합니다.
//!    - 어느 위치든 `.git` 구성 요소: Git 메타데이터(중첩 저장소·submodule 포함,
//!      [`ProtectedArea::GitMetadata`]).
//!    - 첫 구성 요소가 Stellar 저장 디렉터리 `design/`·`.stellar/`
//!      ([`ProtectedArea::StellarStorage`]).
//!    - 첫 구성 요소가 `assets/`(content 저장소 자체, [`ProtectedArea::AssetStore`]).
//!
//!    비교는 구성 요소마다 ASCII 대소문자를 무시하고, HFS+가 이름 비교에서 무시하는 Unicode
//!    code point(U+200C~U+200F, U+202A~U+202E, U+206A~U+206F, U+FEFF)와 NTFS가 무시하는 끝
//!    `.`·공백을 지운 뒤 합니다(`.GIT`, `.g\u{200c}it`, `.git.` 모두 Git 메타데이터). 보호 이름은
//!    모두 ASCII입니다. 파일 시스템이 같은 항목으로 보는 다른 표기(대소문자·정규화 차이 등)는
//!    canonicalize 결과가 디스크의 실제 이름을 돌려주므로 해석 경로 판정에서도 걸립니다.
//! 2. **호출자 정책** — 고정 거부를 통과한 나머지는 [`ProjectSourcePolicy`]가 명시적으로
//!    [`ProjectSourceDecision::Allow`]를 돌려줘야 읽습니다. project 계층이 여기에 신뢰
//!    매트릭스(신뢰되지 않은 프로젝트는 [`DenyProjectSources`] — 기본 거부)와 `git check-ignore`
//!    (무시되는 파일 거부)를 연결합니다. 이 crate는 Git을 실행하지 않습니다.
//!
//! # 경계 밖
//!
//! - 사용자가 이번 실행에서 직접 고른 경로(첫 import, `update_from_selected_source`)는 사용자
//!   의도이므로 이 판정을 거치지 않습니다. record·Git·캐시에서 온 경로를 그 API에 넘기면 안
//!   됩니다.
//! - 프로젝트 밖 원본은 기기 key 출처 지문으로 확인합니다([`crate::origin`]).
//! - NTFS 8.3 짧은 이름(`GIT~1`)·alternate data stream 표기는 다루지 않습니다(Windows 지원
//!   보류).

use std::fmt;
use std::path::Path;

use crate::path::ASSETS_DIR;

/// Git 메타데이터 디렉터리(또는 linked worktree의 gitfile) 이름.
pub const GIT_DIR_NAME: &str = ".git";

/// project root 바로 아래의 Stellar 저장 디렉터리(아키텍처 19절).
pub const STELLAR_STORAGE_DIRS: [&str; 2] = ["design", ".stellar"];

/// 고정 거부 영역.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtectedArea {
    /// Git 메타데이터(`.git` 구성 요소, 위치 무관).
    GitMetadata,
    /// Stellar 저장 디렉터리(`design/`·`.stellar/`).
    StellarStorage,
    /// 에셋 content 저장소(`assets/`).
    AssetStore,
}

impl fmt::Display for ProtectedArea {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtectedArea::GitMetadata => f.write_str("Git 메타데이터(.git)"),
            ProtectedArea::StellarStorage => {
                f.write_str("Stellar 저장 디렉터리(design/·.stellar/)")
            }
            ProtectedArea::AssetStore => f.write_str("에셋 저장소(assets/)"),
        }
    }
}

/// HFS+가 이름 비교에서 무시하는 code point(Git `is_hfs_dotgit`과 같은 목록).
fn is_hfs_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{200c}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{206a}'..='\u{206f}' | '\u{feff}'
    )
}

/// 보호 이름 비교용 정규화: 무시 code point 제거, ASCII 소문자, 끝 `.`·공백 제거.
fn fold_component(component: &str) -> String {
    let mut folded: String = component
        .chars()
        .filter(|c| !is_hfs_ignorable(*c))
        .map(|c| c.to_ascii_lowercase())
        .collect();
    while folded.ends_with(['.', ' ']) {
        folded.pop();
    }
    folded
}

/// 구성 요소 목록(project root 기준 순서)이 고정 거부 영역이면 그 영역을 돌려줍니다.
pub fn protected_area<'a>(components: impl IntoIterator<Item = &'a str>) -> Option<ProtectedArea> {
    let mut first = true;
    let mut found = None;
    for component in components {
        let folded = fold_component(component);
        if folded == GIT_DIR_NAME {
            return Some(ProtectedArea::GitMetadata);
        }
        if first {
            if STELLAR_STORAGE_DIRS.contains(&folded.as_str()) {
                found = Some(ProtectedArea::StellarStorage);
            } else if folded == ASSETS_DIR {
                found = Some(ProtectedArea::AssetStore);
            }
            first = false;
        }
    }
    found
}

/// `/` 구분 프로젝트 상대 경로가 고정 거부 영역이면 그 영역을 돌려줍니다.
///
/// ```
/// use stellar_assets::project_source::{protected_project_path, ProtectedArea};
/// assert_eq!(protected_project_path(".git/config"), Some(ProtectedArea::GitMetadata));
/// assert_eq!(protected_project_path("vendor/lib/.GIT/config"), Some(ProtectedArea::GitMetadata));
/// assert_eq!(protected_project_path("design/current"), Some(ProtectedArea::StellarStorage));
/// assert_eq!(protected_project_path("Assets/ab/x.png"), Some(ProtectedArea::AssetStore));
/// assert_eq!(protected_project_path("images/design/logo.png"), None);
/// ```
pub fn protected_project_path(relative_path: &str) -> Option<ProtectedArea> {
    protected_area(relative_path.split('/'))
}

/// [`ProjectSourcePolicy`]에 넘기는 판정 입력. 고정 거부를 이미 통과한 경로입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectSourceCandidate<'a> {
    recorded_path: &'a str,
    resolved_path: &'a str,
    resolved_absolute: &'a Path,
}

impl<'a> ProjectSourceCandidate<'a> {
    pub(crate) fn new(
        recorded_path: &'a str,
        resolved_path: &'a str,
        resolved_absolute: &'a Path,
    ) -> ProjectSourceCandidate<'a> {
        ProjectSourceCandidate {
            recorded_path,
            resolved_path,
            resolved_absolute,
        }
    }

    /// record에 기록된 `/` 구분 상대 경로(신뢰할 수 없는 입력).
    pub fn recorded_path(&self) -> &'a str {
        self.recorded_path
    }

    /// canonicalize(symlink 해석)한 실제 원본의 canonical project root 기준 `/` 구분 상대
    /// 경로. 프로젝트 안 symlink를 거치면 기록 경로와 다릅니다(예: `images/logo.png` →
    /// `.env`). 정책은 **두 경로 모두** 판정해야 합니다.
    pub fn resolved_path(&self) -> &'a str {
        self.resolved_path
    }

    /// 실제 원본의 canonical 절대 경로.
    pub fn resolved_absolute(&self) -> &'a Path {
        self.resolved_absolute
    }
}

/// 프로젝트 내부 원본 판정 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectSourceDecision {
    /// 읽어서 `assets/`로 복사해도 됩니다.
    Allow,
    /// 읽지 않습니다(기존 asset 유지).
    Deny,
}

/// record의 프로젝트 내부 원본을 읽어도 되는지 판정하는 호출자 정책.
///
/// project 계층이 구현합니다. 신뢰되지 않은 프로젝트는 항상 [`ProjectSourceDecision::Deny`]
/// ([`DenyProjectSources`])여야 하고, 신뢰된 프로젝트도 Git이 무시하는 파일(`git check-ignore`)
/// 처럼 commit되면 안 되는 파일은 거부해야 합니다. 판정 중 실패(Git 실행 실패 등)는
/// `Deny`로 처리합니다(fail closed).
pub trait ProjectSourcePolicy {
    /// 판정합니다.
    fn decide(&self, candidate: &ProjectSourceCandidate<'_>) -> ProjectSourceDecision;
}

/// 모든 프로젝트 내부 원본을 거부합니다(신뢰되지 않은 프로젝트의 기본 정책).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct DenyProjectSources;

impl ProjectSourcePolicy for DenyProjectSources {
    fn decide(&self, _candidate: &ProjectSourceCandidate<'_>) -> ProjectSourceDecision {
        ProjectSourceDecision::Deny
    }
}

impl<F> ProjectSourcePolicy for F
where
    F: Fn(&ProjectSourceCandidate<'_>) -> ProjectSourceDecision,
{
    fn decide(&self, candidate: &ProjectSourceCandidate<'_>) -> ProjectSourceDecision {
        self(candidate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_metadata_is_protected_anywhere_in_any_spelling() {
        for path in [
            ".git",
            ".git/config",
            ".GIT/config",
            ".Git/HEAD",
            "sub/.git/config",
            "vendor/lib/.git/modules/x/config",
            ".g\u{200c}it/config",
            ".\u{feff}git/config",
            "\u{200e}.git\u{202e}/config",
            ".git./config",
            ".git /config",
            ".git. . /config",
        ] {
            assert_eq!(
                protected_project_path(path),
                Some(ProtectedArea::GitMetadata),
                "{path:?}"
            );
        }
    }

    #[test]
    fn stellar_storage_and_assets_are_protected_only_at_root() {
        for (path, area) in [
            ("design/current", ProtectedArea::StellarStorage),
            (
                "Design/objects/ab/x.starchunk",
                ProtectedArea::StellarStorage,
            ),
            (".stellar/cache/x", ProtectedArea::StellarStorage),
            (".STELLAR/app", ProtectedArea::StellarStorage),
            ("assets/ab/sha256-ab.png", ProtectedArea::AssetStore),
            ("ASSETS/x", ProtectedArea::AssetStore),
            ("assets.", ProtectedArea::AssetStore),
            // `.git`은 위치와 무관하게 우선합니다.
            ("design/.git/config", ProtectedArea::GitMetadata),
        ] {
            assert_eq!(protected_project_path(path), Some(area), "{path:?}");
        }
        for path in [
            "images/logo.png",
            "images/design/logo.png",
            "src/assets/logo.png",
            "docs/.stellar/x",
            ".github/workflows/ci.yml",
            ".gitignore",
            ".gitattributes",
            "git/config",
            ".gitx/config",
            "designs/a.png",
            "assets-src/a.png",
            ".env",
        ] {
            assert_eq!(protected_project_path(path), None, "{path:?}");
        }
    }

    #[test]
    fn deny_policy_and_closures_decide() {
        let resolved = Path::new("/p/.env");
        let candidate = ProjectSourceCandidate::new("images/logo.png", ".env", resolved);
        assert_eq!(
            DenyProjectSources.decide(&candidate),
            ProjectSourceDecision::Deny
        );
        let ignored = |c: &ProjectSourceCandidate<'_>| {
            if [c.recorded_path(), c.resolved_path()].contains(&".env") {
                ProjectSourceDecision::Deny
            } else {
                ProjectSourceDecision::Allow
            }
        };
        assert_eq!(ignored.decide(&candidate), ProjectSourceDecision::Deny);
        let plain = ProjectSourceCandidate::new("images/a.png", "images/a.png", resolved);
        assert_eq!(ignored.decide(&plain), ProjectSourceDecision::Allow);
        assert_eq!(candidate.resolved_absolute(), resolved);
    }

    #[test]
    fn areas_are_described() {
        assert!(ProtectedArea::GitMetadata.to_string().contains(".git"));
        assert!(ProtectedArea::AssetStore.to_string().contains("assets/"));
    }
}
