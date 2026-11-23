//! 동의에 묶인 전송 대기열 (XVI.8.4·XVI.8.5, ADR-0109 결정 6·7).
//!
//! [`TelemetryGate`]는 동의 상태와 전송 대기열을 함께 둡니다. 대기열은 [`PayloadPreview`]만 받으므로, 전송자는
//! 사용자가 preview로 본 bytes를 그대로 받습니다. 넣기와 보내기는 `Granted`에서만 동작합니다.
//!
//! 네트워크 전송자는 이 저장소에 구현하지 않습니다. [`UploadTransport`] trait만 두고, 테스트는 기록용 전송자를
//! 씁니다.
//!
//! # 철회 (XVI.8.5)
//!
//! `Granted`를 떠나는 전이([`TelemetryGate::apply`]의 철회·거절)는 대기열을 모두 버립니다. 이후 넣기·보내기는 다시
//! `Grant`할 때까지 막힙니다. 로컬 store는 건드리지 않고 보존 정책([`crate::store::RetentionPolicy`])을 따릅니다.
//!
//! 영속 대기열은 진단 디렉터리의 [`OUTBOX_FILE`]입니다. [`TelemetryGate::save`]는 동의 파일을 먼저 쓰고 대기열
//! 파일을 쓰거나 지웁니다. 그 사이에 끊겨 대기열 파일이 남아도, [`TelemetryGate::open`]은 동의가 `Granted`가
//! 아니면 대기열 파일을 지우고 버립니다. `Granted`여도 저장된 payload는 scanner로 다시 검사하고, 통과하지 못한
//! 것은 버립니다.
//!
//! ```text
//! stellar.telemetry-outbox v1
//! payload bytes=<n>
//! <번들 n bytes>
//! ```

use std::collections::VecDeque;
use std::fmt;
use std::io;
use std::path::Path;

use crate::bundle::PayloadPreview;
use crate::consent::{
    read_consent, write_consent, ConsentEvent, ConsentLoad, ConsentRecord, ConsentState,
    Transition, TransitionError, CONSENT_FILE,
};
use crate::store::{read_limited, write_private_file, StoreError, MAX_DIAGNOSTIC_BYTES};

/// 대기열 상한.
pub const MAX_QUEUED_UPLOADS: usize = 16;

/// 대기열 파일 첫 줄.
pub const OUTBOX_HEADER: &str = "stellar.telemetry-outbox v1";

/// 대기열 파일 이름(진단 디렉터리 아래).
pub const OUTBOX_FILE: &str = "outbox";

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
            UploadBlocked::QueueFull => write!(
                f,
                "전송 대기열이 가득 찼습니다({MAX_QUEUED_UPLOADS}개 또는 {MAX_DIAGNOSTIC_BYTES} bytes)"
            ),
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

/// [`TelemetryGate::apply`] 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsentChange {
    /// 적용한 전이.
    pub transition: Transition,
    /// 버린 대기 payload 수(`Granted`를 떠날 때만 0보다 큼).
    pub discarded: usize,
}

/// [`TelemetryGate::open`] 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedGate {
    /// 동의 상태와 대기열.
    pub gate: TelemetryGate,
    /// 동의 파일을 읽은 결과.
    pub consent_load: ConsentLoad,
    /// 동의가 `Granted`가 아니어서 버린 대기열 파일의 payload 수.
    pub discarded_without_consent: usize,
    /// 다시 검사를 통과하지 못했거나 형식이 깨져 버린 payload 수(형식이 깨진 파일은 1).
    pub discarded_invalid: usize,
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
        let queued_bytes: usize = self.queue.iter().map(PayloadPreview::len).sum();
        if self.queue.len() >= MAX_QUEUED_UPLOADS
            || queued_bytes + preview.len() > MAX_DIAGNOSTIC_BYTES - OUTBOX_OVERHEAD
        {
            return Err(UploadBlocked::QueueFull);
        }
        self.queue.push_back(preview);
        Ok(())
    }

    /// 동의 사건을 적용합니다. `Granted`를 떠나면 대기열을 모두 버립니다(XVI.8.5).
    pub fn apply(
        &mut self,
        event: ConsentEvent,
        at_ms: u64,
    ) -> Result<ConsentChange, TransitionError> {
        let transition = self.consent.apply(event, at_ms)?;
        let discarded = if transition.leaves_granted() {
            let n = self.queue.len();
            self.queue.clear();
            n
        } else {
            0
        };
        Ok(ConsentChange {
            transition,
            discarded,
        })
    }

    /// 진단 디렉터리에서 동의 상태와 대기열을 엽니다. 동의가 `Granted`가 아니면 남은 대기열 파일을 지웁니다.
    pub fn open(dir: &Path) -> io::Result<OpenedGate> {
        let consent_load = read_consent(&dir.join(CONSENT_FILE));
        let mut opened = OpenedGate {
            gate: TelemetryGate::new(consent_load.record()),
            consent_load,
            discarded_without_consent: 0,
            discarded_invalid: 0,
        };
        let outbox = dir.join(OUTBOX_FILE);
        let bytes = match read_limited(&outbox) {
            Ok(bytes) => Some(bytes),
            Err(StoreError::Io(io::ErrorKind::NotFound)) => None,
            Err(StoreError::Io(kind)) => return Err(io::Error::from(kind)),
            // 상한을 넘는 파일은 형식이 깨진 것으로 봅니다.
            Err(_) => Some(Vec::new()),
        };
        let Some(bytes) = bytes else {
            return Ok(opened);
        };
        let entries = parse_outbox(&bytes);
        if !opened.gate.consent.state.allows_automatic_upload() {
            opened.discarded_without_consent = entries.as_ref().map_or(1, Vec::len);
            remove_if_exists(&outbox)?;
            return Ok(opened);
        }
        match entries {
            Some(entries) => {
                for entry in entries {
                    match PayloadPreview::revalidate(entry) {
                        Ok(preview) if opened.gate.queue.len() < MAX_QUEUED_UPLOADS => {
                            opened.gate.queue.push_back(preview);
                        }
                        _ => opened.discarded_invalid += 1,
                    }
                }
            }
            None => opened.discarded_invalid = 1,
        }
        Ok(opened)
    }

    /// 동의 파일을 쓴 뒤 대기열 파일을 쓰거나(비어 있으면) 지웁니다.
    pub fn save(&self, dir: &Path) -> io::Result<()> {
        write_consent(&dir.join(CONSENT_FILE), &self.consent)?;
        let outbox = dir.join(OUTBOX_FILE);
        if self.queue.is_empty() || !self.consent.state.allows_automatic_upload() {
            return remove_if_exists(&outbox);
        }
        let mut bytes = format!("{OUTBOX_HEADER}\n").into_bytes();
        for preview in &self.queue {
            bytes.extend_from_slice(format!("payload bytes={}\n", preview.len()).as_bytes());
            bytes.extend_from_slice(preview.bytes());
        }
        write_private_file(&outbox, &bytes)
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

/// 대기열 파일 머리 줄과 항목 머리 줄 여유(bytes).
const OUTBOX_OVERHEAD: usize = 64 + MAX_QUEUED_UPLOADS * 32;

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// 대기열 파일을 항목 bytes 목록으로 나눕니다. 형식이 깨졌으면 `None`.
fn parse_outbox(bytes: &[u8]) -> Option<Vec<Vec<u8>>> {
    let header = format!("{OUTBOX_HEADER}\n");
    let mut rest = bytes.strip_prefix(header.as_bytes())?;
    let mut entries = Vec::new();
    while !rest.is_empty() {
        let newline = rest.iter().position(|b| *b == b'\n')?;
        let line = std::str::from_utf8(&rest[..newline]).ok()?;
        let len = line.strip_prefix("payload bytes=")?;
        if len.is_empty() || (len.len() > 1 && len.starts_with('0')) {
            return None;
        }
        let len: usize = len.parse().ok()?;
        let body = rest.get(newline + 1..newline + 1 + len)?;
        entries.push(body.to_vec());
        rest = &rest[newline + 1 + len..];
        if entries.len() > MAX_QUEUED_UPLOADS {
            return None;
        }
    }
    Some(entries)
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

    struct Dir(std::path::PathBuf);

    impl Dir {
        fn new(tag: &str) -> Dir {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "stellar-gate-{tag}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Dir(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn revoke_discards_queue_and_blocks_later_uploads() {
        // 검증 기준(XVI.8.5): revoke fixture network request 0.
        let mut gate = TelemetryGate::new(ConsentRecord::default());
        let granted = gate.apply(ConsentEvent::Grant, 10).unwrap();
        assert_eq!(granted.discarded, 0);
        for code in 0..3 {
            gate.enqueue(preview(code)).unwrap();
        }
        let revoked = gate.apply(ConsentEvent::Revoke, 20).unwrap();
        assert_eq!(revoked.discarded, 3);
        assert_eq!(revoked.transition.to, ConsentState::Declined);
        assert_eq!(gate.queued(), 0);
        let mut transport = RecordingTransport::default();
        assert!(gate.flush(&mut transport).blocked);
        assert_eq!(
            gate.enqueue(preview(9)),
            Err(UploadBlocked::NoConsent(ConsentState::Declined))
        );
        assert!(gate.flush(&mut transport).blocked);
        assert_eq!(transport.requests.len(), 0);
        // 다시 동의해도 버린 payload는 돌아오지 않습니다.
        gate.apply(ConsentEvent::Grant, 30).unwrap();
        assert_eq!(gate.flush(&mut transport).sent, 0);
        assert_eq!(transport.requests.len(), 0);
        // Granted에서 Decline도 철회와 같이 버립니다.
        gate.enqueue(preview(1)).unwrap();
        assert_eq!(gate.apply(ConsentEvent::Decline, 40).unwrap().discarded, 1);
        assert_eq!(gate.flush(&mut transport).kept, 0);
        assert_eq!(transport.requests.len(), 0);
        // 철회할 동의가 없으면 오류이고 아무것도 바뀌지 않습니다.
        assert_eq!(
            gate.apply(ConsentEvent::Revoke, 50),
            Err(TransitionError::NothingToRevoke)
        );
    }

    #[test]
    fn persisted_revoke_removes_the_outbox_and_keeps_local_store() {
        let dir = Dir::new("persist");
        // 로컬 store(보존 정책을 따르고 철회로 지우지 않음).
        let mut store = DiagnosticStore::default();
        store.record_feature_failure(Feature::Save, 1, 1);
        let store_path = dir.0.join(crate::store::STORE_FILE);
        store.write_file(&store_path).unwrap();
        let store_bytes = std::fs::read(&store_path).unwrap();

        let mut opened = TelemetryGate::open(&dir.0).unwrap();
        assert_eq!(opened.consent_load, ConsentLoad::Missing);
        opened.gate.apply(ConsentEvent::Grant, 1).unwrap();
        opened.gate.enqueue(preview(1)).unwrap();
        opened.gate.enqueue(preview(2)).unwrap();
        opened.gate.save(&dir.0).unwrap();
        let reopened = TelemetryGate::open(&dir.0).unwrap();
        assert_eq!(reopened.gate.queued(), 2);
        assert_eq!(
            reopened
                .gate
                .pending()
                .map(|p| p.bytes().to_vec())
                .collect::<Vec<_>>(),
            vec![preview(1).bytes().to_vec(), preview(2).bytes().to_vec()]
        );

        let mut gate = reopened.gate;
        assert_eq!(gate.apply(ConsentEvent::Revoke, 2).unwrap().discarded, 2);
        gate.save(&dir.0).unwrap();
        assert!(!dir.0.join(OUTBOX_FILE).exists());
        let mut after = TelemetryGate::open(&dir.0).unwrap().gate;
        let mut transport = RecordingTransport::default();
        after.flush(&mut transport);
        assert_eq!(transport.requests.len(), 0);
        assert_eq!(std::fs::read(&store_path).unwrap(), store_bytes);
    }

    #[test]
    fn stale_outbox_without_consent_is_discarded_on_open() {
        let dir = Dir::new("stale");
        let mut gate = TelemetryGate::new(granted());
        gate.enqueue(preview(1)).unwrap();
        gate.enqueue(preview(2)).unwrap();
        gate.save(&dir.0).unwrap();
        let outbox = std::fs::read(dir.0.join(OUTBOX_FILE)).unwrap();
        // 철회 저장이 동의 파일만 쓰고 끊긴 상황.
        crate::consent::write_consent(
            &dir.0.join(CONSENT_FILE),
            &ConsentRecord {
                state: ConsentState::Declined,
                changed_ms: 3,
            },
        )
        .unwrap();
        assert!(dir.0.join(OUTBOX_FILE).exists());
        let opened = TelemetryGate::open(&dir.0).unwrap();
        assert_eq!(opened.discarded_without_consent, 2);
        assert!(!dir.0.join(OUTBOX_FILE).exists());
        let mut transport = RecordingTransport::default();
        let mut gate = opened.gate;
        assert!(gate.flush(&mut transport).blocked);
        assert_eq!(transport.requests.len(), 0);
        // 동의 파일이 손상돼도 같습니다(NotAsked로 닫힘).
        std::fs::write(dir.0.join(OUTBOX_FILE), &outbox).unwrap();
        std::fs::write(dir.0.join(CONSENT_FILE), b"broken").unwrap();
        let opened = TelemetryGate::open(&dir.0).unwrap();
        assert_eq!(opened.consent_load, ConsentLoad::Unreadable);
        assert_eq!(opened.discarded_without_consent, 2);
        assert!(!dir.0.join(OUTBOX_FILE).exists());
    }

    #[test]
    fn tampered_outbox_entries_are_dropped() {
        let dir = Dir::new("tamper");
        let mut gate = TelemetryGate::new(granted());
        gate.enqueue(preview(1)).unwrap();
        gate.enqueue(preview(2)).unwrap();
        gate.save(&dir.0).unwrap();
        let path = dir.0.join(OUTBOX_FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        // 같은 길이로 금지 정보를 넣습니다(thread=render → thread=/tmp/x/).
        let tampered = text.replacen("thread=render", "thread=/tmp/x/", 1);
        assert_eq!(tampered.len(), text.len() + 1);
        let tampered = tampered.replacen("uptime_ms=10", "uptime_ms=1", 1);
        std::fs::write(&path, &tampered).unwrap();
        let opened = TelemetryGate::open(&dir.0).unwrap();
        assert_eq!(opened.discarded_invalid, 1);
        assert_eq!(opened.gate.queued(), 1);
        assert_eq!(
            opened.gate.pending().next().unwrap().bytes(),
            preview(2).bytes()
        );
        // 형식이 깨진 파일은 통째로 버립니다.
        for broken in [
            &b"garbage"[..],
            b"stellar.telemetry-outbox v1\npayload bytes=999\nx",
        ] {
            std::fs::write(&path, broken).unwrap();
            let opened = TelemetryGate::open(&dir.0).unwrap();
            assert_eq!((opened.gate.queued(), opened.discarded_invalid), (0, 1));
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
