//! 테스트용 합성 폰트 bytes (XIII.6).
//!
//! 실제 폰트 파일을 저장소에 넣지 않고, probe가 읽는 구조(offset table, table record, name
//! table, collection header, WOFF·WOFF2 header·directory)만 코드로 만듭니다. glyph 데이터는
//! 없습니다.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 테스트용 임시 디렉터리(drop 시 삭제).
pub(crate) struct TempDir {
    /// 디렉터리 경로.
    pub(crate) path: PathBuf,
}

impl TempDir {
    /// 프로세스·호출마다 고유한 빈 디렉터리를 만듭니다.
    pub(crate) fn new(tag: &str) -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("stellar-fonts-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("임시 디렉터리 생성");
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// sfnt TrueType signature.
pub(crate) const SIG_TRUETYPE: [u8; 4] = [0x00, 0x01, 0x00, 0x00];
/// collection signature.
pub(crate) const SIG_TTCF: [u8; 4] = *b"ttcf";

/// `woff2_collection(2)`에서 face 0 flavor의 byte offset(header 48 + directory 6 + collection
/// header 5 + face numTables 1).
pub(crate) const WOFF2_FIRST_FACE_FLAVOR_OFFSET: usize = 60;
/// `woff2_collection(2)`에서 face 0 첫 table index의 byte offset.
pub(crate) const WOFF2_FIRST_FACE_INDEX_OFFSET: usize = 64;

/// name record 하나.
#[derive(Debug, Clone)]
pub(crate) struct NameEntry {
    platform: u16,
    encoding: u16,
    language: u16,
    name_id: u16,
    bytes: Vec<u8>,
}

fn utf16_be(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

impl NameEntry {
    /// Windows Unicode BMP English US.
    pub(crate) fn windows(name_id: u16, text: &str) -> NameEntry {
        NameEntry::windows_lang(name_id, 0x0409, text)
    }

    /// Windows Unicode BMP, 지정 language.
    pub(crate) fn windows_lang(name_id: u16, language: u16, text: &str) -> NameEntry {
        NameEntry::raw(3, 1, language, name_id, utf16_be(text))
    }

    /// Unicode platform.
    pub(crate) fn unicode(name_id: u16, text: &str) -> NameEntry {
        NameEntry::raw(0, 3, 0, name_id, utf16_be(text))
    }

    /// Macintosh Roman English(ASCII bytes).
    pub(crate) fn mac(name_id: u16, text: &str) -> NameEntry {
        NameEntry::raw(1, 0, 0, name_id, text.as_bytes().to_vec())
    }

    /// 임의 record.
    pub(crate) fn raw(
        platform: u16,
        encoding: u16,
        language: u16,
        name_id: u16,
        bytes: Vec<u8>,
    ) -> NameEntry {
        NameEntry {
            platform,
            encoding,
            language,
            name_id,
            bytes,
        }
    }
}

/// family 하나의 Regular·Bold·Italic face 이름(nameID 1·2·6).
pub(crate) fn family_faces(family: &str) -> Vec<Vec<NameEntry>> {
    let ps_family: String = family.chars().filter(|c| !c.is_whitespace()).collect();
    ["Regular", "Bold", "Italic"]
        .iter()
        .map(|style| {
            vec![
                NameEntry::windows(1, family),
                NameEntry::windows(2, style),
                NameEntry::windows(6, &format!("{ps_family}-{style}")),
            ]
        })
        .collect()
}

/// name table bytes.
fn name_table(entries: &[NameEntry]) -> Vec<u8> {
    let count = entries.len() as u16;
    let string_offset = 6 + 12 * count;
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&count.to_be_bytes());
    out.extend_from_slice(&string_offset.to_be_bytes());
    let mut strings = Vec::new();
    for entry in entries {
        for value in [
            entry.platform,
            entry.encoding,
            entry.language,
            entry.name_id,
            entry.bytes.len() as u16,
            strings.len() as u16,
        ] {
            out.extend_from_slice(&value.to_be_bytes());
        }
        strings.extend_from_slice(&entry.bytes);
    }
    out.extend_from_slice(&strings);
    out
}

/// 추가 table 하나(tag, bytes).
pub(crate) type ExtraTable = ([u8; 4], Vec<u8>);

/// face 하나(offset table + table record + table data). table offset은 파일 시작 기준이므로
/// face가 놓일 위치 `base`를 받습니다.
fn face_blob(
    signature: [u8; 4],
    entries: &[NameEntry],
    extra: &[ExtraTable],
    base: u32,
) -> Vec<u8> {
    let mut tables: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    if !entries.is_empty() {
        tables.push((*b"name", name_table(entries)));
    }
    tables.extend(extra.iter().cloned());
    tables.sort_by_key(|(tag, _)| *tag);
    let mut out = Vec::new();
    out.extend_from_slice(&signature);
    out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    out.extend_from_slice(&[0u8; 6]);
    let mut data_offset = base + 12 + 16 * tables.len() as u32;
    let mut data = Vec::new();
    for (tag, bytes) in &tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&data_offset.to_be_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        data.extend_from_slice(bytes);
        data_offset += bytes.len() as u32;
    }
    out.extend_from_slice(&data);
    out
}

/// 단일 face sfnt 파일.
pub(crate) fn sfnt_face(signature: [u8; 4], entries: &[NameEntry]) -> Vec<u8> {
    face_blob(signature, entries, &[], 0)
}

/// 추가 table이 있는 단일 face sfnt 파일.
pub(crate) fn sfnt_face_with_tables(
    signature: [u8; 4],
    entries: &[NameEntry],
    extra: &[ExtraTable],
) -> Vec<u8> {
    face_blob(signature, entries, extra, 0)
}

/// `fsType`만 의미 있는 최소 OS/2 table(`len` bytes, fsType은 offset 8).
pub(crate) fn os2_table(fs_type: u16, len: usize) -> ExtraTable {
    let mut bytes = vec![0u8; len];
    if len >= 10 {
        bytes[8..10].copy_from_slice(&fs_type.to_be_bytes());
    }
    (*b"OS/2", bytes)
}

/// TrueType collection(version 1.0) 파일. face마다 name table 하나.
pub(crate) fn ttc(faces: &[Vec<NameEntry>]) -> Vec<u8> {
    let header_len = 12 + 4 * faces.len() as u32;
    let mut blobs = Vec::new();
    let mut offsets = Vec::new();
    let mut next = header_len;
    for entries in faces {
        let blob = face_blob(SIG_TRUETYPE, entries, &[], next);
        offsets.push(next);
        next += blob.len() as u32;
        blobs.push(blob);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&SIG_TTCF);
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&(faces.len() as u32).to_be_bytes());
    for offset in offsets {
        out.extend_from_slice(&offset.to_be_bytes());
    }
    for blob in blobs {
        out.extend_from_slice(&blob);
    }
    out
}

/// 모든 record가 같은 문자열 구간(offset 0, `record_len` bytes)을 가리키는 name table.
///
/// record는 Windows English US, nameID는 모두 `name_id`이고 문자열은 UTF-16BE `unit` 반복입니다.
pub(crate) fn overlapping_name_table(
    record_count: u16,
    name_id: u16,
    record_len: u16,
    unit: u16,
) -> Vec<u8> {
    let string_offset = 6 + 12 * u32::from(record_count);
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&record_count.to_be_bytes());
    out.extend_from_slice(&(string_offset as u16).to_be_bytes());
    for _ in 0..record_count {
        for value in [3u16, 1, 0x0409, name_id, record_len, 0] {
            out.extend_from_slice(&value.to_be_bytes());
        }
    }
    for _ in 0..record_len / 2 {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

/// 같은 face 하나(`extra` table)를 `count`번 가리키는 collection(모든 face offset이 같음).
pub(crate) fn ttc_repeated(extra: &[ExtraTable], count: u32) -> Vec<u8> {
    let header_len = 12 + 4 * count;
    let mut out = Vec::new();
    out.extend_from_slice(&SIG_TTCF);
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&count.to_be_bytes());
    for _ in 0..count {
        out.extend_from_slice(&header_len.to_be_bytes());
    }
    out.extend_from_slice(&face_blob(SIG_TRUETYPE, &[], extra, header_len));
    out
}

/// face `count`개가 서로 1 byte씩 어긋난 `window_len` bytes name table 구간을 가리키는 collection.
///
/// 구간이 모두 달라 재사용되지 않으므로 파일 하나의 name table 읽기 합계 상한을 검사할 때 씁니다.
/// 구간 내용은 0(빈 name table)입니다.
pub(crate) fn ttc_shifted_name_windows(count: u32, window_len: u32) -> Vec<u8> {
    const FACE_HEADER_LEN: u32 = 12 + 16;
    let header_len = 12 + 4 * count;
    let data_start = header_len + FACE_HEADER_LEN * count;
    let mut out = Vec::new();
    out.extend_from_slice(&SIG_TTCF);
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&count.to_be_bytes());
    for k in 0..count {
        out.extend_from_slice(&(header_len + FACE_HEADER_LEN * k).to_be_bytes());
    }
    for k in 0..count {
        out.extend_from_slice(&SIG_TRUETYPE);
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&[0u8; 6]);
        out.extend_from_slice(b"name");
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&(data_start + k).to_be_bytes());
        out.extend_from_slice(&window_len.to_be_bytes());
    }
    out.resize(out.len() + (window_len + count) as usize, 0);
    out
}

/// WOFF 1.0 header만 있는 파일(length = 44).
pub(crate) fn woff(flavor: [u8; 4]) -> Vec<u8> {
    let mut out = vec![0u8; 44];
    out[0..4].copy_from_slice(b"wOFF");
    out[4..8].copy_from_slice(&flavor);
    out[8..12].copy_from_slice(&44u32.to_be_bytes());
    out
}

/// WOFF2 header만 있는 단일 face 파일(length = 48, table 1개 선언).
pub(crate) fn woff2(flavor: [u8; 4]) -> Vec<u8> {
    let mut out = vec![0u8; 48];
    out[0..4].copy_from_slice(b"wOF2");
    out[4..8].copy_from_slice(&flavor);
    out[8..12].copy_from_slice(&48u32.to_be_bytes());
    out[12..14].copy_from_slice(&1u16.to_be_bytes());
    out
}

/// WOFF2 `255UInt16` 인코딩(값 범위별로 세 가지 코드 경로를 씁니다).
fn u255(value: u16) -> Vec<u8> {
    match value {
        0..=252 => vec![value as u8],
        253..=505 => vec![255, (value - 253) as u8],
        506..=761 => vec![254, (value - 506) as u8],
        _ => {
            let [hi, lo] = value.to_be_bytes();
            vec![253, hi, lo]
        }
    }
}

/// WOFF2 collection: table directory 2개(`name` null transform, `glyf` transform) + face `count`개.
pub(crate) fn woff2_collection(count: u16) -> Vec<u8> {
    let mut out = woff2(SIG_TTCF);
    out[12..14].copy_from_slice(&2u16.to_be_bytes());
    // name(known index 5, version 0 = null transform): flags, origLength.
    out.extend_from_slice(&[5, 0x20]);
    // glyf(known index 10, version 0 = transform): flags, origLength, transformLength.
    out.extend_from_slice(&[10, 0x81, 0x00, 0x40]);
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    out.extend_from_slice(&u255(count));
    for _ in 0..count {
        out.extend_from_slice(&u255(2));
        out.extend_from_slice(&SIG_TRUETYPE);
        out.extend_from_slice(&u255(0));
        out.extend_from_slice(&u255(1));
    }
    let len = out.len() as u32;
    out[8..12].copy_from_slice(&len.to_be_bytes());
    out
}
