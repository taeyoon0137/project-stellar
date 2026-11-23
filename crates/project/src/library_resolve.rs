//! 제품 resolve 연결: cache를 먼저 보는 library 매니페스트 공급자와 `stellar.yaml` 의존성 편집 (XV.4.3·XV.7.4,
//! ADR-0137 결정 3·4).
//!
//! - [`CachedManifestSource`]: resolver가 exact commit의 library 매니페스트를 물으면 공용 cache에 그 `(repository, commit)`
//!   entry가 있을 때 entry의 `stellar.yaml`을 실제 매니페스트 parser([`library_manifest_from_text`])로 읽고 Git을 부르지
//!   않습니다. entry가 없을 때만 fetch된 mirror([`GitManifestSource`])를 읽습니다. lock commit 우선 해석(XV.4.3
//!   `LockPreferringResolver`)과 함께 쓰면 선택자가 바뀌지 않은 의존성은 network 없이 graph를 다시 만듭니다.
//! - [`ManifestEdit`]: `stellar.yaml`을 읽은 bytes hash와 함께 열고, 의존성 선언을 바꾼 canonical 텍스트를 읽은 뒤 파일이
//!   바뀌지 않았을 때만 씁니다(`write_checked`, 다른 writer와 경합하면 쓰지 않음). 쓰기 전에 바꾼 매니페스트를 다시 읽혀
//!   검증합니다([`Manifest::write`]의 loader 제한).

use std::fmt;
use std::io;
use std::path::Path;

use stellar_dependencies::cache_entry::{LibraryEntryStore, Sha256Digest};
use stellar_dependencies::declaration::{CommitId, DependencyDeclaration, RepositoryUrl};
use stellar_dependencies::mirror::RepositoryKeyer;
use stellar_dependencies::resolver::{LibraryManifest, LibraryManifestSource, ManifestSourceError};
use stellar_storage::hash::ContentHash;

use crate::manifest::Manifest;
use crate::manifest_source::library_manifest_from_text;
use crate::probe::read_at_most;
use crate::root::MANIFEST_FILE;
use crate::yaml::load_restricted;
use crate::yaml::loader::MAX_INPUT_BYTES;

/// cache entry를 먼저 보는 매니페스트 공급자(모듈 문서).
pub struct CachedManifestSource<'a, H, K, M> {
    store: &'a LibraryEntryStore<H, K>,
    fallback: M,
    cache_hits: usize,
}

impl<'a, H: Sha256Digest, K: RepositoryKeyer, M: LibraryManifestSource>
    CachedManifestSource<'a, H, K, M>
{
    /// `store`를 먼저 보고, 없으면 `fallback`(mirror 공급자)을 씁니다.
    pub fn new(store: &'a LibraryEntryStore<H, K>, fallback: M) -> Self {
        CachedManifestSource {
            store,
            fallback,
            cache_hits: 0,
        }
    }

    /// cache entry에서 읽은 횟수.
    pub fn cache_hits(&self) -> usize {
        self.cache_hits
    }
}

impl<H: Sha256Digest, K: RepositoryKeyer, M: LibraryManifestSource> LibraryManifestSource
    for CachedManifestSource<'_, H, K, M>
{
    fn manifest_at(
        &mut self,
        repository: &RepositoryUrl,
        commit: &CommitId,
    ) -> Result<LibraryManifest, ManifestSourceError> {
        if let Ok(Some(hit)) = self.store.lookup(repository, commit) {
            let path = self.store.entry_dir(&hit.content_hash).join(MANIFEST_FILE);
            // entry는 immutable이고 symlink를 담지 않습니다(XV.3.3). 정규 파일만, loader 상한까지 읽습니다.
            let regular = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file());
            if regular {
                let text = std::fs::File::open(&path)
                    .ok()
                    .and_then(|file| read_at_most(file, MAX_INPUT_BYTES).ok().flatten())
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .ok_or(ManifestSourceError::InvalidManifest)?;
                self.cache_hits += 1;
                return library_manifest_from_text(&text);
            }
            return Err(ManifestSourceError::NotAStellarProject);
        }
        self.fallback.manifest_at(repository, commit)
    }
}

/// `stellar.yaml` 편집 실패. 경로 원문은 담지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestEditError {
    /// 파일을 읽거나 쓰지 못함(쓰기 경합 포함: 연 뒤 다른 writer가 바꿈).
    Io {
        /// 단계.
        stage: &'static str,
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
    /// 정규 파일이 아니거나(symlink) UTF-8·문법·스키마 오류.
    Invalid,
    /// 바꾼 매니페스트를 canonical 텍스트로 쓸 수 없음(다시 읽히지 않을 출력).
    Encode,
}

impl fmt::Display for ManifestEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestEditError::Io { stage, kind } => {
                write!(f, "{MANIFEST_FILE} {stage} 실패: {kind:?}")
            }
            ManifestEditError::Invalid => {
                write!(f, "{MANIFEST_FILE}이 올바른 매니페스트가 아닙니다")
            }
            ManifestEditError::Encode => {
                write!(
                    f,
                    "바꾼 {MANIFEST_FILE}을 canonical 텍스트로 쓸 수 없습니다"
                )
            }
        }
    }
}

impl std::error::Error for ManifestEditError {}

/// 읽은 bytes hash와 함께 연 `stellar.yaml`(모듈 문서).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEdit {
    manifest: Manifest,
    original: Vec<u8>,
}

impl ManifestEdit {
    /// `project_root/stellar.yaml`을 엽니다. symlink·정규 파일이 아닌 항목은 거부합니다.
    pub fn open(project_root: &Path) -> Result<ManifestEdit, ManifestEditError> {
        let path = project_root.join(MANIFEST_FILE);
        let meta = std::fs::symlink_metadata(&path).map_err(|e| ManifestEditError::Io {
            stage: "stat",
            kind: e.kind(),
        })?;
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(ManifestEditError::Invalid);
        }
        let file = std::fs::File::open(&path).map_err(|e| ManifestEditError::Io {
            stage: "open",
            kind: e.kind(),
        })?;
        let original = read_at_most(file, MAX_INPUT_BYTES)
            .map_err(|e| ManifestEditError::Io {
                stage: "read",
                kind: e.kind(),
            })?
            .ok_or(ManifestEditError::Invalid)?;
        let text = std::str::from_utf8(&original).map_err(|_| ManifestEditError::Invalid)?;
        let value = load_restricted(text).map_err(|_| ManifestEditError::Invalid)?;
        let manifest = Manifest::from_yaml(&value).map_err(|_| ManifestEditError::Invalid)?;
        Ok(ManifestEdit { manifest, original })
    }

    /// 매니페스트.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// 의존성 선언.
    pub fn declarations(&self) -> &[DependencyDeclaration] {
        &self.manifest.dependencies
    }

    /// 의존성 선언(편집).
    pub fn declarations_mut(&mut self) -> &mut Vec<DependencyDeclaration> {
        &mut self.manifest.dependencies
    }

    /// 바꾼 매니페스트의 canonical bytes. 읽은 bytes와 같으면(바꿀 것 없음) `None`입니다.
    pub fn encoded(&self) -> Result<Option<Vec<u8>>, ManifestEditError> {
        let text = self
            .manifest
            .write()
            .map_err(|_| ManifestEditError::Encode)?;
        // 다시 읽히는지와 같은 모델인지 확인합니다(쓰기 전 검증).
        let reread = load_restricted(&text)
            .ok()
            .and_then(|value| Manifest::from_yaml(&value).ok());
        if reread.as_ref() != Some(&self.manifest) {
            return Err(ManifestEditError::Encode);
        }
        Ok((text.as_bytes() != self.original.as_slice()).then(|| text.into_bytes()))
    }

    /// `bytes`를 씁니다. 연 뒤 파일이 바뀌었으면 쓰지 않고 오류입니다.
    pub fn write(&self, project_root: &Path, bytes: &[u8]) -> Result<(), ManifestEditError> {
        stellar_storage::source_file::write_checked(
            &project_root.join(MANIFEST_FILE),
            bytes,
            Some(ContentHash::compute(&self.original)),
            MAX_INPUT_BYTES as u64,
        )
        .map_err(|e| ManifestEditError::Io {
            stage: "write",
            kind: e.kind(),
        })
    }

    /// [`ManifestEdit::write`]로 쓴 `written`을 연 시점 bytes로 되돌립니다(뒤 단계가 실패했을 때). 그 사이 다른 writer가
    /// 바꿨으면 되돌리지 않고 오류입니다.
    pub fn restore(&self, project_root: &Path, written: &[u8]) -> Result<(), ManifestEditError> {
        stellar_storage::source_file::write_checked(
            &project_root.join(MANIFEST_FILE),
            &self.original,
            Some(ContentHash::compute(written)),
            MAX_INPUT_BYTES as u64,
        )
        .map_err(|e| ManifestEditError::Io {
            stage: "restore",
            kind: e.kind(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest_source::test_git::TempDir;

    const TEXT: &str = "schema_version: 4\nproject:\n  id: \"0195c530-3acd-7bc1-8414-c213f61cb960\"\n  name: \"앱\"\ntarget_profiles: []\ndependencies:\n  ds:\n    source: git\n    library_id: \"0195c530-3acd-7bc1-8414-c213f61cb961\"\n    repository: \"https://example.invalid/ds.git\"\n    version: \"v1\"\n";

    #[test]
    fn edit_writes_canonical_text_only_when_unchanged_since_open() {
        let dir = TempDir::new("manifest-edit");
        let root = dir.path();
        std::fs::write(root.join(MANIFEST_FILE), TEXT).unwrap();
        let mut edit = ManifestEdit::open(root).unwrap();
        assert_eq!(edit.declarations().len(), 1);
        let component = stellar_core_types::reference::ComponentRef::new(
            "0195c530-3acd-7bc1-8414-c213f61cb9a0".parse().unwrap(),
        );
        edit.declarations_mut()[0].components.insert(component);
        let bytes = edit.encoded().unwrap().expect("바뀜");
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(
            text.contains("components:\n      - \"0195c530-3acd-7bc1-8414-c213f61cb9a0\""),
            "{text}"
        );
        edit.write(root, &bytes).unwrap();
        let reopened = ManifestEdit::open(root).unwrap();
        assert!(reopened.declarations()[0].components.contains(&component));
        assert_eq!(reopened.encoded().unwrap(), None, "다시 열면 바꿀 것 없음");
        // 되돌리기는 연 시점 bytes로 돌아갑니다.
        edit.restore(root, &bytes).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join(MANIFEST_FILE)).unwrap(),
            TEXT
        );
        // 연 뒤 다른 writer가 바꿨으면 쓰지 않습니다.
        let stale = ManifestEdit::open(root).unwrap();
        std::fs::write(root.join(MANIFEST_FILE), format!("{TEXT}\n")).unwrap();
        assert!(matches!(
            stale.write(root, &bytes),
            Err(ManifestEditError::Io { stage: "write", .. })
        ));
        assert_eq!(
            std::fs::read_to_string(root.join(MANIFEST_FILE)).unwrap(),
            format!("{TEXT}\n")
        );
    }
}
