//! sparse synthetic asset provider와 기준 장비 test data manifest (XVI.3.4).
//!
//! 프로젝트 에셋 총량 20GB(아키텍처 §5.3)는 저장소·CI에 실제 파일로 두지 않습니다. 대신
//!
//! - **provider**: 에셋 하나를 `(seed, 크기, media 종류)`로만 표현하고, 내용은 읽을 때
//!   [`BLOCK_BYTES`] 단위 block을 seed에서 즉석 생성합니다([`SyntheticAssetReader`],
//!   `Read + Seek`). 저장 공간을 쓰지 않고(sparse), 어떤 offset도 그 block만 만들어 바로 읽으며,
//!   메모리는 입력 크기와 무관하게 block 하나입니다. 같은 seed는 언제나 같은 bytes입니다.
//! - **asset set**: 총량과 seed에서 media 종류별 byte 비중·크기 범위로 에셋 목록을 결정론적으로
//!   만듭니다([`SyntheticAssetSet`]). 총 bytes는 요청값과 정확히 같습니다.
//! - **manifest**: 기준 장비(`REF-MAC`)에서 쓸 data set을 고정하는 작은 YAML 파일입니다
//!   ([`AssetManifest`], `benchmarks/fixtures/data/reference-assets-20gib.yaml`). 에셋 목록 자체가
//!   아니라 생성 매개변수, 종류별 개수·bytes, 전체 목록 digest를 담습니다. 장비에서는 manifest를
//!   검증([`AssetManifest::verify`])해 같은 목록을 재생성한 뒤 stream으로 import하거나
//!   [`materialize`]로 파일을 씁니다.
//!
//! 20GB는 보수적으로 20 GiB(21,474,836,480 bytes)로 해석합니다. 내용은 media 종류의 magic bytes로
//! 시작하지만 나머지는 난수라 **디코딩 가능한 이미지·영상이 아닙니다** — I/O·hash·복사 경로
//! benchmark용이며, 디코딩 benchmark에는 별도 실제 sample이 필요합니다.

use std::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use stellar_assets::hash_stream::{hash_reader_with, StreamDigest, STREAM_BUFFER_SIZE};
use stellar_project::yaml::{load_restricted, write_canonical_checked, YamlValue};
use stellar_storage::hash::ContentHash;

use crate::rng::SplitMix64;
use crate::GENERATOR_VERSION;

/// 내용 생성 block 크기(bytes). reader가 한 번에 돌려주는 최대 bytes이기도 합니다.
pub const BLOCK_BYTES: usize = 64 * 1024;

/// 1 GiB.
pub const GIB: u64 = 1024 * 1024 * 1024;

/// 기준 프로젝트 에셋 총량: 20 GiB.
pub const REFERENCE_TOTAL_BYTES: u64 = 20 * GIB;

/// 기준 manifest의 seed.
pub const REFERENCE_SEED: u64 = 1;

/// asset set 총량 상한(오입력 방지): 1 TiB.
pub const MAX_TOTAL_BYTES: u64 = 1024 * GIB;

/// asset set 에셋 수 상한.
pub const MAX_ASSETS: usize = 200_000;

/// manifest schema 이름.
pub const MANIFEST_SCHEMA: &str = "stellar-bench-assets/1";

/// manifest가 대상으로 하는 실행 환경 코드.
pub const MANIFEST_ENVIRONMENT: &str = "REF-MAC";

/// 기준 manifest 파일 경로(crate root 기준).
pub const REFERENCE_MANIFEST_PATH: &str = "data/reference-assets-20gib.yaml";

/// media 종류(class).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetClass {
    /// 안정 이름(경로·manifest 키).
    pub name: &'static str,
    /// media type.
    pub media_type: &'static str,
    /// 파일 확장자.
    pub extension: &'static str,
    /// 총량 중 비중(‰). 모든 class 합은 1000입니다.
    pub share_permille: u64,
    /// 크기 하한(bytes, 마지막 꼬리 에셋은 예외로 더 작을 수 있음).
    pub min_bytes: u64,
    /// 크기 상한(bytes).
    pub max_bytes: u64,
    /// 내용 앞의 magic bytes.
    pub magic: &'static [u8],
}

/// 기준 class 구성: UI 스크린샷·아이콘 PNG, 사진 JPEG, 벡터 SVG, 폰트, 영상.
pub const ASSET_CLASSES: [AssetClass; 5] = [
    AssetClass {
        name: "png",
        media_type: "image/png",
        extension: "png",
        share_permille: 300,
        min_bytes: 16 * 1024,
        max_bytes: 8 * 1024 * 1024,
        magic: &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
    },
    AssetClass {
        name: "jpeg",
        media_type: "image/jpeg",
        extension: "jpg",
        share_permille: 350,
        min_bytes: 256 * 1024,
        max_bytes: 24 * 1024 * 1024,
        magic: &[0xFF, 0xD8, 0xFF, 0xE0],
    },
    AssetClass {
        name: "svg",
        media_type: "image/svg+xml",
        extension: "svg",
        share_permille: 10,
        min_bytes: 512,
        max_bytes: 256 * 1024,
        magic: b"<svg xmlns=\"http://www.w3.org/2000/svg\">",
    },
    AssetClass {
        name: "font",
        media_type: "font/otf",
        extension: "otf",
        share_permille: 40,
        min_bytes: 32 * 1024,
        max_bytes: 16 * 1024 * 1024,
        magic: b"OTTO",
    },
    AssetClass {
        name: "video",
        media_type: "video/mp4",
        extension: "mp4",
        share_permille: 300,
        min_bytes: 16 * 1024 * 1024,
        max_bytes: GIB,
        magic: &[
            0, 0, 0, 0x18, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm',
        ],
    },
];

/// asset provider·manifest 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetFixtureError {
    /// 명세가 범위를 벗어납니다.
    InvalidSpec {
        /// 이유.
        reason: &'static str,
    },
    /// manifest 형식 오류(제한 YAML 해석 실패 포함).
    Manifest {
        /// 이유.
        reason: String,
    },
    /// 내용 stream 읽기·hash 실패.
    Stream {
        /// 이유.
        reason: String,
    },
    /// manifest가 현재 generator로 재생성한 목록과 다릅니다.
    Mismatch {
        /// 다른 항목.
        field: &'static str,
    },
}

impl fmt::Display for AssetFixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetFixtureError::InvalidSpec { reason } => write!(f, "asset 명세 오류: {reason}"),
            AssetFixtureError::Manifest { reason } => write!(f, "asset manifest 오류: {reason}"),
            AssetFixtureError::Stream { reason } => write!(f, "asset stream 오류: {reason}"),
            AssetFixtureError::Mismatch { field } => {
                write!(f, "asset manifest가 재생성 결과와 다릅니다: {field}")
            }
        }
    }
}

impl std::error::Error for AssetFixtureError {}

fn manifest_error(reason: impl Into<String>) -> AssetFixtureError {
    AssetFixtureError::Manifest {
        reason: reason.into(),
    }
}

/// synthetic 에셋 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticAsset {
    /// project 상대 경로(`assets/<class>/<번호>.<ext>`).
    pub path: String,
    /// class index([`ASSET_CLASSES`]).
    pub class: usize,
    /// 크기(bytes).
    pub size: u64,
    /// 내용 seed.
    pub seed: u64,
}

impl SyntheticAsset {
    /// class.
    pub fn class(&self) -> &'static AssetClass {
        &ASSET_CLASSES[self.class.min(ASSET_CLASSES.len() - 1)]
    }

    /// media type.
    pub fn media_type(&self) -> &'static str {
        self.class().media_type
    }

    /// 내용 reader(처음부터).
    pub fn reader(&self) -> SyntheticAssetReader {
        SyntheticAssetReader {
            seed: self.seed,
            size: self.size,
            magic: self.class().magic,
            pos: 0,
            cached: None,
            block: vec![0u8; BLOCK_BYTES],
        }
    }
}

/// synthetic 에셋 내용 stream. 메모리는 block 하나([`BLOCK_BYTES`])로 고정입니다.
#[derive(Debug, Clone)]
pub struct SyntheticAssetReader {
    seed: u64,
    size: u64,
    magic: &'static [u8],
    pos: u64,
    cached: Option<u64>,
    block: Vec<u8>,
}

impl SyntheticAssetReader {
    /// 전체 크기.
    pub fn len(&self) -> u64 {
        self.size
    }

    /// 크기 0인지.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// 현재 위치.
    pub fn position(&self) -> u64 {
        self.pos
    }

    /// 내부 buffer 크기(입력 크기와 무관).
    pub fn buffer_bytes(&self) -> usize {
        self.block.len()
    }

    fn fill(&mut self, index: u64) {
        let mut rng = SplitMix64::new(self.seed ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        for chunk in self.block.chunks_mut(8) {
            let bytes = rng.next_u64().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
        if index == 0 {
            let n = self.magic.len().min(self.block.len());
            self.block[..n].copy_from_slice(&self.magic[..n]);
        }
        self.cached = Some(index);
    }
}

impl Read for SyntheticAssetReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.size || buf.is_empty() {
            return Ok(0);
        }
        let block = BLOCK_BYTES as u64;
        let index = self.pos / block;
        if self.cached != Some(index) {
            self.fill(index);
        }
        let offset = (self.pos % block) as usize;
        let available = (BLOCK_BYTES - offset) as u64;
        let n = (buf.len() as u64).min(available).min(self.size - self.pos) as usize;
        buf[..n].copy_from_slice(&self.block[offset..offset + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for SyntheticAssetReader {
    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        let (base, delta) = match target {
            SeekFrom::Start(offset) => {
                self.pos = offset;
                return Ok(offset);
            }
            SeekFrom::End(delta) => (self.size, delta),
            SeekFrom::Current(delta) => (self.pos, delta),
        };
        let next = if delta >= 0 {
            base.checked_add(delta.unsigned_abs())
        } else {
            base.checked_sub(delta.unsigned_abs())
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "잘못된 seek 위치"))?;
        self.pos = next;
        Ok(next)
    }
}

/// class별 요약.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassSummary {
    /// class 이름.
    pub name: String,
    /// media type.
    pub media_type: String,
    /// 에셋 수.
    pub count: u64,
    /// bytes 합.
    pub total_bytes: u64,
    /// 가장 큰 에셋 bytes.
    pub largest_bytes: u64,
}

/// 총량·seed에서 만든 에셋 목록.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticAssetSet {
    /// seed.
    pub seed: u64,
    /// 요청 총량(= 실제 합).
    pub total_bytes: u64,
    /// 에셋(class 순, class 안에서는 번호 순).
    pub assets: Vec<SyntheticAsset>,
}

impl SyntheticAssetSet {
    /// `seed`와 총량으로 목록을 만듭니다. class 비중대로 나누고 마지막 class가 나머지를 받아 합이
    /// 정확히 `total_bytes`입니다.
    pub fn generate(seed: u64, total_bytes: u64) -> Result<SyntheticAssetSet, AssetFixtureError> {
        if total_bytes == 0 || total_bytes > MAX_TOTAL_BYTES {
            return Err(AssetFixtureError::InvalidSpec {
                reason: "총량은 1 byte..=1 TiB여야 합니다",
            });
        }
        let mut rng = SplitMix64::new(seed ^ 0xA55E_7000_0000_0008);
        let mut assets = Vec::new();
        let mut assigned = 0u64;
        for (class_index, class) in ASSET_CLASSES.iter().enumerate() {
            let budget = if class_index + 1 == ASSET_CLASSES.len() {
                total_bytes - assigned
            } else {
                ((u128::from(total_bytes) * u128::from(class.share_permille)) / 1000) as u64
            };
            assigned += budget;
            let mut stream = rng.fork(class_index as u64);
            let mut remaining = budget;
            let mut number = 0u64;
            while remaining > 0 {
                if assets.len() >= MAX_ASSETS {
                    return Err(AssetFixtureError::InvalidSpec {
                        reason: "에셋 수가 상한을 넘습니다",
                    });
                }
                let size = if remaining <= class.max_bytes {
                    remaining
                } else {
                    let drawn = stream.range(class.min_bytes, class.max_bytes);
                    // 남는 꼬리가 하한보다 작아지지 않게 합니다.
                    if remaining - drawn < class.min_bytes {
                        remaining - class.min_bytes
                    } else {
                        drawn
                    }
                };
                number += 1;
                assets.push(SyntheticAsset {
                    path: format!("assets/{}/{number:06}.{}", class.name, class.extension),
                    class: class_index,
                    size,
                    seed: stream.next_u64(),
                });
                remaining -= size;
            }
        }
        Ok(SyntheticAssetSet {
            seed,
            total_bytes,
            assets,
        })
    }

    /// 기준 장비용 20 GiB set.
    pub fn reference() -> Result<SyntheticAssetSet, AssetFixtureError> {
        SyntheticAssetSet::generate(REFERENCE_SEED, REFERENCE_TOTAL_BYTES)
    }

    /// 실제 bytes 합.
    pub fn sum_bytes(&self) -> u64 {
        self.assets.iter().map(|a| a.size).sum()
    }

    /// class별 요약([`ASSET_CLASSES`] 순).
    pub fn classes(&self) -> Vec<ClassSummary> {
        ASSET_CLASSES
            .iter()
            .enumerate()
            .map(|(i, class)| {
                let members = self.assets.iter().filter(|a| a.class == i);
                let mut summary = ClassSummary {
                    name: class.name.to_string(),
                    media_type: class.media_type.to_string(),
                    count: 0,
                    total_bytes: 0,
                    largest_bytes: 0,
                };
                for asset in members {
                    summary.count += 1;
                    summary.total_bytes += asset.size;
                    summary.largest_bytes = summary.largest_bytes.max(asset.size);
                }
                summary
            })
            .collect()
    }

    /// 전체 목록(경로·media type·크기·seed)의 digest.
    pub fn entries_digest(&self) -> ContentHash {
        let mut buf = Vec::with_capacity(self.assets.len() * 64 + 32);
        buf.extend_from_slice(b"stellar/bench-assets/v1\0");
        buf.extend_from_slice(&GENERATOR_VERSION.to_le_bytes());
        buf.extend_from_slice(&(BLOCK_BYTES as u64).to_le_bytes());
        for asset in &self.assets {
            buf.extend_from_slice(&(asset.path.len() as u64).to_le_bytes());
            buf.extend_from_slice(asset.path.as_bytes());
            let media = asset.media_type().as_bytes();
            buf.extend_from_slice(&(media.len() as u64).to_le_bytes());
            buf.extend_from_slice(media);
            buf.extend_from_slice(&asset.size.to_le_bytes());
            buf.extend_from_slice(&asset.seed.to_le_bytes());
        }
        ContentHash::compute(&buf)
    }

    /// 기준 장비 manifest.
    pub fn manifest(&self) -> AssetManifest {
        AssetManifest {
            generator_version: GENERATOR_VERSION,
            environment: MANIFEST_ENVIRONMENT.to_string(),
            seed: self.seed,
            total_bytes: self.total_bytes,
            block_bytes: BLOCK_BYTES as u64,
            assets: self.assets.len() as u64,
            classes: self.classes(),
            entries_digest: self.entries_digest(),
        }
    }
}

/// 에셋 내용을 import 경로와 같은 streaming hasher(`stellar_assets`, XIII.5.2)와 같은 buffer
/// 크기로 끝까지 읽어 content hash와 크기를 냅니다. 메모리는 reader block + 읽기 buffer뿐입니다.
pub fn stream_digest(asset: &SyntheticAsset) -> Result<StreamDigest, AssetFixtureError> {
    let mut reader = asset.reader();
    let mut buffer = vec![0u8; STREAM_BUFFER_SIZE];
    hash_reader_with(&mut reader, &mut buffer, |_| Ok(())).map_err(|e| AssetFixtureError::Stream {
        reason: format!("{e:?}"),
    })
}

/// 에셋 하나를 `dir/<asset.path>`에 stream으로 씁니다(메모리 block 하나). 쓴 경로를 반환합니다.
///
/// 경로 component는 ASCII 영숫자·`.`·`_`·`-`만 허용하고 `..`·절대 경로를 거부합니다.
pub fn materialize(asset: &SyntheticAsset, dir: &Path) -> io::Result<PathBuf> {
    let mut target = dir.to_path_buf();
    for component in asset.path.split('/') {
        let safe = !component.is_empty()
            && component != "."
            && component != ".."
            && component
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if !safe {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "안전하지 않은 에셋 경로",
            ));
        }
        target.push(component);
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(&target)?;
    let mut reader = asset.reader();
    let mut buf = vec![0u8; BLOCK_BYTES];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
    }
    file.sync_all()?;
    Ok(target)
}

/// set 전체를 `dir`에 파일로 씁니다(기준 장비 import benchmark 준비). 쓴 bytes 합을 반환합니다.
///
/// 기존 파일을 덮어쓰지 않도록 `dir`은 없거나 비어 있어야 합니다.
pub fn materialize_set(set: &SyntheticAssetSet, dir: &Path) -> Result<u64, AssetFixtureError> {
    let io_error = |e: io::Error| AssetFixtureError::Stream {
        reason: e.to_string(),
    };
    if dir.exists() {
        let mut entries = std::fs::read_dir(dir).map_err(io_error)?;
        if entries.next().is_some() {
            return Err(AssetFixtureError::InvalidSpec {
                reason: "출력 디렉터리가 비어 있지 않습니다",
            });
        }
    }
    std::fs::create_dir_all(dir).map_err(io_error)?;
    let mut written = 0u64;
    for asset in &set.assets {
        let path = materialize(asset, dir).map_err(io_error)?;
        written += std::fs::metadata(&path).map_err(io_error)?.len();
    }
    Ok(written)
}

/// 기준 장비 test data manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetManifest {
    /// generator version.
    pub generator_version: u32,
    /// 실행 환경 코드.
    pub environment: String,
    /// seed.
    pub seed: u64,
    /// 총량(bytes).
    pub total_bytes: u64,
    /// 내용 block 크기.
    pub block_bytes: u64,
    /// 에셋 수.
    pub assets: u64,
    /// class별 요약.
    pub classes: Vec<ClassSummary>,
    /// 전체 목록 digest.
    pub entries_digest: ContentHash,
}

fn scalar(value: impl ToString) -> YamlValue {
    YamlValue::scalar(value.to_string())
}

fn mapping(pairs: Vec<(&str, YamlValue)>) -> YamlValue {
    YamlValue::Mapping(pairs.into_iter().map(|(k, v)| (scalar(k), v)).collect())
}

/// 10진 u64(부호·밑줄·공백·불필요한 선행 0 금지).
fn parse_u64(text: &str, field: &str) -> Result<u64, AssetFixtureError> {
    let canonical = !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    if !canonical {
        return Err(manifest_error(format!("{field}: 10진 정수가 아닙니다")));
    }
    text.parse::<u64>()
        .map_err(|_| manifest_error(format!("{field}: u64 범위를 넘습니다")))
}

/// 매핑의 키 집합이 정확히 `keys`인지 확인합니다(누락·추가 키 거부).
fn expect_keys<'a>(
    value: &'a YamlValue,
    keys: &[&str],
    context: &str,
) -> Result<&'a [(YamlValue, YamlValue)], AssetFixtureError> {
    let pairs = value
        .as_mapping()
        .ok_or_else(|| manifest_error(format!("{context}: 매핑이 아닙니다")))?;
    if pairs.len() != keys.len() {
        return Err(manifest_error(format!("{context}: 키 수가 다릅니다")));
    }
    for (pair, expected) in pairs.iter().zip(keys) {
        if pair.0.as_scalar() != Some(*expected) {
            return Err(manifest_error(format!(
                "{context}: 키 순서·이름이 다릅니다({expected} 기대)"
            )));
        }
    }
    Ok(pairs)
}

fn text_of<'a>(value: &'a YamlValue, field: &str) -> Result<&'a str, AssetFixtureError> {
    value
        .as_scalar()
        .ok_or_else(|| manifest_error(format!("{field}: 스칼라가 아닙니다")))
}

const TOP_KEYS: [&str; 10] = [
    "schema",
    "generator_version",
    "environment",
    "seed",
    "total_bytes",
    "block_bytes",
    "assets",
    "classes",
    "entries_digest",
    "materialize",
];

const CLASS_KEYS: [&str; 5] = [
    "name",
    "media_type",
    "count",
    "total_bytes",
    "largest_bytes",
];

/// manifest의 재현 안내(장비에서 실행할 명령).
const MATERIALIZE_NOTE: &str =
    "cargo run -p stellar-bench-fixtures --release -- assets-20gib --out <dir>";

impl AssetManifest {
    /// canonical YAML 텍스트(제한 writer, 다시 읽힘을 확인).
    pub fn to_yaml(&self) -> Result<String, AssetFixtureError> {
        let classes = self
            .classes
            .iter()
            .map(|c| {
                mapping(vec![
                    ("name", scalar(&c.name)),
                    ("media_type", scalar(&c.media_type)),
                    ("count", scalar(c.count)),
                    ("total_bytes", scalar(c.total_bytes)),
                    ("largest_bytes", scalar(c.largest_bytes)),
                ])
            })
            .collect();
        let value = mapping(vec![
            ("schema", scalar(MANIFEST_SCHEMA)),
            ("generator_version", scalar(self.generator_version)),
            ("environment", scalar(&self.environment)),
            ("seed", scalar(self.seed)),
            ("total_bytes", scalar(self.total_bytes)),
            ("block_bytes", scalar(self.block_bytes)),
            ("assets", scalar(self.assets)),
            ("classes", YamlValue::Sequence(classes)),
            ("entries_digest", scalar(self.entries_digest)),
            ("materialize", scalar(MATERIALIZE_NOTE)),
        ]);
        write_canonical_checked(&value).map_err(|e| manifest_error(e.to_string()))
    }

    /// manifest 텍스트를 엄격하게 해석합니다(제한 YAML, 정확한 키 집합·순서, 10진 정수).
    pub fn parse(text: &str) -> Result<AssetManifest, AssetFixtureError> {
        let value = load_restricted(text).map_err(|e| manifest_error(e.to_string()))?;
        let top = expect_keys(&value, &TOP_KEYS, "manifest")?;
        let field = |i: usize| &top[i].1;
        if text_of(field(0), "schema")? != MANIFEST_SCHEMA {
            return Err(manifest_error("schema가 다릅니다"));
        }
        let generator_version = u32::try_from(parse_u64(
            text_of(field(1), "generator_version")?,
            "generator_version",
        )?)
        .map_err(|_| manifest_error("generator_version: u32 범위를 넘습니다"))?;
        let environment = text_of(field(2), "environment")?.to_string();
        let seed = parse_u64(text_of(field(3), "seed")?, "seed")?;
        let total_bytes = parse_u64(text_of(field(4), "total_bytes")?, "total_bytes")?;
        let block_bytes = parse_u64(text_of(field(5), "block_bytes")?, "block_bytes")?;
        let assets = parse_u64(text_of(field(6), "assets")?, "assets")?;
        let class_values = field(7)
            .as_sequence()
            .ok_or_else(|| manifest_error("classes: 시퀀스가 아닙니다"))?;
        if class_values.len() > ASSET_CLASSES.len() {
            return Err(manifest_error("classes: 항목이 너무 많습니다"));
        }
        let mut classes = Vec::with_capacity(class_values.len());
        for class in class_values {
            let pairs = expect_keys(class, &CLASS_KEYS, "class")?;
            classes.push(ClassSummary {
                name: text_of(&pairs[0].1, "class.name")?.to_string(),
                media_type: text_of(&pairs[1].1, "class.media_type")?.to_string(),
                count: parse_u64(text_of(&pairs[2].1, "class.count")?, "class.count")?,
                total_bytes: parse_u64(
                    text_of(&pairs[3].1, "class.total_bytes")?,
                    "class.total_bytes",
                )?,
                largest_bytes: parse_u64(
                    text_of(&pairs[4].1, "class.largest_bytes")?,
                    "class.largest_bytes",
                )?,
            });
        }
        let entries_digest = text_of(field(8), "entries_digest")?
            .parse::<ContentHash>()
            .map_err(|_| manifest_error("entries_digest: content hash가 아닙니다"))?;
        if text_of(field(9), "materialize")? != MATERIALIZE_NOTE {
            return Err(manifest_error("materialize 안내가 다릅니다"));
        }
        Ok(AssetManifest {
            generator_version,
            environment,
            seed,
            total_bytes,
            block_bytes,
            assets,
            classes,
            entries_digest,
        })
    }

    /// 현재 generator로 같은 목록을 재생성해 manifest와 모든 항목이 같은지 확인하고 그 목록을
    /// 반환합니다. generator version·block 크기·digest가 다르면 거부합니다.
    pub fn verify(&self) -> Result<SyntheticAssetSet, AssetFixtureError> {
        if self.generator_version != GENERATOR_VERSION {
            return Err(AssetFixtureError::Mismatch {
                field: "generator_version",
            });
        }
        if self.block_bytes != BLOCK_BYTES as u64 {
            return Err(AssetFixtureError::Mismatch {
                field: "block_bytes",
            });
        }
        if self.environment != MANIFEST_ENVIRONMENT {
            return Err(AssetFixtureError::Mismatch {
                field: "environment",
            });
        }
        let set = SyntheticAssetSet::generate(self.seed, self.total_bytes)?;
        let expected = set.manifest();
        if expected.assets != self.assets {
            return Err(AssetFixtureError::Mismatch { field: "assets" });
        }
        if expected.classes != self.classes {
            return Err(AssetFixtureError::Mismatch { field: "classes" });
        }
        if expected.entries_digest != self.entries_digest {
            return Err(AssetFixtureError::Mismatch {
                field: "entries_digest",
            });
        }
        Ok(set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_shares_sum_to_one() {
        assert_eq!(
            ASSET_CLASSES.iter().map(|c| c.share_permille).sum::<u64>(),
            1000
        );
        for class in ASSET_CLASSES {
            assert!(class.min_bytes * 2 <= class.max_bytes, "{}", class.name);
        }
    }

    #[test]
    fn strict_integer_parsing() {
        assert_eq!(parse_u64("0", "x"), Ok(0));
        assert_eq!(parse_u64("123", "x"), Ok(123));
        for bad in [
            "",
            "-1",
            "+1",
            "01",
            "1_000",
            " 1",
            "1e3",
            "18446744073709551616",
        ] {
            assert!(parse_u64(bad, "x").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn seek_rejects_negative_positions() {
        let asset = SyntheticAsset {
            path: "assets/png/000001.png".into(),
            class: 0,
            size: 10,
            seed: 1,
        };
        let mut reader = asset.reader();
        assert!(reader.seek(SeekFrom::Current(-1)).is_err());
        assert_eq!(reader.seek(SeekFrom::End(-3)).unwrap(), 7);
        assert_eq!(reader.seek(SeekFrom::End(5)).unwrap(), 15);
        let mut buf = [0u8; 4];
        assert_eq!(reader.read(&mut buf).unwrap(), 0, "끝 너머는 EOF");
    }
}
