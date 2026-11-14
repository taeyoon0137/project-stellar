//! 폰트 파일 container probe와 face 이름 (XIII.6.1).
//!
//! 폰트 파일의 **header와 name table만** 읽어 container 형식, face 수, face별 이름을 얻습니다.
//! glyph·outline은 해석하지 않습니다(shaping·rasterization은 Phase X 텍스트 엔진 범위).
//!
//! # 지원 container
//!
//! | 형식 | signature | face 수 | 이름 |
//! |---|---|---|---|
//! | TrueType | `00 01 00 00`, `true` | 1 | name table |
//! | OpenType(CFF) | `OTTO` | 1 | name table |
//! | Collection | `ttcf` | header의 `numFonts` | face마다 name table |
//! | WOFF | `wOFF` | 1 | 읽지 않음(압축 table) |
//! | WOFF2 | `wOF2` | 1, flavor가 `ttcf`면 collection directory의 `numFonts` | 읽지 않음(Brotli) |
//!
//! # 신뢰하지 않는 입력
//!
//! 프로젝트 폰트는 외부 파일이므로 모든 offset·개수를 파일 길이와 상한으로 검사합니다.
//! - 모든 읽기는 `[offset, offset + len)`이 파일 길이 안인지 먼저 확인합니다([`FontParseError::Truncated`]).
//! - face 수는 [`MAX_FACES_PER_FILE`], sfnt table 수는 [`MAX_SFNT_TABLES`], name table 크기는
//!   [`MAX_NAME_TABLE_BYTES`]로 제한합니다. 할당 크기는 항상 이 상한과 파일 길이 이하입니다.
//! - 작업량도 입력 크기에 비례하게 묶습니다. 파일 하나에서 같은 name table 구간(offset, 길이)은 한
//!   번만 읽어 해석하고(collection이 같은 face·name table을 반복 가리켜도 재해석 없음), 서로 다른 구간을
//!   읽은 bytes 합계는 [`MAX_NAME_TABLE_BYTES_PER_FILE`]로 제한합니다. 넘으면 그 뒤 face의 이름만
//!   비웁니다.
//! - name record는 decode 전에 원본 길이로 거르고(아래 이름 선택 규칙), nameID 하나에서 decode를
//!   시도하는 후보는 순위 순 [`MAX_NAME_CANDIDATES_PER_ID`]개까지입니다. 같은 문자열 구간을 가리키는
//!   record가 아무리 많아도 name table 하나의 decode 작업량은 상수 상한(nameID 5종 × 64개 × 2,050 bytes
//!   = 656,000 bytes) 이하입니다. 이 상수는 읽은 bytes보다 약 100배 클 수 있으므로(약 6 KB table),
//!   파일 하나의 모든 name table에 걸친 decode 원본 bytes 합계도 [`MAX_NAME_DECODE_BYTES_PER_FILE`]로
//!   제한합니다. 넘는 후보는 decode하지 않고 제외합니다(face 순서대로 쓰므로 결정론적).
//! - 재귀 없이 반복문으로만 읽습니다.
//!
//! container 구조(collection offset, face signature, WOFF header) 오류는 파일 전체를 거부합니다.
//! name table은 렌더링에 필수가 아닌 metadata라서, 없거나 손상되었으면 그 face의 이름만 비우고
//! face는 유지합니다(결정론: 같은 bytes는 항상 같은 결과).
//!
//! # 임베딩 권한 선언 (XIII.6.6)
//!
//! OS/2 table의 `fsType`(offset 8, `u16`)을 [`EmbeddingPermissions`]로 읽습니다. 폰트 제작자가 파일에
//! 선언한 참고 정보이며, 라이선스 상태(`crate::fonts::license::FontLicenseState`)를 자동으로 바꾸지
//! 않습니다. OS/2 table이 없거나 10 bytes보다 짧거나 파일 밖이면 `None`입니다.
//!
//! # 이름 선택 규칙(결정론)
//!
//! family는 nameID 16(typographic family), 없으면 1, subfamily는 17, 없으면 2, PostScript 이름은
//! 6입니다. 같은 nameID의 record가 여럿이면 아래 순위의 첫 번째로 decode 가능한 값을 씁니다.
//! 1. Windows(3) Unicode BMP/full(1·10) English US(0x0409)
//! 2. Windows(3) Unicode BMP/full(1·10), language id 오름차순
//! 3. Unicode(0), encoding·language id 오름차순
//! 4. Macintosh(1) Roman(0) English(0) — ASCII인 경우만
//!
//! UTF-16BE가 깨졌거나, 제어 문자가 있거나, 비었거나, [`MAX_FACE_NAME_BYTES`]를 넘는 값은
//! 후보에서 제외합니다. 유효한 이름은 UTF-16 code unit 하나가 UTF-8 1 byte 이상이므로, 원본
//! bytes가 UTF-16이면 `2 × MAX_FACE_NAME_BYTES + 2`, Macintosh ASCII면 `MAX_FACE_NAME_BYTES + 1`
//! (끝 NUL 하나 허용)을 넘는 record는 decode하지 않고 제외합니다. 순위 순 앞
//! [`MAX_NAME_CANDIDATES_PER_ID`]개 후보가 모두 제외되면 그 이름은 없습니다.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{ErrorKind, Read, Seek, SeekFrom};

/// 파일 하나에 허용하는 최대 face 수.
pub const MAX_FACES_PER_FILE: u32 = 4096;

/// sfnt face 하나(또는 WOFF2 directory)에 허용하는 최대 table 수.
pub const MAX_SFNT_TABLES: u16 = 1024;

/// 읽는 name table 최대 크기(byte). 넘으면 이름을 읽지 않습니다.
pub const MAX_NAME_TABLE_BYTES: u32 = 1 << 20;

/// face 이름 하나의 최대 UTF-8 길이(byte).
pub const MAX_FACE_NAME_BYTES: usize = 1024;

/// 파일 하나에서 읽는 서로 다른 name table 구간의 bytes 합계 상한. 넘으면 그 뒤 face의 이름을 읽지
/// 않습니다(face 하나 상한 [`MAX_NAME_TABLE_BYTES`]의 16배).
pub const MAX_NAME_TABLE_BYTES_PER_FILE: u64 = 16 * MAX_NAME_TABLE_BYTES as u64;

/// nameID 하나에서 decode를 시도하는 후보 record 수 상한(순위 순).
pub const MAX_NAME_CANDIDATES_PER_ID: usize = 64;

/// 파일 하나에서 name record decode를 시도하는 원본 bytes 합계 상한. 넘는 후보는 decode하지 않고
/// 제외합니다(읽기 합계 상한 [`MAX_NAME_TABLE_BYTES_PER_FILE`]과 같은 값).
pub const MAX_NAME_DECODE_BYTES_PER_FILE: u64 = MAX_NAME_TABLE_BYTES_PER_FILE;

/// UTF-16BE name record 원본 bytes 상한(유효 이름 + 끝 NUL 하나).
const MAX_UTF16_NAME_RECORD_BYTES: usize = 2 * MAX_FACE_NAME_BYTES + 2;

/// Macintosh Roman(ASCII) name record 원본 bytes 상한(유효 이름 + 끝 NUL 하나).
const MAX_MAC_NAME_RECORD_BYTES: usize = MAX_FACE_NAME_BYTES + 1;

const SIG_TRUETYPE: [u8; 4] = [0x00, 0x01, 0x00, 0x00];
const SIG_TRUE: [u8; 4] = *b"true";
const SIG_OTTO: [u8; 4] = *b"OTTO";
const SIG_TTCF: [u8; 4] = *b"ttcf";
const SIG_WOFF: [u8; 4] = *b"wOFF";
const SIG_WOFF2: [u8; 4] = *b"wOF2";
const TAG_NAME: [u8; 4] = *b"name";
const TAG_OS2: [u8; 4] = *b"OS/2";
/// OS/2 table에서 `fsType`까지 읽는 데 필요한 최소 길이(version, xAvgCharWidth, usWeightClass,
/// usWidthClass, fsType).
const OS2_FS_TYPE_END: u32 = 10;
const TAG_GLYF: [u8; 4] = *b"glyf";
const TAG_LOCA: [u8; 4] = *b"loca";

/// sfnt offset table 크기(sfntVersion, numTables, searchRange, entrySelector, rangeShift).
const SFNT_HEADER_LEN: u64 = 12;
/// sfnt table record 크기(tag, checksum, offset, length).
const SFNT_TABLE_RECORD_LEN: u64 = 16;
/// TTC header 고정부(tag, major, minor, numFonts).
const TTC_HEADER_LEN: u64 = 12;
/// WOFF 1.0 header 크기.
const WOFF_HEADER_LEN: u64 = 44;
/// WOFF2 header 크기.
const WOFF2_HEADER_LEN: u64 = 48;

/// 폰트 파일 container 형식.
///
/// 숫자 tag는 저장 포맷 계약입니다([`crate::fonts::project_font`]). 새 형식은 새 tag로만 추가합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FontFileFormat {
    /// TrueType outline sfnt(`00010000`, `true`).
    TrueType,
    /// CFF outline OpenType(`OTTO`).
    OpenTypeCff,
    /// TrueType/OpenType collection(`ttcf`).
    Collection,
    /// WOFF 1.0.
    Woff,
    /// WOFF 2.0(collection 포함 가능).
    Woff2,
}

impl FontFileFormat {
    /// 모든 형식(tag 오름차순).
    pub const ALL: [FontFileFormat; 5] = [
        FontFileFormat::TrueType,
        FontFileFormat::OpenTypeCff,
        FontFileFormat::Collection,
        FontFileFormat::Woff,
        FontFileFormat::Woff2,
    ];

    /// 안정 numeric tag.
    pub const fn tag(self) -> u8 {
        match self {
            FontFileFormat::TrueType => 1,
            FontFileFormat::OpenTypeCff => 2,
            FontFileFormat::Collection => 3,
            FontFileFormat::Woff => 4,
            FontFileFormat::Woff2 => 5,
        }
    }

    /// numeric tag에서 형식을 복원합니다. 알 수 없는 tag는 `None`.
    pub const fn from_tag(tag: u8) -> Option<FontFileFormat> {
        match tag {
            1 => Some(FontFileFormat::TrueType),
            2 => Some(FontFileFormat::OpenTypeCff),
            3 => Some(FontFileFormat::Collection),
            4 => Some(FontFileFormat::Woff),
            5 => Some(FontFileFormat::Woff2),
            _ => None,
        }
    }

    /// 에셋 import에 쓰는 media type(`config/asset-import.yaml` 허용 목록과 같은 값).
    pub const fn media_type(self) -> &'static str {
        match self {
            FontFileFormat::TrueType => "font/ttf",
            FontFileFormat::OpenTypeCff => "font/otf",
            FontFileFormat::Collection => "font/collection",
            FontFileFormat::Woff => "font/woff",
            FontFileFormat::Woff2 => "font/woff2",
        }
    }

    /// 형식상 face가 정확히 하나인지 여부(collection 가능 형식은 `false`).
    pub const fn is_single_face(self) -> bool {
        matches!(
            self,
            FontFileFormat::TrueType | FontFileFormat::OpenTypeCff | FontFileFormat::Woff
        )
    }

    /// 진단·로그용 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            FontFileFormat::TrueType => "truetype",
            FontFileFormat::OpenTypeCff => "opentype-cff",
            FontFileFormat::Collection => "collection",
            FontFileFormat::Woff => "woff",
            FontFileFormat::Woff2 => "woff2",
        }
    }
}

impl fmt::Display for FontFileFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// face 이름 값 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaceNameError {
    /// 빈 문자열입니다. 이름이 없으면 `None`을 씁니다.
    Empty,
    /// [`MAX_FACE_NAME_BYTES`]를 넘습니다.
    TooLong {
        /// 실제 길이(byte).
        len: usize,
    },
    /// 제어 문자가 있습니다.
    ControlCharacter,
}

impl fmt::Display for FaceNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FaceNameError::Empty => f.write_str("face 이름이 비었습니다"),
            FaceNameError::TooLong { len } => write!(
                f,
                "face 이름이 너무 깁니다: {len} bytes > {MAX_FACE_NAME_BYTES} bytes"
            ),
            FaceNameError::ControlCharacter => f.write_str("face 이름에 제어 문자가 있습니다"),
        }
    }
}

impl std::error::Error for FaceNameError {}

/// face 이름 값 하나를 검사합니다.
pub(crate) fn validate_face_name(value: &str) -> Result<(), FaceNameError> {
    if value.is_empty() {
        return Err(FaceNameError::Empty);
    }
    if value.len() > MAX_FACE_NAME_BYTES {
        return Err(FaceNameError::TooLong { len: value.len() });
    }
    if value.chars().any(char::is_control) {
        return Err(FaceNameError::ControlCharacter);
    }
    Ok(())
}

/// face 하나의 이름(name table에서 읽은 값). 모든 값은 선택적입니다.
///
/// 값이 있으면 비어 있지 않고, 제어 문자가 없고, [`MAX_FACE_NAME_BYTES`] 이하입니다.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontFaceNames {
    family: Option<String>,
    subfamily: Option<String>,
    postscript: Option<String>,
}

impl FontFaceNames {
    /// 이름을 검사해 만듭니다.
    pub fn new(
        family: Option<String>,
        subfamily: Option<String>,
        postscript: Option<String>,
    ) -> Result<FontFaceNames, FaceNameError> {
        for value in [&family, &subfamily, &postscript].into_iter().flatten() {
            validate_face_name(value)?;
        }
        Ok(FontFaceNames {
            family,
            subfamily,
            postscript,
        })
    }

    /// family 이름(nameID 16, 없으면 1).
    pub fn family(&self) -> Option<&str> {
        self.family.as_deref()
    }

    /// subfamily 이름(nameID 17, 없으면 2).
    pub fn subfamily(&self) -> Option<&str> {
        self.subfamily.as_deref()
    }

    /// PostScript 이름(nameID 6).
    pub fn postscript(&self) -> Option<&str> {
        self.postscript.as_deref()
    }

    /// 모든 이름이 없으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.family.is_none() && self.subfamily.is_none() && self.postscript.is_none()
    }
}

/// OS/2 `fsType` 사용 권한 수준(OpenType spec, 가장 덜 제한적인 bit 우선).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EmbeddingLevel {
    /// 제한 없음(`0x0000`).
    Installable,
    /// 제한 라이선스 임베딩(`0x0002`).
    RestrictedLicense,
    /// 미리보기·인쇄만(`0x0004`).
    PreviewAndPrint,
    /// 편집 가능 임베딩(`0x0008`).
    Editable,
}

/// 폰트 파일이 선언한 임베딩 권한(OS/2 `fsType` 원본 bits). 참고 정보입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EmbeddingPermissions(u16);

impl EmbeddingPermissions {
    /// 원본 `fsType` 값으로 만듭니다.
    pub const fn from_fs_type(fs_type: u16) -> EmbeddingPermissions {
        EmbeddingPermissions(fs_type)
    }

    /// 원본 `fsType` 값.
    pub const fn fs_type(self) -> u16 {
        self.0
    }

    /// 사용 권한 수준. 여러 bit가 켜져 있으면 가장 덜 제한적인 수준입니다(OpenType spec).
    pub const fn level(self) -> EmbeddingLevel {
        if self.0 & 0x0008 != 0 {
            EmbeddingLevel::Editable
        } else if self.0 & 0x0004 != 0 {
            EmbeddingLevel::PreviewAndPrint
        } else if self.0 & 0x0002 != 0 {
            EmbeddingLevel::RestrictedLicense
        } else {
            EmbeddingLevel::Installable
        }
    }

    /// subsetting 금지(`0x0100`).
    pub const fn no_subsetting(self) -> bool {
        self.0 & 0x0100 != 0
    }

    /// bitmap 임베딩만 허용(`0x0200`).
    pub const fn bitmap_only(self) -> bool {
        self.0 & 0x0200 != 0
    }
}

/// face 하나의 probe 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct FontFaceInfo {
    /// face 이름(WOFF·WOFF2나 name table이 없으면 비어 있음).
    pub names: FontFaceNames,
    /// OS/2 `fsType` 임베딩 권한 선언(없거나 WOFF·WOFF2면 `None`). 참고 정보입니다(XIII.6.6).
    pub embedding: Option<EmbeddingPermissions>,
}

/// 폰트 파일 probe 결과.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FontFileInfo {
    /// container 형식.
    pub format: FontFileFormat,
    /// face 목록(index = face index). 항상 1개 이상 [`MAX_FACES_PER_FILE`]개 이하입니다.
    pub faces: Vec<FontFaceInfo>,
}

impl FontFileInfo {
    /// face 수.
    pub fn face_count(&self) -> u32 {
        // 생성 경로상 MAX_FACES_PER_FILE 이하입니다.
        u32::try_from(self.faces.len()).unwrap_or(u32::MAX)
    }
}

/// 폰트 파일 probe 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontParseError {
    /// 읽기 실패.
    Io {
        /// I/O 오류 종류.
        kind: ErrorKind,
        /// 원인 메시지.
        message: String,
    },
    /// 필요한 구간이 파일 끝을 넘습니다.
    Truncated {
        /// 읽으려던 offset.
        offset: u64,
        /// 읽으려던 길이.
        len: u64,
    },
    /// 알려진 폰트 signature가 아닙니다.
    UnknownSignature {
        /// 파일 첫 4 bytes.
        signature: [u8; 4],
    },
    /// collection version이 지원 범위(1.0, 2.0)가 아닙니다.
    UnsupportedCollectionVersion {
        /// major version.
        major: u16,
        /// minor version.
        minor: u16,
    },
    /// face가 하나도 없습니다.
    NoFaces,
    /// face 수가 [`MAX_FACES_PER_FILE`]를 넘습니다.
    TooManyFaces {
        /// 선언된 face 수.
        count: u32,
    },
    /// table 수가 [`MAX_SFNT_TABLES`]를 넘습니다.
    TooManyTables {
        /// 선언된 table 수.
        count: u16,
    },
    /// collection의 face가 sfnt signature로 시작하지 않습니다.
    InvalidFaceSignature {
        /// face index.
        index: u32,
        /// 그 위치의 4 bytes.
        signature: [u8; 4],
    },
    /// WOFF·WOFF2 header가 형식 규칙을 어깁니다.
    InvalidWoffHeader {
        /// 위반 규칙.
        reason: &'static str,
    },
    /// WOFF2 table·collection directory가 형식 규칙을 어깁니다.
    InvalidWoff2Directory {
        /// 위반 규칙.
        reason: &'static str,
    },
}

impl FontParseError {
    fn io(e: &std::io::Error) -> FontParseError {
        FontParseError::Io {
            kind: e.kind(),
            message: e.to_string(),
        }
    }
}

impl fmt::Display for FontParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FontParseError::Io { message, .. } => write!(f, "폰트 파일 읽기 실패: {message}"),
            FontParseError::Truncated { offset, len } => write!(
                f,
                "폰트 파일이 잘렸습니다: offset {offset}에서 {len} bytes를 읽을 수 없습니다"
            ),
            FontParseError::UnknownSignature { signature } => {
                write!(f, "알 수 없는 폰트 signature입니다: {signature:02x?}")
            }
            FontParseError::UnsupportedCollectionVersion { major, minor } => {
                write!(f, "지원하지 않는 collection version입니다: {major}.{minor}")
            }
            FontParseError::NoFaces => f.write_str("폰트 파일에 face가 없습니다"),
            FontParseError::TooManyFaces { count } => write!(
                f,
                "폰트 파일의 face 수가 상한을 넘습니다: {count} > {MAX_FACES_PER_FILE}"
            ),
            FontParseError::TooManyTables { count } => write!(
                f,
                "폰트 table 수가 상한을 넘습니다: {count} > {MAX_SFNT_TABLES}"
            ),
            FontParseError::InvalidFaceSignature { index, signature } => write!(
                f,
                "collection face {index}의 signature가 sfnt가 아닙니다: {signature:02x?}"
            ),
            FontParseError::InvalidWoffHeader { reason } => {
                write!(f, "WOFF header가 잘못되었습니다: {reason}")
            }
            FontParseError::InvalidWoff2Directory { reason } => {
                write!(f, "WOFF2 directory가 잘못되었습니다: {reason}")
            }
        }
    }
}

impl std::error::Error for FontParseError {}

/// `reader`(길이 `file_len`)의 폰트 container를 probe합니다.
///
/// header와 name table만 읽습니다. reader 위치는 바뀝니다.
pub fn read_font_file<R: Read + Seek>(
    reader: &mut R,
    file_len: u64,
) -> Result<FontFileInfo, FontParseError> {
    read_font_file_with(reader, file_len, &mut NameTables::default())
}

/// [`read_font_file`] 본체. `names`는 이 파일 하나의 name table 읽기·decode 상태입니다.
fn read_font_file_with<R: Read + Seek>(
    reader: &mut R,
    file_len: u64,
    names: &mut NameTables,
) -> Result<FontFileInfo, FontParseError> {
    let mut file = Bounded {
        reader,
        len: file_len,
    };
    let signature = file.array_at::<4>(0)?;
    match signature {
        SIG_TRUETYPE | SIG_TRUE | SIG_OTTO => {
            let format = if signature == SIG_OTTO {
                FontFileFormat::OpenTypeCff
            } else {
                FontFileFormat::TrueType
            };
            let face = read_sfnt_face(&mut file, 0, names)?;
            Ok(FontFileInfo {
                format,
                faces: vec![face],
            })
        }
        SIG_TTCF => read_collection(&mut file, names),
        SIG_WOFF => read_woff(&mut file),
        SIG_WOFF2 => read_woff2(&mut file),
        signature => Err(FontParseError::UnknownSignature { signature }),
    }
}

/// 파일 길이로 모든 읽기를 제한하는 reader.
struct Bounded<'r, R> {
    reader: &'r mut R,
    len: u64,
}

impl<R: Read + Seek> Bounded<'_, R> {
    /// `[offset, offset + len)`이 파일 안인지 확인합니다.
    fn check(&self, offset: u64, len: u64) -> Result<(), FontParseError> {
        match offset.checked_add(len) {
            Some(end) if end <= self.len => Ok(()),
            _ => Err(FontParseError::Truncated { offset, len }),
        }
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), FontParseError> {
        let len = buf.len() as u64;
        self.check(offset, len)?;
        self.reader
            .seek(SeekFrom::Start(offset))
            .map_err(|e| FontParseError::io(&e))?;
        self.reader.read_exact(buf).map_err(|e| {
            if e.kind() == ErrorKind::UnexpectedEof {
                // 선언 길이보다 실제 파일이 짧습니다(읽는 도중 잘림).
                FontParseError::Truncated { offset, len }
            } else {
                FontParseError::io(&e)
            }
        })
    }

    fn array_at<const N: usize>(&mut self, offset: u64) -> Result<[u8; N], FontParseError> {
        let mut out = [0u8; N];
        self.read_at(offset, &mut out)?;
        Ok(out)
    }

    fn u16_at(&mut self, offset: u64) -> Result<u16, FontParseError> {
        Ok(u16::from_be_bytes(self.array_at::<2>(offset)?))
    }

    fn u32_at(&mut self, offset: u64) -> Result<u32, FontParseError> {
        Ok(u32::from_be_bytes(self.array_at::<4>(offset)?))
    }

    /// `[offset, offset + len)`을 읽습니다. `len`은 호출자가 상한으로 제한한 값입니다.
    fn vec_at(&mut self, offset: u64, len: u32) -> Result<Vec<u8>, FontParseError> {
        self.check(offset, u64::from(len))?;
        let mut out = vec![0u8; len as usize];
        self.read_at(offset, &mut out)?;
        Ok(out)
    }
}

fn is_sfnt_signature(signature: [u8; 4]) -> bool {
    matches!(signature, SIG_TRUETYPE | SIG_TRUE | SIG_OTTO)
}

fn check_face_count(count: u32) -> Result<usize, FontParseError> {
    if count == 0 {
        return Err(FontParseError::NoFaces);
    }
    if count > MAX_FACES_PER_FILE {
        return Err(FontParseError::TooManyFaces { count });
    }
    Ok(count as usize)
}

/// TTC: `ttcf`, major, minor, numFonts, offset[numFonts].
fn read_collection<R: Read + Seek>(
    file: &mut Bounded<'_, R>,
    names: &mut NameTables,
) -> Result<FontFileInfo, FontParseError> {
    let major = file.u16_at(4)?;
    let minor = file.u16_at(6)?;
    if !matches!(major, 1 | 2) || minor != 0 {
        return Err(FontParseError::UnsupportedCollectionVersion { major, minor });
    }
    let count = check_face_count(file.u32_at(8)?)?;
    // offset 표 전체가 파일 안인지 먼저 확인합니다(count <= MAX_FACES_PER_FILE).
    let table = file.vec_at(TTC_HEADER_LEN, (count as u32) * 4)?;
    let mut faces = Vec::with_capacity(count);
    for (index, raw) in table.chunks_exact(4).enumerate() {
        let offset = u64::from(u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]));
        // index < count <= MAX_FACES_PER_FILE.
        let index = index as u32;
        let signature = file.array_at::<4>(offset)?;
        if !is_sfnt_signature(signature) {
            return Err(FontParseError::InvalidFaceSignature { index, signature });
        }
        faces.push(read_sfnt_face(file, offset, names)?);
    }
    Ok(FontFileInfo {
        format: FontFileFormat::Collection,
        faces,
    })
}

/// 파일 하나의 name table 읽기 상태(모듈 문서 "신뢰하지 않는 입력").
///
/// 같은 구간(offset, 길이)은 한 번만 읽어 해석한 결과를 재사용하고, 서로 다른 구간을 읽은 bytes
/// 합계를 [`MAX_NAME_TABLE_BYTES_PER_FILE`], decode 원본 bytes 합계를 [`MAX_NAME_DECODE_BYTES_PER_FILE`]로
/// 제한합니다. face 순서대로 쓰므로 결과는 결정론적입니다.
#[derive(Default)]
struct NameTables {
    parsed: BTreeMap<(u64, u32), FontFaceNames>,
    bytes_read: u64,
    /// 이 파일의 모든 name table에 걸친 decode 작업량.
    work: NameParseWork,
}

impl NameTables {
    /// `[offset, offset + length)` name table의 이름. 파일 밖이거나 예산을 넘으면 빈 이름입니다.
    fn names<R: Read + Seek>(
        &mut self,
        file: &mut Bounded<'_, R>,
        offset: u64,
        length: u32,
    ) -> Result<FontFaceNames, FontParseError> {
        if let Some(names) = self.parsed.get(&(offset, length)) {
            return Ok(names.clone());
        }
        // name table이 파일 밖이면 이름만 비웁니다(모듈 문서). 읽지 않으므로 예산을 쓰지 않습니다.
        if file.check(offset, u64::from(length)).is_err() {
            return Ok(FontFaceNames::default());
        }
        let total = self.bytes_read.saturating_add(u64::from(length));
        if total > MAX_NAME_TABLE_BYTES_PER_FILE {
            return Ok(FontFaceNames::default());
        }
        self.bytes_read = total;
        let names = match file.vec_at(offset, length) {
            Ok(bytes) => parse_name_table(&bytes, &mut self.work),
            // 선언 길이보다 실제 파일이 짧음(읽는 도중 잘림).
            Err(FontParseError::Truncated { .. }) => FontFaceNames::default(),
            Err(e) => return Err(e),
        };
        self.parsed.insert((offset, length), names.clone());
        Ok(names)
    }
}

/// sfnt face 하나(offset table 위치 `face_offset`)의 이름을 읽습니다.
///
/// offset table과 table record는 구조 오류로 거부하고, name table 문제는 이름만 비웁니다.
fn read_sfnt_face<R: Read + Seek>(
    file: &mut Bounded<'_, R>,
    face_offset: u64,
    name_tables: &mut NameTables,
) -> Result<FontFaceInfo, FontParseError> {
    let num_tables = file.u16_at(face_offset + 4)?;
    if num_tables > MAX_SFNT_TABLES {
        return Err(FontParseError::TooManyTables { count: num_tables });
    }
    let records = file.vec_at(
        face_offset + SFNT_HEADER_LEN,
        u32::from(num_tables) * SFNT_TABLE_RECORD_LEN as u32,
    )?;
    let mut name_table: Option<(u64, u32)> = None;
    let mut os2_table: Option<(u64, u32)> = None;
    for record in records.chunks_exact(SFNT_TABLE_RECORD_LEN as usize) {
        let slot = if record[0..4] == TAG_NAME {
            &mut name_table
        } else if record[0..4] == TAG_OS2 {
            &mut os2_table
        } else {
            continue;
        };
        let offset = u32::from_be_bytes([record[8], record[9], record[10], record[11]]);
        let length = u32::from_be_bytes([record[12], record[13], record[14], record[15]]);
        // 같은 tag가 여러 번 나오면 첫 record를 씁니다(결정론).
        if slot.is_none() {
            *slot = Some((u64::from(offset), length));
        }
    }
    let embedding = match os2_table {
        Some((offset, length)) if length >= OS2_FS_TYPE_END => match file.u16_at(offset + 8) {
            Ok(fs_type) => Some(EmbeddingPermissions::from_fs_type(fs_type)),
            // OS/2 table이 파일 밖이면 선언 없음으로 봅니다(모듈 문서).
            Err(FontParseError::Truncated { .. }) => None,
            Err(e) => return Err(e),
        },
        _ => None,
    };
    let names = match name_table {
        Some((offset, length)) if length <= MAX_NAME_TABLE_BYTES => {
            name_tables.names(file, offset, length)?
        }
        _ => FontFaceNames::default(),
    };
    Ok(FontFaceInfo { names, embedding })
}

/// WOFF 1.0: header만 검사합니다. collection을 지원하지 않는 형식이므로 face는 1개입니다.
fn read_woff<R: Read + Seek>(file: &mut Bounded<'_, R>) -> Result<FontFileInfo, FontParseError> {
    let header = file.vec_at(0, WOFF_HEADER_LEN as u32)?;
    let flavor = [header[4], header[5], header[6], header[7]];
    let length = u32::from_be_bytes([header[8], header[9], header[10], header[11]]);
    let reserved = u16::from_be_bytes([header[14], header[15]]);
    check_woff_length(length, file.len)?;
    if reserved != 0 {
        return Err(FontParseError::InvalidWoffHeader {
            reason: "reserved 값이 0이 아닙니다",
        });
    }
    if flavor == SIG_TTCF {
        return Err(FontParseError::InvalidWoffHeader {
            reason: "WOFF 1.0은 collection flavor를 지원하지 않습니다",
        });
    }
    Ok(FontFileInfo {
        format: FontFileFormat::Woff,
        faces: vec![FontFaceInfo::default()],
    })
}

fn check_woff_length(declared: u32, file_len: u64) -> Result<(), FontParseError> {
    if u64::from(declared) != file_len {
        return Err(FontParseError::InvalidWoffHeader {
            reason: "header의 length가 파일 크기와 다릅니다",
        });
    }
    Ok(())
}

/// WOFF2: header를 검사하고, flavor가 `ttcf`면 table directory를 건너뛰어 collection directory의
/// face 수와 face별 table index를 검사합니다.
fn read_woff2<R: Read + Seek>(file: &mut Bounded<'_, R>) -> Result<FontFileInfo, FontParseError> {
    let header = file.vec_at(0, WOFF2_HEADER_LEN as u32)?;
    let flavor = [header[4], header[5], header[6], header[7]];
    let length = u32::from_be_bytes([header[8], header[9], header[10], header[11]]);
    let num_tables = u16::from_be_bytes([header[12], header[13]]);
    let reserved = u16::from_be_bytes([header[14], header[15]]);
    check_woff_length(length, file.len)?;
    if reserved != 0 {
        return Err(FontParseError::InvalidWoffHeader {
            reason: "reserved 값이 0이 아닙니다",
        });
    }
    if num_tables == 0 {
        return Err(FontParseError::InvalidWoffHeader {
            reason: "table이 없습니다",
        });
    }
    if num_tables > MAX_SFNT_TABLES {
        return Err(FontParseError::TooManyTables { count: num_tables });
    }
    if flavor != SIG_TTCF {
        return Ok(FontFileInfo {
            format: FontFileFormat::Woff2,
            faces: vec![FontFaceInfo::default()],
        });
    }

    file.reader
        .seek(SeekFrom::Start(WOFF2_HEADER_LEN))
        .map_err(|e| FontParseError::io(&e))?;
    let mut cursor = Cursor {
        reader: std::io::BufReader::new(&mut *file.reader).take(file.len - WOFF2_HEADER_LEN),
        offset: WOFF2_HEADER_LEN,
    };
    for _ in 0..num_tables {
        let flags = cursor.u8()?;
        let tag = match flags & 0x3F {
            0x3F => cursor.array::<4>()?,
            index => KNOWN_TABLE_TAGS[index as usize],
        };
        cursor.base128()?;
        let version = flags >> 6;
        let transformed = if tag == TAG_GLYF || tag == TAG_LOCA {
            version != 3
        } else {
            version != 0
        };
        if transformed {
            cursor.base128()?;
        }
    }
    let version = u32::from_be_bytes(cursor.array::<4>()?);
    if !matches!(version, 0x0001_0000 | 0x0002_0000) {
        return Err(FontParseError::UnsupportedCollectionVersion {
            major: (version >> 16) as u16,
            minor: version as u16,
        });
    }
    let count = check_face_count(u32::from(cursor.u255()?))?;
    let mut faces = Vec::with_capacity(count);
    for index in 0..count {
        let face_tables = cursor.u255()?;
        if face_tables == 0 || face_tables > num_tables {
            return Err(FontParseError::InvalidWoff2Directory {
                reason: "collection face의 table 수가 directory 범위를 벗어납니다",
            });
        }
        let signature = cursor.array::<4>()?;
        if !is_sfnt_signature(signature) {
            return Err(FontParseError::InvalidFaceSignature {
                // index < count <= MAX_FACES_PER_FILE.
                index: index as u32,
                signature,
            });
        }
        for _ in 0..face_tables {
            if cursor.u255()? >= num_tables {
                return Err(FontParseError::InvalidWoff2Directory {
                    reason: "collection face의 table index가 directory 범위를 벗어납니다",
                });
            }
        }
        faces.push(FontFaceInfo::default());
    }
    Ok(FontFileInfo {
        format: FontFileFormat::Woff2,
        faces,
    })
}

/// WOFF2 known table tag(flags 하위 6 bit index 0..=62, WOFF2 spec 5.1).
const KNOWN_TABLE_TAGS: [[u8; 4]; 63] = [
    *b"cmap", *b"head", *b"hhea", *b"hmtx", *b"maxp", *b"name", *b"OS/2", *b"post", *b"cvt ",
    *b"fpgm", *b"glyf", *b"loca", *b"prep", *b"CFF ", *b"VORG", *b"EBDT", *b"EBLC", *b"gasp",
    *b"hdmx", *b"kern", *b"LTSH", *b"PCLT", *b"VDMX", *b"vhea", *b"vmtx", *b"BASE", *b"GDEF",
    *b"GPOS", *b"GSUB", *b"EBSC", *b"JSTF", *b"MATH", *b"CBDT", *b"CBLC", *b"COLR", *b"CPAL",
    *b"SVG ", *b"sbix", *b"acnt", *b"avar", *b"bdat", *b"bloc", *b"bsln", *b"cvar", *b"fdsc",
    *b"feat", *b"fmtx", *b"fvar", *b"gvar", *b"hsty", *b"just", *b"lcar", *b"mort", *b"morx",
    *b"opbd", *b"prop", *b"trak", *b"Zapf", *b"Silf", *b"Glat", *b"Gloc", *b"Feat", *b"Sill",
];

/// 파일 길이로 제한된 순차 reader(WOFF2 directory용).
struct Cursor<R> {
    reader: R,
    offset: u64,
}

impl<R: Read> Cursor<R> {
    fn array<const N: usize>(&mut self) -> Result<[u8; N], FontParseError> {
        let mut out = [0u8; N];
        match self.reader.read_exact(&mut out) {
            Ok(()) => {
                self.offset += N as u64;
                Ok(out)
            }
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => Err(FontParseError::Truncated {
                offset: self.offset,
                len: N as u64,
            }),
            Err(e) => Err(FontParseError::io(&e)),
        }
    }

    fn u8(&mut self) -> Result<u8, FontParseError> {
        Ok(self.array::<1>()?[0])
    }

    /// WOFF2 `UIntBase128`: 최대 5 bytes, 선행 0 byte 금지, u32 overflow 금지.
    fn base128(&mut self) -> Result<u32, FontParseError> {
        let mut value: u32 = 0;
        for i in 0..5 {
            let byte = self.u8()?;
            if i == 0 && byte == 0x80 {
                return Err(FontParseError::InvalidWoff2Directory {
                    reason: "UIntBase128에 선행 0이 있습니다",
                });
            }
            if value & 0xFE00_0000 != 0 {
                return Err(FontParseError::InvalidWoff2Directory {
                    reason: "UIntBase128 값이 u32를 넘습니다",
                });
            }
            value = (value << 7) | u32::from(byte & 0x7F);
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(FontParseError::InvalidWoff2Directory {
            reason: "UIntBase128이 5 bytes를 넘습니다",
        })
    }

    /// WOFF2 `255UInt16`.
    fn u255(&mut self) -> Result<u16, FontParseError> {
        const LOWEST_U_CODE: u16 = 253;
        match self.u8()? {
            253 => Ok(u16::from_be_bytes(self.array::<2>()?)),
            254 => Ok(u16::from(self.u8()?) + LOWEST_U_CODE * 2),
            255 => Ok(u16::from(self.u8()?) + LOWEST_U_CODE),
            code => Ok(u16::from(code)),
        }
    }
}

/// name record 정렬 key: (순위, language id, encoding id, record index). record index가 있어 서로
/// 다릅니다.
type NameRecordKey = (u8, u16, u16, usize);

/// name record 하나(nameID·순위·길이 필터 후).
struct NameRecord {
    name_id: u16,
    key: NameRecordKey,
    platform: u16,
    bytes_offset: usize,
    bytes_len: usize,
}

/// name table 해석 작업량(decode를 시도한 record 수와 그 원본 bytes 합). 상한 검사에 씁니다.
#[derive(Debug, Default)]
struct NameParseWork {
    decode_attempts: usize,
    decoded_bytes: usize,
}

/// 이름 선택 순위(모듈 문서 "이름 선택 규칙"). 대상이 아니면 `None`.
fn name_rank(platform: u16, encoding: u16, language: u16) -> Option<u8> {
    match (platform, encoding, language) {
        (3, 1 | 10, 0x0409) => Some(0),
        (3, 1 | 10, _) => Some(1),
        (0, _, _) => Some(2),
        (1, 0, 0) => Some(3),
        _ => None,
    }
}

/// 유효 이름이 될 수 있는 record 원본 bytes 상한(모듈 문서 "이름 선택 규칙").
fn max_name_record_bytes(platform: u16) -> usize {
    if platform == 1 {
        MAX_MAC_NAME_RECORD_BYTES
    } else {
        MAX_UTF16_NAME_RECORD_BYTES
    }
}

/// name table bytes(메모리, [`MAX_NAME_TABLE_BYTES`] 이하)에서 이름을 고릅니다.
///
/// 순위 대상이 아니거나 원본 길이가 상한을 넘는 record는 decode 전에 거르고, nameID마다 순위 순 앞
/// [`MAX_NAME_CANDIDATES_PER_ID`]개만 decode합니다. 작업량은 `work`에 더하며, `work`의 decode 합계가
/// [`MAX_NAME_DECODE_BYTES_PER_FILE`]을 넘게 되는 후보는 decode하지 않고 제외합니다.
fn parse_name_table(table: &[u8], work: &mut NameParseWork) -> FontFaceNames {
    let read_u16 = |at: usize| -> Option<u16> {
        table
            .get(at..at.checked_add(2)?)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
    };
    let (Some(count), Some(string_offset)) = (read_u16(2), read_u16(4)) else {
        return FontFaceNames::default();
    };
    let string_offset = usize::from(string_offset);
    let mut records: Vec<NameRecord> = Vec::new();
    for index in 0..usize::from(count) {
        let at = 6 + index * 12;
        let Some(raw) = table.get(at..at + 12) else {
            // record 배열이 table 밖으로 나가면 거기까지만 씁니다.
            break;
        };
        let field = |k: usize| u16::from_be_bytes([raw[k * 2], raw[k * 2 + 1]]);
        let (platform, encoding, language, name_id) = (field(0), field(1), field(2), field(3));
        if !matches!(name_id, 1 | 2 | 6 | 16 | 17) {
            continue;
        }
        let Some(rank) = name_rank(platform, encoding, language) else {
            continue;
        };
        let bytes_len = usize::from(field(4));
        if bytes_len > max_name_record_bytes(platform) {
            continue;
        }
        records.push(NameRecord {
            name_id,
            key: (rank, language, encoding, index),
            platform,
            bytes_offset: string_offset + usize::from(field(5)),
            bytes_len,
        });
    }
    let mut pick = |name_id: u16| -> Option<String> {
        let mut candidates: Vec<&NameRecord> = records
            .iter()
            .filter(|record| record.name_id == name_id)
            .collect();
        if candidates.len() > MAX_NAME_CANDIDATES_PER_ID {
            candidates.select_nth_unstable_by_key(MAX_NAME_CANDIDATES_PER_ID - 1, |r| r.key);
            candidates.truncate(MAX_NAME_CANDIDATES_PER_ID);
        }
        candidates.sort_unstable_by_key(|r| r.key);
        candidates.into_iter().find_map(|r| {
            let bytes = table.get(r.bytes_offset..r.bytes_offset.checked_add(r.bytes_len)?)?;
            let decoded = (work.decoded_bytes as u64).saturating_add(bytes.len() as u64);
            if decoded > MAX_NAME_DECODE_BYTES_PER_FILE {
                return None;
            }
            work.decode_attempts += 1;
            work.decoded_bytes += bytes.len();
            let text = if r.platform == 1 {
                bytes
                    .is_ascii()
                    .then(|| String::from_utf8_lossy(bytes).into_owned())?
            } else {
                decode_utf16_be(bytes)?
            };
            let text = text.trim_end_matches('\0').to_string();
            validate_face_name(&text).ok().map(|()| text)
        })
    };
    let family = pick(16).or_else(|| pick(1));
    let subfamily = pick(17).or_else(|| pick(2));
    let postscript = pick(6);
    FontFaceNames {
        family,
        subfamily,
        postscript,
    }
}

/// UTF-16BE bytes를 decode합니다. 홀수 길이·짝 없는 surrogate는 `None`.
fn decode_utf16_be(bytes: &[u8]) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let units = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]));
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::test_fixtures::{
        family_faces, sfnt_face, ttc, woff, woff2, woff2_collection, NameEntry,
        WOFF2_FIRST_FACE_FLAVOR_OFFSET, WOFF2_FIRST_FACE_INDEX_OFFSET,
    };
    use std::io::Cursor as IoCursor;

    fn probe(bytes: &[u8]) -> Result<FontFileInfo, FontParseError> {
        read_font_file(&mut IoCursor::new(bytes), bytes.len() as u64)
    }

    fn names(family: &str, sub: &str, ps: &str) -> FontFaceNames {
        FontFaceNames::new(
            Some(family.to_string()),
            Some(sub.to_string()),
            Some(ps.to_string()),
        )
        .unwrap()
    }

    #[test]
    fn format_tags_are_stable_golden() {
        let tags: Vec<(u8, &str, &str)> = FontFileFormat::ALL
            .iter()
            .map(|f| (f.tag(), f.name(), f.media_type()))
            .collect();
        assert_eq!(
            tags,
            vec![
                (1, "truetype", "font/ttf"),
                (2, "opentype-cff", "font/otf"),
                (3, "collection", "font/collection"),
                (4, "woff", "font/woff"),
                (5, "woff2", "font/woff2"),
            ]
        );
        for format in FontFileFormat::ALL {
            assert_eq!(FontFileFormat::from_tag(format.tag()), Some(format));
        }
        assert_eq!(FontFileFormat::from_tag(0), None);
        assert_eq!(FontFileFormat::from_tag(6), None);
    }

    #[test]
    fn single_sfnt_faces_are_probed_with_names() {
        let ttf = sfnt_face(SIG_TRUETYPE, &family_faces("Fixture Sans")[0]);
        let info = probe(&ttf).unwrap();
        assert_eq!(info.format, FontFileFormat::TrueType);
        assert_eq!(info.face_count(), 1);
        assert_eq!(
            info.faces[0].names,
            names("Fixture Sans", "Regular", "FixtureSans-Regular")
        );

        let otf = sfnt_face(SIG_OTTO, &family_faces("Fixture Sans")[1]);
        let info = probe(&otf).unwrap();
        assert_eq!(info.format, FontFileFormat::OpenTypeCff);
        assert_eq!(
            info.faces[0].names,
            names("Fixture Sans", "Bold", "FixtureSans-Bold")
        );
    }

    #[test]
    fn collection_faces_are_counted_and_named_in_order() {
        let bytes = ttc(&family_faces("Fixture Sans"));
        let info = probe(&bytes).unwrap();
        assert_eq!(info.format, FontFileFormat::Collection);
        assert_eq!(info.face_count(), 3);
        let ps: Vec<_> = info
            .faces
            .iter()
            .map(|f| f.names.postscript().unwrap().to_string())
            .collect();
        assert_eq!(
            ps,
            [
                "FixtureSans-Regular",
                "FixtureSans-Bold",
                "FixtureSans-Italic"
            ]
        );
    }

    #[test]
    fn name_selection_prefers_typographic_family_and_windows_english() {
        let entries = [
            // Mac Roman family(순위 4).
            NameEntry::mac(1, "Mac Family"),
            // Windows 다른 언어(순위 2).
            NameEntry::windows_lang(1, 0x0412, "한국어 Family"),
            // Windows English US(순위 1).
            NameEntry::windows(1, "Legacy Family"),
            // typographic family(nameID 16)가 1보다 우선합니다.
            NameEntry::unicode(16, "Typo Family"),
            NameEntry::mac(2, "Mac Sub"),
            NameEntry::windows(6, "Typo-PS"),
        ];
        let info = probe(&sfnt_face(SIG_TRUETYPE, &entries)).unwrap();
        let names = &info.faces[0].names;
        assert_eq!(names.family(), Some("Typo Family"));
        assert_eq!(names.subfamily(), Some("Mac Sub"));
        assert_eq!(names.postscript(), Some("Typo-PS"));

        let only_other_language = [NameEntry::windows_lang(1, 0x0412, "한국어 Family")];
        let info = probe(&sfnt_face(SIG_TRUETYPE, &only_other_language)).unwrap();
        assert_eq!(info.faces[0].names.family(), Some("한국어 Family"));
    }

    #[test]
    fn broken_names_are_skipped_without_rejecting_the_face() {
        let entries = [
            // 짝 없는 high surrogate.
            NameEntry::raw(3, 1, 0x0409, 1, vec![0xD8, 0x00]),
            // 제어 문자.
            NameEntry::windows(6, "Bad\u{1}Name"),
            // 비ASCII Mac Roman은 decode하지 않습니다.
            NameEntry::raw(1, 0, 0, 2, vec![0x8E]),
            // 홀수 길이 UTF-16.
            NameEntry::raw(3, 1, 0x0409, 2, vec![0x00]),
            // 낮은 순위의 유효 family가 대신 선택됩니다.
            NameEntry::unicode(1, "Fallback Family"),
        ];
        let info = probe(&sfnt_face(SIG_TRUETYPE, &entries)).unwrap();
        let names = &info.faces[0].names;
        assert_eq!(names.family(), Some("Fallback Family"));
        assert_eq!(names.subfamily(), None);
        assert_eq!(names.postscript(), None);
    }

    #[test]
    fn missing_or_out_of_file_name_table_keeps_the_face() {
        // name table 없음.
        let info = probe(&sfnt_face(SIG_TRUETYPE, &[])).unwrap();
        assert!(info.faces[0].names.is_empty());
        // name table offset을 파일 밖으로 옮깁니다(table record offset 필드: 12 + 8).
        let mut bytes = sfnt_face(SIG_TRUETYPE, &family_faces("Fixture Sans")[0]);
        bytes[20..24].copy_from_slice(&0x00FF_FFFFu32.to_be_bytes());
        let info = probe(&bytes).unwrap();
        assert_eq!(info.face_count(), 1);
        assert!(info.faces[0].names.is_empty());
    }

    #[test]
    fn woff_and_woff2_headers_are_counted() {
        let info = probe(&woff(SIG_TRUETYPE)).unwrap();
        assert_eq!(info.format, FontFileFormat::Woff);
        assert_eq!(info.face_count(), 1);
        let info = probe(&woff2(SIG_OTTO)).unwrap();
        assert_eq!(info.format, FontFileFormat::Woff2);
        assert_eq!(info.face_count(), 1);
        // 255UInt16의 세 가지 코드 경로(직접, 253 word, 254/255 one-more-byte).
        for count in [2u16, 260, 520, 1000] {
            let info = probe(&woff2_collection(count)).unwrap();
            assert_eq!(info.format, FontFileFormat::Woff2);
            assert_eq!(info.face_count(), u32::from(count));
        }
        assert!(info.faces.iter().all(|f| f.names.is_empty()));
    }

    #[test]
    fn hostile_headers_are_rejected_with_typed_errors() {
        assert_eq!(
            probe(b"abcdxxxxxxxx"),
            Err(FontParseError::UnknownSignature {
                signature: *b"abcd"
            })
        );
        assert_eq!(
            probe(b"tt"),
            Err(FontParseError::Truncated { offset: 0, len: 4 })
        );

        let mut bytes = ttc(&family_faces("Fixture Sans"));
        // face 수 0, 상한 초과, offset 표가 파일 밖.
        bytes[8..12].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(probe(&bytes), Err(FontParseError::NoFaces));
        bytes[8..12].copy_from_slice(&(MAX_FACES_PER_FILE + 1).to_be_bytes());
        assert_eq!(
            probe(&bytes),
            Err(FontParseError::TooManyFaces {
                count: MAX_FACES_PER_FILE + 1
            })
        );
        bytes[8..12].copy_from_slice(&MAX_FACES_PER_FILE.to_be_bytes());
        assert!(matches!(
            probe(&bytes),
            Err(FontParseError::Truncated { .. })
        ));

        // 잘못된 collection version.
        let mut bytes = ttc(&family_faces("Fixture Sans"));
        bytes[4..6].copy_from_slice(&3u16.to_be_bytes());
        assert_eq!(
            probe(&bytes),
            Err(FontParseError::UnsupportedCollectionVersion { major: 3, minor: 0 })
        );

        // face offset이 sfnt가 아닌 곳·파일 밖을 가리킵니다.
        let mut bytes = ttc(&family_faces("Fixture Sans"));
        bytes[16..20].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(
            probe(&bytes),
            Err(FontParseError::InvalidFaceSignature {
                index: 1,
                signature: SIG_TTCF
            })
        );
        bytes[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(
            probe(&bytes),
            Err(FontParseError::Truncated { .. })
        ));

        // table 수 상한 초과.
        let mut bytes = sfnt_face(SIG_TRUETYPE, &family_faces("Fixture Sans")[0]);
        bytes[4..6].copy_from_slice(&(MAX_SFNT_TABLES + 1).to_be_bytes());
        assert_eq!(
            probe(&bytes),
            Err(FontParseError::TooManyTables {
                count: MAX_SFNT_TABLES + 1
            })
        );
    }

    #[test]
    fn hostile_woff_headers_are_rejected() {
        let mut bytes = woff(SIG_TRUETYPE);
        bytes.push(0);
        assert!(matches!(
            probe(&bytes),
            Err(FontParseError::InvalidWoffHeader { .. })
        ));
        let mut bytes = woff(SIG_TTCF);
        assert!(matches!(
            probe(&bytes),
            Err(FontParseError::InvalidWoffHeader { .. })
        ));
        bytes[4..8].copy_from_slice(&SIG_TRUETYPE);
        bytes[14] = 1;
        assert!(matches!(
            probe(&bytes),
            Err(FontParseError::InvalidWoffHeader { .. })
        ));

        // WOFF2 collection: 잘린 directory, 범위 밖 table index, 잘못된 flavor.
        let full = woff2_collection(2);
        let mut cut = full[..full.len() - 10].to_vec();
        let len = cut.len() as u32;
        cut[8..12].copy_from_slice(&len.to_be_bytes());
        assert!(matches!(probe(&cut), Err(FontParseError::Truncated { .. })));
        let mut bad_index = full.clone();
        bad_index[WOFF2_FIRST_FACE_INDEX_OFFSET] = 9;
        assert!(matches!(
            probe(&bad_index),
            Err(FontParseError::InvalidWoff2Directory { .. })
        ));
        let mut bad_flavor = full.clone();
        let at = WOFF2_FIRST_FACE_FLAVOR_OFFSET;
        bad_flavor[at..at + 4].copy_from_slice(b"xxxx");
        assert_eq!(
            probe(&bad_flavor),
            Err(FontParseError::InvalidFaceSignature {
                index: 0,
                signature: *b"xxxx"
            })
        );
    }

    #[test]
    fn base128_rules() {
        let parse = |bytes: &[u8]| {
            Cursor {
                reader: bytes,
                offset: 0,
            }
            .base128()
        };
        assert_eq!(parse(&[0x3F]), Ok(63));
        assert_eq!(parse(&[0x81, 0x00]), Ok(128));
        assert_eq!(parse(&[0x8F, 0xFF, 0xFF, 0xFF, 0x7F]), Ok(u32::MAX));
        assert!(matches!(
            parse(&[0x80, 0x01]),
            Err(FontParseError::InvalidWoff2Directory { .. })
        ));
        assert!(matches!(
            parse(&[0x90, 0x80, 0x80, 0x80, 0x00]),
            Err(FontParseError::InvalidWoff2Directory { .. })
        ));
        assert!(matches!(
            parse(&[0x81, 0x81, 0x81, 0x81, 0x81, 0x01]),
            Err(FontParseError::InvalidWoff2Directory { .. })
        ));
        assert!(matches!(
            parse(&[0x81]),
            Err(FontParseError::Truncated { .. })
        ));
    }

    #[test]
    fn face_name_validation() {
        assert_eq!(
            FontFaceNames::new(Some(String::new()), None, None),
            Err(FaceNameError::Empty)
        );
        assert_eq!(
            FontFaceNames::new(None, Some("a\nb".into()), None),
            Err(FaceNameError::ControlCharacter)
        );
        assert_eq!(
            FontFaceNames::new(None, None, Some("x".repeat(MAX_FACE_NAME_BYTES + 1))),
            Err(FaceNameError::TooLong {
                len: MAX_FACE_NAME_BYTES + 1
            })
        );
        assert!(FontFaceNames::new(None, None, Some("x".repeat(MAX_FACE_NAME_BYTES))).is_ok());
    }

    #[test]
    fn probe_is_deterministic() {
        let bytes = ttc(&family_faces("Fixture Sans"));
        assert_eq!(probe(&bytes), probe(&bytes));
    }

    #[test]
    fn os2_fs_type_is_read_as_embedding_declaration() {
        use crate::fonts::test_fixtures::{os2_table, sfnt_face_with_tables};
        let names = &family_faces("Fixture Sans")[0];
        let cases = [
            (0x0000, EmbeddingLevel::Installable),
            (0x0002, EmbeddingLevel::RestrictedLicense),
            (0x0004, EmbeddingLevel::PreviewAndPrint),
            (0x0008, EmbeddingLevel::Editable),
            // 여러 bit면 가장 덜 제한적인 수준.
            (0x000E, EmbeddingLevel::Editable),
            (0x0306, EmbeddingLevel::PreviewAndPrint),
        ];
        for (fs_type, level) in cases {
            let bytes = sfnt_face_with_tables(SIG_TRUETYPE, names, &[os2_table(fs_type, 96)]);
            let face = &probe(&bytes).unwrap().faces[0];
            let embedding = face.embedding.unwrap();
            assert_eq!(embedding.fs_type(), fs_type);
            assert_eq!(embedding.level(), level, "fsType {fs_type:#06x}");
            assert_eq!(embedding.no_subsetting(), fs_type & 0x0100 != 0);
            assert_eq!(embedding.bitmap_only(), fs_type & 0x0200 != 0);
            // 이름 읽기는 그대로입니다.
            assert_eq!(face.names.family(), Some("Fixture Sans"));
        }
        // OS/2 없음·너무 짧음·파일 밖이면 선언 없음.
        assert_eq!(
            probe(&sfnt_face(SIG_TRUETYPE, names)).unwrap().faces[0].embedding,
            None
        );
        let short = sfnt_face_with_tables(SIG_TRUETYPE, names, &[os2_table(2, 9)]);
        assert_eq!(probe(&short).unwrap().faces[0].embedding, None);
        let mut outside = sfnt_face_with_tables(SIG_TRUETYPE, &[], &[os2_table(2, 96)]);
        // table record(하나뿐) offset 필드를 파일 밖으로 옮깁니다.
        outside[20..24].copy_from_slice(&0x00FF_FFFFu32.to_be_bytes());
        assert_eq!(probe(&outside).unwrap().faces[0].embedding, None);
    }

    /// 읽은 bytes를 세는 reader.
    struct CountingReader<'a> {
        inner: IoCursor<&'a [u8]>,
        bytes_read: u64,
    }

    impl Read for CountingReader<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.bytes_read += n as u64;
            Ok(n)
        }
    }

    impl Seek for CountingReader<'_> {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(pos)
        }
    }

    /// probe 결과와 읽은 bytes 수.
    fn probe_counting(bytes: &[u8]) -> (Result<FontFileInfo, FontParseError>, u64) {
        let mut reader = CountingReader {
            inner: IoCursor::new(bytes),
            bytes_read: 0,
        };
        let result = read_font_file(&mut reader, bytes.len() as u64);
        (result, reader.bytes_read)
    }

    #[test]
    fn oversized_overlapping_name_records_are_not_decoded() {
        use crate::fonts::test_fixtures::{overlapping_name_table, sfnt_face_with_tables};
        // 리뷰 재현 입력: 5,460개 nameID-1 record가 모두 같은 65,534 bytes 구간을 가리킴(약 128 KiB).
        let table = overlapping_name_table(5460, 1, 65534, u16::from(b'A'));
        let mut work = NameParseWork::default();
        let names = parse_name_table(&table, &mut work);
        // 유효 이름 상한(2 × 1024 + 2 bytes)을 넘는 record는 decode 전에 제외됩니다.
        assert!(names.is_empty());
        assert_eq!(work.decode_attempts, 0);
        assert_eq!(work.decoded_bytes, 0);
        let bytes = sfnt_face_with_tables(SIG_TRUETYPE, &[], &[(*b"name", table)]);
        let info = probe(&bytes).unwrap();
        assert_eq!(info.face_count(), 1);
        assert!(info.faces[0].names.is_empty());
    }

    #[test]
    fn name_decode_candidates_are_capped_per_name_id() {
        use crate::fonts::test_fixtures::overlapping_name_table;
        // 길이 상한 이내지만 제어 문자(U+0001)라 모두 거부되는 record 5,460개.
        let table = overlapping_name_table(5460, 1, 2048, 0x0001);
        let mut work = NameParseWork::default();
        let names = parse_name_table(&table, &mut work);
        assert!(names.is_empty());
        // nameID 1만 후보가 있고 순위 순 앞 MAX_NAME_CANDIDATES_PER_ID개만 decode합니다.
        assert_eq!(work.decode_attempts, MAX_NAME_CANDIDATES_PER_ID);
        assert_eq!(work.decoded_bytes, MAX_NAME_CANDIDATES_PER_ID * 2048);

        // 상한 경계: 정확히 2 × 1024 + 2 bytes(끝 NUL 하나)인 이름은 decode되어 선택됩니다.
        let mut entries = vec![0u8; 0];
        for _ in 0..MAX_FACE_NAME_BYTES {
            entries.extend_from_slice(&u16::from(b'A').to_be_bytes());
        }
        entries.extend_from_slice(&[0, 0]);
        let exact = NameEntry::raw(3, 1, 0x0409, 1, entries.clone());
        let info = probe(&sfnt_face(SIG_TRUETYPE, &[exact])).unwrap();
        assert_eq!(
            info.faces[0].names.family().map(str::len),
            Some(MAX_FACE_NAME_BYTES)
        );
        // 2 bytes 더 길면(끝 NUL 둘) decode하지 않고 제외합니다.
        entries.extend_from_slice(&[0, 0]);
        let over = NameEntry::raw(3, 1, 0x0409, 1, entries);
        let info = probe(&sfnt_face(SIG_TRUETYPE, &[over])).unwrap();
        assert_eq!(info.faces[0].names.family(), None);
    }

    #[test]
    fn repeated_collection_faces_read_the_name_table_once() {
        use crate::fonts::test_fixtures::{overlapping_name_table, ttc_repeated};
        let table = overlapping_name_table(5460, 1, 2048, 0x0001);
        let table_len = table.len() as u64;
        let bytes = ttc_repeated(&[(*b"name", table)], MAX_FACES_PER_FILE);
        let (result, bytes_read) = probe_counting(&bytes);
        let info = result.unwrap();
        assert_eq!(info.face_count(), MAX_FACES_PER_FILE);
        // name table은 한 번만 읽습니다. 나머지는 face마다 offset table·record 수십 bytes입니다.
        let per_face_overhead = 64 * u64::from(MAX_FACES_PER_FILE);
        assert!(
            bytes_read <= table_len + per_face_overhead,
            "읽은 bytes {bytes_read} > {table_len} + {per_face_overhead}"
        );
    }

    #[test]
    fn distinct_name_tables_share_a_per_file_decode_budget() {
        use crate::fonts::test_fixtures::{overlapping_name_table_for, ttc_distinct_copies};
        // 리뷰 재현 입력: 약 6 KB name table 하나에 nameID 5종 × 64개 record가 모두 같은 2,050 bytes
        // 구간(제어 문자라 거부)을 가리킴. table 하나의 decode는 320 × 2,050 = 656,000 bytes입니다.
        let table = overlapping_name_table_for(&[1, 2, 6, 16, 17], 64, 2050, 0x0001);
        let mut work = NameParseWork::default();
        assert!(parse_name_table(&table, &mut work).is_empty());
        let per_table = work.decoded_bytes as u64;
        assert_eq!(per_table, 5 * 64 * 2050);
        // 서로 다른 구간의 사본 40개: 읽기는 약 236 KB(읽기 상한 안)지만 상한이 없으면 decode는
        // 26 MB를 넘습니다.
        let count = 40;
        assert!(per_table * u64::from(count) > MAX_NAME_DECODE_BYTES_PER_FILE);
        let bytes = ttc_distinct_copies(&[(*b"name", table)], count);
        let mut names = NameTables::default();
        let info = read_font_file_with(
            &mut IoCursor::new(&bytes[..]),
            bytes.len() as u64,
            &mut names,
        )
        .unwrap();
        assert_eq!(info.face_count(), count);
        assert!(info.faces.iter().all(|face| face.names.is_empty()));
        assert!(names.bytes_read < MAX_NAME_TABLE_BYTES_PER_FILE);
        let decoded = names.work.decoded_bytes as u64;
        assert!(
            decoded <= MAX_NAME_DECODE_BYTES_PER_FILE,
            "decode bytes {decoded} > {MAX_NAME_DECODE_BYTES_PER_FILE}"
        );
        // 상한 안의 앞 table들은 실제로 decode했습니다.
        assert!(decoded + per_table > MAX_NAME_DECODE_BYTES_PER_FILE);
    }

    #[test]
    fn distinct_name_table_reads_are_capped_per_file() {
        use crate::fonts::test_fixtures::ttc_shifted_name_windows;
        // 구간이 모두 다른 1 MiB name table 40개: 상한이 없으면 40 MiB를 읽습니다.
        let count = 40;
        let bytes = ttc_shifted_name_windows(count, MAX_NAME_TABLE_BYTES);
        let (result, bytes_read) = probe_counting(&bytes);
        let info = result.unwrap();
        assert_eq!(info.face_count(), count);
        assert!(info.faces.iter().all(|face| face.names.is_empty()));
        let overhead = 64 * u64::from(count) + 4 * u64::from(count) + 12;
        assert!(
            bytes_read <= MAX_NAME_TABLE_BYTES_PER_FILE + overhead,
            "읽은 bytes {bytes_read} > {MAX_NAME_TABLE_BYTES_PER_FILE} + {overhead}"
        );
        // 상한 안의 앞 face들은 실제로 읽었습니다(16개 × 1 MiB).
        assert!(bytes_read >= MAX_NAME_TABLE_BYTES_PER_FILE);
    }
}
