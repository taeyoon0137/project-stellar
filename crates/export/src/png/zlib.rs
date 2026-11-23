//! zlib(RFC 1950)·DEFLATE(RFC 1951) 압축·해제와 CRC-32·Adler-32 (XX.2.3).
//!
//! export crate는 외부 crate 없이 PNG를 쓰고 읽습니다(ADR-0103 결정 4).
//!
//! - **압축**([`compress`]): LZ77(32 KiB 창, 3-byte hash chain, chain 길이 상한 [`MAX_CHAIN`])과 고정 Huffman
//!   block 하나입니다. 입력이 같으면 출력 bytes가 같습니다(결정론). 압축 중 `keep_going`이 거짓을 돌려주면
//!   [`Cancelled`]로 멈춥니다.
//! - **해제**([`decompress`]): stored·고정·동적 Huffman block을 모두 읽습니다. 신뢰하지 않는 입력을 가정해 출력
//!   상한(`max_output`)을 넘거나 잘린·잘못된 입력이면 [`InflateError`]이고 panic하지 않습니다. Adler-32를 검사합니다.

use std::fmt;

/// hash chain을 따라가는 최대 후보 수.
pub const MAX_CHAIN: usize = 64;

/// LZ77 창 크기(DEFLATE 상한).
const WINDOW: usize = 32 * 1024;

/// 최소·최대 match 길이.
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;

/// hash table 크기(2^15).
const HASH_BITS: u32 = 15;
const HASH_SIZE: usize = 1 << HASH_BITS;

/// 취소 확인 간격(입력 byte).
const CANCEL_STRIDE: usize = 64 * 1024;

/// CRC-32(IEEE, PNG chunk) table.
const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
};

/// 진행 중 CRC-32를 갱신합니다(초기값 `0xFFFF_FFFF`, 끝에 반전).
pub fn crc32_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for byte in bytes {
        crc = CRC_TABLE[((crc ^ u32::from(*byte)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc
}

/// bytes의 CRC-32.
pub fn crc32(bytes: &[u8]) -> u32 {
    crc32_update(0xFFFF_FFFF, bytes) ^ 0xFFFF_FFFF
}

/// Adler-32.
pub fn adler32(bytes: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let (mut a, mut b) = (1u32, 0u32);
    // 5552 byte마다 나머지를 취하면 u32가 넘치지 않습니다(zlib 규칙).
    for chunk in bytes.chunks(5552) {
        for byte in chunk {
            a += u32::from(*byte);
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

/// 압축이 취소되었습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

/// LSB-first bit writer.
struct BitWriter {
    out: Vec<u8>,
    bits: u64,
    count: u32,
}

impl BitWriter {
    fn new(capacity: usize) -> BitWriter {
        BitWriter {
            out: Vec::with_capacity(capacity),
            bits: 0,
            count: 0,
        }
    }

    /// 값의 하위 `n`비트를 LSB부터 씁니다.
    fn put(&mut self, value: u32, n: u32) {
        self.bits |= u64::from(value) << self.count;
        self.count += n;
        while self.count >= 8 {
            self.out.push(self.bits as u8);
            self.bits >>= 8;
            self.count -= 8;
        }
    }

    /// Huffman code(`len`비트, MSB가 먼저 나가야 함)를 씁니다.
    fn put_code(&mut self, code: u32, len: u32) {
        self.put(reverse_bits(code, len), len);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.count > 0 {
            self.out.push(self.bits as u8);
        }
        self.out
    }
}

fn reverse_bits(mut code: u32, len: u32) -> u32 {
    let mut out = 0;
    for _ in 0..len {
        out = (out << 1) | (code & 1);
        code >>= 1;
    }
    out
}

/// 길이 code 257..=285의 기준 길이와 추가 bit 수.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// 거리 code 0..=29의 기준 거리와 추가 bit 수.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// 고정 Huffman literal/length code(RFC 1951 3.2.6).
fn put_fixed_literal(writer: &mut BitWriter, symbol: u32) {
    match symbol {
        0..=143 => writer.put_code(0x30 + symbol, 8),
        144..=255 => writer.put_code(0x190 + (symbol - 144), 9),
        256..=279 => writer.put_code(symbol - 256, 7),
        _ => writer.put_code(0xC0 + (symbol - 280), 8),
    }
}

fn put_match(writer: &mut BitWriter, length: usize, distance: usize) {
    // 길이 code: 기준 길이가 length 이하인 마지막 항목.
    let li = LENGTH_BASE
        .iter()
        .rposition(|base| usize::from(*base) <= length)
        .unwrap_or(0);
    put_fixed_literal(writer, 257 + li as u32);
    let extra = u32::from(LENGTH_EXTRA[li]);
    if extra > 0 {
        writer.put((length - usize::from(LENGTH_BASE[li])) as u32, extra);
    }
    let di = DIST_BASE
        .iter()
        .rposition(|base| usize::from(*base) <= distance)
        .unwrap_or(0);
    writer.put_code(di as u32, 5);
    let extra = u32::from(DIST_EXTRA[di]);
    if extra > 0 {
        writer.put((distance - usize::from(DIST_BASE[di])) as u32, extra);
    }
}

fn hash3(data: &[u8], at: usize) -> usize {
    let value =
        (u32::from(data[at]) << 16) | (u32::from(data[at + 1]) << 8) | u32::from(data[at + 2]);
    (value.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
}

/// `data`를 zlib stream으로 압축합니다(모듈 문서). `keep_going`은 약 64 KiB마다 불립니다.
pub fn compress(data: &[u8], keep_going: &mut dyn FnMut() -> bool) -> Result<Vec<u8>, Cancelled> {
    let mut writer = BitWriter::new(data.len() / 2 + 64);
    // zlib header: CM=8(deflate), CINFO=7(32K 창), FLEVEL=2(기본). (0x78·256 + 0x9C) % 31 == 0.
    writer.put(0x78, 8);
    writer.put(0x9C, 8);
    // 마지막 block, 고정 Huffman.
    writer.put(1, 1);
    writer.put(1, 2);
    let mut head = vec![u32::MAX; HASH_SIZE];
    let mut prev = vec![u32::MAX; WINDOW];
    let mut at = 0usize;
    let mut next_check = CANCEL_STRIDE;
    let insert = |head: &mut [u32], prev: &mut [u32], pos: usize| {
        if pos + MIN_MATCH <= data.len() {
            let h = hash3(data, pos);
            prev[pos % WINDOW] = head[h];
            head[h] = pos as u32;
        }
    };
    while at < data.len() {
        if at >= next_check {
            if !keep_going() {
                return Err(Cancelled);
            }
            next_check = at + CANCEL_STRIDE;
        }
        let mut best_len = 0usize;
        let mut best_dist = 0usize;
        if at + MIN_MATCH <= data.len() {
            let max_len = (data.len() - at).min(MAX_MATCH);
            let mut candidate = head[hash3(data, at)];
            let mut chain = 0;
            while candidate != u32::MAX && chain < MAX_CHAIN {
                let pos = candidate as usize;
                if at - pos > WINDOW - 1 || pos >= at {
                    break;
                }
                let mut len = 0;
                while len < max_len && data[pos + len] == data[at + len] {
                    len += 1;
                }
                if len > best_len {
                    best_len = len;
                    best_dist = at - pos;
                    if len == max_len {
                        break;
                    }
                }
                let next = prev[pos % WINDOW];
                // 창 밖이거나 앞으로 가지 않는 연결은 끊습니다(오래된 slot 재사용).
                if next == u32::MAX || next as usize >= pos {
                    break;
                }
                candidate = next;
                chain += 1;
            }
        }
        if best_len >= MIN_MATCH {
            put_match(&mut writer, best_len, best_dist);
            for pos in at..at + best_len {
                insert(&mut head, &mut prev, pos);
            }
            at += best_len;
        } else {
            put_fixed_literal(&mut writer, u32::from(data[at]));
            insert(&mut head, &mut prev, at);
            at += 1;
        }
    }
    put_fixed_literal(&mut writer, 256);
    let mut out = writer.finish();
    out.extend_from_slice(&adler32(data).to_be_bytes());
    Ok(out)
}

/// 해제 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InflateError {
    /// 입력이 중간에 끝났습니다.
    Truncated,
    /// zlib header가 잘못되었습니다(압축 방식·창 크기·검사값·preset dictionary).
    BadHeader,
    /// block 형식이 잘못되었습니다.
    BadBlock,
    /// Huffman code 정의가 잘못되었습니다.
    BadCode,
    /// 거리가 이미 쓴 출력보다 멉니다.
    BadDistance,
    /// 출력이 상한을 넘습니다.
    TooLarge,
    /// Adler-32가 맞지 않습니다.
    Checksum,
}

impl fmt::Display for InflateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            InflateError::Truncated => "압축 data가 중간에 끝났습니다",
            InflateError::BadHeader => "zlib header가 잘못되었습니다",
            InflateError::BadBlock => "deflate block 형식이 잘못되었습니다",
            InflateError::BadCode => "Huffman code 정의가 잘못되었습니다",
            InflateError::BadDistance => "deflate 거리가 출력보다 멉니다",
            InflateError::TooLarge => "해제한 data가 상한을 넘습니다",
            InflateError::Checksum => "Adler-32 검사값이 맞지 않습니다",
        })
    }
}

impl std::error::Error for InflateError {}

/// LSB-first bit reader.
struct BitReader<'a> {
    data: &'a [u8],
    at: usize,
    bits: u64,
    count: u32,
}

impl BitReader<'_> {
    fn need(&mut self, n: u32) -> Result<(), InflateError> {
        while self.count < n {
            let byte = *self.data.get(self.at).ok_or(InflateError::Truncated)?;
            self.at += 1;
            self.bits |= u64::from(byte) << self.count;
            self.count += 8;
        }
        Ok(())
    }

    fn bits(&mut self, n: u32) -> Result<u32, InflateError> {
        if n == 0 {
            return Ok(0);
        }
        self.need(n)?;
        let value = (self.bits & ((1u64 << n) - 1)) as u32;
        self.bits >>= n;
        self.count -= n;
        Ok(value)
    }

    fn align_byte(&mut self) {
        let drop = self.count % 8;
        self.bits >>= drop;
        self.count -= drop;
    }
}

/// canonical Huffman 해독 표(길이별 개수와 길이 순 symbol).
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Huffman, InflateError> {
        let mut counts = [0u16; 16];
        for len in lengths {
            counts[usize::from(*len)] += 1;
        }
        counts[0] = 0;
        // 과포화(over-subscribed) code는 거부합니다. 불완전 code는 허용합니다(RFC 1951 거리 code 하나 등).
        let mut left: i32 = 1;
        for count in &counts[1..] {
            left <<= 1;
            left -= i32::from(*count);
            if left < 0 {
                return Err(InflateError::BadCode);
            }
        }
        let mut offsets = [0u16; 16];
        for len in 1..15 {
            offsets[len + 1] = offsets[len] + counts[len];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (symbol, len) in lengths.iter().enumerate() {
            if *len != 0 {
                let slot = &mut offsets[usize::from(*len)];
                symbols[usize::from(*slot)] = symbol as u16;
                *slot += 1;
            }
        }
        Ok(Huffman { counts, symbols })
    }

    fn decode(&self, reader: &mut BitReader<'_>) -> Result<u16, InflateError> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= reader.bits(1)? as i32;
            let count = i32::from(self.counts[len]);
            if code - first < count {
                return self
                    .symbols
                    .get((index + code - first) as usize)
                    .copied()
                    .ok_or(InflateError::BadCode);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(InflateError::BadCode)
    }
}

fn fixed_tables() -> Result<(Huffman, Huffman), InflateError> {
    let mut lengths = [0u8; 288];
    for (symbol, len) in lengths.iter_mut().enumerate() {
        *len = match symbol {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    Ok((Huffman::new(&lengths)?, Huffman::new(&[5u8; 30])?))
}

fn dynamic_tables(reader: &mut BitReader<'_>) -> Result<(Huffman, Huffman), InflateError> {
    const ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let nlen = reader.bits(5)? as usize + 257;
    let ndist = reader.bits(5)? as usize + 1;
    let ncode = reader.bits(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return Err(InflateError::BadBlock);
    }
    let mut code_lengths = [0u8; 19];
    for slot in ORDER.iter().take(ncode) {
        code_lengths[*slot] = reader.bits(3)? as u8;
    }
    let code = Huffman::new(&code_lengths)?;
    let mut lengths = vec![0u8; nlen + ndist];
    let mut at = 0;
    while at < lengths.len() {
        let symbol = code.decode(reader)?;
        let (value, repeat) = match symbol {
            0..=15 => (symbol as u8, 1),
            16 => {
                let previous = *lengths
                    .get(at.wrapping_sub(1))
                    .filter(|_| at > 0)
                    .ok_or(InflateError::BadBlock)?;
                (previous, 3 + reader.bits(2)? as usize)
            }
            17 => (0, 3 + reader.bits(3)? as usize),
            18 => (0, 11 + reader.bits(7)? as usize),
            _ => return Err(InflateError::BadCode),
        };
        if at + repeat > lengths.len() {
            return Err(InflateError::BadBlock);
        }
        lengths[at..at + repeat].fill(value);
        at += repeat;
    }
    if lengths[256] == 0 {
        // 끝 symbol이 없는 code는 block을 끝낼 수 없습니다.
        return Err(InflateError::BadCode);
    }
    Ok((
        Huffman::new(&lengths[..nlen])?,
        Huffman::new(&lengths[nlen..])?,
    ))
}

/// zlib stream을 해제합니다. 출력이 `max_output` byte를 넘으면 [`InflateError::TooLarge`]입니다.
pub fn decompress(data: &[u8], max_output: usize) -> Result<Vec<u8>, InflateError> {
    let header = data.get(..2).ok_or(InflateError::Truncated)?;
    let (cmf, flg) = (header[0], header[1]);
    if cmf & 0x0F != 8 || cmf >> 4 > 7 || (u16::from(cmf) * 256 + u16::from(flg)) % 31 != 0 {
        return Err(InflateError::BadHeader);
    }
    if flg & 0x20 != 0 {
        // preset dictionary는 PNG에서 쓰지 않습니다.
        return Err(InflateError::BadHeader);
    }
    let mut reader = BitReader {
        data,
        at: 2,
        bits: 0,
        count: 0,
    };
    let mut out: Vec<u8> = Vec::new();
    loop {
        let last = reader.bits(1)? == 1;
        match reader.bits(2)? {
            0 => {
                reader.align_byte();
                let len = reader.bits(16)?;
                let nlen = reader.bits(16)?;
                if len != !nlen & 0xFFFF {
                    return Err(InflateError::BadBlock);
                }
                // align 뒤 bit buffer는 byte 단위로 비어 있습니다.
                for _ in 0..len {
                    if out.len() >= max_output {
                        return Err(InflateError::TooLarge);
                    }
                    out.push(reader.bits(8)? as u8);
                }
            }
            kind @ (1 | 2) => {
                let (lit, dist) = if kind == 1 {
                    fixed_tables()?
                } else {
                    dynamic_tables(&mut reader)?
                };
                loop {
                    let symbol = usize::from(lit.decode(&mut reader)?);
                    if symbol < 256 {
                        if out.len() >= max_output {
                            return Err(InflateError::TooLarge);
                        }
                        out.push(symbol as u8);
                        continue;
                    }
                    if symbol == 256 {
                        break;
                    }
                    let li = symbol - 257;
                    if li >= LENGTH_BASE.len() {
                        return Err(InflateError::BadCode);
                    }
                    let length = usize::from(LENGTH_BASE[li])
                        + reader.bits(u32::from(LENGTH_EXTRA[li]))? as usize;
                    let di = usize::from(dist.decode(&mut reader)?);
                    if di >= DIST_BASE.len() {
                        return Err(InflateError::BadCode);
                    }
                    let distance = usize::from(DIST_BASE[di])
                        + reader.bits(u32::from(DIST_EXTRA[di]))? as usize;
                    if distance > out.len() {
                        return Err(InflateError::BadDistance);
                    }
                    if out.len() + length > max_output {
                        return Err(InflateError::TooLarge);
                    }
                    let start = out.len() - distance;
                    for i in 0..length {
                        let byte = out[start + i];
                        out.push(byte);
                    }
                }
            }
            _ => return Err(InflateError::BadBlock),
        }
        if last {
            break;
        }
    }
    reader.align_byte();
    let mut trailer = [0u8; 4];
    for byte in &mut trailer {
        *byte = reader.bits(8)? as u8;
    }
    if u32::from_be_bytes(trailer) != adler32(&out) {
        return Err(InflateError::Checksum);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn always() -> impl FnMut() -> bool {
        || true
    }

    #[test]
    fn crc_and_adler_match_reference_values() {
        // 표준 검사 문자열 "123456789".
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(adler32(b""), 1);
    }

    #[test]
    fn round_trips_repetitive_and_random_data() {
        let mut seed = 0x1234_5678u32;
        let mut random = Vec::new();
        for _ in 0..100_000 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            random.push(seed as u8);
        }
        let repetitive: Vec<u8> = (0..200_000u32).map(|i| (i % 7) as u8).collect();
        for input in [&b""[..], b"a", b"abcabcabcabc", &random, &repetitive] {
            let packed = compress(input, &mut always()).unwrap();
            assert_eq!(decompress(&packed, input.len()).unwrap(), input);
        }
        // 반복 data는 크게 줄어듭니다.
        let packed = compress(&repetitive, &mut always()).unwrap();
        assert!(packed.len() < repetitive.len() / 20, "{}", packed.len());
    }

    #[test]
    fn compression_is_deterministic() {
        let input: Vec<u8> = (0..50_000u32).map(|i| (i * 31 % 251) as u8).collect();
        assert_eq!(
            compress(&input, &mut always()).unwrap(),
            compress(&input, &mut always()).unwrap()
        );
    }

    #[test]
    fn cancellation_stops_compression() {
        let input = vec![0u8; 3 * CANCEL_STRIDE];
        let mut calls = 0;
        let result = compress(&input, &mut || {
            calls += 1;
            false
        });
        assert_eq!(result, Err(Cancelled));
        assert_eq!(calls, 1);
    }

    #[test]
    fn reads_stored_and_dynamic_blocks() {
        // stored block 하나("hello"): zlib header 78 01, BFINAL=1 BTYPE=00.
        let mut stored = vec![0x78, 0x01, 0x01, 0x05, 0x00, 0xFA, 0xFF];
        stored.extend_from_slice(b"hello");
        stored.extend_from_slice(&adler32(b"hello").to_be_bytes());
        assert_eq!(decompress(&stored, 16).unwrap(), b"hello");
        // 동적 Huffman block(BTYPE=2): 참조 zlib(level 9)이 만든 stream.
        const DYNAMIC: &[u8] = &[
            0x78, 0xDA, 0x85, 0x93, 0x4B, 0x0A, 0xC3, 0x40, 0x0C, 0x43, 0xF7, 0x39, 0x85, 0x8F,
            0x90, 0x4C, 0xE6, 0xE3, 0x39, 0x41, 0x76, 0xA5, 0x90, 0x13, 0x38, 0x10, 0xDA, 0xC5,
            0x0C, 0x84, 0x30, 0xF7, 0x27, 0xDB, 0x2E, 0x8A, 0xDF, 0x5A, 0x58, 0x96, 0x2C, 0x79,
            0x1F, 0x67, 0x6B, 0x76, 0xCB, 0xFB, 0xB5, 0xC9, 0x2C, 0xD6, 0xAE, 0xAF, 0xC9, 0x71,
            0x0E, 0x93, 0x8F, 0xF5, 0x6E, 0xD3, 0xFE, 0x03, 0x2F, 0x3E, 0x1C, 0x7D, 0xB8, 0x02,
            0x79, 0xF6, 0xF1, 0x90, 0x7C, 0x7C, 0x85, 0xF9, 0x08, 0xFB, 0x33, 0xC8, 0x57, 0x70,
            0xBF, 0x82, 0x7C, 0xA0, 0x8F, 0xC5, 0xC7, 0x4B, 0x00, 0x7E, 0x50, 0x07, 0xEA, 0x33,
            0xCC, 0x57, 0xBA, 0x3E, 0xD8, 0x2F, 0x54, 0x2D, 0xD8, 0x9F, 0x80, 0xBF, 0x52, 0xFA,
            0x54, 0x4E, 0xEA, 0x36, 0xED, 0x07, 0xFE, 0x04, 0xFE, 0x15, 0xE2, 0x81, 0xF3, 0x07,
            0x68, 0x8F, 0x02, 0x7F, 0x02, 0xF9, 0x01, 0xE2, 0x51, 0x7A, 0x2E, 0x7A, 0x1E, 0xF0,
            0xB7, 0xC0, 0xBC, 0x42, 0xFC, 0xF9, 0x0F, 0xFE, 0x00, 0x7D, 0x09, 0xA0, 0xBD,
        ];
        assert_eq!((DYNAMIC[2] >> 1) & 3, 2);
        let text: Vec<u8> = (0..40u32)
            .flat_map(|i| format!("Stellar PNG {} alpha beta gamma\n", i * i % 97).into_bytes())
            .collect();
        assert_eq!(decompress(DYNAMIC, 4096).unwrap(), text);
    }

    #[test]
    fn rejects_malformed_and_oversized_input() {
        let packed = compress(&vec![7u8; 10_000], &mut always()).unwrap();
        assert_eq!(decompress(&packed, 100), Err(InflateError::TooLarge));
        assert_eq!(
            decompress(&packed[..packed.len() - 6], 20_000),
            Err(InflateError::Truncated)
        );
        let mut bad_sum = packed.clone();
        let last = bad_sum.len() - 1;
        bad_sum[last] ^= 1;
        assert_eq!(decompress(&bad_sum, 20_000), Err(InflateError::Checksum));
        assert_eq!(decompress(&[0x78, 0x00], 10), Err(InflateError::BadHeader));
        assert_eq!(decompress(&[], 10), Err(InflateError::Truncated));
        // 임의 bytes는 panic 없이 오류입니다.
        let mut seed = 99u32;
        for _ in 0..200 {
            let mut noise = vec![0x78, 0x9C];
            for _ in 0..64 {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                noise.push((seed >> 16) as u8);
            }
            let _ = decompress(&noise, 4096);
        }
    }
}
