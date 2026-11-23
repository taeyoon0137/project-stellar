//! 동의에 묶인 전송 대기열 (XVI.8.4·XVI.8.5, ADR-0109 결정 6·7).
//!
//! [`TelemetryGate`]는 동의 상태와 전송 대기열을 함께 둡니다. 대기열은 [`PayloadPreview`]만 받으므로, 전송자는
//! 사용자가 preview로 본 bytes를 그대로 받습니다. 넣기와 보내기는 `Granted`에서만 동작합니다.
//!
//! 네트워크 전송자는 이 저장소에 구현하지 않습니다. [`UploadTransport`] trait만 두고, 테스트는 기록용 전송자를
//! 씁니다.

use std::collections::VecDeque;
use std::fmt;

use crate::bundle::PayloadPreview;
use crate::consent::{ConsentRecord, ConsentState};

/// 대기열 상한.
pub const MAX_QUEUED_UPLOADS: usize = 16;

/// 전송 실패. 원인 내용은 담지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportError;

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "진단 전송에 실패했습니다")
    }
}

impl std::error::Error for TransportError {}

/// payload를 보내는 쪽. 받은 bytes를 바꾸지 않고 보내야 합니다.
pub trait UploadTransport {
    /// payload 하나를 보냅니다.
    fn send(&mut self, payload: &[u8]) -> Result<(), TransportError>;
}

/// 대기열에 넣지 못한 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadBlocked {
    /// 동의하지 않았습니다(자동 전송은 `Granted`에서만).
    NoConsent(ConsentState),
    /// 대기열이 가득 찼습니다.
    QueueFull,
}

impl fmt::Display for UploadBlocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UploadBlocked::NoConsent(state) => write!(
                f,
                "텔레메트리 동의 상태가 {}라 전송할 수 없습니다",
                state.name()
            ),
            UploadBlocked::QueueFull => {
                write!(f, "전송 대기열이 가득 찼습니다({MAX_QUEUED_UPLOADS}개)")
            }
        }
    }
}

impl std::error::Error for UploadBlocked {}

/// [`TelemetryGate::flush`] 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FlushReport {
    /// 보낸 payload 수.
    pub sent: usize,
    /// 남은 payload 수.
    pub kept: usize,
    /// 동의가 없어 전송자를 부르지 않았는지.
    pub blocked: bool,
    /// 전송자가 실패를 돌려줬는지(실패한 payload부터 남깁니다).
    pub failed: bool,
}

/// 동의 상태와 전송 대기열.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TelemetryGate {
    consent: ConsentRecord,
    queue: VecDeque<PayloadPreview>,
}

impl TelemetryGate {
    /// 빈 대기열과 `consent`로 시작합니다.
    pub fn new(consent: ConsentRecord) -> TelemetryGate {
        TelemetryGate {
            consent,
            queue: VecDeque::new(),
        }
    }

    /// 동의 record.
    pub const fn consent(&self) -> ConsentRecord {
        self.consent
    }

    /// 대기 중인 payload 수.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// 대기 중인 payload(오래된 것부터).
    pub fn pending(&self) -> impl Iterator<Item = &PayloadPreview> + '_ {
        self.queue.iter()
    }

    /// preview를 자동 전송 대기열에 넣습니다. `Granted`에서만 받습니다.
    pub fn enqueue(&mut self, preview: PayloadPreview) -> Result<(), UploadBlocked> {
        if !self.consent.state.allows_automatic_upload() {
            return Err(UploadBlocked::NoConsent(self.consent.state));
        }
        if self.queue.len() >= MAX_QUEUED_UPLOADS {
            return Err(UploadBlocked::QueueFull);
        }
        self.queue.push_back(preview);
        Ok(())
    }

    /// 대기열을 오래된 것부터 보냅니다. `Granted`가 아니면 전송자를 부르지 않습니다.
    pub fn flush(&mut self, transport: &mut dyn UploadTransport) -> FlushReport {
        if !self.consent.state.allows_automatic_upload() {
            return FlushReport {
                kept: self.queue.len(),
                blocked: true,
                ..FlushReport::default()
            };
        }
        let mut report = FlushReport::default();
        while let Some(front) = self.queue.front() {
            if transport.send(front.bytes()).is_err() {
                report.failed = true;
                break;
            }
            self.queue.pop_front();
            report.sent += 1;
        }
        report.kept = self.queue.len();
        report
    }
}

/// 테스트용 기록 전송자(네트워크 없음).
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct RecordingTransport {
    /// 받은 payload(요청 순서).
    pub requests: Vec<Vec<u8>>,
    /// 이 수만큼 보낸 뒤 실패합니다.
    pub fail_after: Option<usize>,
}

#[cfg(test)]
impl UploadTransport for RecordingTransport {
    fn send(&mut self, payload: &[u8]) -> Result<(), TransportError> {
        if self.fail_after.is_some_and(|n| self.requests.len() >= n) {
            return Err(TransportError);
        }
        self.requests.push(payload.to_vec());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::{preview_payload, BundleEnvironment};
    use crate::scanner::ScanContext;
    use crate::store::{AppCrash, Arch, CrashKind, DiagnosticStore, Feature, ThreadRole, Version3};

    fn preview(code: u32) -> PayloadPreview {
        let mut store = DiagnosticStore::new(8);
        store.push_crash(AppCrash {
            at_ms: 1_763_856_000_000,
            kind: CrashKind::Signal,
            code,
            thread: ThreadRole::Render,
            uptime_ms: 10,
        });
        store.record_feature_failure(Feature::Render, code, 5);
        preview_payload(
            &BundleEnvironment {
                app_version: Version3::new(0, 1, 0),
                os_version: Version3::new(26, 1, 0),
                arch: Arch::Arm64,
            },
            &store,
            &ScanContext::new(),
        )
        .unwrap()
    }

    fn granted() -> ConsentRecord {
        ConsentRecord {
            state: ConsentState::Granted,
            changed_ms: 1,
        }
    }

    #[test]
    fn uploaded_bytes_equal_preview_bytes() {
        // 검증 기준(XVI.8.4): preview bytes와 실제 upload bytes 동일.
        let mut gate = TelemetryGate::new(granted());
        let previews: Vec<PayloadPreview> = (0..3).map(preview).collect();
        let shown: Vec<Vec<u8>> = previews.iter().map(|p| p.bytes().to_vec()).collect();
        for p in previews {
            gate.enqueue(p).unwrap();
        }
        let mut transport = RecordingTransport::default();
        let report = gate.flush(&mut transport);
        assert_eq!(
            report,
            FlushReport {
                sent: 3,
                kept: 0,
                blocked: false,
                failed: false
            }
        );
        assert_eq!(transport.requests, shown);
        assert_eq!(gate.queued(), 0);
    }

    #[test]
    fn upload_needs_granted_consent() {
        // 자동 전송은 Granted에서만(XVI.8.3).
        for state in [ConsentState::NotAsked, ConsentState::Declined] {
            let mut gate = TelemetryGate::new(ConsentRecord {
                state,
                changed_ms: 0,
            });
            assert_eq!(
                gate.enqueue(preview(1)),
                Err(UploadBlocked::NoConsent(state))
            );
            let mut transport = RecordingTransport::default();
            let report = gate.flush(&mut transport);
            assert!(report.blocked);
            assert!(transport.requests.is_empty());
        }
    }

    #[test]
    fn queue_is_bounded_and_failures_keep_order() {
        let mut gate = TelemetryGate::new(granted());
        for code in 0..MAX_QUEUED_UPLOADS as u32 {
            gate.enqueue(preview(code)).unwrap();
        }
        assert_eq!(gate.enqueue(preview(99)), Err(UploadBlocked::QueueFull));
        let mut transport = RecordingTransport {
            fail_after: Some(2),
            ..RecordingTransport::default()
        };
        let report = gate.flush(&mut transport);
        assert_eq!((report.sent, report.kept, report.failed), (2, 14, true));
        // 실패한 payload가 대기열 맨 앞에 남습니다.
        assert_eq!(gate.pending().next().unwrap().bytes(), preview(2).bytes());
    }
}
