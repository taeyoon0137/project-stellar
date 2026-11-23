//! 원문 보기 버퍼 (XXIX.2.12): 제어 문자 주입 무력화, 상한 유지·잘림 표시, 진단·snapshot에 내용 0, 실측.

mod support;

use std::time::Instant;

use stellar_agent_host::raw::{RawBuffer, RawStream, DEFAULT_RAW_CAP_BYTES};
use stellar_agent_host::session::{
    AgentSession, AgentSessionId, SessionEffect, SessionEvent, SessionMode, UserStartRequest,
};
use support::*;

const SECRET_MARK: &str = "raw-content-marker-91b2";

#[test]
fn injected_controls_are_preserved_raw_and_neutralized_for_display() {
    let s = setup(
        "raw-ctl",
        "codex",
        &format!(
            "{OK_PROBES_CODEX}[session]\nout \\x1b]0;{SECRET_MARK}\\x07\\x1b[2J\\x1b[31mred\\r\\n\nerr \\x1b[?1049h\\x00\\x7f\\n\nout not json\\n\nexit 0\n"
        ),
    );
    let v = FixtureVerifier::accepting();
    let pass = run_preflight(&s.profile, &s.target, &v, &s.home, &s.scratch).unwrap();
    let output = output_for(&s.profile);
    pass.launch("x".into(), None, output.clone())
        .unwrap()
        .wait();
    let raw = output.raw_bytes();
    assert!(raw.starts_with(b"\x1b]0;"));
    assert!(raw.windows(8).any(|w| w == b"\x1b[?1049h"));
    let shown = output.raw_display_text();
    assert!(
        !shown
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t'),
        "{shown:?}"
    );
    assert!(shown.contains('\u{241b}'));
    // 디코드가 실패해도 원문은 보존됩니다.
    assert!(output.decode_failure().is_some());
    assert!(shown.contains("not json"));
    // Debug·진단 표현에 내용 0.
    assert!(!format!("{output:?}").contains(SECRET_MARK));
    assert!(!format!("{:?}", output.raw_stats()).contains(SECRET_MARK));
    // 세션 종료 진단 기록에도 내용 0(id·duration·code만).
    let mut session = AgentSession::start_by_user(UserStartRequest {
        session: AgentSessionId(5),
        mode: SessionMode::ReadOnly,
        now_ms: 0,
        reconnect_grace_ms: 1,
    });
    let effects = session.apply(SessionEvent::ProcessExited, 3).unwrap();
    for effect in effects {
        if let SessionEffect::RecordDiagnostic(d) = effect {
            assert!(!d.to_string().contains(SECRET_MARK));
        }
    }
}

#[test]
fn overflow_keeps_memory_cap_and_marks_truncation() {
    let mut buffer = RawBuffer::with_cap(1024 * 1024);
    let chunk = vec![b'y'; 64 * 1024];
    for _ in 0..64 {
        buffer.push(RawStream::Stdout, &chunk);
        buffer.push(RawStream::Stderr, b"e\n");
        assert!(buffer.len() <= buffer.cap());
    }
    assert!(buffer.truncated());
    assert_eq!(buffer.received(), 64 * (64 * 1024 + 2));
    assert_eq!(buffer.dropped() + buffer.len() as u64, buffer.received());
    assert!(buffer.display_text().starts_with("[앞부분 "));
}

#[test]
fn measure_default_cap_cost() {
    // 상한 값 실측 근거: 기본 상한(8 MiB)을 64 KiB 조각으로 4배 넘게 채우는 비용과 표시 텍스트 준비 비용.
    let mut buffer = RawBuffer::with_cap(DEFAULT_RAW_CAP_BYTES);
    let chunk = vec![b'z'; 64 * 1024];
    let started = Instant::now();
    for _ in 0..(4 * DEFAULT_RAW_CAP_BYTES / chunk.len()) {
        buffer.push(RawStream::Stdout, &chunk);
    }
    let append = started.elapsed();
    let started = Instant::now();
    let text = buffer.display_text();
    let display = started.elapsed();
    assert_eq!(buffer.len(), DEFAULT_RAW_CAP_BYTES);
    assert!(text.len() >= DEFAULT_RAW_CAP_BYTES);
    eprintln!(
        "raw cap {} MiB: 32 MiB append {append:?}, display_text {display:?}",
        DEFAULT_RAW_CAP_BYTES / (1024 * 1024)
    );
}
