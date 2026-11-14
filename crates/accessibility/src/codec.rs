//! 접근성 모델 공용 canonical binary 도우미.
//!
//! canonical 규칙(V.1 R1~R3)을 따릅니다: 정수는 little-endian 고정 폭, 가변 길이 값은 `u32` LE 길이
//! 프리픽스, 문자열은 UTF-8입니다. crate 그래프상 이 crate는 core-types에만 의존하므로(storage·document
//! 없음) 직접 구현합니다. 디코더는 남은 입력보다 긴 길이·개수를 할당 전에 거부합니다.

use std::fmt;

use stellar_core_types::ids::StableId;

/// 접근성 binary 형식 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodecError {
    /// 원인.
    pub reason: &'static str,
}

impl CodecError {
    pub(crate) const fn new(reason: &'static str) -> CodecError {
        CodecError { reason }
    }
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "접근성 binary 형식 위반: {}", self.reason)
    }
}

impl std::error::Error for CodecError {}

/// `u8`을 기록합니다.
pub(crate) fn put_u8(buf: &mut Vec<u8>, value: u8) {
    buf.push(value);
}

/// `u16` LE를 기록합니다.
pub(crate) fn put_u16(buf: &mut Vec<u8>, value: u16) {
    buf.extend_from_slice(&value.to_le_bytes());
}

/// `u32` LE를 기록합니다.
pub(crate) fn put_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

/// 개수(`usize`)를 `u32` LE로 기록합니다. `u32` 범위를 넘는 개수는 모델 상한 밖입니다.
pub(crate) fn put_count(buf: &mut Vec<u8>, count: usize) {
    put_u32(buf, u32::try_from(count).unwrap_or(u32::MAX));
}

/// `StableId` 16바이트를 기록합니다.
pub(crate) fn put_id(buf: &mut Vec<u8>, id: StableId) {
    buf.extend_from_slice(&id.to_bytes());
}

/// 길이 프리픽스 bytes를 기록합니다.
pub(crate) fn put_bytes(buf: &mut Vec<u8>, bytes: &[u8]) {
    put_count(buf, bytes.len());
    buf.extend_from_slice(bytes);
}

/// `Option<u32>`를 flag(0/1) + `u32` LE로 기록합니다.
pub(crate) fn put_opt_u32(buf: &mut Vec<u8>, value: Option<u32>) {
    match value {
        Some(value) => {
            put_u8(buf, 1);
            put_u32(buf, value);
        }
        None => put_u8(buf, 0),
    }
}

/// 앞에서부터 읽는 커서. 경계를 넘으면 [`CodecError`].
pub(crate) struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// 입력 전체를 읽는 커서를 만듭니다.
    pub(crate) fn new(input: &'a [u8]) -> Reader<'a> {
        Reader { input, pos: 0 }
    }

    /// 남은 bytes 수.
    pub(crate) fn remaining(&self) -> usize {
        self.input.len() - self.pos
    }

    /// `n` 바이트를 읽습니다.
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], CodecError> {
        if self.remaining() < n {
            return Err(CodecError::new("입력 부족"));
        }
        let slice = &self.input[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    /// `u8`을 읽습니다.
    pub(crate) fn u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.take(1)?[0])
    }

    /// `u16` LE를 읽습니다.
    pub(crate) fn u16(&mut self) -> Result<u16, CodecError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    /// `u32` LE를 읽습니다.
    pub(crate) fn u32(&mut self) -> Result<u32, CodecError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// 원소 하나가 최소 `min_element` bytes인 목록의 개수(`u32` LE)를 읽습니다. 남은 입력으로 담을 수
    /// 없는 개수는 할당 전에 거부합니다.
    pub(crate) fn count(&mut self, min_element: usize) -> Result<usize, CodecError> {
        let count = self.u32()? as usize;
        if count.saturating_mul(min_element.max(1)) > self.remaining() {
            return Err(CodecError::new("개수가 남은 입력보다 큼"));
        }
        Ok(count)
    }

    /// `StableId` 16바이트를 읽습니다.
    pub(crate) fn id(&mut self) -> Result<StableId, CodecError> {
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(self.take(16)?);
        Ok(StableId::from_bytes(bytes))
    }

    /// `0`/`1` bool을 읽습니다. 그 외 값은 거부합니다.
    pub(crate) fn bool(&mut self) -> Result<bool, CodecError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(CodecError::new("bool flag가 0/1이 아님")),
        }
    }

    /// 길이 프리픽스 bytes를 읽습니다.
    pub(crate) fn bytes(&mut self) -> Result<&'a [u8], CodecError> {
        let len = self.count(1)?;
        self.take(len)
    }

    /// flag + `u32` LE 형식의 `Option<u32>`를 읽습니다.
    pub(crate) fn opt_u32(&mut self) -> Result<Option<u32>, CodecError> {
        if self.bool()? {
            Ok(Some(self.u32()?))
        } else {
            Ok(None)
        }
    }

    /// 입력을 정확히 다 읽었는지 확인합니다.
    pub(crate) fn expect_end(&self) -> Result<(), CodecError> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(CodecError::new("trailing bytes"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_count_is_rejected_before_allocation() {
        let mut buf = Vec::new();
        put_u32(&mut buf, u32::MAX);
        let mut reader = Reader::new(&buf);
        assert_eq!(
            reader.count(16),
            Err(CodecError::new("개수가 남은 입력보다 큼"))
        );
    }

    #[test]
    fn primitives_round_trip_and_trailing_bytes_fail() {
        let mut buf = Vec::new();
        put_u8(&mut buf, 7);
        put_u16(&mut buf, 0x0102);
        put_opt_u32(&mut buf, Some(9));
        put_opt_u32(&mut buf, None);
        put_bytes(&mut buf, b"ab");
        put_id(&mut buf, StableId::from_bytes([3; 16]));
        let mut reader = Reader::new(&buf);
        assert_eq!(reader.u8(), Ok(7));
        assert_eq!(reader.u16(), Ok(0x0102));
        assert_eq!(reader.opt_u32(), Ok(Some(9)));
        assert_eq!(reader.opt_u32(), Ok(None));
        assert_eq!(reader.bytes(), Ok(&b"ab"[..]));
        assert_eq!(reader.id(), Ok(StableId::from_bytes([3; 16])));
        assert_eq!(reader.expect_end(), Ok(()));

        buf.push(0);
        let mut reader = Reader::new(&buf);
        reader.take(buf.len() - 1).expect("앞부분");
        assert_eq!(reader.expect_end(), Err(CodecError::new("trailing bytes")));
    }
}
