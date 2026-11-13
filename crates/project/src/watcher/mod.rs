//! 프로젝트 파일 감시기 (XIII.2).
//!
//! 기술 아키텍처 22.5: Stellar는 프로젝트 폴더, 에셋과 허용된 symlink 대상을 파일 감시기로
//! 관찰하고, 외부 에디터·시스템 Git·스크립트의 직접 수정을 같은 경로로 처리합니다.
//!
//! - [`event`]: 플랫폼 독립 event 모델과 canonical 인코딩(XIII.2.1).
//! - [`roots`]: 감시 root(프로젝트 root·외부 symlink target allowlist)와 경로 변환(XIII.2.2).
//! - [`fsevents`]: macOS FSEvents adapter와 플랫폼 독립 normalize 규칙(XIII.2.2).

pub mod event;
pub mod fsevents;
pub mod roots;

use std::time::Duration;

pub use event::{FileWatchEvent, OverflowReason, WatchEventKind, WatchPath, WatchRootId};

/// 플랫폼 파일 감시 adapter의 공통 인터페이스.
///
/// adapter는 OS 알림을 [`FileWatchEvent`]로 normalize해서 내보냅니다. 경로는 감시 root 기준
/// 상대 경로이며, OS가 알림을 잃었거나 정확한 변경을 알 수 없으면
/// [`FileWatchEvent::Overflow`]를 내보냅니다(조용히 버리지 않음).
pub trait ProjectFileWatcher {
    /// 지금까지 쌓인 event를 순서대로 꺼냅니다(기다리지 않음).
    fn drain(&mut self) -> Vec<FileWatchEvent>;

    /// event가 하나 이상 생기거나 `timeout`이 지날 때까지 기다린 뒤 쌓인 event를 꺼냅니다.
    fn wait(&mut self, timeout: Duration) -> Vec<FileWatchEvent>;
}
