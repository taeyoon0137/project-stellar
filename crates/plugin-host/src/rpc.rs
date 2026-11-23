//! Host RPC 오류와 상한 (XXI.4.2·XXI.4.3, ADR-0097 결정 7).

use crate::json::{JsonNumber, JsonValue as J};

/// 메시지 최대 byte 수.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// batch 최대 항목 수.
pub const MAX_BATCH_ITEMS: usize = 64;

/// session당 처리 중 메시지 최대 수.
pub const MAX_IN_FLIGHT: usize = 16;

/// 1초 창에 받는 최대 메시지 수(batch는 1개).
pub const MAX_MESSAGES_PER_SECOND: usize = 20;

/// 대용량 응답 조각 크기(byte).
pub const CHUNK_BYTES: usize = 256 * 1024;

/// RPC 상한 묶음.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpcLimits {
    /// 메시지 최대 byte 수.
    pub max_message_bytes: usize,
    /// batch 최대 항목 수.
    pub max_batch_items: usize,
    /// 처리 중 메시지 최대 수.
    pub max_in_flight: usize,
    /// 1초 창 최대 메시지 수.
    pub max_messages_per_second: usize,
    /// 응답 조각 크기.
    pub chunk_bytes: usize,
}

impl Default for RpcLimits {
    fn default() -> Self {
        RpcLimits {
            max_message_bytes: MAX_MESSAGE_BYTES,
            max_batch_items: MAX_BATCH_ITEMS,
            max_in_flight: MAX_IN_FLIGHT,
            max_messages_per_second: MAX_MESSAGES_PER_SECOND,
            chunk_bytes: CHUNK_BYTES,
        }
    }
}

impl RpcLimits {
    /// IDL `RpcLimits` record.
    pub fn to_json(&self) -> J {
        let n = |v: usize| J::Number(JsonNumber::from_u64(v as u64));
        J::object(vec![
            ("max_message_bytes", n(self.max_message_bytes)),
            ("max_batch_items", n(self.max_batch_items)),
            ("max_in_flight", n(self.max_in_flight)),
            ("max_messages_per_second", n(self.max_messages_per_second)),
            ("chunk_bytes", n(self.chunk_bytes)),
        ])
    }
}

/// RPC 오류. `code`는 고정 진단 code, `message`는 고정 문구입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    /// 진단 code.
    pub code: &'static str,
    /// 사람이 읽는 고정 문구.
    pub message: &'static str,
    /// 구조화 부가 정보.
    pub data: Option<J>,
}

impl RpcError {
    /// 오류를 만듭니다.
    pub fn new(code: &'static str, message: &'static str) -> RpcError {
        RpcError {
            code,
            message,
            data: None,
        }
    }

    /// 부가 정보를 붙입니다.
    pub fn with_data(mut self, data: J) -> RpcError {
        self.data = Some(data);
        self
    }

    /// `error` object.
    pub fn to_json(&self) -> J {
        let mut fields = vec![
            ("code", J::string(self.code)),
            ("message", J::string(self.message)),
        ];
        if let Some(data) = &self.data {
            fields.push(("data", data.clone()));
        }
        J::object(fields)
    }
}
