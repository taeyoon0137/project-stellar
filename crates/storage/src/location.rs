//! chunk cache의 위치 (ADR-0122).
//!
//! 텍스트 원본(`design/**/*.stellar.yaml`)이 Git 원본이고 chunk generation은 로컬 cache입니다(ADR-0059 결정 3,
//! ADR-0088). cache는 Git이 추적하지 않는 `.stellar/cache/chunks/` 아래에 둡니다.
//!
//! ```text
//! <프로젝트>/.stellar/cache/
//! ├── .gitignore                     # `*` (Git 제외, cache를 처음 쓸 때 만듦)
//! ├── text-source.stamp              # 원본·cache 짝 (ADR-0088)
//! └── chunks/
//!     ├── current                    # current generation pointer
//!     ├── generations/<hash>.starmanifest
//!     └── objects/<prefix>/<hash>.starchunk
//! ```
//!
//! 저장소의 모든 경로 함수(current pointer·manifest·chunk object·scanner·GC)와 앱 열기·CLI·MCP·merge driver·
//! semantic Git은 이 모듈의 함수로 cache 위치를 얻습니다. 같은 경로를 다른 곳에서 따로 조립하지 않습니다.
//!
//! # 옛 배치(legacy)
//!
//! ADR-0122 이전 프로젝트는 같은 파일을 `design/current`·`design/generations/`·`design/objects/`에 두었고 Git이
//! 추적했습니다. 저장소는 옛 배치를 읽지 않습니다. [`move_legacy_layout`]이 옛 배치를 새 위치로 한 번 옮기고
//! (내용 주소 파일은 같은 이름이면 같은 bytes), Git index에서 빼는 일은 Git을 아는 호출자
//! (`stellar_project::cache`)가 합니다.

use std::fmt;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use crate::atomic_file::write_file_atomic;
use crate::fs_guard::{ensure_real_dirs, read_regular_file, FileIdentity, Link};

/// 로컬 cache 폴더(프로젝트 root 기준 구성 요소). 텍스트 원본 stamp와 같은 폴더입니다.
pub const LOCAL_CACHE_DIR: [&str; 2] = [".stellar", "cache"];

/// chunk cache 폴더(프로젝트 root 기준 구성 요소).
pub const CACHE_DIR: [&str; 3] = [".stellar", "cache", "chunks"];

/// chunk cache 폴더의 `/` 구분 상대 경로.
pub const CACHE_RELATIVE: &str = ".stellar/cache/chunks";

/// current pointer 파일 이름(cache 폴더 안).
pub const CURRENT_FILE: &str = "current";

/// manifest 폴더 이름(cache 폴더 안).
pub const GENERATIONS_DIR: &str = "generations";

/// chunk object 폴더 이름(cache 폴더 안).
pub const OBJECTS_DIR: &str = "objects";

/// 로컬 cache 폴더의 Git 제외 규칙(`.stellar/cache/.gitignore`).
pub const CACHE_IGNORE: &[u8] = b"*\n";

/// 옛 배치의 폴더(프로젝트 root 기준).
pub const LEGACY_DIR: &str = "design";

/// 옛 배치에서 Git이 추적하던 cache 경로(`/` 구분, 프로젝트 root 기준). index에서 뺄 때 씁니다.
pub const LEGACY_PATHS: [&str; 3] = ["design/current", "design/generations", "design/objects"];

/// chunk cache 폴더 절대 경로.
pub fn cache_dir(root: &Path) -> PathBuf {
    CACHE_DIR
        .iter()
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

/// cache 폴더 아래 `area` 구성 요소까지의 root 기준 구성 요소(디렉터리 생성·검사용).
pub fn cache_components<'a>(area: &[&'a str]) -> Vec<&'a str> {
    CACHE_DIR
        .iter()
        .copied()
        .chain(area.iter().copied())
        .collect()
}

/// `/` 구분 상대 경로가 chunk cache 안 경로인지(cache 폴더 자체 포함). Git 경로 분류에 씁니다.
pub fn is_cache_path(path: &[u8]) -> bool {
    let prefix = CACHE_RELATIVE.as_bytes();
    path == prefix || (path.starts_with(prefix) && path.get(prefix.len()) == Some(&b'/'))
}

/// `/` 구분 상대 경로가 옛 배치의 cache 경로(`design/current`, `design/generations/…`, `design/objects/…`)인지.
pub fn is_legacy_cache_path(path: &[u8]) -> bool {
    path == b"design/current"
        || path.starts_with(b"design/generations/")
        || path.starts_with(b"design/objects/")
}

/// cache 안 상대 경로(`.stellar/cache/chunks/<나머지>`)의 옛 배치 상대 경로(`design/<나머지>`). 옛 commit의 Git
/// tree에서 같은 파일을 찾을 때 씁니다. cache 밖 경로면 `None`입니다.
pub fn legacy_relative_path(relative: &Path) -> Option<PathBuf> {
    let mut components = relative.components();
    for expected in CACHE_DIR {
        match components.next() {
            Some(std::path::Component::Normal(name)) if name == expected => {}
            _ => return None,
        }
    }
    let rest = components.as_path();
    (!rest.as_os_str().is_empty()).then(|| Path::new(LEGACY_DIR).join(rest))
}

/// `/` 구분 상대 경로가 로컬 cache 폴더(`.stellar/cache/…`) 안인지. 이 경로는 Git 밖입니다.
pub fn is_local_cache_path(path: &[u8]) -> bool {
    path.starts_with(b".stellar/cache/")
}

/// `.stellar/cache/.gitignore`(`*`)가 없으면 만듭니다. 이미 있으면 내용과 관계없이 그대로 둡니다(사용자 설정을
/// 덮어쓰지 않음). `.stellar`·`.stellar/cache`가 symlink면 따라가지 않고 거부합니다.
pub fn ensure_cache_ignore(root: &Path) -> io::Result<()> {
    let dirs = ensure_real_dirs(root, &LOCAL_CACHE_DIR).map_err(|(_, error)| error)?;
    let (dir, _) = dirs.last().expect("구성 요소가 있음");
    let ignore = dir.join(".gitignore");
    match std::fs::symlink_metadata(&ignore) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            write_file_atomic(&ignore, CACHE_IGNORE)
        }
        Err(error) => Err(error),
    }
}

/// [`move_legacy_layout`] 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LegacyMove {
    /// 옛 배치가 있었는지.
    pub found: bool,
    /// 새 위치로 옮긴 파일 수.
    pub moved: usize,
    /// 새 위치에 같은 bytes가 이미 있어 지운 옛 파일 수.
    pub deduplicated: usize,
    /// 새 위치에 다른 bytes가 있어 옮기지 않고 남긴 옛 파일(root 기준 `/` 경로). 내용 주소 파일이 다르면 한쪽이
    /// 손상된 것이라 어느 쪽도 지우지 않습니다.
    pub kept: Vec<String>,
    /// 옛 current pointer가 새 current pointer를 바꿨는지. 바뀐 cache는 원본과의 짝(stamp)을 알 수 없으므로
    /// 호출자가 stamp를 지웁니다.
    pub current_replaced: bool,
}

/// 옛 배치 이동 실패.
#[derive(Debug)]
pub struct LegacyMoveError {
    /// 실패한 경로.
    pub path: PathBuf,
    /// 원인.
    pub source: io::Error,
}

impl fmt::Display for LegacyMoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "옛 chunk cache 배치를 옮기지 못했습니다 ({}): {}",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for LegacyMoveError {}

fn fail(path: &Path) -> impl FnOnce(io::Error) -> LegacyMoveError + '_ {
    move |source| LegacyMoveError {
        path: path.to_path_buf(),
        source,
    }
}

/// 경로가 실제 디렉터리면 `true`, 없으면 `false`. symlink거나 디렉터리가 아니면 `None`(건드리지 않음).
fn real_dir(path: &Path) -> io::Result<Option<bool>> {
    match FileIdentity::lstat(path) {
        Ok((meta, _)) if meta.file_type().is_symlink() || !meta.is_dir() => Ok(None),
        Ok(_) => Ok(Some(true)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Some(false)),
        Err(error) => Err(error),
    }
}

/// 옛 파일 하나를 새 위치로 옮깁니다. 새 위치에 같은 bytes가 있으면 옛 파일을 지우고, 다르면 둘 다 둡니다.
fn move_file(
    from: &Path,
    to: &Path,
    relative: String,
    outcome: &mut LegacyMove,
) -> Result<(), LegacyMoveError> {
    match FileIdentity::lstat(from) {
        Ok((meta, _)) if meta.is_file() => {}
        // symlink·디렉터리 등 정규 파일이 아닌 항목은 옮기지 않습니다(저장소 파일이 아님).
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(fail(from)(error)),
    }
    match FileIdentity::lstat(to) {
        Err(error) if error.kind() == ErrorKind::NotFound => {
            match std::fs::rename(from, to) {
                Ok(()) => outcome.moved += 1,
                // 다른 writer가 먼저 옮김.
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(fail(from)(error)),
            }
        }
        Ok(_) => {
            let limit = std::fs::metadata(from).map_err(fail(from))?.len();
            let old = read_regular_file(from, limit, Link::NoFollow).map_err(fail(from))?;
            let same = read_regular_file(to, limit, Link::NoFollow)
                .map(|new| new == old)
                .unwrap_or(false);
            if same {
                match std::fs::remove_file(from) {
                    Ok(()) => outcome.deduplicated += 1,
                    Err(error) if error.kind() == ErrorKind::NotFound => {}
                    Err(error) => return Err(fail(from)(error)),
                }
            } else {
                outcome.kept.push(relative);
            }
        }
        Err(error) => return Err(fail(to)(error)),
    }
    Ok(())
}

/// 옛 폴더 하나(`design/generations` 또는 `design/objects/<shard>`)의 파일을 옮깁니다.
fn move_area(
    root: &Path,
    legacy: &Path,
    area: &[&str],
    relative: &str,
    outcome: &mut LegacyMove,
) -> Result<(), LegacyMoveError> {
    if real_dir(legacy).map_err(fail(legacy))? != Some(true) {
        return Ok(());
    }
    let dirs = ensure_real_dirs(root, &cache_components(area))
        .map_err(|(path, source)| LegacyMoveError { path, source })?;
    let (target, _) = dirs.last().expect("구성 요소가 있음").clone();
    let mut names: Vec<_> = std::fs::read_dir(legacy)
        .map_err(fail(legacy))?
        .filter_map(|entry| entry.ok().map(|entry| entry.file_name()))
        .collect();
    names.sort();
    for name in names {
        let Some(text) = name.to_str() else { continue };
        move_file(
            &legacy.join(&name),
            &target.join(&name),
            format!("{relative}/{text}"),
            outcome,
        )?;
    }
    // 비었을 때만 지웁니다(옮기지 않은 파일·다른 항목은 남김).
    let _ = std::fs::remove_dir(legacy);
    Ok(())
}

/// 옛 배치(`design/current`·`design/generations/`·`design/objects/`)를 `.stellar/cache/chunks/`로 옮깁니다.
///
/// - 내용 주소 파일(manifest·chunk)은 새 위치에 없으면 rename하고, 같은 bytes가 있으면 옛 파일을 지웁니다. 다른
///   bytes면 둘 다 남깁니다([`LegacyMove::kept`]).
/// - 옛 current pointer는 새 pointer를 바꿉니다. 옛 배치가 작업 트리에 있다는 것은 아직 옮기지 않았거나 Git이 옛
///   branch를 checkout해 다시 놓은 것이라, 작업 트리의 Git 상태가 로컬 cache보다 새 정보이기 때문입니다. 바꿨으면
///   [`LegacyMove::current_replaced`]입니다.
/// - `design`이나 옛 폴더가 symlink면 따라가지 않고 그 항목을 옮기지 않습니다.
/// - 처음 쓸 때 `.stellar/cache/.gitignore`를 만듭니다([`ensure_cache_ignore`]).
///
/// 여러 프로세스가 함께 불러도 안전합니다(rename이 먼저 끝난 쪽이 이기고 나머지는 없는 파일을 건너뜀). Git
/// index는 건드리지 않습니다.
pub fn move_legacy_layout(root: &Path) -> Result<LegacyMove, LegacyMoveError> {
    let mut outcome = LegacyMove::default();
    let design = root.join(LEGACY_DIR);
    if real_dir(&design).map_err(fail(&design))? != Some(true) {
        return Ok(outcome);
    }
    let current = design.join(CURRENT_FILE);
    let generations = design.join(GENERATIONS_DIR);
    let objects = design.join(OBJECTS_DIR);
    let exists = |path: &Path| std::fs::symlink_metadata(path).is_ok();
    if !exists(&current) && !exists(&generations) && !exists(&objects) {
        return Ok(outcome);
    }
    outcome.found = true;
    ensure_cache_ignore(root).map_err(fail(root))?;
    // 저장 순서(chunk → manifest → pointer)와 같은 순서로 옮겨 중간에 멈춰도 새 pointer가 없는 세대를 가리키지
    // 않습니다.
    if real_dir(&objects).map_err(fail(&objects))? == Some(true) {
        let mut shards: Vec<_> = std::fs::read_dir(&objects)
            .map_err(fail(&objects))?
            .filter_map(|entry| entry.ok().map(|entry| entry.file_name()))
            .collect();
        shards.sort();
        for shard in shards {
            let Some(name) = shard.to_str() else { continue };
            move_area(
                root,
                &objects.join(name),
                &[OBJECTS_DIR, name],
                &format!("design/objects/{name}"),
                &mut outcome,
            )?;
        }
        let _ = std::fs::remove_dir(&objects);
    }
    move_area(
        root,
        &generations,
        &[GENERATIONS_DIR],
        "design/generations",
        &mut outcome,
    )?;
    match FileIdentity::lstat(&current) {
        Ok((meta, _)) if meta.is_file() => {
            let dirs = ensure_real_dirs(root, &CACHE_DIR)
                .map_err(|(path, source)| LegacyMoveError { path, source })?;
            let (dir, _) = dirs.last().expect("구성 요소가 있음");
            let target = dir.join(CURRENT_FILE);
            let previous = read_regular_file(&target, 1024, Link::NoFollow).ok();
            let legacy =
                read_regular_file(&current, 1024, Link::NoFollow).map_err(fail(&current))?;
            if previous.as_deref() == Some(&legacy[..]) {
                std::fs::remove_file(&current).map_err(fail(&current))?;
                outcome.deduplicated += 1;
            } else {
                match std::fs::rename(&current, &target) {
                    Ok(()) => {
                        outcome.moved += 1;
                        outcome.current_replaced = previous.is_some();
                    }
                    Err(error) if error.kind() == ErrorKind::NotFound => {}
                    Err(error) => return Err(fail(&current)(error)),
                }
            }
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(fail(&current)(error)),
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs_guard::test_support::TempDir;

    #[test]
    fn cache_paths_live_under_the_local_cache() {
        let root = Path::new("/p");
        assert_eq!(cache_dir(root), Path::new("/p/.stellar/cache/chunks"));
        assert!(is_cache_path(b".stellar/cache/chunks"));
        assert!(is_cache_path(b".stellar/cache/chunks/current"));
        assert!(is_cache_path(
            b".stellar/cache/chunks/objects/ab/x.starchunk"
        ));
        assert!(!is_cache_path(b".stellar/cache/chunksx"));
        assert!(!is_cache_path(b"design/current"));
        assert!(is_legacy_cache_path(b"design/current"));
        assert!(is_legacy_cache_path(b"design/objects/ab/x.starchunk"));
        assert!(is_legacy_cache_path(b"design/generations/x.starmanifest"));
        assert!(!is_legacy_cache_path(b"design/document.stellar.yaml"));
        assert!(!is_legacy_cache_path(b"design/currently"));
        assert_eq!(
            legacy_relative_path(Path::new(".stellar/cache/chunks/objects/ab/x.starchunk")),
            Some(PathBuf::from("design/objects/ab/x.starchunk"))
        );
        assert_eq!(
            legacy_relative_path(Path::new(".stellar/cache/chunks/current")),
            Some(PathBuf::from("design/current"))
        );
        assert_eq!(
            legacy_relative_path(Path::new(".stellar/cache/chunks")),
            None
        );
        assert_eq!(legacy_relative_path(Path::new("design/current")), None);
        assert!(is_local_cache_path(b".stellar/cache/text-source.stamp"));
        assert!(!is_local_cache_path(b".stellar/journal/x"));
    }

    #[test]
    fn legacy_layout_moves_once_and_deduplicates() {
        let dir = TempDir::new("legacy-move");
        let root = &dir.path;
        std::fs::create_dir_all(root.join("design/objects/ab")).unwrap();
        std::fs::create_dir_all(root.join("design/generations")).unwrap();
        std::fs::write(root.join("design/objects/ab/sha256-ab.starchunk"), b"c").unwrap();
        std::fs::write(root.join("design/generations/sha256-g.starmanifest"), b"m").unwrap();
        std::fs::write(root.join("design/current"), b"p\n").unwrap();
        std::fs::write(root.join("design/document.stellar.yaml"), b"src").unwrap();
        let moved = move_legacy_layout(root).unwrap();
        assert!(moved.found);
        assert_eq!(moved.moved, 3);
        assert!(!moved.current_replaced);
        let cache = cache_dir(root);
        assert_eq!(std::fs::read(cache.join("current")).unwrap(), b"p\n");
        assert_eq!(
            std::fs::read(cache.join("objects/ab/sha256-ab.starchunk")).unwrap(),
            b"c"
        );
        assert_eq!(
            std::fs::read(cache.join("generations/sha256-g.starmanifest")).unwrap(),
            b"m"
        );
        assert!(!root.join("design/objects").exists());
        assert!(!root.join("design/generations").exists());
        assert!(!root.join("design/current").exists());
        // 텍스트 원본은 그대로입니다.
        assert!(root.join("design/document.stellar.yaml").exists());
        assert_eq!(
            std::fs::read(root.join(".stellar/cache/.gitignore")).unwrap(),
            CACHE_IGNORE
        );
        // 두 번째는 할 일이 없습니다.
        assert_eq!(move_legacy_layout(root).unwrap(), LegacyMove::default());

        // Git이 옛 branch를 checkout해 옛 파일을 다시 놓은 경우: 같은 chunk는 지우고, 다른 pointer는 새 pointer를
        // 바꿉니다.
        std::fs::create_dir_all(root.join("design/objects/ab")).unwrap();
        std::fs::write(root.join("design/objects/ab/sha256-ab.starchunk"), b"c").unwrap();
        std::fs::write(root.join("design/current"), b"q\n").unwrap();
        let again = move_legacy_layout(root).unwrap();
        assert_eq!(again.deduplicated, 1);
        assert_eq!(again.moved, 1);
        assert!(again.current_replaced);
        assert_eq!(std::fs::read(cache.join("current")).unwrap(), b"q\n");

        // 다른 bytes의 내용 주소 파일은 둘 다 남깁니다.
        std::fs::create_dir_all(root.join("design/objects/ab")).unwrap();
        std::fs::write(root.join("design/objects/ab/sha256-ab.starchunk"), b"x").unwrap();
        let kept = move_legacy_layout(root).unwrap();
        assert_eq!(kept.kept, ["design/objects/ab/sha256-ab.starchunk"]);
        assert_eq!(
            std::fs::read(cache.join("objects/ab/sha256-ab.starchunk")).unwrap(),
            b"c"
        );
    }

    #[cfg(unix)]
    #[test]
    fn legacy_symlink_directories_are_not_followed() {
        let dir = TempDir::new("legacy-move-link");
        let outside = TempDir::new("legacy-move-outside");
        std::fs::create_dir_all(outside.path.join("objects/ab")).unwrap();
        std::fs::write(outside.path.join("objects/ab/sha256-ab.starchunk"), b"c").unwrap();
        std::os::unix::fs::symlink(&outside.path, dir.path.join("design")).unwrap();
        assert_eq!(
            move_legacy_layout(&dir.path).unwrap(),
            LegacyMove::default()
        );
        assert!(outside.path.join("objects/ab/sha256-ab.starchunk").exists());
    }

    #[test]
    fn ignore_rule_is_created_once_and_never_overwritten() {
        let dir = TempDir::new("cache-ignore");
        ensure_cache_ignore(&dir.path).unwrap();
        let ignore = dir.path.join(".stellar/cache/.gitignore");
        assert_eq!(std::fs::read(&ignore).unwrap(), CACHE_IGNORE);
        std::fs::write(&ignore, b"custom\n").unwrap();
        ensure_cache_ignore(&dir.path).unwrap();
        assert_eq!(std::fs::read(&ignore).unwrap(), b"custom\n");
    }
}
