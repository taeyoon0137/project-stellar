//! Metal frame CPU timestamp trace (XI.8.5).
//!
//! 아키텍처 §6.1의 T5("GPU 명령 제출 및 Present 예약 완료")를 backend에서 잽니다. frame마다 encode
//! 시작·끝, present 예약, submit(commit 반환) 시각을 [`TraceClock`]으로 기록합니다. Metal은 present를
//! commit 전에 command buffer에 붙이므로(`presentDrawable:`) commit이 돌아온 시각이 곧 T5입니다
//! ([`MetalFrameTrace::t5_nanos`]).
//!
//! 시계는 [`MetalBackend::set_trace_clock`](crate::MetalBackend::set_trace_clock)으로 공통 계층과 같은
//! 기준점의 시계를 넣을 수 있습니다. 그러면 T0~T4(입력·graph compile, `RenderGraphCompileTrace`)와 같은
//! 축에서 비교할 수 있습니다.
//!
//! **비민감 기록만**: 모든 field는 정적 이름과 정수입니다(II.3.2·XI.4.6과 같은 규칙).

use std::fmt;
use std::fmt::Write as _;

use stellar_renderer_core::graph_trace::TraceClock;

/// 보관하는 frame trace 최대 수(넘으면 오래된 것부터 버림).
pub const MAX_FRAME_TRACES: usize = 64;

/// 제출한 frame 하나의 CPU timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MetalFrameTrace {
    /// frame 번호.
    pub frame: u64,
    /// submission 번호.
    pub submission: u64,
    /// encode 시작(ns, [`TraceClock`] 기준).
    pub encode_start_nanos: u64,
    /// encode 끝(command buffer 기록 완료).
    pub encode_end_nanos: u64,
    /// present 예약 시각(offscreen frame은 `None`).
    pub present_nanos: Option<u64>,
    /// submit(commit 반환) 시각 = T5.
    pub submit_nanos: u64,
    /// encode한 graph pass 수.
    pub pass_count: u32,
    /// encode한 draw 수.
    pub draw_count: u32,
}

impl MetalFrameTrace {
    /// 정적 event 이름.
    pub const EVENT: &'static str = "renderer.metal.frame";

    /// T5(GPU 명령 제출 및 Present 예약 완료).
    pub const fn t5_nanos(&self) -> u64 {
        self.submit_nanos
    }

    /// (정적 field 이름, 값) 목록. 순서는 고정입니다. present가 없으면 `present`=0, `present_ns`=0.
    pub fn fields(&self) -> [(&'static str, u64); 10] {
        [
            ("frame", self.frame),
            ("submission", self.submission),
            ("encode_start_ns", self.encode_start_nanos),
            ("encode_end_ns", self.encode_end_nanos),
            ("present", u64::from(self.present_nanos.is_some())),
            ("present_ns", self.present_nanos.unwrap_or(0)),
            ("submit_ns", self.submit_nanos),
            (
                "encode_cpu_ns",
                self.encode_end_nanos
                    .saturating_sub(self.encode_start_nanos),
            ),
            ("passes", u64::from(self.pass_count)),
            ("draws", u64::from(self.draw_count)),
        ]
    }

    /// 고정 형식 한 줄(`<event> name=value ...`).
    pub fn to_line(&self) -> String {
        let mut line = String::from(Self::EVENT);
        for (name, value) in self.fields() {
            // String에 쓰기는 실패하지 않습니다.
            let _ = write!(line, " {name}={value}");
        }
        line
    }
}

/// backend가 쓰는 시계(Debug 출력용 감쌈).
pub(crate) struct Clock(pub(crate) Box<dyn TraceClock>);

impl fmt::Debug for Clock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TraceClock")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_line_is_fixed_format() {
        let trace = MetalFrameTrace {
            frame: 7,
            submission: 3,
            encode_start_nanos: 100,
            encode_end_nanos: 250,
            present_nanos: Some(260),
            submit_nanos: 300,
            pass_count: 2,
            draw_count: 5,
        };
        assert_eq!(trace.t5_nanos(), 300);
        assert_eq!(
            trace.to_line(),
            "renderer.metal.frame frame=7 submission=3 encode_start_ns=100 encode_end_ns=250 \
             present=1 present_ns=260 submit_ns=300 encode_cpu_ns=150 passes=2 draws=5"
        );
        let offscreen = MetalFrameTrace {
            present_nanos: None,
            encode_end_nanos: 50,
            ..trace
        };
        let fields = offscreen.fields();
        assert_eq!(fields[4], ("present", 0));
        assert_eq!(fields[5], ("present_ns", 0));
        // 시계가 뒤로 가도 넘치지 않습니다.
        assert_eq!(fields[7], ("encode_cpu_ns", 0));
    }
}
