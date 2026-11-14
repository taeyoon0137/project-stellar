//! 폰트 레코드 (X.8.1).
//!
//! - [`FontSource`]: 폰트 파일 출처(Project, System, External). 숫자 tag는 저장 포맷의
//!   일부이므로 한 번 정한 뒤 바꾸지 않습니다.
//! - [`FontFileHash`]: 폰트 파일 콘텐츠 해시(SHA-256, ADR-0004). `text` crate는 `storage`에
//!   의존할 수 없으므로 hash를 직접 계산하지 않고, 폰트 import·discovery 계층이 계산한
//!   32바이트 digest를 받습니다. 문자열 형식은 저장 `ContentHash`와 같은
//!   `sha256:<64자리 소문자 16진수>`입니다.
//! - [`FontFaceRef`]: 파일 hash + face index + variation axes. 같은 파일 안의 face와 variable
//!   font instance를 정확히 가리킵니다.
//!
//! canonical binary(저장 primitive 규칙 R1~R5: little-endian, 고정 폭 정수, 정렬, 유한 float):
//!
//! ```text
//! u8    source tag (1=Project, 2=System, 3=External)
//! [32]  file hash SHA-256 digest
//! u32   face index
//! u32   variation axis 개수
//! { [4] axis tag, f32 value } × 개수   — tag 오름차순, tag당 하나
//! ```
//!
//! decode는 canonical이 아닌 입력(정렬 위반·중복 tag·비유한 값·음의 0(`-0.0`)·알 수 없는 tag·
//! 잔여 bytes)을 거부하므로, 같은 값은 항상 같은 bytes 하나로만 표현됩니다. 생성자는 `-0.0`을
//! `+0.0`으로 정규화하므로 canonical bytes에는 `-0.0`이 나타나지 않습니다.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use crate::value::FiniteF32;

/// OpenType 4글자 tag(예: `wght`, `liga`). 출력 가능한 ASCII(0x20..=0x7E)만 허용합니다.
///
/// 문서 모델의 style run tag(VI.7.2)와 같은 규칙입니다. `text` crate는 `document`에 의존할 수
/// 없으므로 같은 규칙의 값 타입을 따로 둡니다.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OpenTypeTag([u8; 4]);

impl OpenTypeTag {
    /// 4바이트 tag를 만듭니다. 비ASCII·제어 문자는 거부합니다.
    pub fn new(tag: [u8; 4]) -> Option<OpenTypeTag> {
        tag.iter()
            .all(|b| (0x20..=0x7E).contains(b))
            .then_some(OpenTypeTag(tag))
    }

    /// 정확히 4바이트 ASCII 문자열에서 tag를 만듭니다(예: `"wght"`).
    pub fn parse(tag: &str) -> Option<OpenTypeTag> {
        let bytes: [u8; 4] = tag.as_bytes().try_into().ok()?;
        OpenTypeTag::new(bytes)
    }

    /// 원본 bytes.
    pub const fn bytes(&self) -> [u8; 4] {
        self.0
    }
}

impl fmt::Display for OpenTypeTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 생성 시 출력 가능한 ASCII만 허용했으므로 문자 변환이 항상 가능합니다.
        for byte in self.0 {
            write!(f, "{}", char::from(byte))?;
        }
        Ok(())
    }
}

impl fmt::Debug for OpenTypeTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OpenTypeTag({self})")
    }
}

/// variable font axis 설정 하나(tag, 값).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontVariation {
    /// axis tag(예: `wght`).
    pub tag: OpenTypeTag,
    /// axis 값(유한 값).
    pub value: FiniteF32,
}

/// 폰트 파일 출처 (X.8.1, 아키텍처 11.3).
///
/// 숫자 tag는 저장 포맷 계약입니다. 새 출처는 새 tag로만 추가합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FontSource {
    /// 프로젝트 에셋으로 포함된 폰트(재현 가능).
    Project,
    /// 실행 환경에 설치된 시스템 폰트. 다른 환경에서 누락될 수 있습니다.
    System,
    /// 버전 고정 외부 패키지가 제공하는 폰트.
    External,
}

impl FontSource {
    /// 모든 출처(tag 오름차순).
    pub const ALL: [FontSource; 3] = [
        FontSource::Project,
        FontSource::System,
        FontSource::External,
    ];

    /// 안정 numeric tag.
    pub const fn tag(self) -> u8 {
        match self {
            FontSource::Project => 1,
            FontSource::System => 2,
            FontSource::External => 3,
        }
    }

    /// numeric tag에서 출처를 복원합니다. 알 수 없는 tag는 `None`.
    pub const fn from_tag(tag: u8) -> Option<FontSource> {
        match tag {
            1 => Some(FontSource::Project),
            2 => Some(FontSource::System),
            3 => Some(FontSource::External),
            _ => None,
        }
    }

    /// 진단·로그용 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            FontSource::Project => "project",
            FontSource::System => "system",
            FontSource::External => "external",
        }
    }
}

impl fmt::Display for FontSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 폰트 파일 hash 문자열 파싱 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseFontFileHashError {
    /// `<algorithm>:<hex>` 형태가 아닙니다.
    MissingSeparator,
    /// `sha256` 이외의 알고리즘 이름입니다.
    UnknownAlgorithm {
        /// 입력에 있던 알고리즘 이름.
        name: String,
    },
    /// hex 길이가 64가 아닙니다.
    InvalidLength {
        /// 실제 hex 길이(byte).
        actual: usize,
    },
    /// 소문자 16진수가 아닌 문자가 있습니다.
    InvalidHexDigit {
        /// 위반 문자.
        found: char,
    },
}

impl fmt::Display for ParseFontFileHashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseFontFileHashError::MissingSeparator => {
                f.write_str("폰트 파일 hash는 '<algorithm>:<hex>' 형태여야 합니다")
            }
            ParseFontFileHashError::UnknownAlgorithm { name } => {
                write!(f, "폰트 파일 hash 알고리즘이 sha256이 아닙니다: {name}")
            }
            ParseFontFileHashError::InvalidLength { actual } => {
                write!(
                    f,
                    "폰트 파일 hash hex 길이가 잘못되었습니다: 기대 64, 실제 {actual}"
                )
            }
            ParseFontFileHashError::InvalidHexDigit { found } => {
                write!(f, "소문자 16진수가 아닌 문자입니다: {found:?}")
            }
        }
    }
}

impl std::error::Error for ParseFontFileHashError {}

/// 폰트 파일 콘텐츠 해시(SHA-256 digest, ADR-0004).
///
/// 값은 호출자가 폰트 파일 bytes 전체로 계산한 SHA-256 digest입니다. 이 타입은 계산하지 않고
/// 보관·비교·직렬화만 합니다. 문자열 형식은 `sha256:<64자리 소문자 16진수>`(71자)입니다.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontFileHash([u8; 32]);

impl FontFileHash {
    /// 문자열 형식의 알고리즘 prefix.
    pub const ALGORITHM: &'static str = "sha256";

    /// 호출자가 계산한 SHA-256 digest로부터 만듭니다.
    pub const fn from_sha256_digest(digest: [u8; 32]) -> FontFileHash {
        FontFileHash(digest)
    }

    /// SHA-256 digest bytes.
    pub const fn sha256_digest(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for FontFileHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(FontFileHash::ALGORITHM)?;
        f.write_str(":")?;
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for FontFileHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FontFileHash({self})")
    }
}

impl FromStr for FontFileHash {
    type Err = ParseFontFileHashError;

    fn from_str(s: &str) -> Result<FontFileHash, ParseFontFileHashError> {
        let (name, hex) = s
            .split_once(':')
            .ok_or(ParseFontFileHashError::MissingSeparator)?;
        if name != FontFileHash::ALGORITHM {
            return Err(ParseFontFileHashError::UnknownAlgorithm {
                name: name.to_string(),
            });
        }
        if let Some(found) = hex.chars().find(|c| !matches!(c, '0'..='9' | 'a'..='f')) {
            return Err(ParseFontFileHashError::InvalidHexDigit { found });
        }
        // 이 시점에서 hex는 ASCII만 포함하므로 byte 길이 = 문자 수.
        if hex.len() != 64 {
            return Err(ParseFontFileHashError::InvalidLength { actual: hex.len() });
        }
        let mut digest = [0u8; 32];
        for (slot, pair) in digest.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
            *slot = (hex_value(pair[0]) << 4) | hex_value(pair[1]);
        }
        Ok(FontFileHash(digest))
    }
}

/// 소문자 16진수 ASCII 한 글자의 값(호출 전에 검증됨).
fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        _ => byte - b'a' + 10,
    }
}

/// 폰트 레코드 binary decode 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontRecordError {
    /// 입력이 끝나기 전에 필드가 끊겼습니다.
    Truncated,
    /// 레코드 뒤에 남는 bytes가 있습니다.
    TrailingBytes {
        /// 남은 byte 수.
        remaining: usize,
    },
    /// 알 수 없는 font source tag입니다.
    UnknownSourceTag {
        /// 입력의 tag.
        tag: u8,
    },
    /// axis tag가 출력 가능한 ASCII가 아닙니다.
    InvalidAxisTag {
        /// 입력의 tag bytes.
        bytes: [u8; 4],
    },
    /// axis 값이 유한하지 않습니다(NaN 또는 ±무한대).
    NonFiniteAxisValue {
        /// 문제 axis.
        tag: OpenTypeTag,
    },
    /// axis 값이 canonical 형식이 아닙니다(`-0.0`). canonical bytes는 `+0.0`만 씁니다.
    NonCanonicalAxisValue {
        /// 문제 axis.
        tag: OpenTypeTag,
    },
    /// variation axis가 tag 오름차순·중복 없음 규칙을 어깁니다.
    NonCanonicalVariations,
}

impl fmt::Display for FontRecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FontRecordError::Truncated => f.write_str("폰트 레코드 입력이 부족합니다"),
            FontRecordError::TrailingBytes { remaining } => {
                write!(f, "폰트 레코드 뒤에 {remaining} bytes가 남았습니다")
            }
            FontRecordError::UnknownSourceTag { tag } => {
                write!(f, "알 수 없는 font source tag입니다: {tag}")
            }
            FontRecordError::InvalidAxisTag { bytes } => {
                write!(
                    f,
                    "variation axis tag가 출력 가능한 ASCII가 아닙니다: {bytes:02x?}"
                )
            }
            FontRecordError::NonFiniteAxisValue { tag } => {
                write!(f, "variation axis {tag} 값이 유한하지 않습니다")
            }
            FontRecordError::NonCanonicalAxisValue { tag } => {
                write!(
                    f,
                    "variation axis {tag} 값이 canonical 형식이 아닙니다(-0.0은 +0.0으로 기록)"
                )
            }
            FontRecordError::NonCanonicalVariations => f.write_str(
                "variation axis가 tag 오름차순이 아니거나 같은 tag가 두 번 이상 나옵니다",
            ),
        }
    }
}

impl std::error::Error for FontRecordError {}

/// 특정 폰트 파일의 face 하나와 variable font instance를 가리키는 참조 (X.8.1).
///
/// canonical 형식: variation axis는 tag 오름차순, tag당 하나입니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontFaceRef {
    source: FontSource,
    file_hash: FontFileHash,
    face_index: u32,
    variations: Vec<FontVariation>,
}

impl FontFaceRef {
    /// canonical 참조를 만듭니다. variation은 tag 오름차순으로 정렬하고, 같은 tag가 여러 번
    /// 나오면 마지막 값이 우선합니다(VI.7.2 style run 규칙과 동일).
    pub fn new(
        source: FontSource,
        file_hash: FontFileHash,
        face_index: u32,
        variations: Vec<FontVariation>,
    ) -> FontFaceRef {
        let mut map = BTreeMap::new();
        for variation in variations {
            map.insert(variation.tag, variation.value);
        }
        FontFaceRef {
            source,
            file_hash,
            face_index,
            variations: map
                .into_iter()
                .map(|(tag, value)| FontVariation { tag, value })
                .collect(),
        }
    }

    /// 폰트 파일 출처.
    pub fn source(&self) -> FontSource {
        self.source
    }

    /// 폰트 파일 콘텐츠 해시.
    pub fn file_hash(&self) -> FontFileHash {
        self.file_hash
    }

    /// 파일 안의 face index(collection이 아니면 0).
    pub fn face_index(&self) -> u32 {
        self.face_index
    }

    /// variation axis 설정(tag 오름차순).
    pub fn variations(&self) -> &[FontVariation] {
        &self.variations
    }

    /// canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(41 + self.variations.len() * 8);
        self.encode_into(&mut buf);
        buf
    }

    /// canonical bytes에서 복원합니다. canonical이 아닌 입력은 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<FontFaceRef, FontRecordError> {
        let mut reader = ByteReader::new(bytes);
        let face = FontFaceRef::decode_from(&mut reader)?;
        reader.finish()?;
        Ok(face)
    }

    /// `buf` 끝에 canonical bytes를 덧붙입니다(상위 레코드 인코딩용).
    pub fn encode_into(&self, buf: &mut Vec<u8>) {
        buf.push(self.source.tag());
        buf.extend_from_slice(self.file_hash.sha256_digest());
        buf.extend_from_slice(&self.face_index.to_le_bytes());
        // variation 개수는 생성 경로상 u32 범위를 넘을 수 없습니다(axis tag가 고유하므로
        // 최대 95^4 < u32::MAX).
        buf.extend_from_slice(&(self.variations.len() as u32).to_le_bytes());
        for variation in &self.variations {
            buf.extend_from_slice(&variation.tag.bytes());
            buf.extend_from_slice(&variation.value.to_le_bytes());
        }
    }

    /// reader에서 레코드 하나를 읽습니다(상위 레코드 디코딩용).
    pub fn decode_from(reader: &mut ByteReader<'_>) -> Result<FontFaceRef, FontRecordError> {
        let tag = reader.take_u8()?;
        let source = FontSource::from_tag(tag).ok_or(FontRecordError::UnknownSourceTag { tag })?;
        let file_hash = FontFileHash::from_sha256_digest(reader.take_array::<32>()?);
        let face_index = reader.take_u32()?;
        let count = reader.take_u32()? as usize;
        // 악성 개수로 큰 할당을 만들지 않도록 남은 입력으로 가능한 개수까지만 예약합니다.
        let mut variations = Vec::with_capacity(count.min(reader.remaining() / 8));
        let mut previous: Option<OpenTypeTag> = None;
        for _ in 0..count {
            let bytes = reader.take_array::<4>()?;
            let tag = OpenTypeTag::new(bytes).ok_or(FontRecordError::InvalidAxisTag { bytes })?;
            let value_bytes = reader.take_array::<4>()?;
            // `FiniteF32::from_le_bytes`는 -0.0을 +0.0으로 정규화해 받아들이므로, 같은 값의 두 번째
            // bytes 형태가 생기지 않도록 여기서 먼저 거부합니다.
            if u32::from_le_bytes(value_bytes) == NEGATIVE_ZERO_BITS {
                return Err(FontRecordError::NonCanonicalAxisValue { tag });
            }
            let value = FiniteF32::from_le_bytes(value_bytes)
                .map_err(|_| FontRecordError::NonFiniteAxisValue { tag })?;
            if previous.is_some_and(|prev| prev >= tag) {
                return Err(FontRecordError::NonCanonicalVariations);
            }
            previous = Some(tag);
            variations.push(FontVariation { tag, value });
        }
        Ok(FontFaceRef {
            source,
            file_hash,
            face_index,
            variations,
        })
    }
}

/// `-0.0f32`의 bit 패턴. canonical bytes에는 나타나지 않습니다.
const NEGATIVE_ZERO_BITS: u32 = 0x8000_0000;

/// 공유 font/typography canonical record를 앞에서부터 읽는 bounded reader.
pub struct ByteReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    /// 입력 전체를 읽는 reader를 만듭니다.
    pub fn new(bytes: &'a [u8]) -> ByteReader<'a> {
        ByteReader { bytes, pos: 0 }
    }

    /// 아직 읽지 않은 byte 수.
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    /// 길이가 확인된 byte slice를 소비합니다.
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], FontRecordError> {
        if self.remaining() < n {
            return Err(FontRecordError::Truncated);
        }
        let slice = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    /// 고정 길이 배열을 읽습니다.
    pub fn take_array<const N: usize>(&mut self) -> Result<[u8; N], FontRecordError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    /// `u8` 하나를 읽습니다.
    pub fn take_u8(&mut self) -> Result<u8, FontRecordError> {
        Ok(self.take_array::<1>()?[0])
    }

    /// little-endian `u32`를 읽습니다.
    pub fn take_u32(&mut self) -> Result<u32, FontRecordError> {
        Ok(u32::from_le_bytes(self.take_array::<4>()?))
    }

    /// 입력을 모두 읽었는지 확인합니다.
    pub fn finish(&self) -> Result<(), FontRecordError> {
        match self.remaining() {
            0 => Ok(()),
            remaining => Err(FontRecordError::TrailingBytes { remaining }),
        }
    }
}
