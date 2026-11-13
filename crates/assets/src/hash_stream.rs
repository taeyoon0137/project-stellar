//! streaming content hash (XIII.5.2).
//!
//! 대형 에셋을 메모리에 모두 올리지 않고 고정 크기 buffer로 나눠 읽으며 ADR-0004
//! content hash(SHA-256)를 계산합니다. storage의 [`ContentHash::compute`]는 입력 전체
//! slice만 받고 SHA-256 구현이 crate 내부(private)라서, 같은 FIPS 180-4 알고리즘의
//! **증분(incremental)** 버전을 여기 둡니다. 결과는 모든 분할 방식에서
//! `ContentHash::compute`와 같아야 하며 테스트로 고정합니다.

use std::io::{ErrorKind, Read};

use stellar_storage::hash::{ContentHash, ParseContentHashError};

/// import streaming buffer 크기(byte). 입력 크기와 무관하게 읽기 buffer는 이 크기로 고정됩니다.
pub const STREAM_BUFFER_SIZE: usize = 64 * 1024;

/// SHA-256 초기 hash 값(H0~H7, FIPS 180-4 §5.3.3).
const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// SHA-256 라운드 상수(K0~K63, FIPS 180-4 §4.2.2).
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// 512bit block 하나를 압축합니다(FIPS 180-4 §6.2.2).
fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for (i, word) in block.chunks_exact(4).enumerate() {
        w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let big_s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ (!e & g);
        let temp1 = h
            .wrapping_add(big_s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let big_s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = big_s0.wrapping_add(maj);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *slot = slot.wrapping_add(value);
    }
}

/// 증분 SHA-256 content hasher.
///
/// 내부 상태는 고정 크기(상태 32 bytes + 미완성 block 64 bytes)이며 입력 길이에 따라
/// 커지지 않습니다.
#[derive(Debug, Clone)]
pub struct StreamingHasher {
    state: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    total_len: u64,
}

impl Default for StreamingHasher {
    fn default() -> StreamingHasher {
        StreamingHasher::new()
    }
}

impl StreamingHasher {
    /// 빈 입력 상태로 시작합니다.
    pub fn new() -> StreamingHasher {
        StreamingHasher {
            state: H0,
            block: [0u8; 64],
            block_len: 0,
            total_len: 0,
        }
    }

    /// 지금까지 입력한 byte 수.
    pub fn bytes_hashed(&self) -> u64 {
        self.total_len
    }

    /// 입력을 이어 붙입니다. 분할 방식과 무관하게 같은 결과를 냅니다.
    pub fn update(&mut self, mut data: &[u8]) {
        self.total_len = self.total_len.wrapping_add(data.len() as u64);
        if self.block_len > 0 {
            let take = (64 - self.block_len).min(data.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&data[..take]);
            self.block_len += take;
            data = &data[take..];
            if self.block_len < 64 {
                return;
            }
            let block = self.block;
            compress(&mut self.state, &block);
            self.block_len = 0;
        }
        let mut blocks = data.chunks_exact(64);
        for block in blocks.by_ref() {
            let mut full = [0u8; 64];
            full.copy_from_slice(block);
            compress(&mut self.state, &full);
        }
        let rest = blocks.remainder();
        self.block[..rest.len()].copy_from_slice(rest);
        self.block_len = rest.len();
    }

    /// 패딩 후 32바이트 digest를 반환합니다(FIPS 180-4 §5.1.1).
    pub fn finish_digest(mut self) -> [u8; 32] {
        let bit_len = self.total_len.wrapping_mul(8);
        let mut last = [0u8; 128];
        last[..self.block_len].copy_from_slice(&self.block[..self.block_len]);
        last[self.block_len] = 0x80;
        let tail = if self.block_len < 56 { 64 } else { 128 };
        last[tail - 8..tail].copy_from_slice(&bit_len.to_be_bytes());
        for chunk in last[..tail].chunks_exact(64) {
            let mut block = [0u8; 64];
            block.copy_from_slice(chunk);
            compress(&mut self.state, &block);
        }
        let mut digest = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            digest[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        digest
    }

    /// ADR-0004 [`ContentHash`]로 마무리합니다.
    ///
    /// storage는 digest로부터 직접 만드는 생성자를 공개하지 않아 정규 문자열
    /// (`sha256:<hex>`)을 parse합니다. 64자리 소문자 hex라 실패하지 않지만, 프로덕션
    /// 경로에 panic을 두지 않기 위해(panic 정책 II.1.3) `Result`로 반환합니다.
    pub fn finish(self) -> Result<ContentHash, ParseContentHashError> {
        let hex: String = self
            .finish_digest()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        format!("sha256:{hex}").parse()
    }
}

/// SHA-256 block 크기(byte). HMAC key padding 단위입니다.
const SHA256_BLOCK_LEN: usize = 64;

/// HMAC-SHA-256(RFC 2104, FIPS 198-1)을 위의 SHA-256으로 계산합니다.
///
/// block 크기(64 bytes)보다 긴 key는 먼저 SHA-256으로 줄입니다. 외부 원본 출처 지문
/// ([`crate::origin`])의 keyed hash로 쓰며, RFC 4231 test vector로 고정합니다.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block_key = [0u8; SHA256_BLOCK_LEN];
    if key.len() > SHA256_BLOCK_LEN {
        let mut hasher = StreamingHasher::new();
        hasher.update(key);
        block_key[..32].copy_from_slice(&hasher.finish_digest());
    } else {
        block_key[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36u8; SHA256_BLOCK_LEN];
    let mut outer_pad = [0x5cu8; SHA256_BLOCK_LEN];
    for ((inner, outer), k) in inner_pad
        .iter_mut()
        .zip(outer_pad.iter_mut())
        .zip(block_key)
    {
        *inner ^= k;
        *outer ^= k;
    }
    let mut inner = StreamingHasher::new();
    inner.update(&inner_pad);
    inner.update(message);
    let inner_digest = inner.finish_digest();
    let mut outer = StreamingHasher::new();
    outer.update(&outer_pad);
    outer.update(&inner_digest);
    outer.finish_digest()
}

/// streaming 읽기 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamDigest {
    /// content hash.
    pub hash: ContentHash,
    /// 읽은 byte 수.
    pub size: u64,
}

/// streaming hash 실패.
#[derive(Debug)]
pub enum StreamHashError {
    /// 읽기 실패.
    Io(std::io::Error),
    /// digest를 ContentHash로 만들 수 없습니다(발생하지 않아야 함).
    Hash(ParseContentHashError),
}

/// `reader`를 끝까지 `buffer` 크기씩 읽으며 hash와 크기를 계산합니다.
///
/// `ErrorKind::Interrupted`는 재시도합니다. 읽은 bytes는 `on_chunk`에도 전달됩니다(복사 등).
pub fn hash_reader_with<R: Read>(
    reader: &mut R,
    buffer: &mut [u8],
    mut on_chunk: impl FnMut(&[u8]) -> std::io::Result<()>,
) -> Result<StreamDigest, StreamHashError> {
    let mut hasher = StreamingHasher::new();
    loop {
        let n = match reader.read(buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(StreamHashError::Io(e)),
        };
        hasher.update(&buffer[..n]);
        on_chunk(&buffer[..n]).map_err(StreamHashError::Io)?;
    }
    let size = hasher.bytes_hashed();
    let hash = hasher.finish().map_err(StreamHashError::Hash)?;
    Ok(StreamDigest { hash, size })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn streamed(input: &[u8], split: usize) -> ContentHash {
        let mut hasher = StreamingHasher::new();
        for part in input.chunks(split.max(1)) {
            hasher.update(part);
        }
        assert_eq!(hasher.bytes_hashed(), input.len() as u64);
        hasher.finish().unwrap()
    }

    #[test]
    fn matches_known_vectors() {
        // FIPS 180-4 / NIST 예제.
        assert_eq!(
            streamed(b"", 1).to_string(),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            streamed(b"abc", 1).to_string(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            streamed(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
                7
            )
            .to_string(),
            "sha256:248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn every_length_and_split_matches_storage_compute() {
        // padding 경계(55·56·63·64·65·119·120·128)를 포함한 모든 길이 × 여러 분할.
        let data: Vec<u8> = (0..300u32).map(|i| (i * 31 + 7) as u8).collect();
        for len in 0..=data.len() {
            let input = &data[..len];
            let expected = ContentHash::compute(input);
            for split in [1, 3, 55, 63, 64, 65, 128, 1000] {
                assert_eq!(streamed(input, split), expected, "len {len} split {split}");
            }
        }
    }

    /// 요청된 buffer 크기와 총 읽기 바이트를 기록하는 합성 reader.
    struct SyntheticReader {
        remaining: u64,
        max_request: usize,
        interrupted_once: bool,
    }

    impl Read for SyntheticReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if !self.interrupted_once {
                self.interrupted_once = true;
                return Err(std::io::Error::from(ErrorKind::Interrupted));
            }
            self.max_request = self.max_request.max(buf.len());
            let n = (buf.len() as u64).min(self.remaining) as usize;
            for (i, byte) in buf[..n].iter_mut().enumerate() {
                *byte = ((self.remaining as usize + i) % 251) as u8;
            }
            self.remaining -= n as u64;
            Ok(n)
        }
    }

    #[test]
    fn reader_is_consumed_with_fixed_buffer() {
        let total = 3 * STREAM_BUFFER_SIZE as u64 + 17;
        let mut reader = SyntheticReader {
            remaining: total,
            max_request: 0,
            interrupted_once: false,
        };
        let mut buffer = vec![0u8; STREAM_BUFFER_SIZE];
        let mut copied: Vec<u8> = Vec::new();
        let digest = hash_reader_with(&mut reader, &mut buffer, |chunk| {
            copied.extend_from_slice(chunk);
            Ok(())
        })
        .unwrap();
        assert_eq!(digest.size, total);
        assert_eq!(reader.max_request, STREAM_BUFFER_SIZE, "buffer 크기 고정");
        assert_eq!(digest.hash, ContentHash::compute(&copied));
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn hmac_matches_rfc_4231_vectors() {
        let key4: Vec<u8> = (1u8..=25).collect();
        let cases: [(&[u8], &[u8], &str); 6] = [
            (
                &[0x0b; 20],
                b"Hi There",
                "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
            ),
            (
                b"Jefe",
                b"what do ya want for nothing?",
                "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
            ),
            (
                &[0xaa; 20],
                &[0xdd; 50],
                "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
            ),
            (
                &key4,
                &[0xcd; 50],
                "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b",
            ),
            // block보다 긴 key(131 bytes)는 먼저 hash합니다.
            (
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First",
                "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
            ),
            (
                &[0xaa; 131],
                b"This is a test using a larger than block-size key and a larger than \
                  block-size data. The key needs to be hashed before being used by the \
                  HMAC algorithm.",
                "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2",
            ),
        ];
        for (key, message, expected) in cases {
            assert_eq!(hex(&hmac_sha256(key, message)), expected);
        }
        // 정확히 block 크기인 key와 빈 message(경계).
        assert_eq!(
            hex(&hmac_sha256(&[b'k'; 64], b"")),
            "83026a325aaee70e36cfe607536aa1054104ad1077c36134810d4ccded1ccd3b"
        );
    }

    #[test]
    fn chunk_callback_error_stops_reading() {
        let mut reader: &[u8] = &[1, 2, 3];
        let mut buffer = [0u8; 2];
        let err = hash_reader_with(&mut reader, &mut buffer, |_| {
            Err(std::io::Error::other("중단"))
        })
        .unwrap_err();
        assert!(matches!(err, StreamHashError::Io(_)));
    }
}
