//! fixture content hash.
//!
//! fixture 하나의 정체는 `(generator version, fixture 종류, seed, 규모 매개변수)`이고, 같은
//! 정체는 언제나 같은 내용을 만들어야 합니다(XVI.3.1 "같은 seed content hash 동일"). 내용은
//! **실제 저장 경로의 결과물**로 판정합니다: 문서는 `stellar_project::document_io`가 만든 chunk
//! model(논리 ChunkId·kind·schema version·canonical payload)을, 그 밖의 resource는 각 타입의
//! canonical 인코딩을 넣습니다.
//!
//! hash 입력은 길이 프리픽스 record의 나열이라 경계가 모호하지 않습니다. payload 자체 대신
//! payload의 content hash(ADR-0004 SHA-256)를 넣어, 수백 MB 규모 fixture도 작은 버퍼로
//! 누적합니다.

use stellar_core_types::ids::StableId;
use stellar_project::document_io::serialize::SerializedDocument;
use stellar_storage::hash::ContentHash;

use crate::GENERATOR_VERSION;

/// hash 입력 domain 분리 prefix.
const DIGEST_DOMAIN: &[u8] = b"stellar/bench-fixture/v1\0";

/// record tag. 숫자 값은 hash 입력 형식의 일부입니다.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
enum Tag {
    Parameter = 1,
    Document = 2,
    Chunk = 3,
    Resource = 4,
}

/// fixture content hash 누적기.
#[derive(Debug, Clone)]
pub struct FixtureDigest {
    buf: Vec<u8>,
    chunks: u64,
    payload_bytes: u64,
}

impl FixtureDigest {
    /// fixture 종류 이름과 seed로 시작합니다. generator version이 함께 들어갑니다.
    pub fn new(fixture: &str, seed: u64) -> FixtureDigest {
        let mut buf = Vec::with_capacity(4096);
        buf.extend_from_slice(DIGEST_DOMAIN);
        buf.extend_from_slice(&GENERATOR_VERSION.to_le_bytes());
        put_bytes(&mut buf, fixture.as_bytes());
        buf.extend_from_slice(&seed.to_le_bytes());
        FixtureDigest {
            buf,
            chunks: 0,
            payload_bytes: 0,
        }
    }

    /// 규모 매개변수(이름, 값)를 넣습니다. 같은 seed라도 매개변수가 다르면 hash가 다릅니다.
    pub fn parameter(&mut self, name: &str, value: u64) {
        self.buf.push(Tag::Parameter as u8);
        put_bytes(&mut self.buf, name.as_bytes());
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// 직렬화된 문서 하나(manifest → ViewTree → subtree 순의 모든 chunk)를 넣습니다.
    pub fn document(&mut self, document: &SerializedDocument) {
        let chunks = document.chunks();
        self.buf.push(Tag::Document as u8);
        self.buf.extend_from_slice(&document.document_id.to_bytes());
        self.buf
            .extend_from_slice(&(chunks.len() as u64).to_le_bytes());
        for chunk in chunks {
            self.buf.push(Tag::Chunk as u8);
            self.buf.extend_from_slice(&chunk.chunk_id.to_bytes());
            self.buf.extend_from_slice(&chunk.kind.0.to_le_bytes());
            self.buf
                .extend_from_slice(&chunk.schema_version.to_le_bytes());
            self.payload(&chunk.payload);
        }
    }

    /// 문서 밖 resource(컴포넌트 library, scene column 등)의 canonical bytes를 넣습니다.
    pub fn resource(&mut self, label: &str, owner: StableId, bytes: &[u8]) {
        self.buf.push(Tag::Resource as u8);
        put_bytes(&mut self.buf, label.as_bytes());
        self.buf.extend_from_slice(&owner.to_bytes());
        self.payload(bytes);
    }

    fn payload(&mut self, bytes: &[u8]) {
        self.buf
            .extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        self.buf
            .extend_from_slice(ContentHash::compute(bytes).digest());
        self.chunks += 1;
        self.payload_bytes += bytes.len() as u64;
    }

    /// 지금까지 넣은 chunk·resource 수.
    pub fn chunk_count(&self) -> u64 {
        self.chunks
    }

    /// 지금까지 넣은 payload bytes 합.
    pub fn payload_bytes(&self) -> u64 {
        self.payload_bytes
    }

    /// 최종 content hash.
    pub fn finish(self) -> ContentHash {
        ContentHash::compute(&self.buf)
    }
}

fn put_bytes(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    buf.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_separates_identity_fields() {
        let base = |fixture: &str, seed: u64| FixtureDigest::new(fixture, seed).finish();
        assert_eq!(base("a", 1), base("a", 1));
        assert_ne!(base("a", 1), base("a", 2));
        assert_ne!(base("a", 1), base("b", 1));

        let mut x = FixtureDigest::new("a", 1);
        x.parameter("nodes", 10);
        let mut y = FixtureDigest::new("a", 1);
        y.parameter("nodes", 11);
        assert_ne!(x.finish(), y.finish());

        // 길이 프리픽스로 경계가 모호하지 않습니다.
        let mut p = FixtureDigest::new("a", 1);
        p.resource("ab", StableId::NIL, b"c");
        let mut q = FixtureDigest::new("a", 1);
        q.resource("a", StableId::NIL, b"bc");
        assert_eq!(p.chunk_count(), 1);
        assert_eq!(p.payload_bytes(), 1);
        assert_ne!(p.finish(), q.finish());
    }
}
