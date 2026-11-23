//! PNG 내보내기 실행과 저장 (XX.2.3, ADR-0103 결정 5).
//!
//! [`PngExport::start`]는 진행 허가([`ConfirmedExport`])를 받아 worker([`PngJob`])에서 렌더·encode를 시작하고,
//! [`PngExport::finish`]는 결과 bytes를 요청 출력 경로에 storage atomic 쓰기([`write_file_atomic_observed`]: 같은
//! 디렉터리 temp 생성 → 기록 → flush → fsync → rename → 부모 디렉터리 sync)로 저장합니다.
//!
//! - **취소**: worker 중 취소면 bytes가 없고 파일을 쓰지 않습니다. 쓰는 중 취소는 rename 직전까지 단계마다
//!   확인해 멈추며, storage가 이 호출이 만든 temp를 지웁니다. rename이 끝난 뒤의 취소는 이미 게시된 결과입니다.
//! - **실패**: 렌더·encode·쓰기 어느 단계에서 실패해도 최종 경로는 이전 상태(없음 또는 이전 파일)를 유지합니다.
//!   부분 파일이 최종 경로에 생기지 않습니다(rename이 마지막 변경 단계).
//! - 출력 디렉터리는 만들지 않습니다. 없으면 쓰기 오류입니다.
//!
//! 이 모듈은 장면(scene) snapshot을 만들지 않습니다. 저장 문서에서 scene을 만드는 일은 앱 runtime의 canvas scene
//! source가 하며, headless CLI·MCP가 그 경로에 닿으려면 scene source가 ops가 쓸 수 있는 crate로 옮겨져야 합니다
//! (ADR-0103 결정 7).

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use stellar_export::png::{GamutSummary, PngExportError, PngJob, PngOptions};
use stellar_export::preflight::ConfirmedExport;
use stellar_export::raster::{CancelToken, OffscreenTarget, OutputProfile};
use stellar_storage::atomic_file::{
    write_file_atomic_observed, NoopObserver, WriteObserver, WriteStage,
};

/// 저장한 PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngSaveOutcome {
    /// 최종 경로.
    pub path: PathBuf,
    /// 파일 byte 수.
    pub bytes: u64,
    /// 너비.
    pub width: u32,
    /// 높이.
    pub height: u32,
    /// 출력 profile.
    pub profile: OutputProfile,
    /// gamut 요약.
    pub gamut: GamutSummary,
}

/// PNG 내보내기·저장 실패.
#[derive(Debug)]
pub enum PngSaveError {
    /// 렌더·encode 실패(취소 제외).
    Export(PngExportError),
    /// 취소되었습니다. 최종 경로는 바뀌지 않았습니다.
    Cancelled,
    /// 쓰기 실패. 최종 경로는 이전 상태입니다.
    Write {
        /// 대상 경로.
        path: PathBuf,
        /// 원인.
        error: std::io::Error,
    },
}

impl fmt::Display for PngSaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PngSaveError::Export(error) => write!(f, "{error}"),
            PngSaveError::Cancelled => f.write_str("PNG 내보내기가 취소되었습니다"),
            PngSaveError::Write { path, error } => {
                write!(f, "PNG를 {}에 저장하지 못했습니다: {error}", path.display())
            }
        }
    }
}

impl std::error::Error for PngSaveError {}

impl From<PngExportError> for PngSaveError {
    fn from(error: PngExportError) -> PngSaveError {
        match error {
            PngExportError::Cancelled => PngSaveError::Cancelled,
            other => PngSaveError::Export(other),
        }
    }
}

/// 쓰기 단계마다 취소를 확인하고, 주입된 관찰자(fault injection)에 위임합니다.
struct CancelAware<'a> {
    cancel: &'a CancelToken,
    inner: &'a mut dyn WriteObserver,
    cancelled: bool,
}

impl WriteObserver for CancelAware<'_> {
    fn before(&mut self, stage: WriteStage) -> std::io::Result<()> {
        // rename 뒤(부모 디렉터리 sync)에는 결과가 이미 게시되어 취소하지 않습니다.
        if stage != WriteStage::SyncParentDir && self.cancel.is_cancelled() {
            self.cancelled = true;
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "취소"));
        }
        self.inner.before(stage)
    }
}

/// 진행 중인 PNG 내보내기.
#[derive(Debug)]
pub struct PngExport {
    job: PngJob,
    path: PathBuf,
}

impl PngExport {
    /// worker에서 렌더·encode를 시작합니다. 출력 경로는 요청의 경로입니다.
    pub fn start(
        export: ConfirmedExport,
        target: Arc<dyn OffscreenTarget + Send + Sync>,
        options: PngOptions,
    ) -> Result<PngExport, PngSaveError> {
        let path = export
            .capture()
            .request()
            .output_path()
            .as_path()
            .to_path_buf();
        let job = PngJob::spawn(export, target, options)?;
        Ok(PngExport { job, path })
    }

    /// 취소 신호(진행 정도 포함).
    pub fn cancel_token(&self) -> CancelToken {
        self.job.cancel_token()
    }

    /// 취소를 요청합니다.
    pub fn cancel(&self) {
        self.job.cancel();
    }

    /// 출력 경로.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// 결과를 기다려 저장합니다(모듈 문서).
    pub fn finish(self) -> Result<PngSaveOutcome, PngSaveError> {
        self.finish_observed(&mut NoopObserver)
    }

    /// [`PngExport::finish`]에 쓰기 단계 관찰자를 주입한 버전(fault injection 테스트용).
    pub fn finish_observed(
        self,
        observer: &mut dyn WriteObserver,
    ) -> Result<PngSaveOutcome, PngSaveError> {
        let cancel = self.job.cancel_token();
        let file = self.job.wait()?;
        let mut guard = CancelAware {
            cancel: &cancel,
            inner: observer,
            cancelled: false,
        };
        match write_file_atomic_observed(&self.path, file.bytes(), &mut guard) {
            Ok(()) => Ok(PngSaveOutcome {
                path: self.path,
                bytes: file.bytes().len() as u64,
                width: file.width(),
                height: file.height(),
                profile: file.profile(),
                gamut: file.gamut().clone(),
            }),
            Err(_) if guard.cancelled => Err(PngSaveError::Cancelled),
            Err(error) => Err(PngSaveError::Write {
                path: self.path,
                error,
            }),
        }
    }
}

#[cfg(test)]
mod tests;
