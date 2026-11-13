//! 레이아웃 canonical bytes 보조 writer/reader (X.1.1, crate 내부).
//!
//! 모든 다중 바이트 값은 little-endian입니다. float은 [`FiniteF32`] 비트(`-0.0`은 `+0.0`)로
//! 쓰고, 읽을 때 비유한 값을 거부합니다. 이 형식은 런타임·fixture 비교용이며 저장 포맷이
//! 아닙니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::{FiniteF32, Insets, Length};

use crate::error::LayoutError;
use crate::rounding::LayoutUnit;

/// canonical bytes writer.
#[derive(Debug, Default)]
pub(crate) struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub(crate) fn new() -> Writer {
        Writer { buf: Vec::new() }
    }

    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub(crate) fn u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    pub(crate) fn bool(&mut self, value: bool) {
        self.buf.push(u8::from(value));
    }

    pub(crate) fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    /// 길이(`usize`)를 `u32` LE로 씁니다. 레이아웃 입력은 `u32` 범위를 넘는 목록을 갖지 않습니다
    /// (넘으면 saturate되어 디코딩 시 길이 불일치로 거부됩니다).
    pub(crate) fn len(&mut self, len: usize) {
        self.u32(u32::try_from(len).unwrap_or(u32::MAX));
    }

    pub(crate) fn i32(&mut self, value: i32) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn finite(&mut self, value: FiniteF32) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn length(&mut self, value: Length) {
        self.finite(value.raw());
    }

    pub(crate) fn unit(&mut self, value: LayoutUnit) {
        self.i32(value.raw());
    }

    pub(crate) fn id(&mut self, id: StableId) {
        self.bytes(&id.to_bytes());
    }

    pub(crate) fn order_key(&mut self, key: &OrderKey) {
        self.len(key.byte_len());
        self.bytes(key.as_bytes());
    }

    pub(crate) fn insets(&mut self, insets: &Insets) {
        let (top, right, bottom, left) = insets.components();
        for value in [top, right, bottom, left] {
            self.bytes(&value.to_le_bytes());
        }
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.buf
    }
}

/// canonical bytes reader.
#[derive(Debug)]
pub(crate) struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(input: &'a [u8]) -> Reader<'a> {
        Reader { input, pos: 0 }
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], LayoutError> {
        if self.input.len() - self.pos < n {
            return Err(LayoutError::Malformed {
                reason: "입력 부족",
            });
        }
        let slice = &self.input[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], LayoutError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, LayoutError> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn bool(&mut self, reason: &'static str) -> Result<bool, LayoutError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(LayoutError::Malformed { reason }),
        }
    }

    pub(crate) fn u16(&mut self) -> Result<u16, LayoutError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, LayoutError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    /// 목록 길이. 남은 bytes로 표현할 수 없는 길이는 거부합니다(과대 할당 방지).
    pub(crate) fn len(&mut self, min_item_bytes: usize) -> Result<usize, LayoutError> {
        let len = usize::try_from(self.u32()?).map_err(|_| LayoutError::Malformed {
            reason: "목록 길이",
        })?;
        let remaining = self.input.len() - self.pos;
        if len.saturating_mul(min_item_bytes.max(1)) > remaining {
            return Err(LayoutError::Malformed {
                reason: "목록 길이가 남은 입력보다 큽니다",
            });
        }
        Ok(len)
    }

    pub(crate) fn i32(&mut self) -> Result<i32, LayoutError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub(crate) fn finite(&mut self, field: &'static str) -> Result<FiniteF32, LayoutError> {
        FiniteF32::from_le_bytes(self.array()?).map_err(|_| LayoutError::NonFinite { field })
    }

    pub(crate) fn length(&mut self, field: &'static str) -> Result<Length, LayoutError> {
        Ok(Length::from_raw(self.finite(field)?))
    }

    pub(crate) fn unit(&mut self) -> Result<LayoutUnit, LayoutError> {
        Ok(LayoutUnit::from_raw(self.i32()?))
    }

    pub(crate) fn id(&mut self) -> Result<StableId, LayoutError> {
        Ok(StableId::from_bytes(self.array()?))
    }

    pub(crate) fn order_key(&mut self) -> Result<OrderKey, LayoutError> {
        let len = self.len(1)?;
        let bytes = self.take(len)?.to_vec();
        OrderKey::from_bytes(bytes).map_err(|_| LayoutError::Malformed {
            reason: "OrderKey 정규형",
        })
    }

    pub(crate) fn insets(&mut self, field: &'static str) -> Result<Insets, LayoutError> {
        let top = self.finite(field)?.get();
        let right = self.finite(field)?.get();
        let bottom = self.finite(field)?.get();
        let left = self.finite(field)?.get();
        Insets::new(top, right, bottom, left).map_err(|_| LayoutError::NonFinite { field })
    }

    pub(crate) fn expect_magic(
        &mut self,
        magic: &[u8; 4],
        version: u16,
    ) -> Result<(), LayoutError> {
        if self.take(4)? != magic {
            return Err(LayoutError::Malformed {
                reason: "magic 불일치",
            });
        }
        if self.u16()? != version {
            return Err(LayoutError::Malformed {
                reason: "지원하지 않는 version",
            });
        }
        Ok(())
    }

    pub(crate) fn expect_end(&self) -> Result<(), LayoutError> {
        if self.pos == self.input.len() {
            Ok(())
        } else {
            Err(LayoutError::Malformed {
                reason: "trailing bytes",
            })
        }
    }
}
