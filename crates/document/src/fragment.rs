//! Fragment payload (VI.4).
//!
//! Fragment는 자식을 묶는 **zero-size marker**입니다. 렌더·layout 필드가 없고
//! 좌표계도 만들지 않습니다(VI.3.2). 공통 node column(VI.2.2: id, kind, parent,
//! order, visible, locked, 표시 이름) 외에 어떤 필드도 저장하지 않습니다.

use std::fmt;

/// Fragment payload 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FragmentError {
    /// payload bytes가 비어 있지 않습니다(Fragment는 필드가 없음).
    UnexpectedPayload {
        /// 발견된 byte 수.
        len: usize,
    },
}

impl fmt::Display for FragmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FragmentError::UnexpectedPayload { len } => {
                write!(f, "Fragment payload는 비어 있어야 합니다({len} bytes 발견)")
            }
        }
    }
}

impl std::error::Error for FragmentError {}

/// Fragment node payload: 필드 없는 marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FragmentPayload;

impl FragmentPayload {
    /// canonical bytes: 항상 빈 bytes(공통 metadata 외 추가 필드 없음).
    pub fn encode(&self) -> Vec<u8> {
        Vec::new()
    }

    /// bytes에서 복원합니다. 비어 있지 않으면 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<FragmentPayload, FragmentError> {
        if bytes.is_empty() {
            Ok(FragmentPayload)
        } else {
            Err(FragmentError::UnexpectedPayload { len: bytes.len() })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialized_payload_has_no_fields() {
        // 검증 기준(VI.4.1): payload serialized bytes가 metadata 외 추가 필드 없음.
        let bytes = FragmentPayload.encode();
        assert!(bytes.is_empty());
        assert_eq!(FragmentPayload::decode(&bytes).unwrap(), FragmentPayload);
        // 타입 자체도 zero-size.
        assert_eq!(std::mem::size_of::<FragmentPayload>(), 0);
    }

    #[test]
    fn non_empty_payload_is_rejected() {
        assert_eq!(
            FragmentPayload::decode(&[0x00]).unwrap_err(),
            FragmentError::UnexpectedPayload { len: 1 }
        );
    }
}
