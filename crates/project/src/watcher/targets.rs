//! 외부 symlink target 감시 동적 등록·해제 (XIII.4.4).
//!
//! 기술 아키텍처 19.3 "외부 링크의 변경도 파일 감시": 프로젝트 안 symlink가 가리키는 **프로젝트 밖**
//! target을 감시 root로 등록하고, 링크가 생기거나 다른 target으로 바뀌거나 사라지면 등록을 다시
//! 맞춥니다.
//!
//! - 감시할 target([`desired_external_targets`]): resolver(XIII.4) 트리 순회에서 프로젝트 안에서
//!   밖으로 처음 넘어가는 항목의 실제 target. 외부 디렉터리 안의 하위 항목은 그 target root가 이미
//!   포함하므로 따로 등록하지 않으며, 순회도 외부 디렉터리 안으로 내려가지 않습니다
//!   ([`ExternalDescent::Stop`] — `~` 같은 큰 외부 target의 내용 전체를 훑지 않음).
//! - **비신뢰 프로젝트**는 외부 target을 감시하지 않습니다. 외부 target의 하위 이름·변화는 외부 내용이며
//!   capability matrix(XVI.7.1)의 외부 symlink 읽기가 금지이기 때문입니다.
//! - 다시 맞추는 시점([`ExternalTargetSync::needs_resync`]): 넘침, 알려진 링크(또는 그 상위 경로)의
//!   변화, 새로 생기거나 바뀐 경로가 symlink일 때.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::event::{FileWatchEvent, WatchPath, WatchRootId};
use super::roots::WatchRoots;
use crate::path_resolver::{
    ExternalAccess, ExternalDescent, PathResolver, ResolveError, TargetLocation,
};

/// `.git`은 순회하지 않습니다.
const GIT_DIR: &str = ".git";

/// 감시해야 할 외부 target(실제 경로 → 그 target으로 이어지는 프로젝트 상대 링크 경로들).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalTargets {
    /// target → 링크 경로(정렬).
    pub targets: BTreeMap<PathBuf, BTreeSet<String>>,
}

/// 프로젝트에서 감시할 외부 symlink target을 구합니다. 비신뢰 프로젝트는 비어 있습니다.
pub fn desired_external_targets(resolver: &PathResolver) -> Result<ExternalTargets, ResolveError> {
    if resolver.external_access() == ExternalAccess::MetadataOnly {
        return Ok(ExternalTargets::default());
    }
    // 외부 디렉터리 안으로 내려가지 않으므로 외부 항목의 부모는 항상 프로젝트 안입니다.
    let walk = resolver.walk_tree_with(ExternalDescent::Stop, |path| {
        path.split('/').next() != Some(GIT_DIR)
    })?;
    let mut targets = ExternalTargets::default();
    for entry in &walk.entries {
        if entry.location != TargetLocation::External {
            continue;
        }
        let resolved = resolver.resolve(&entry.path)?;
        targets
            .targets
            .entry(resolved.target)
            .or_default()
            .insert(entry.path.clone());
    }
    Ok(targets)
}

/// 외부 target root를 동적으로 추가·해제할 수 있는 감시 adapter.
pub trait ExternalTargetWatch {
    /// adapter 오류.
    type Error: std::fmt::Display;

    /// 현재 감시 root.
    fn watch_roots(&self) -> &WatchRoots;

    /// 외부 target을 감시 root로 추가합니다(이미 있으면 기존 id).
    fn add_external_target(&mut self, target: &Path) -> Result<WatchRootId, Self::Error>;

    /// 외부 감시 root를 해제합니다.
    fn remove_external_target(&mut self, id: WatchRootId) -> Result<bool, Self::Error>;
}

/// 동기화 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// 새로 등록한 root(id, 실제 경로).
    pub added: Vec<(WatchRootId, PathBuf)>,
    /// 해제한 root.
    pub removed: Vec<WatchRootId>,
}

impl SyncReport {
    /// 바뀐 것이 없는지.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// 동기화 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError<E> {
    /// 프로젝트 트리 해석 실패.
    Resolve(ResolveError),
    /// adapter 등록·해제 실패.
    Watch(E),
}

impl<E: std::fmt::Display> std::fmt::Display for SyncError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::Resolve(e) => write!(f, "외부 target 해석 실패: {e}"),
            SyncError::Watch(e) => write!(f, "외부 target 감시 등록 실패: {e}"),
        }
    }
}

impl<E: std::fmt::Debug + std::fmt::Display> std::error::Error for SyncError<E> {}

/// 외부 target 등록 상태(알려진 링크 경로).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalTargetSync {
    links: BTreeSet<String>,
}

impl ExternalTargetSync {
    /// 빈 상태.
    pub fn new() -> ExternalTargetSync {
        ExternalTargetSync::default()
    }

    /// 외부 target으로 이어지는 알려진 링크 경로.
    pub fn links(&self) -> &BTreeSet<String> {
        &self.links
    }

    /// 프로젝트를 다시 훑어 감시 root를 맞춥니다: 새 target 등록, 더 이상 가리키지 않는 target 해제.
    pub fn sync<W: ExternalTargetWatch>(
        &mut self,
        watcher: &mut W,
        resolver: &PathResolver,
    ) -> Result<SyncReport, SyncError<W::Error>> {
        let desired = desired_external_targets(resolver).map_err(SyncError::Resolve)?;
        let current: Vec<(WatchRootId, PathBuf)> = watcher
            .watch_roots()
            .iter()
            .filter(|root| !root.id.is_project())
            .map(|root| (root.id, root.path.clone()))
            .collect();
        let mut report = SyncReport::default();
        for (id, path) in &current {
            if !desired.targets.contains_key(path) {
                watcher
                    .remove_external_target(*id)
                    .map_err(SyncError::Watch)?;
                report.removed.push(*id);
            }
        }
        for target in desired.targets.keys() {
            if current.iter().any(|(_, path)| path == target) {
                continue;
            }
            let id = watcher
                .add_external_target(target)
                .map_err(SyncError::Watch)?;
            report.added.push((id, target.clone()));
        }
        self.links = desired.targets.into_values().flatten().collect();
        Ok(report)
    }

    /// event가 외부 target 등록을 다시 맞춰야 하는지 판정합니다.
    pub fn needs_resync(&self, events: &[FileWatchEvent], roots: &WatchRoots) -> bool {
        events.iter().any(|event| match event {
            FileWatchEvent::Overflow(_) => true,
            other => other
                .paths()
                .iter()
                .any(|path| self.touches_link(path, roots)),
        })
    }

    fn touches_link(&self, path: &WatchPath, roots: &WatchRoots) -> bool {
        if !path.root().is_project() {
            return false;
        }
        let known = self
            .links
            .iter()
            .any(|link| WatchPath::project(link.as_str()).is_ok_and(|link| link.starts_with(path)));
        known
            || roots
                .absolute(path)
                .and_then(|absolute| std::fs::symlink_metadata(absolute).ok())
                .is_some_and(|meta| meta.file_type().is_symlink())
    }
}

#[cfg(target_os = "macos")]
impl ExternalTargetWatch for super::fsevents::FsEventsWatcher {
    type Error = super::fsevents::WatcherError;

    fn watch_roots(&self) -> &WatchRoots {
        self.roots()
    }

    fn add_external_target(&mut self, target: &Path) -> Result<WatchRootId, Self::Error> {
        super::fsevents::FsEventsWatcher::add_external_target(self, target)
    }

    fn remove_external_target(&mut self, id: WatchRootId) -> Result<bool, Self::Error> {
        super::fsevents::FsEventsWatcher::remove_external_target(self, id)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::trust::TrustStatus;
    use crate::watcher::roots::WatchRootError;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-watch-targets-test-{}-{n}",
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

    /// 등록·해제만 기록하는 adapter.
    struct FakeWatch {
        roots: WatchRoots,
    }

    impl ExternalTargetWatch for FakeWatch {
        type Error = WatchRootError;
        fn watch_roots(&self) -> &WatchRoots {
            &self.roots
        }
        fn add_external_target(&mut self, target: &Path) -> Result<WatchRootId, WatchRootError> {
            self.roots.add_external(target)
        }
        fn remove_external_target(&mut self, id: WatchRootId) -> Result<bool, WatchRootError> {
            Ok(self.roots.remove(id)?.is_some())
        }
    }

    struct Fixture {
        dir: TempDir,
        project: PathBuf,
        a: PathBuf,
        b: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = TempDir::new();
        let project = dir.path.join("project");
        let a = dir.path.join("outside-a");
        let b = dir.path.join("outside-b");
        for d in [&project, &a, &b] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::create_dir_all(project.join(".git")).unwrap();
        std::fs::create_dir_all(a.join("sub")).unwrap();
        std::fs::write(a.join("sub/x.png"), b"x").unwrap();
        std::fs::write(b.join("font.otf"), b"f").unwrap();
        let a = std::fs::canonicalize(a).unwrap();
        let b = std::fs::canonicalize(b).unwrap();
        Fixture { dir, project, a, b }
    }

    #[test]
    fn desired_targets_are_top_level_external_targets() {
        let fx = fixture();
        symlink(&fx.a, fx.project.join("assets")).unwrap();
        symlink(fx.b.join("font.otf"), fx.project.join("font.otf")).unwrap();
        // 내부 링크를 거쳐 외부로 가는 경로도 같은 target.
        symlink("assets", fx.project.join("alias")).unwrap();
        // .git 안의 링크는 무시.
        symlink(&fx.b, fx.project.join(".git/ignored")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();
        let targets = desired_external_targets(&resolver).unwrap();
        let expected: BTreeMap<PathBuf, BTreeSet<String>> = [
            (
                fx.a.clone(),
                ["alias", "assets"].map(String::from).into_iter().collect(),
            ),
            (
                fx.b.join("font.otf"),
                ["font.otf"].map(String::from).into_iter().collect(),
            ),
        ]
        .into_iter()
        .collect();
        assert_eq!(targets.targets, expected);
        // 비신뢰 프로젝트는 외부 target을 감시하지 않음.
        let untrusted = resolver.with_trust(TrustStatus::NotTrusted);
        assert!(desired_external_targets(&untrusted)
            .unwrap()
            .targets
            .is_empty());
    }

    #[test]
    fn sync_registers_and_unregisters_as_links_change() {
        let fx = fixture();
        let link = fx.project.join("assets");
        symlink(&fx.a, &link).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();
        let mut watch = FakeWatch {
            roots: WatchRoots::new(&fx.project).unwrap(),
        };
        let mut sync = ExternalTargetSync::new();

        let first = sync.sync(&mut watch, &resolver).unwrap();
        assert_eq!(first.added, vec![(WatchRootId(1), fx.a.clone())]);
        assert!(sync.sync(&mut watch, &resolver).unwrap().is_empty());

        // target 교체: 링크가 outside-b를 가리킴 → b 등록, a 해제.
        std::fs::remove_file(&link).unwrap();
        symlink(&fx.b, &link).unwrap();
        let replaced = sync.sync(&mut watch, &resolver).unwrap();
        assert_eq!(replaced.removed, vec![WatchRootId(1)]);
        assert_eq!(replaced.added, vec![(WatchRootId(2), fx.b.clone())]);

        // 링크 삭제 → 해제, 비신뢰 전환 → 외부 target 전부 해제.
        std::fs::remove_file(&link).unwrap();
        assert_eq!(
            sync.sync(&mut watch, &resolver).unwrap().removed,
            vec![WatchRootId(2)]
        );
        symlink(&fx.a, &link).unwrap();
        sync.sync(&mut watch, &resolver).unwrap();
        let untrusted = resolver.clone().with_trust(TrustStatus::NotTrusted);
        assert_eq!(sync.sync(&mut watch, &untrusted).unwrap().removed.len(), 1);
        assert_eq!(watch.roots.iter().count(), 1);
    }

    #[test]
    fn resync_is_needed_only_for_link_changes_or_overflow() {
        use crate::watcher::event::OverflowReason;
        let fx = fixture();
        std::fs::create_dir_all(fx.project.join("media")).unwrap();
        symlink(&fx.a, fx.project.join("media/assets")).unwrap();
        std::fs::write(fx.project.join("stellar.yaml"), b"x").unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();
        let mut watch = FakeWatch {
            roots: WatchRoots::new(&fx.project).unwrap(),
        };
        let mut sync = ExternalTargetSync::new();
        sync.sync(&mut watch, &resolver).unwrap();
        let roots = &watch.roots;
        let p = |s: &str| WatchPath::project(s).unwrap();
        // 일반 파일·외부 root 안의 변화는 다시 맞출 필요 없음.
        assert!(!sync.needs_resync(&[FileWatchEvent::Modified(p("stellar.yaml"))], roots));
        assert!(!sync.needs_resync(
            &[FileWatchEvent::Created(
                WatchPath::new(WatchRootId(1), "sub/y.png").unwrap()
            )],
            roots
        ));
        // 알려진 링크·그 상위 디렉터리, 새 symlink, 넘침.
        assert!(sync.needs_resync(&[FileWatchEvent::Removed(p("media/assets"))], roots));
        assert!(sync.needs_resync(&[FileWatchEvent::Removed(p("media"))], roots));
        symlink(&fx.b, fx.project.join("new-link")).unwrap();
        assert!(sync.needs_resync(&[FileWatchEvent::Created(p("new-link"))], roots));
        assert!(sync.needs_resync(
            &[FileWatchEvent::Overflow(OverflowReason::EventsDropped)],
            roots
        ));
        let _ = &fx.dir;
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn replaced_target_is_watched_after_dynamic_registration() {
        use crate::watcher::fsevents::FsEventsWatcher;
        use crate::watcher::ProjectFileWatcher;
        use std::time::{Duration, Instant};

        let fx = fixture();
        let link = fx.project.join("assets");
        symlink(&fx.a, &link).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();
        let mut watcher = FsEventsWatcher::start(&fx.project, &[]).unwrap();
        let mut sync = ExternalTargetSync::new();
        let first = sync.sync(&mut watcher, &resolver).unwrap();
        let a_root = first.added[0].0;
        assert_eq!(watcher.watched_roots(), vec![WatchRootId::PROJECT, a_root]);

        /// `pred`를 만족하는 event가 올 때까지 모읍니다. 링크 변화가 보이면 등록을 다시 맞춥니다.
        fn pump(
            watcher: &mut FsEventsWatcher,
            sync: &mut ExternalTargetSync,
            resolver: &PathResolver,
            seen: &mut Vec<FileWatchEvent>,
            pred: impl Fn(&FsEventsWatcher, &[FileWatchEvent]) -> bool,
        ) -> bool {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                watcher.flush();
                let events = watcher.wait(Duration::from_millis(100));
                if sync.needs_resync(&events, watcher.roots()) {
                    sync.sync(watcher, resolver).unwrap();
                }
                seen.extend(events);
                if pred(watcher, seen) {
                    return true;
                }
            }
            false
        }

        // 처음 target 안의 변화.
        let mut seen = Vec::new();
        std::fs::write(fx.a.join("sub/new.png"), b"n").unwrap();
        let in_a = FileWatchEvent::Created(WatchPath::new(a_root, "sub/new.png").unwrap());
        assert!(
            pump(&mut watcher, &mut sync, &resolver, &mut seen, |_, e| e
                .contains(&in_a)),
            "a: {seen:?}"
        );

        // target replace: 링크를 outside-b로 바꿈 → 링크 event로 동적 재등록.
        std::fs::remove_file(&link).unwrap();
        symlink(&fx.b, &link).unwrap();
        let mut seen = Vec::new();
        let b_path = fx.b.clone();
        assert!(
            pump(&mut watcher, &mut sync, &resolver, &mut seen, |w, _| w
                .roots()
                .iter()
                .any(|r| r.path == b_path)),
            "링크 교체 event로 재등록되지 않음: {seen:?}"
        );
        let b_root = watcher
            .roots()
            .iter()
            .find(|r| r.path == fx.b)
            .map(|r| r.id)
            .expect("새 target 등록");
        assert!(watcher.roots().get(a_root).is_none(), "이전 target 해제");
        assert_eq!(watcher.watched_roots(), vec![WatchRootId::PROJECT, b_root]);

        // 새 target의 변화 수신, 이전 target 변화는 수신하지 않음.
        let mut seen = Vec::new();
        std::fs::write(fx.a.join("sub/late.png"), b"l").unwrap();
        std::fs::write(fx.b.join("added.otf"), b"o").unwrap();
        let in_b = FileWatchEvent::Created(WatchPath::new(b_root, "added.otf").unwrap());
        assert!(
            pump(&mut watcher, &mut sync, &resolver, &mut seen, |_, e| e
                .contains(&in_b)),
            "b: {seen:?}"
        );
        assert!(seen
            .iter()
            .flat_map(|e| e.paths())
            .all(|p| p.root() != a_root && !p.relative().contains("late")));
    }
}
