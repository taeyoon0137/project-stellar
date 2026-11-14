//! history journal 기록 위치와 기록 (XXV.1.2, 로드맵).
//!
//! 창 history의 완료된 transaction(정방향·역방향)과 선택 항목을 XXV.1.1 journal로 기록합니다. 위치는
//! 두 정책 중 하나입니다([`JournalLocation`]):
//!
//! - **OS 앱 데이터**(권장, 복구 스냅샷 §19과 같은 정책): `<app-data>/history-journal/<project-key>/`.
//!   프로젝트 폴더 밖이므로 Git이 볼 수 없습니다. `project-key`는 프로젝트 이름·경로 텍스트를 담지 않는
//!   caller 제공 토큰입니다(예: project crate가 ProjectIdentity hash로 만든 값, V.9.1과 같은 규칙).
//! - **프로젝트 내부 ignored path**: `<project>/.stellar/cache/history/`(아키텍처 §19 "Git 제외").
//!   프로젝트 `.gitignore`에 의존하지 않도록 디렉터리 안에 `*` 한 줄짜리 guard `.gitignore`를 둡니다 —
//!   하위 `.gitignore`가 상위 규칙보다 우선하므로 guard 자신을 포함한 모든 파일이 Git status에 나타나지
//!   않습니다.
//!
//! **Git 추적 금지 검사**([`JournalStore::check_untracked`])는 기록할 때마다 다시 실행합니다. 앱 데이터는
//! 프로젝트 밖(symlink 해석 후)이어야 하고, 프로젝트 내부 경로는 symlink로 프로젝트 밖을 가리키지 않아야
//! 하며 guard 내용이 정확해야 합니다. 이미 Git이 추적 중인 파일이 그 경로에 있는지는 Git index를 읽어야
//! 알 수 있으므로 git-porcelain을 쓰는 상위 계층(project)의 몫입니다.
//!
//! **기록 시점 계약**: journal의 undo stack 맨 위 문서 상태는 base 세대 내용과 같아야 합니다(XXV.1.1).
//! 따라서 [`JournalStore::record_save_point`]는 문서를 그 base 세대로 저장한 직후(자동 저장 포함) 부릅니다.
//! preview(drag·scrub) 중에는 완료되지 않은 변경이 문서에 있어 거부합니다.
//!
//! 기록은 같은 디렉터리 임시 파일에 쓴 뒤 rename으로 교체합니다(부분 파일 노출 없음). 기존 journal이 이
//! writer보다 높은 최소 writer version을 요구하면(V.8.1 읽기 전용·거부) 덮어쓰지 않습니다.
//!
//! ```text
//! <journal-dir>/
//! ├── .gitignore                 # 프로젝트 내부 정책만: "*"
//! └── <window-session-uuid>.stlhjnl
//! ```

use std::fmt;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use stellar_commands::id::WindowSessionId;

use crate::journal::{
    HistoryJournal, JournalAccess, JournalBase, JournalCompaction, JournalError, JournalHeader,
    JOURNAL_HEADER_LEN, JOURNAL_SCHEMA_VERSION,
};
use crate::journal_retention::JournalRetention;
use crate::window::WindowHistory;

/// journal 파일 확장자.
pub const JOURNAL_EXTENSION: &str = "stlhjnl";
/// 앱 데이터 아래 journal 디렉터리 이름.
pub const APP_DATA_DIR: &str = "history-journal";
/// 프로젝트 내부 journal 디렉터리(프로젝트 root 기준).
pub const PROJECT_LOCAL_DIR: [&str; 3] = [".stellar", "cache", "history"];
/// 프로젝트 내부 정책의 guard 파일 이름.
pub const GUARD_FILE: &str = ".gitignore";
/// guard 내용: 디렉터리 안 모든 파일(guard 자신 포함)을 Git에서 제외합니다.
pub const GUARD_CONTENT: &str =
    "# Stellar history journal (XXV.1.2): Git에 추적하지 않습니다.\n*\n";
/// project key 최대 길이.
pub const PROJECT_KEY_MAX_LEN: usize = 128;

/// 앱 데이터 정책에서 프로젝트를 구분하는 토큰. 소문자 ASCII 영숫자와 `-`만 허용합니다(경로에 프로젝트
/// 이름·텍스트를 노출하지 않음, V.9.1 경로 정책).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JournalProjectKey(String);

impl JournalProjectKey {
    /// 토큰을 검증해 만듭니다.
    pub fn new(key: &str) -> Result<JournalProjectKey, JournalStoreError> {
        let valid = !key.is_empty()
            && key.len() <= PROJECT_KEY_MAX_LEN
            && key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if valid {
            Ok(JournalProjectKey(key.to_string()))
        } else {
            Err(JournalStoreError::InvalidProjectKey)
        }
    }

    /// 토큰 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// journal 기록 위치 정책.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalLocation {
    /// OS 앱 데이터 `<app_data_dir>/history-journal/<project_key>/`.
    AppData {
        /// 플랫폼 셸이 주입하는 앱 데이터 경로(정규화된 절대 경로).
        app_data_dir: PathBuf,
        /// 프로젝트 구분 토큰.
        project_key: JournalProjectKey,
    },
    /// 프로젝트 내부 `.stellar/cache/history/`(guard `.gitignore` 포함).
    ProjectLocal,
}

/// journal 저장 오류. 경로 원문은 담지 않습니다(XV.1.4 경로 민감값).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalStoreError {
    /// 경로가 절대 경로가 아니거나 `.`·`..` component를 포함합니다.
    NotNormalizedAbsolute,
    /// project key 형식이 아닙니다.
    InvalidProjectKey,
    /// 앱 데이터 journal 디렉터리가 프로젝트 폴더 안에 있습니다(Git이 볼 수 있음).
    InsideProject,
    /// 프로젝트 폴더가 journal 디렉터리 안에 있습니다.
    ProjectInsideStore,
    /// 프로젝트 내부 journal 디렉터리가 symlink로 프로젝트 밖을 가리킵니다.
    EscapesProject,
    /// guard `.gitignore`가 있지만 내용이 달라 Git 제외를 보장할 수 없습니다.
    GuardMismatch,
    /// preview가 진행 중이라 완료된 상태가 아닙니다.
    PreviewActive,
    /// undo·redo 재생 중이라 완료된 상태가 아닙니다(XXV.1.4 기준점 변경).
    Replaying,
    /// 기존 journal이 이 writer보다 높은 최소 writer version을 요구합니다(V.8.1 읽기 전용).
    WriterTooOld {
        /// journal이 요구하는 최소 writer schema.
        required: u32,
        /// 이 writer의 schema.
        writer: u32,
    },
    /// 기존 journal을 이 writer가 읽을 수 없어 덮어쓰지 않습니다(다른 형식·최소 reader·모순 선언).
    NotWritable(JournalError),
    /// 파일 이름과 journal 안 창 id가 다릅니다.
    WindowMismatch,
    /// retention 정책 값이 모순입니다(XXV.1.5).
    InvalidRetention {
        /// 원인.
        reason: &'static str,
    },
    /// journal bytes 오류.
    Journal(JournalError),
    /// 파일시스템 I/O 실패.
    Io {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
}

impl fmt::Display for JournalStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JournalStoreError::NotNormalizedAbsolute => {
                f.write_str("journal 경로는 정규화된 절대 경로여야 합니다")
            }
            JournalStoreError::InvalidProjectKey => {
                f.write_str("project key는 소문자 영숫자와 '-'로 된 1~128자여야 합니다")
            }
            JournalStoreError::InsideProject => {
                f.write_str("앱 데이터 journal은 프로젝트 폴더 밖에 있어야 합니다")
            }
            JournalStoreError::ProjectInsideStore => {
                f.write_str("프로젝트 폴더가 journal 디렉터리 안에 있습니다")
            }
            JournalStoreError::EscapesProject => {
                f.write_str("프로젝트 내부 journal 경로가 프로젝트 밖을 가리킵니다")
            }
            JournalStoreError::GuardMismatch => {
                f.write_str("journal guard .gitignore 내용이 달라 Git 제외를 보장할 수 없습니다")
            }
            JournalStoreError::PreviewActive => {
                f.write_str("preview가 진행 중이라 journal을 기록할 수 없습니다")
            }
            JournalStoreError::Replaying => {
                f.write_str("undo·redo 재생 중이라 journal session을 바꿀 수 없습니다")
            }
            JournalStoreError::WriterTooOld { required, writer } => write!(
                f,
                "기존 journal은 schema {required} 이상 writer만 덮어쓸 수 있습니다(현재 {writer})"
            ),
            JournalStoreError::NotWritable(error) => {
                write!(f, "기존 journal을 덮어쓸 수 없습니다: {error}")
            }
            JournalStoreError::WindowMismatch => {
                f.write_str("journal 파일의 창 id가 파일 이름과 다릅니다")
            }
            JournalStoreError::InvalidRetention { reason } => {
                write!(f, "journal retention 정책 오류: {reason}")
            }
            JournalStoreError::Journal(error) => write!(f, "{error}"),
            JournalStoreError::Io { kind } => write!(f, "journal I/O 실패: {kind:?}"),
        }
    }
}

impl std::error::Error for JournalStoreError {}

impl From<std::io::Error> for JournalStoreError {
    fn from(error: std::io::Error) -> JournalStoreError {
        JournalStoreError::Io { kind: error.kind() }
    }
}

impl From<JournalError> for JournalStoreError {
    fn from(error: JournalError) -> JournalStoreError {
        JournalStoreError::Journal(error)
    }
}

fn is_normalized_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|c| !matches!(c, Component::CurDir | Component::ParentDir))
}

/// 아직 없을 수 있는 경로를 존재하는 가장 가까운 조상까지 canonicalize한 뒤 나머지를 붙입니다.
pub(crate) fn resolve_lenient(path: &Path) -> Result<PathBuf, JournalStoreError> {
    let mut existing = path;
    let mut rest = Vec::new();
    loop {
        match std::fs::canonicalize(existing) {
            Ok(mut out) => {
                for part in rest.iter().rev() {
                    out.push(part);
                }
                return Ok(out);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
                    return Err(e.into());
                };
                rest.push(name.to_os_string());
                existing = parent;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// 임시 파일 이름 표지(`.<name>.tmp-<pid>-<n>`). retention이 남은 임시 파일을 찾을 때 씁니다.
pub(crate) const TEMP_MARKER: &str = ".tmp-";

/// 같은 디렉터리 임시 파일에 쓰고 rename으로 교체합니다. 임시 파일 이름은 `.`으로 시작합니다.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), JournalStoreError> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = path
        .parent()
        .ok_or(JournalStoreError::NotNormalizedAbsolute)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(JournalStoreError::NotNormalizedAbsolute)?;
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = dir.join(format!(".{name}{TEMP_MARKER}{}-{n}", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}

/// 한 프로젝트의 history journal 저장소.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalStore {
    project_root: PathBuf,
    dir: PathBuf,
    location: JournalLocation,
    retention: JournalRetention,
}

impl JournalStore {
    /// 저장소를 열고 Git 추적 금지 검사를 통과한 디렉터리를 준비합니다(프로젝트 내부 정책은 guard 생성).
    pub fn open(
        project_root: &Path,
        location: JournalLocation,
    ) -> Result<JournalStore, JournalStoreError> {
        if !is_normalized_absolute(project_root) {
            return Err(JournalStoreError::NotNormalizedAbsolute);
        }
        let project_root = std::fs::canonicalize(project_root)?;
        let dir = match &location {
            JournalLocation::AppData {
                app_data_dir,
                project_key,
            } => {
                if !is_normalized_absolute(app_data_dir) {
                    return Err(JournalStoreError::NotNormalizedAbsolute);
                }
                app_data_dir.join(APP_DATA_DIR).join(project_key.as_str())
            }
            JournalLocation::ProjectLocal => PROJECT_LOCAL_DIR
                .iter()
                .fold(project_root.clone(), |path, part| path.join(part)),
        };
        let store = JournalStore {
            project_root,
            dir,
            location,
            retention: JournalRetention::default(),
        };
        store.check_location()?;
        std::fs::create_dir_all(&store.dir)?;
        if store.location == JournalLocation::ProjectLocal {
            store.ensure_guard()?;
        }
        store.check_untracked()?;
        Ok(store)
    }

    /// retention 정책을 바꿉니다(XXV.1.5). 정책이 모순이면 거부합니다.
    pub fn with_retention(
        mut self,
        retention: JournalRetention,
    ) -> Result<JournalStore, JournalStoreError> {
        retention.validate()?;
        self.retention = retention;
        Ok(self)
    }

    /// retention 정책(XXV.1.5).
    pub fn retention(&self) -> &JournalRetention {
        &self.retention
    }

    /// journal 디렉터리.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 위치 정책.
    pub fn location(&self) -> &JournalLocation {
        &self.location
    }

    /// 프로젝트 root(canonical).
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// 창의 journal 파일 경로.
    pub fn journal_path(&self, window: WindowSessionId) -> PathBuf {
        self.dir.join(format!("{window}.{JOURNAL_EXTENSION}"))
    }

    /// 경로 관계 검사(디스크는 바꾸지 않음).
    fn check_location(&self) -> Result<(), JournalStoreError> {
        let resolved = resolve_lenient(&self.dir)?;
        match self.location {
            JournalLocation::AppData { .. } => {
                if resolved.starts_with(&self.project_root) {
                    return Err(JournalStoreError::InsideProject);
                }
                if self.project_root.starts_with(&resolved) {
                    return Err(JournalStoreError::ProjectInsideStore);
                }
            }
            JournalLocation::ProjectLocal => {
                if !resolved.starts_with(&self.project_root) {
                    return Err(JournalStoreError::EscapesProject);
                }
            }
        }
        Ok(())
    }

    fn guard_path(&self) -> PathBuf {
        self.dir.join(GUARD_FILE)
    }

    /// guard가 없으면 만들고, 있으면 내용이 정확한지 확인합니다.
    fn ensure_guard(&self) -> Result<(), JournalStoreError> {
        match std::fs::read(self.guard_path()) {
            Ok(bytes) if bytes == GUARD_CONTENT.as_bytes() => Ok(()),
            Ok(_) => Err(JournalStoreError::GuardMismatch),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                write_atomic(&self.guard_path(), GUARD_CONTENT.as_bytes())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Git 추적 금지 검사: 앱 데이터는 프로젝트 밖, 프로젝트 내부 경로는 프로젝트 안이면서 guard가 정확해야
    /// 합니다. 기록 전마다 다시 실행해 열린 뒤 바뀐 경로(symlink 교체, guard 수정)도 거부합니다.
    pub fn check_untracked(&self) -> Result<(), JournalStoreError> {
        self.check_location()?;
        if self.location == JournalLocation::ProjectLocal {
            match std::fs::read(self.guard_path()) {
                Ok(bytes) if bytes == GUARD_CONTENT.as_bytes() => {}
                Ok(_) => return Err(JournalStoreError::GuardMismatch),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(JournalStoreError::GuardMismatch)
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// 기존 journal을 이 writer가 덮어써도 되는지(V.8.1 편집 가능일 때만). 깨진 파일(magic·길이 위반)은
    /// 이 형식의 잔여물이라 덮어씁니다.
    fn ensure_overwritable(&self, path: &Path) -> Result<(), JournalStoreError> {
        let mut prefix = Vec::with_capacity(JOURNAL_HEADER_LEN);
        match std::fs::File::open(path) {
            Ok(file) => {
                file.take(JOURNAL_HEADER_LEN as u64)
                    .read_to_end(&mut prefix)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        match JournalHeader::decode_prefix(&prefix) {
            Ok(header) => match header.access(JOURNAL_SCHEMA_VERSION) {
                Ok(JournalAccess::Editable) => Ok(()),
                Ok(JournalAccess::ReadOnly) => Err(JournalStoreError::WriterTooOld {
                    required: header.minimum_writer_version,
                    writer: JOURNAL_SCHEMA_VERSION,
                }),
                Err(error) => Err(JournalStoreError::NotWritable(error)),
            },
            Err(JournalError::BadMagic | JournalError::Malformed { .. }) => Ok(()),
            Err(error) => Err(JournalStoreError::NotWritable(error)),
        }
    }

    /// journal을 그 창의 파일로 기록합니다(원자적 교체). 기록한 경로를 돌려줍니다. retention 정책의 journal
    /// 하나 상한을 넘으면 현재 위치에서 먼 항목부터 버려 기록합니다(XXV.1.5 compaction).
    pub fn write(&self, journal: &HistoryJournal) -> Result<PathBuf, JournalStoreError> {
        Ok(self.write_reporting(journal)?.0)
    }

    /// [`JournalStore::write`]와 같고 compaction 결과도 돌려줍니다.
    pub fn write_reporting(
        &self,
        journal: &HistoryJournal,
    ) -> Result<(PathBuf, JournalCompaction), JournalStoreError> {
        self.check_untracked()?;
        let path = self.journal_path(journal.header.window);
        self.ensure_overwritable(&path)?;
        let (bytes, compaction) = journal.encode_bounded(
            self.retention.max_journal_bytes,
            self.retention.max_journal_entries,
        );
        write_atomic(&path, &bytes)?;
        Ok((path, compaction))
    }

    /// 저장 직후(문서 = base 세대) 창 history의 완료된 항목 전체(undo·redo, 정방향·역방향)를 기록합니다.
    pub fn record_save_point(
        &self,
        window: &WindowHistory,
        base: JournalBase,
    ) -> Result<PathBuf, JournalStoreError> {
        if window.history().preview_active() {
            return Err(JournalStoreError::PreviewActive);
        }
        self.write(&HistoryJournal::capture(
            window.window(),
            base,
            window.history(),
        ))
    }

    /// 창의 journal을 읽습니다. 없으면 `Ok(None)`.
    pub fn read(
        &self,
        window: WindowSessionId,
    ) -> Result<Option<HistoryJournal>, JournalStoreError> {
        let bytes = match std::fs::read(self.journal_path(window)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let journal = HistoryJournal::decode(&bytes)?;
        if journal.header.window != window {
            return Err(JournalStoreError::WindowMismatch);
        }
        Ok(Some(journal))
    }

    /// 창의 journal을 지웁니다. 없으면 아무 일도 하지 않습니다.
    pub fn remove(&self, window: WindowSessionId) -> Result<(), JournalStoreError> {
        match std::fs::remove_file(self.journal_path(window)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::test_fixtures::{sid, small_state, tx};
    use crate::journal::tests::{base, rich_history};
    use crate::journal::{JOURNAL_FORMAT_VERSION, JOURNAL_MAGIC};
    use crate::test_fs::{list_files, FixtureGit, TempDir};
    use stellar_commands::command::Command;

    fn app_data(dir: &Path) -> JournalLocation {
        JournalLocation::AppData {
            app_data_dir: dir.to_path_buf(),
            project_key: JournalProjectKey::new("sha256-0123abcd").unwrap(),
        }
    }

    /// 두 정책으로 각각 기록·읽기·삭제하고, 프로젝트 Git status와 파일 목록이 변하지 않는지 확인합니다.
    #[test]
    fn recording_journals_leaves_git_status_unchanged() {
        // 검증 기준(XXV.1.2): `git status` 변화 없음.
        let project = TempDir::new("journal-project");
        let app = TempDir::new("journal-appdata");
        let home = TempDir::new("journal-home");
        let git = FixtureGit::find(home.path());
        match &git {
            Some(git) => git.init_repository(project.path()),
            None => {
                std::fs::write(project.path().join("stellar.yaml"), "schema_version: 1\n").unwrap()
            }
        }
        // 사용자가 .stellar 전체를 다시 포함시키는 규칙을 둬도 guard가 우선합니다.
        std::fs::write(
            project.path().join(".gitignore"),
            "!.stellar/\n!.stellar/**\n",
        )
        .unwrap();
        if let Some(git) = &git {
            git.ok(project.path(), &["add", ".gitignore"]);
            git.ok(project.path(), &["commit", "-q", "-m", "ignore rules"]);
        }
        let status_before = git.as_ref().map(|g| g.status(project.path(), false));
        let app_files_before = list_files(app.path());
        assert!(app_files_before.is_empty());

        let history = rich_history();
        for location in [app_data(app.path()), JournalLocation::ProjectLocal] {
            let store = JournalStore::open(project.path(), location.clone()).unwrap();
            let path = store.record_save_point(&history, base(7)).unwrap();
            assert_eq!(path, store.journal_path(history.window()));
            assert!(path.starts_with(store.dir()));
            // 완료된 transaction과 역 transaction(undo·redo 모두)이 그대로 기록됩니다.
            let read = store.read(history.window()).unwrap().unwrap();
            assert_eq!(
                read,
                HistoryJournal::capture(history.window(), base(7), history.history())
            );
            assert_eq!(read.undo.len() + read.redo.len(), 6);
            // 다시 기록(덮어쓰기)해도 임시 파일이 남지 않습니다.
            store.record_save_point(&history, base(8)).unwrap();
            assert_eq!(
                store.read(history.window()).unwrap().unwrap().header.base,
                base(8)
            );
            let leftovers: Vec<String> = list_files(store.dir())
                .into_iter()
                .filter(|p| p.contains(".tmp-"))
                .collect();
            assert!(leftovers.is_empty(), "{leftovers:?}");
            match location {
                JournalLocation::AppData { .. } => {
                    assert!(store.dir().starts_with(app.path()));
                    assert!(!store.dir().starts_with(project.path()));
                    assert!(
                        !store
                            .dir()
                            .display()
                            .to_string()
                            .contains("journal-project"),
                        "앱 데이터 경로에 프로젝트 경로 텍스트 없음"
                    );
                }
                JournalLocation::ProjectLocal => {
                    assert_eq!(
                        store.dir(),
                        project.path().join(".stellar/cache/history").as_path()
                    );
                    assert_eq!(
                        std::fs::read_to_string(store.dir().join(GUARD_FILE)).unwrap(),
                        GUARD_CONTENT
                    );
                    if let Some(git) = &git {
                        let (ignored, _) = git.run(
                            project.path(),
                            &["check-ignore", "-q", path.to_str().unwrap()],
                        );
                        assert!(ignored, "journal 파일은 Git 제외");
                    }
                }
            }
            // Git status는 기록 전과 같습니다(untracked 포함).
            if let (Some(git), Some(before)) = (&git, &status_before) {
                assert_eq!(&git.status(project.path(), false), before, "{location:?}");
            }
        }
        // 앱 데이터 정책은 프로젝트 폴더에 아무것도 만들지 않습니다(ignored 포함 status 비교).
        let project_only = TempDir::new("journal-project-appdata-only");
        let app_only = TempDir::new("journal-appdata-only");
        if let Some(git) = &git {
            git.init_repository(project_only.path());
        } else {
            std::fs::write(project_only.path().join("stellar.yaml"), "x\n").unwrap();
        }
        let files_before = list_files(project_only.path());
        let ignored_before = git.as_ref().map(|g| g.status(project_only.path(), true));
        let store = JournalStore::open(project_only.path(), app_data(app_only.path())).unwrap();
        store.record_save_point(&history, base(1)).unwrap();
        store.remove(history.window()).unwrap();
        store.remove(history.window()).unwrap();
        assert_eq!(store.read(history.window()).unwrap(), None);
        assert_eq!(list_files(project_only.path()), files_before);
        if let (Some(git), Some(before)) = (&git, ignored_before) {
            assert_eq!(git.status(project_only.path(), true), before);
            assert_eq!(before, "", "깨끗한 저장소");
        }
    }

    #[test]
    fn git_tracking_guards_reject_unsafe_locations() {
        let project = TempDir::new("journal-guard-project");
        let outside = TempDir::new("journal-guard-outside");
        // 앱 데이터가 프로젝트 안이면 거부(Git이 볼 수 있음), 프로젝트를 품어도 거부.
        assert_eq!(
            JournalStore::open(project.path(), app_data(&project.path().join("app"))),
            Err(JournalStoreError::InsideProject)
        );
        let nested = project.path().join("history-journal/sha256-0123abcd/sub");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(
            JournalStore::open(&nested, app_data(project.path())),
            Err(JournalStoreError::ProjectInsideStore)
        );
        assert_eq!(
            JournalStore::open(Path::new("relative"), JournalLocation::ProjectLocal),
            Err(JournalStoreError::NotNormalizedAbsolute)
        );
        assert_eq!(
            JournalStore::open(project.path(), app_data(&outside.path().join("../x"))),
            Err(JournalStoreError::NotNormalizedAbsolute)
        );
        for bad in ["", "Upper", "has space", "../x", &"a".repeat(129)] {
            assert_eq!(
                JournalProjectKey::new(bad),
                Err(JournalStoreError::InvalidProjectKey),
                "{bad:?}"
            );
        }

        // guard 내용이 바뀌면 열기·기록 모두 거부합니다(기록 전 재검사).
        let store = JournalStore::open(project.path(), JournalLocation::ProjectLocal).unwrap();
        let history = rich_history();
        std::fs::write(store.dir().join(GUARD_FILE), "!*\n").unwrap();
        assert_eq!(
            store.record_save_point(&history, base(1)),
            Err(JournalStoreError::GuardMismatch)
        );
        assert!(!store.journal_path(history.window()).exists());
        assert_eq!(
            JournalStore::open(project.path(), JournalLocation::ProjectLocal),
            Err(JournalStoreError::GuardMismatch)
        );
        std::fs::remove_file(store.dir().join(GUARD_FILE)).unwrap();
        assert_eq!(
            store.record_save_point(&history, base(1)),
            Err(JournalStoreError::GuardMismatch),
            "guard가 사라져도 거부"
        );

        // `.stellar`가 프로젝트 밖을 가리키는 symlink면 거부합니다.
        #[cfg(unix)]
        {
            let linked = TempDir::new("journal-guard-linked");
            std::os::unix::fs::symlink(outside.path(), linked.path().join(".stellar")).unwrap();
            assert_eq!(
                JournalStore::open(linked.path(), JournalLocation::ProjectLocal),
                Err(JournalStoreError::EscapesProject)
            );
            assert!(
                list_files(outside.path()).is_empty(),
                "밖에 아무것도 만들지 않음"
            );
        }
    }

    #[test]
    fn preview_and_newer_writer_journals_are_not_overwritten() {
        let project = TempDir::new("journal-writer-project");
        let store = JournalStore::open(project.path(), JournalLocation::ProjectLocal).unwrap();
        let window = WindowSessionId::from_stable_id(sid(600));
        let mut history = WindowHistory::new(window);
        let mut doc = small_state();
        let t = tx(&doc, 1_000, Command::rename_node(sid(11), "a"));
        history.record_document(&mut doc, &t, None).unwrap();
        // preview 중에는 완료 상태가 아니라 기록하지 않습니다.
        history.begin_pointer_gesture(1).unwrap();
        let t = tx(&doc, 1_001, Command::rename_node(sid(12), "drag"));
        history.update_preview(&mut doc, &t).unwrap();
        assert_eq!(
            store.record_save_point(&history, base(1)),
            Err(JournalStoreError::PreviewActive)
        );
        history.end_preview().unwrap();
        let path = store.record_save_point(&history, base(1)).unwrap();

        // 미래 writer가 최소 writer version을 올린 journal(읽기 전용)은 덮어쓰지 않습니다.
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[12..16].copy_from_slice(&3u32.to_le_bytes()); // schema
        bytes[20..24].copy_from_slice(&2u32.to_le_bytes()); // 최소 writer
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(
            store.record_save_point(&history, base(2)),
            Err(JournalStoreError::WriterTooOld {
                required: 2,
                writer: 1
            })
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "미래 journal 보존");
        // 다른 형식 버전도 덮어쓰지 않습니다.
        let mut foreign = JOURNAL_MAGIC.to_vec();
        foreign.extend_from_slice(&(JOURNAL_FORMAT_VERSION + 1).to_le_bytes());
        std::fs::write(&path, &foreign).unwrap();
        assert_eq!(
            store.record_save_point(&history, base(2)),
            Err(JournalStoreError::NotWritable(
                JournalError::UnsupportedFormat {
                    found: JOURNAL_FORMAT_VERSION + 1
                }
            ))
        );
        // 깨진 잔여물(magic 아님)은 덮어씁니다.
        std::fs::write(&path, b"garbage").unwrap();
        store.record_save_point(&history, base(2)).unwrap();
        assert_eq!(store.read(window).unwrap().unwrap().header.base, base(2));
        // 파일 이름과 내용의 창이 다르면 읽기를 거부합니다.
        let other = WindowSessionId::from_stable_id(sid(601));
        std::fs::copy(&path, store.journal_path(other)).unwrap();
        assert_eq!(store.read(other), Err(JournalStoreError::WindowMismatch));
    }
}
