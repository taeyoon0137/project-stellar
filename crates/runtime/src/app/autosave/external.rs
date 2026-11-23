//! 외부 변경 감시 → 창 반영 연결 (UX.4.1 runtime 연결, XIII.4.4, ADR-0083 결정 3).
//!
//! 감시 thread(`stellar-external-watch`) 하나가 열린 프로젝트마다 파일 감시기(macOS FSEvents)와 coalescer
//! (XIII.2.5)를 두고, current pointer가 바뀐 조용한 구간마다 그 프로젝트 창들의 반영 job을 document thread
//! command lane에 보냅니다. 반영 판단(자기 저장 hash 무시, 제스처 뒤 반영, dirty면 stale, 깨진 generation 거부)은
//! [`ProjectWindowSession::sync_external`]이 합니다. 그래서 감시는 "무언가 바뀌었다"만 알리면 되고, coalescer의
//! 자기 저장 기록은 비워 둡니다(자기 저장도 job 하나를 만들지만 hash 비교에서 `Unchanged`).
//!
//! 신뢰된 프로젝트는 외부 symlink target 감시 root를 [`ExternalTargetSync::sync`]로 맞춥니다(XIII.4.4). 처음 열 때
//! 한 번, 그 뒤에는 넘침·링크 변화 event에서 다시 맞춥니다. 파일 감시기를 만들 수 없는 플랫폼·상황에서는 같은
//! 주기로 current pointer bytes만 비교합니다.

use super::*;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::Weak;

use stellar_project::watcher::coalesce::{ChangeCoalescer, ExternalChange};
use stellar_project::watcher::own_save::OwnSaveLedger;
use stellar_project::watcher::targets::ExternalTargetSync;
use stellar_storage::generation::current::read_current_pointer;
use stellar_storage::hash::ContentHash;

/// 감시 thread가 event를 모으는 간격.
const POLL: Duration = Duration::from_millis(50);

/// coalescer 조용한 구간(저장 한 번의 여러 파일 event를 하나로 묶음).
const QUIET: Duration = Duration::from_millis(30);

/// 감시 thread 이름.
const THREAD_NAME: &str = "stellar-external-watch";

/// 감시 thread 소유자([`AutosaveClock`] 안).
#[derive(Debug)]
pub(crate) struct ExternalWatch {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ExternalWatch {
    /// 감시 thread를 시작합니다. 만들 수 없으면 `None`(외부 변경은 [`AppRuntime::sync_external_changes`]로만 반영).
    pub(super) fn start(sender: DocumentSender, shared: Arc<AppShared>) -> Option<ExternalWatch> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let weak = Arc::downgrade(&shared);
        drop(shared);
        match std::thread::Builder::new()
            .name(THREAD_NAME.into())
            .spawn(move || supervise(&flag, &sender, &weak))
        {
            Ok(thread) => Some(ExternalWatch {
                stop,
                thread: Some(thread),
            }),
            Err(error) => {
                eprintln!("Stellar external watch: {error}");
                None
            }
        }
    }

    /// 감시를 멈추고 thread가 끝날 때까지 기다립니다(최대 한 poll 간격).
    pub(super) fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// 프로젝트 하나의 감시 상태.
struct ProjectWatch {
    root: std::path::PathBuf,
    #[cfg(target_os = "macos")]
    watcher: Option<stellar_project::watcher::fsevents::FsEventsWatcher>,
    #[cfg(target_os = "macos")]
    resolver: Option<stellar_project::path_resolver::PathResolver>,
    coalescer: ChangeCoalescer,
    ledger: OwnSaveLedger,
    targets: ExternalTargetSync,
    /// 감시기가 없을 때 비교하는 마지막 current pointer.
    last_pointer: Option<ContentHash>,
    /// 감시를 시작하기 전(창을 연 뒤 감시가 붙기까지)의 변화를 놓치지 않도록 첫 poll에서 한 번 반영합니다.
    initial: bool,
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

impl ProjectWatch {
    fn start(summary: &ProjectSummary) -> ProjectWatch {
        let root = summary.canonical_root.clone();
        let pointer = read_current_pointer(&root).ok();
        #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
        let mut watch = ProjectWatch {
            #[cfg(target_os = "macos")]
            watcher: stellar_project::watcher::fsevents::FsEventsWatcher::start(&root, &[]).ok(),
            #[cfg(target_os = "macos")]
            resolver: stellar_project::identity::ProjectIdentity::new(summary.project_id, &root)
                .ok()
                .map(|identity| {
                    stellar_project::path_resolver::PathResolver::for_project(
                        &identity,
                        summary.trust,
                    )
                }),
            coalescer: ChangeCoalescer::new(&root, summary.project_id, pointer, nanos(QUIET)),
            ledger: OwnSaveLedger::new(),
            targets: ExternalTargetSync::new(),
            last_pointer: pointer,
            initial: true,
            root,
        };
        #[cfg(target_os = "macos")]
        watch.resync_targets();
        watch
    }

    /// 외부 symlink target 감시 root를 맞춥니다(XIII.4.4). 비신뢰 프로젝트는 비어 있습니다.
    #[cfg(target_os = "macos")]
    fn resync_targets(&mut self) {
        if let (Some(watcher), Some(resolver)) = (self.watcher.as_mut(), self.resolver.as_ref()) {
            if let Err(error) = self.targets.sync(watcher, resolver) {
                eprintln!("Stellar external watch targets: {error}");
            }
        }
    }

    /// 쌓인 event를 처리하고, 창에 반영할 변화가 있으면 `true`입니다.
    fn poll(&mut self, now: Duration) -> bool {
        let initial = std::mem::take(&mut self.initial);
        #[cfg(target_os = "macos")]
        if let Some(watcher) = self.watcher.as_mut() {
            use stellar_project::watcher::ProjectFileWatcher;
            let events = watcher.drain();
            if !events.is_empty() {
                let resync = self.targets.needs_resync(&events, watcher.roots());
                self.coalescer.push(&events, nanos(now));
                if resync {
                    self.resync_targets();
                }
            }
            let changed = self
                .coalescer
                .poll(nanos(now), &self.ledger)
                .iter()
                .any(|change| !matches!(change, ExternalChange::Files { .. }));
            return changed || initial;
        }
        let pointer = read_current_pointer(&self.root).ok();
        if pointer != self.last_pointer {
            self.last_pointer = pointer;
            return true;
        }
        initial
    }
}

fn supervise(stop: &AtomicBool, sender: &DocumentSender, shared: &Weak<AppShared>) {
    let origin = Instant::now();
    let mut watches: BTreeMap<ProjectSessionId, ProjectWatch> = BTreeMap::new();
    while !stop.load(Ordering::SeqCst) {
        let Some(app) = shared.upgrade() else {
            return;
        };
        let state = app.current_state();
        watches.retain(|project, _| state.project(*project).is_some());
        for summary in &state.projects {
            watches
                .entry(summary.project)
                .or_insert_with(|| ProjectWatch::start(summary));
        }
        let now = origin.elapsed();
        let due: Vec<ProjectSessionId> = watches
            .iter_mut()
            .filter_map(|(project, watch)| watch.poll(now).then_some(*project))
            .collect();
        for project in due {
            let job_shared = Arc::clone(&app);
            // 반영은 편집 command와 같은 lane에서 순서대로 합니다. 대기열이 차면 다음 변화가 다시 알립니다.
            let _ = sender.try_submit(
                SubmitOptions::command(),
                move |ctx: &mut DocumentContext| sync_project(ctx, &job_shared, project),
            );
        }
        drop(app);
        std::thread::sleep(POLL);
    }
}
