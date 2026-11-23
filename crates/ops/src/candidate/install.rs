//! merge driver 등록 (XXIV.2.3, ADR-0081 결정 9).
//!
//! 사용자가 명시적으로 부른 등록 명령만 이 함수를 씁니다. 저장소 로컬 config(`merge.stellar.name`·
//! `merge.stellar.driver`)와 프로젝트 `.gitattributes`의 한 줄만 바꾸며, 전역·시스템 config는 포트에 쓰기 동작이 없어
//! 바꿀 수 없습니다. 같은 값이 이미 있으면 아무것도 쓰지 않습니다(멱등).

use std::fmt;
use std::path::{Path, PathBuf};

use stellar_storage::hash::ContentHash;
use stellar_storage::source_file;

use super::component_file::COMPONENT_SOURCE_PATTERN;
use super::git_port::{GitPortError, MergeGit};

/// `.gitattributes`의 merge driver 이름.
pub const DRIVER_NAME: &str = "stellar";
/// `merge.<이름>.name` 값.
pub const DRIVER_DESCRIPTION: &str = "Stellar semantic merge";
/// `.gitattributes` 크기 상한.
const MAX_ATTRIBUTES_BYTES: u64 = 1024 * 1024;

/// 등록 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    /// 새로 쓰거나 바꾼 저장소 로컬 config key.
    pub config_changed: Vec<String>,
    /// `.gitattributes` 경로.
    pub attributes_path: PathBuf,
    /// `.gitattributes`에 줄을 더했는지.
    pub attributes_added: bool,
    /// 등록한 driver 명령.
    pub driver: String,
}

/// 등록 실패. 실패 전에 쓴 항목은 보고에 남지 않으므로 다시 실행하면 이어서 맞춥니다(멱등).
#[derive(Debug)]
pub enum InstallError {
    /// 프로젝트 루트(`stellar.yaml`)가 아님.
    NotProject,
    /// Git 실패.
    Git(GitPortError),
    /// `.gitattributes` 입출력 실패.
    Io(std::io::Error),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstallError::NotProject => {
                f.write_str("stellar.yaml이 있는 프로젝트 루트가 필요합니다")
            }
            InstallError::Git(e) => write!(f, "{e}"),
            InstallError::Io(e) => write!(f, ".gitattributes를 쓸 수 없습니다: {e}"),
        }
    }
}

impl std::error::Error for InstallError {}

/// 프로젝트 `.gitattributes`에 더하는 컴포넌트 원본 줄.
pub fn attributes_line() -> String {
    format!("{COMPONENT_SOURCE_PATTERN} merge={DRIVER_NAME}")
}

/// 프로젝트 `.gitattributes`에 더하는 줄 전체: 텍스트 원본 경로(ADR-0113 결정 5)와 컴포넌트 원본 줄. 컴포넌트 줄이
/// 마지막입니다.
pub fn attributes_lines() -> Vec<String> {
    crate::merge_text::ATTRIBUTE_PATTERNS
        .iter()
        .map(|pattern| format!("{pattern} merge={DRIVER_NAME}"))
        .chain(std::iter::once(attributes_line()))
        .collect()
}

/// driver를 저장소 로컬에 등록합니다. `driver_command`는 Git이 shell로 실행할 명령(`… %O %A %B %P`)입니다.
pub fn install_driver(
    git: &dyn MergeGit,
    project: &Path,
    driver_command: &str,
) -> Result<InstallReport, InstallError> {
    if !project.join("stellar.yaml").is_file() {
        return Err(InstallError::NotProject);
    }
    let mut changed = Vec::new();
    for (key, value) in [
        (format!("merge.{DRIVER_NAME}.name"), DRIVER_DESCRIPTION),
        (format!("merge.{DRIVER_NAME}.driver"), driver_command),
    ] {
        let current = git.local_config(project, &key).map_err(InstallError::Git)?;
        if current.as_deref() != Some(value) {
            git.set_local_config(project, &key, value)
                .map_err(InstallError::Git)?;
            changed.push(key);
        }
    }
    let attributes_path = project.join(".gitattributes");
    let existing = match source_file::read(&attributes_path, MAX_ATTRIBUTES_BYTES) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(InstallError::Io(e)),
    };
    let text = existing
        .as_deref()
        .map(String::from_utf8_lossy)
        .unwrap_or_default()
        .into_owned();
    let missing: Vec<String> = attributes_lines()
        .into_iter()
        .filter(|line| !text.lines().any(|l| l.trim() == line))
        .collect();
    let present = missing.is_empty();
    if !present {
        let mut next = text.clone();
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        for line in &missing {
            next.push_str(line);
            next.push('\n');
        }
        let expected = existing.as_deref().map(ContentHash::compute);
        source_file::write_checked(
            &attributes_path,
            next.as_bytes(),
            expected,
            MAX_ATTRIBUTES_BYTES,
        )
        .map_err(InstallError::Io)?;
    }
    Ok(InstallReport {
        config_changed: changed,
        attributes_path,
        attributes_added: !present,
        driver: driver_command.to_owned(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::super::git_port::{GitOperation, UnmergedEntry};
    use crate::test_support::{scaffold_project, TempDir};

    /// 메모리 Git: 로컬 config만 흉내 내고 호출 기록을 남깁니다.
    #[derive(Default)]
    pub(crate) struct FakeGit {
        pub(crate) config: RefCell<BTreeMap<String, String>>,
        pub(crate) calls: RefCell<Vec<String>>,
    }

    impl MergeGit for FakeGit {
        fn unmerged(&self, _: &Path) -> Result<Vec<UnmergedEntry>, GitPortError> {
            self.calls.borrow_mut().push("unmerged".into());
            Ok(Vec::new())
        }
        fn blob(&self, _: &Path, _: &str, _: u64) -> Result<Vec<u8>, GitPortError> {
            Err(GitPortError("없음".into()))
        }
        fn add(&self, _: &Path, _: &[PathBuf]) -> Result<(), GitPortError> {
            self.calls.borrow_mut().push("add".into());
            Ok(())
        }
        fn operation(&self, _: &Path) -> Result<GitOperation, GitPortError> {
            Ok(GitOperation::None)
        }
        fn local_config(&self, _: &Path, key: &str) -> Result<Option<String>, GitPortError> {
            Ok(self.config.borrow().get(key).cloned())
        }
        fn set_local_config(&self, _: &Path, key: &str, value: &str) -> Result<(), GitPortError> {
            self.calls.borrow_mut().push(format!("config {key}"));
            self.config.borrow_mut().insert(key.into(), value.into());
            Ok(())
        }
    }

    #[test]
    fn install_is_idempotent_and_touches_only_local_config_and_attributes() {
        let dir = TempDir::new("install");
        scaffold_project(&dir.path);
        let git = FakeGit::default();
        let first = install_driver(&git, &dir.path, "stellar merge-driver %O %A %B %P").unwrap();
        assert_eq!(
            first.config_changed,
            vec!["merge.stellar.name", "merge.stellar.driver"]
        );
        assert!(first.attributes_added);
        let attributes = std::fs::read_to_string(dir.path.join(".gitattributes")).unwrap();
        assert!(
            attributes.starts_with("* text=auto eol=lf\n"),
            "기존 줄 보존"
        );
        assert!(attributes.ends_with(&format!("{}\n", attributes_line())));
        let again = install_driver(&git, &dir.path, "stellar merge-driver %O %A %B %P").unwrap();
        assert!(again.config_changed.is_empty() && !again.attributes_added);
        assert_eq!(
            std::fs::read_to_string(dir.path.join(".gitattributes")).unwrap(),
            attributes
        );
        assert_eq!(
            git.calls.borrow().len(),
            2,
            "두 번째 실행은 config를 쓰지 않음"
        );
    }

    #[test]
    fn install_requires_a_project_root() {
        let dir = TempDir::new("install-none");
        let git = FakeGit::default();
        assert!(matches!(
            install_driver(&git, &dir.path, "x"),
            Err(InstallError::NotProject)
        ));
        assert!(git.calls.borrow().is_empty());
    }
}
