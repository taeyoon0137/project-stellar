//! component 모듈 공용 canonical binary 도우미.
//!
//! canonical 규칙(V.1 R1~R3)을 따릅니다: 정수는 little-endian 고정 폭, 가변 길이 값은
//! `u32` LE 길이 프리픽스, 문자열은 UTF-8입니다. crate 그래프상 storage에 의존하지 않으므로
//! 다른 payload 모듈(frame·modifier 등)과 마찬가지로 이 모듈이 직접 구현합니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;

use crate::node_kind::NodeKind;

/// 디코딩 실패 원인. 각 하위 모듈 오류의 `Malformed` variant로 변환됩니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Malformed(pub(crate) &'static str);

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

/// 개수(`usize`)를 `u32` LE로 기록합니다.
pub(crate) fn put_count(buf: &mut Vec<u8>, count: usize) {
    put_u32(buf, count as u32);
}

/// `StableId` 16바이트를 기록합니다.
pub(crate) fn put_id(buf: &mut Vec<u8>, id: StableId) {
    buf.extend_from_slice(&id.to_bytes());
}

/// bool을 `0`/`1` 한 바이트로 기록합니다.
pub(crate) fn put_bool(buf: &mut Vec<u8>, value: bool) {
    buf.push(u8::from(value));
}

/// 길이 프리픽스 bytes를 기록합니다.
pub(crate) fn put_bytes(buf: &mut Vec<u8>, bytes: &[u8]) {
    put_count(buf, bytes.len());
    buf.extend_from_slice(bytes);
}

/// 길이 프리픽스 UTF-8 문자열을 기록합니다.
pub(crate) fn put_str(buf: &mut Vec<u8>, value: &str) {
    put_bytes(buf, value.as_bytes());
}

/// `Option<StableId>`를 flag(0/1) + id로 기록합니다.
pub(crate) fn put_opt_id(buf: &mut Vec<u8>, id: Option<StableId>) {
    match id {
        Some(id) => {
            put_u8(buf, 1);
            put_id(buf, id);
        }
        None => put_u8(buf, 0),
    }
}

/// NodeKind를 `u16` LE로 기록합니다.
pub(crate) fn put_kind(buf: &mut Vec<u8>, kind: NodeKind) {
    put_u16(buf, kind.as_u16());
}

/// OrderKey를 길이 프리픽스 bytes로 기록합니다.
pub(crate) fn put_order_key(buf: &mut Vec<u8>, key: &OrderKey) {
    put_bytes(buf, key.as_bytes());
}

/// 앞에서부터 읽는 커서. 경계를 넘으면 [`Malformed`].
pub(crate) struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// 입력 전체를 읽는 커서를 만듭니다.
    pub(crate) fn new(input: &'a [u8]) -> Reader<'a> {
        Reader { input, pos: 0 }
    }

    /// `n` 바이트를 읽습니다.
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], Malformed> {
        if self.input.len() - self.pos < n {
            return Err(Malformed("입력 부족"));
        }
        let slice = &self.input[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    /// `u8`을 읽습니다.
    pub(crate) fn u8(&mut self) -> Result<u8, Malformed> {
        Ok(self.take(1)?[0])
    }

    /// `u16` LE를 읽습니다.
    pub(crate) fn u16(&mut self) -> Result<u16, Malformed> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    /// `u32` LE를 읽습니다.
    pub(crate) fn u32(&mut self) -> Result<u32, Malformed> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// 개수(`u32` LE)를 읽습니다.
    pub(crate) fn count(&mut self) -> Result<usize, Malformed> {
        Ok(self.u32()? as usize)
    }

    /// `StableId` 16바이트를 읽습니다.
    pub(crate) fn id(&mut self) -> Result<StableId, Malformed> {
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(self.take(16)?);
        Ok(StableId::from_bytes(bytes))
    }

    /// `0`/`1` bool을 읽습니다. 그 외 값은 거부합니다.
    pub(crate) fn bool(&mut self) -> Result<bool, Malformed> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Malformed("bool flag가 0/1이 아님")),
        }
    }

    /// 길이 프리픽스 bytes를 읽습니다.
    pub(crate) fn bytes(&mut self) -> Result<&'a [u8], Malformed> {
        let len = self.count()?;
        self.take(len)
    }

    /// 길이 프리픽스 UTF-8 문자열을 읽습니다.
    pub(crate) fn string(&mut self) -> Result<String, Malformed> {
        let bytes = self.bytes()?;
        String::from_utf8(bytes.to_vec()).map_err(|_| Malformed("UTF-8 아님"))
    }

    /// flag + id 형식의 `Option<StableId>`를 읽습니다.
    pub(crate) fn opt_id(&mut self) -> Result<Option<StableId>, Malformed> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.id()?)),
            _ => Err(Malformed("option flag가 0/1이 아님")),
        }
    }

    /// NodeKind(`u16` LE)를 읽습니다. 알 수 없는 값은 거부합니다.
    pub(crate) fn kind(&mut self) -> Result<NodeKind, Malformed> {
        NodeKind::from_u16(self.u16()?).ok_or(Malformed("알 수 없는 node kind"))
    }

    /// 길이 프리픽스 OrderKey를 읽습니다. 비정규 키는 거부합니다.
    pub(crate) fn order_key(&mut self) -> Result<OrderKey, Malformed> {
        let bytes = self.bytes()?;
        OrderKey::from_bytes(bytes.to_vec()).map_err(|_| Malformed("잘못된 OrderKey"))
    }

    /// 입력을 정확히 다 읽었는지 확인합니다.
    pub(crate) fn expect_end(&self) -> Result<(), Malformed> {
        if self.pos == self.input.len() {
            Ok(())
        } else {
            Err(Malformed("trailing bytes"))
        }
    }
}

/// 디코딩 시 미리 확보할 용량 상한(악성 개수 값으로 큰 할당을 하지 않도록).
pub(crate) fn capacity_hint(count: usize) -> usize {
    count.min(1024)
}
