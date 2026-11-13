//! 프로젝트 신뢰 grant/revoke API와 OS 사용자 데이터 저장소 (XVI.7.2).
//!
//! IV.5.2의 신뢰 정책([`trust_key`] = [`ProjectIdentity`]의 canonical target)을 그대로 따르며,
//! 신뢰 목록을 **OS 사용자 데이터 영역**의 파일 하나에 기록합니다. 프로젝트 폴더 안의 파일,
//! `.git/config`, Git 전역 설정은 **읽지도 쓰지도 않습니다**(기술 아키텍처 24.3: 신뢰 등록은
//! Git 설정을 변경하지 않음).
//!
//! - 저장 위치: `<data_dir>/trust/trusted-projects.yaml`. `data_dir`은 셸이 정한 Stellar 앱 데이터
//!   디렉터리이며, OS 기본 위치의 base는 [`os_user_data_base`]로 구합니다.
//! - 형식: 제한 YAML(IV.1)의 canonical writer로 기록합니다. 항목은 canonical target 순으로
//!   정렬되어 결정론적입니다.
//! - 기록: 같은 디렉터리의 임시 파일에 쓴 뒤 rename으로 교체합니다(부분 기록 노출 없음). Unix에서는
//!   소유자 전용 권한(`0600`)으로 만듭니다.
//! - 판정: canonical target이 일치하고 **project id도 일치**할 때만 `Trusted`입니다. 같은 경로에
//!   다른 프로젝트가 놓이면 신뢰가 이어지지 않습니다.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use stellar_core_types::ids::StableId;

use crate::identity::{parse_project_id, ProjectIdentity};
use crate::trust::{trust_key, TrustStatus};
use crate::yaml::scalar;
use crate::yaml::{load_restricted, write_canonical, LoaderError, YamlValue};

/// 신뢰 목록 하위 디렉터리 이름.
pub const TRUST_DIR: &str = "trust";
/// 신뢰 목록 파일 이름.
pub const TRUST_FILE: &str = "trusted-projects.yaml";
/// 신뢰 목록 스키마 버전.
pub const TRUST_STORE_SCHEMA_VERSION: u32 = 1;

/// 신뢰 저장소 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustStoreError {
    /// 파일 시스템 오류.
    Io {
        /// 실패한 경로.
        path: PathBuf,
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
    /// 신뢰 목록 파일의 YAML 문법 오류(손상).
    Syntax(LoaderError),
    /// 신뢰 목록 파일의 스키마 오류(손상).
    Schema {
        /// 사유.
        reason: String,
    },
    /// UTF-8로 표현할 수 없는 경로(기록 불가).
    NonUtf8Path {
        /// 문제된 경로.
        path: PathBuf,
    },
}

impl std::fmt::Display for TrustStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrustStoreError::Io { path, kind } => {
                write!(f, "신뢰 목록 입출력 실패 {path:?}: {kind:?}")
            }
            TrustStoreError::Syntax(e) => write!(f, "신뢰 목록 파일 손상: {e}"),
            TrustStoreError::Schema { reason } => write!(f, "신뢰 목록 스키마 오류: {reason}"),
            TrustStoreError::NonUtf8Path { path } => {
                write!(
                    f,
                    "UTF-8이 아닌 경로는 신뢰 목록에 기록할 수 없습니다: {path:?}"
                )
            }
        }
    }
}

impl std::error::Error for TrustStoreError {}

/// OS별 사용자 데이터 영역의 base 디렉터리를 환경 변수로 구합니다.
///
/// - macOS: `$HOME/Library/Application Support`
/// - Windows: `%APPDATA%`
/// - 그 밖의 Unix: `$XDG_DATA_HOME`(절대 경로일 때) 또는 `$HOME/.local/share`
///
/// 앱 전용 하위 폴더 이름은 셸이 정해 이 base에 붙입니다. 환경 변수가 없으면 `None`.
pub fn os_user_data_base() -> Option<PathBuf> {
    user_data_base_from(|key| std::env::var_os(key))
}

/// [`os_user_data_base`]의 순수 함수 버전(환경 조회 주입).
fn user_data_base_from(get: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let non_empty = |key: &str| get(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(target_os = "macos") {
        non_empty("HOME").map(|home| home.join("Library").join("Application Support"))
    } else if cfg!(windows) {
        non_empty("APPDATA")
    } else {
        non_empty("XDG_DATA_HOME")
            .filter(|p| p.is_absolute())
            .or_else(|| non_empty("HOME").map(|home| home.join(".local").join("share")))
    }
}

/// OS 사용자 데이터 영역에 영속하는 신뢰 저장소.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTrustStore {
    /// 신뢰 목록 파일 경로.
    file: PathBuf,
    /// canonical target → project id.
    entries: BTreeMap<PathBuf, StableId>,
}

impl FileTrustStore {
    /// `data_dir`의 신뢰 목록을 엽니다. 파일이 없으면 빈 저장소이며 **파일을 만들지 않습니다**.
    /// 파일이 손상되었으면 오류를 반환합니다(빈 목록으로 덮어쓰지 않음).
    pub fn open(data_dir: &Path) -> Result<FileTrustStore, TrustStoreError> {
        let file = data_dir.join(TRUST_DIR).join(TRUST_FILE);
        let entries = match std::fs::read_to_string(&file) {
            Ok(text) => parse_entries(&text)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(io_error(&file, &e)),
        };
        Ok(FileTrustStore { file, entries })
    }

    /// 신뢰 목록 파일 경로.
    pub fn file_path(&self) -> &Path {
        &self.file
    }

    /// 신뢰된 canonical target 목록(정렬 순).
    pub fn trusted_targets(&self) -> Vec<&Path> {
        self.entries.keys().map(PathBuf::as_path).collect()
    }

    /// identity의 신뢰 상태. canonical target과 project id가 모두 일치해야 `Trusted`입니다.
    pub fn status(&self, identity: &ProjectIdentity) -> TrustStatus {
        match self.entries.get(trust_key(identity)) {
            Some(id) if *id == identity.project_id() => TrustStatus::Trusted,
            _ => TrustStatus::NotTrusted,
        }
    }

    /// identity를 신뢰합니다(grant). 기록에 성공한 뒤에만 메모리 상태를 바꿉니다.
    pub fn grant(&mut self, identity: &ProjectIdentity) -> Result<(), TrustStoreError> {
        let key = trust_key(identity);
        if key.to_str().is_none() {
            return Err(TrustStoreError::NonUtf8Path {
                path: key.to_path_buf(),
            });
        }
        let mut next = self.entries.clone();
        next.insert(key.to_path_buf(), identity.project_id());
        self.persist(&next)?;
        self.entries = next;
        Ok(())
    }

    /// identity의 신뢰를 철회합니다(revoke). 기록이 없으면 파일을 건드리지 않습니다.
    pub fn revoke(&mut self, identity: &ProjectIdentity) -> Result<(), TrustStoreError> {
        let key = trust_key(identity);
        if !self.entries.contains_key(key) {
            return Ok(());
        }
        let mut next = self.entries.clone();
        next.remove(key);
        self.persist(&next)?;
        self.entries = next;
        Ok(())
    }

    /// 신뢰 목록을 임시 파일 + rename으로 기록합니다.
    fn persist(&self, entries: &BTreeMap<PathBuf, StableId>) -> Result<(), TrustStoreError> {
        let text = render_entries(entries)?;
        let dir = match self.file.parent() {
            Some(dir) => dir,
            None => {
                return Err(TrustStoreError::Io {
                    path: self.file.clone(),
                    kind: io::ErrorKind::InvalidInput,
                })
            }
        };
        std::fs::create_dir_all(dir).map_err(|e| io_error(dir, &e))?;

        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = dir.join(format!(".{TRUST_FILE}.tmp-{}-{n}", std::process::id()));
        let result = write_private_file(&temp, text.as_bytes())
            .and_then(|()| std::fs::rename(&temp, &self.file));
        if let Err(e) = result {
            let _ = std::fs::remove_file(&temp);
            return Err(io_error(&self.file, &e));
        }
        Ok(())
    }
}

/// 소유자 전용 권한으로 새 파일을 만들어 내용을 쓰고 디스크에 동기화합니다.
fn write_private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// 신뢰 목록을 canonical YAML로 렌더링합니다(target 정렬 순).
fn render_entries(entries: &BTreeMap<PathBuf, StableId>) -> Result<String, TrustStoreError> {
    let mut items = Vec::with_capacity(entries.len());
    for (target, id) in entries {
        let Some(target_text) = target.to_str() else {
            return Err(TrustStoreError::NonUtf8Path {
                path: target.clone(),
            });
        };
        items.push(YamlValue::Mapping(vec![
            (
                YamlValue::scalar("project_id"),
                YamlValue::scalar(id.to_string()),
            ),
            (YamlValue::scalar("target"), YamlValue::scalar(target_text)),
        ]));
    }
    let root = YamlValue::Mapping(vec![
        (
            YamlValue::scalar("schema_version"),
            YamlValue::scalar(TRUST_STORE_SCHEMA_VERSION.to_string()),
        ),
        (YamlValue::scalar("trusted"), YamlValue::Sequence(items)),
    ]);
    Ok(write_canonical(&root))
}

/// 신뢰 목록 텍스트를 파싱합니다. 알 수 없는 키·중복 target·형식 오류는 손상으로 거부합니다.
fn parse_entries(text: &str) -> Result<BTreeMap<PathBuf, StableId>, TrustStoreError> {
    let value = load_restricted(text).map_err(TrustStoreError::Syntax)?;
    let schema = |reason: &str| TrustStoreError::Schema {
        reason: reason.to_string(),
    };
    let pairs = value
        .as_mapping()
        .ok_or_else(|| schema("최상위는 매핑이어야 합니다"))?;
    for (key, _) in pairs {
        match key.as_scalar() {
            Some("schema_version" | "trusted") => {}
            _ => return Err(schema("알 수 없는 최상위 키")),
        }
    }
    let version = value
        .get("schema_version")
        .and_then(YamlValue::as_scalar)
        .and_then(scalar::parse_u32)
        .ok_or_else(|| schema("schema_version 누락 또는 형식 오류"))?;
    if version != TRUST_STORE_SCHEMA_VERSION {
        return Err(schema("지원하지 않는 schema_version"));
    }
    let items = value
        .get("trusted")
        .and_then(YamlValue::as_sequence)
        .ok_or_else(|| schema("trusted 시퀀스 누락"))?;

    let mut entries = BTreeMap::new();
    for item in items {
        let fields = item
            .as_mapping()
            .ok_or_else(|| schema("trusted 항목은 매핑이어야 합니다"))?;
        for (key, _) in fields {
            match key.as_scalar() {
                Some("project_id" | "target") => {}
                _ => return Err(schema("trusted 항목의 알 수 없는 키")),
            }
        }
        let id = item
            .get("project_id")
            .and_then(YamlValue::as_scalar)
            .and_then(parse_project_id)
            .ok_or_else(|| schema("project_id 누락 또는 형식 오류"))?;
        let target = item
            .get("target")
            .and_then(YamlValue::as_scalar)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .ok_or_else(|| schema("target 누락 또는 절대 경로 아님"))?;
        if entries.insert(target, id).is_some() {
            return Err(schema("중복 target"));
        }
    }
    Ok(entries)
}

/// io 오류를 경로와 함께 변환합니다.
fn io_error(path: &Path, error: &io::Error) -> TrustStoreError {
    TrustStoreError::Io {
        path: path.to_path_buf(),
        kind: error.kind(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::MANIFEST_FILE;
    use std::collections::BTreeMap;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-trust-store-test-{}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    const PROJECT_ID: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";

    /// 프로젝트 폴더(`project/`, 매니페스트·`.git/config`·문서 포함)와 사용자 데이터 폴더(`data/`).
    struct Fixture {
        dir: TempDir,
        project: PathBuf,
        data: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = TempDir::new();
        let project = dir.path.join("project");
        let data = dir.path.join("user data");
        std::fs::create_dir_all(project.join(".git")).unwrap();
        std::fs::create_dir_all(project.join("design")).unwrap();
        std::fs::write(
            project.join(MANIFEST_FILE),
            format!("schema_version: 1\nproject:\n  id: {PROJECT_ID}\n  name: Demo\n"),
        )
        .unwrap();
        std::fs::write(
            project.join(".git/config"),
            "[core]\n\trepositoryformatversion = 0\n",
        )
        .unwrap();
        std::fs::write(project.join("design/page.bin"), [1u8, 2, 3]).unwrap();
        Fixture { dir, project, data }
    }

    fn identity(path: &Path) -> ProjectIdentity {
        ProjectIdentity::new(parse_project_id(PROJECT_ID).unwrap(), path).unwrap()
    }

    /// 디렉터리 트리의 (상대 경로 → bytes) 스냅샷.
    fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let rel = path.strip_prefix(root).unwrap().to_path_buf();
                if path.is_dir() {
                    out.insert(rel, Vec::new());
                    walk(root, &path, out);
                } else {
                    out.insert(rel, std::fs::read(&path).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(root, root, &mut out);
        out
    }

    #[test]
    fn open_missing_store_is_empty_and_creates_nothing() {
        let fx = fixture();
        let store = FileTrustStore::open(&fx.data).unwrap();
        assert_eq!(
            store.status(&identity(&fx.project)),
            TrustStatus::NotTrusted
        );
        assert!(store.trusted_targets().is_empty());
        assert!(!fx.data.exists(), "열기만으로 파일·디렉터리를 만들면 안 됨");
    }

    #[test]
    fn grant_revoke_reopen_round_trip() {
        let fx = fixture();
        let id = identity(&fx.project);

        // grant → 같은 세션에서 Trusted.
        let mut store = FileTrustStore::open(&fx.data).unwrap();
        store.grant(&id).unwrap();
        assert_eq!(store.status(&id), TrustStatus::Trusted);
        assert!(store.file_path().starts_with(&fx.data));
        assert!(store.file_path().is_file());
        drop(store);

        // reopen → 영속된 신뢰 유지.
        let mut reopened = FileTrustStore::open(&fx.data).unwrap();
        assert_eq!(reopened.status(&id), TrustStatus::Trusted);
        assert_eq!(reopened.trusted_targets(), vec![id.canonical_target()]);

        // revoke → reopen 후에도 NotTrusted.
        reopened.revoke(&id).unwrap();
        assert_eq!(reopened.status(&id), TrustStatus::NotTrusted);
        drop(reopened);
        let again = FileTrustStore::open(&fx.data).unwrap();
        assert_eq!(again.status(&id), TrustStatus::NotTrusted);
        assert!(again.trusted_targets().is_empty());
    }

    #[test]
    fn grant_and_revoke_do_not_touch_project_or_git_config() {
        let fx = fixture();
        let before = snapshot(&fx.project);
        let id = identity(&fx.project);

        let mut store = FileTrustStore::open(&fx.data).unwrap();
        store.grant(&id).unwrap();
        assert_eq!(
            snapshot(&fx.project),
            before,
            "grant가 프로젝트 파일을 변경함"
        );
        store.revoke(&id).unwrap();
        assert_eq!(
            snapshot(&fx.project),
            before,
            "revoke가 프로젝트 파일을 변경함"
        );
        // 신뢰 목록은 프로젝트 밖(사용자 데이터 폴더)에만 기록됨.
        assert!(!store.file_path().starts_with(&fx.project));
        assert_eq!(
            std::fs::read_to_string(fx.project.join(".git/config")).unwrap(),
            "[core]\n\trepositoryformatversion = 0\n"
        );
    }

    #[test]
    fn persisted_file_is_canonical_and_sorted() {
        let fx = fixture();
        let other = fx.dir.path.join("another");
        std::fs::create_dir_all(&other).unwrap();
        let a = identity(&fx.project);
        let b = identity(&other);

        let mut store = FileTrustStore::open(&fx.data).unwrap();
        // 삽입 순서와 무관하게 target 정렬 순으로 기록.
        store.grant(&a).unwrap();
        store.grant(&b).unwrap();
        let first = std::fs::read_to_string(store.file_path()).unwrap();

        let data2 = fx.dir.path.join("data2");
        let mut store2 = FileTrustStore::open(&data2).unwrap();
        store2.grant(&b).unwrap();
        store2.grant(&a).unwrap();
        let second = std::fs::read_to_string(store2.file_path()).unwrap();
        assert_eq!(first, second);
        assert!(first.starts_with("schema_version: \"1\"\ntrusted:\n"));
        assert!(first.ends_with('\n'));
        // 임시 파일이 남지 않음.
        let names: Vec<_> = std::fs::read_dir(store.file_path().parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![OsString::from(TRUST_FILE)]);
    }

    #[test]
    fn different_project_id_at_same_target_is_not_trusted() {
        let fx = fixture();
        let mut store = FileTrustStore::open(&fx.data).unwrap();
        store.grant(&identity(&fx.project)).unwrap();
        // 같은 경로에 다른 project id의 프로젝트가 놓인 경우.
        let impostor = ProjectIdentity::new(StableId::new_v7(9, 9, 9), &fx.project).unwrap();
        assert_eq!(store.status(&impostor), TrustStatus::NotTrusted);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_alias_shares_and_rename_drops_persisted_trust() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        let alias = fx.dir.path.join("alias");
        symlink(&fx.project, &alias).unwrap();

        let mut store = FileTrustStore::open(&fx.data).unwrap();
        store.grant(&identity(&fx.project)).unwrap();
        drop(store);

        let reopened = FileTrustStore::open(&fx.data).unwrap();
        // IV.5.2 정책: alias는 신뢰 공유.
        assert_eq!(reopened.status(&identity(&alias)), TrustStatus::Trusted);
        // root rename은 신뢰 비계승.
        let moved = fx.dir.path.join("moved");
        std::fs::remove_file(&alias).unwrap();
        std::fs::rename(&fx.project, &moved).unwrap();
        assert_eq!(reopened.status(&identity(&moved)), TrustStatus::NotTrusted);
    }

    #[cfg(unix)]
    #[test]
    fn trust_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let fx = fixture();
        let mut store = FileTrustStore::open(&fx.data).unwrap();
        store.grant(&identity(&fx.project)).unwrap();
        let mode = std::fs::metadata(store.file_path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn revoke_without_record_writes_nothing() {
        let fx = fixture();
        let mut store = FileTrustStore::open(&fx.data).unwrap();
        store.revoke(&identity(&fx.project)).unwrap();
        assert!(!fx.data.exists());
    }

    #[test]
    fn corrupt_store_is_rejected_and_preserved() {
        let fx = fixture();
        let file = fx.data.join(TRUST_DIR).join(TRUST_FILE);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        for bad in [
            "trusted: &a []\n",                                // anchor(문법 손상)
            "schema_version: 1\n",                             // trusted 누락
            "schema_version: 2\ntrusted: []\n",                // 미지원 버전
            "schema_version: 1\ntrusted: []\nextra: x\n",      // 알 수 없는 키
            "schema_version: 1\ntrusted:\n  - target: rel\n    project_id: 0195c530-3acd-7bc1-8414-c213f61cb965\n",
        ] {
            std::fs::write(&file, bad).unwrap();
            assert!(FileTrustStore::open(&fx.data).is_err(), "입력: {bad:?}");
            // 손상 파일을 덮어쓰지 않음.
            assert_eq!(std::fs::read_to_string(&file).unwrap(), bad);
        }
        let dup = format!(
            "schema_version: 1\ntrusted:\n  - project_id: {PROJECT_ID}\n    target: /a\n  - project_id: {PROJECT_ID}\n    target: /a\n"
        );
        std::fs::write(&file, &dup).unwrap();
        assert_eq!(
            FileTrustStore::open(&fx.data),
            Err(TrustStoreError::Schema {
                reason: "중복 target".to_string()
            })
        );
    }

    #[test]
    fn user_data_base_follows_platform_convention() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |key: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| OsString::from(*v))
            }
        };
        let home_only = env(&[
            ("HOME", "/home/u"),
            ("APPDATA", "C:\\Users\\u\\AppData\\Roaming"),
        ]);
        let base = user_data_base_from(home_only);
        if cfg!(target_os = "macos") {
            assert_eq!(
                base,
                Some(PathBuf::from("/home/u/Library/Application Support"))
            );
        } else if cfg!(windows) {
            assert_eq!(base, Some(PathBuf::from("C:\\Users\\u\\AppData\\Roaming")));
        } else {
            assert_eq!(base, Some(PathBuf::from("/home/u/.local/share")));
        }
        // 환경 변수가 없으면 None(추측하지 않음).
        assert_eq!(user_data_base_from(env(&[])), None);
    }
}
