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

/// 직렬화한 응답을 보낼 메시지 목록으로 바꿉니다(XXI.4.4, ADR-0097 결정 8).
///
/// `chunk_bytes` 이하이면 그대로 하나입니다. 넘으면 응답 문자열을 UTF-8 경계에서 `chunk_bytes` 이하 조각으로 자르고
/// `{"stellar_rpc":1,"id":…,"chunk":{"index":i,"count":n,"data":"…"}}` 메시지 n개로 보냅니다. 받는 쪽은 `data`를
/// index 순으로 이어 붙인 문자열을 원래 응답으로 해석합니다(생성 TypeScript client가 합니다). 응답 문자열은 조각으로
/// 한 번만 복사합니다.
pub fn chunk_reply(id: &J, reply: String, chunk_bytes: usize) -> Vec<String> {
    let chunk_bytes = chunk_bytes.max(4);
    if reply.len() <= chunk_bytes {
        return vec![reply];
    }
    let mut pieces = Vec::with_capacity(reply.len() / chunk_bytes + 1);
    let mut start = 0;
    while start < reply.len() {
        let mut end = (start + chunk_bytes).min(reply.len());
        while !reply.is_char_boundary(end) {
            end -= 1;
        }
        pieces.push(&reply[start..end]);
        start = end;
    }
    let count = J::Number(JsonNumber::from_u64(pieces.len() as u64));
    pieces
        .iter()
        .enumerate()
        .map(|(index, piece)| {
            crate::session::envelope(
                id,
                (
                    "chunk",
                    J::object(vec![
                        ("index", J::Number(JsonNumber::from_u64(index as u64))),
                        ("count", count.clone()),
                        ("data", J::String((*piece).to_owned())),
                    ]),
                ),
            )
        })
        .collect()
}

/// [`chunk_reply`] 조각을 이어 붙입니다(test·Rust 수신 쪽용). 조각이 빠지거나 순서 표기가 틀리면 `None`.
pub fn join_chunks(messages: &[J]) -> Option<String> {
    let mut parts: Vec<Option<&str>> = Vec::new();
    for message in messages {
        let chunk = message.get("chunk")?;
        let index = usize::try_from(chunk.get("index")?.as_i64()?).ok()?;
        let count = usize::try_from(chunk.get("count")?.as_i64()?).ok()?;
        if parts.is_empty() {
            parts = vec![None; count];
        }
        if count != parts.len() || index >= count || parts[index].is_some() {
            return None;
        }
        parts[index] = Some(chunk.get("data")?.as_str()?);
    }
    parts
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .map(|p| p.concat())
}
