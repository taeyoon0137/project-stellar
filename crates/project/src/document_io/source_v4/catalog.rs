//! 텍스트 asset catalog(TS-ASSET, ADR-0077 결정 1·3).
//!
//! `design/assets/catalog.stellar.yaml` 한 파일이 이미지 asset record, 등록 글꼴(ProjectFontRecord와 그
//! AssetRecord), 문서별 Text style 표의 원본입니다. 문서·View·컴포넌트 원본은 여기 항목을 StableId·file hash·
//! style index로 참조만 합니다. catalog 밖에 같은 record를 복제하지 않습니다.
//!
//! - 이미지: asset StableId, content hash, media type, 크기, 출처(`none`, `project <상대 경로>`,
//!   `external <출처 지문> <파일 이름>`).
//! - 글꼴: file hash, 글꼴 asset id, 형식(media type), face 이름, 라이선스 상태·필드, 글꼴 파일 asset record.
//! - Text style 표: 문서 id, 기본 typography·글자색, typography resource 전체(CSS longhand 일곱 개), 문단 style,
//!   문자 style(OpenType feature·variation 포함), 문서가 등록한 글꼴 file hash 목록.
//!
//! 보존한 unknown optional field·고정 face를 지정한 family·해석할 수 없는 typography bytes는
//! 텍스트로 정확히 복원할 수 없어 쓰기를 거부합니다(결정 3). 읽은 값은 binary record로 다시 인코딩해 원본 bytes와
//! 같은지 확인합니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_assets::id::AssetId;
use stellar_assets::media_type::MediaType;
use stellar_assets::record::{AssetRecord, SourceLocation, SourceMetadata};
use stellar_core_types::font::FontFileHash;
use stellar_core_types::ids::StableId;
use stellar_document::text_styles::{DocumentFontAsset, DocumentTextStyles};
use stellar_storage::hash::ContentHash;

use super::super::text_codec::{field, fields, mapping, scalar, text};
use super::super::text_source::TextSourceError;
use super::css::{self, invalid, unsupported};
use super::styles;
use crate::fonts::license::{FontLicenseInfo, FontLicenseState, LicenseChangeOrigin, LicenseField};
use crate::fonts::project_font::ProjectFontRecord;
use crate::fonts::sfnt::{FontFaceNames, FontFileFormat};
use crate::yaml::{load_restricted, write_canonical_checked, YamlValue};

/// catalog 파일 형식 이름.
pub const CATALOG_FORMAT: &str = "stellar-asset-catalog";
/// catalog 쓰기 버전.
pub const CATALOG_VERSION: &str = "4";
/// `design/` 기준 catalog 경로.
pub const CATALOG_PATH: &str = "assets/catalog.stellar.yaml";

/// catalog가 담는 프로젝트 범위 record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AssetCatalog {
    /// asset StableId → 이미지 record.
    pub images: BTreeMap<StableId, AssetRecord>,
    /// 문서 StableId → 그 문서의 Text style 표(등록 글꼴 포함).
    pub text_styles: BTreeMap<StableId, DocumentTextStyles>,
}

fn line(value: impl Into<String>) -> YamlValue {
    YamlValue::scalar(value.into())
}

fn get<'a>(value: &'a YamlValue, key: &str) -> Result<&'a str, TextSourceError> {
    text(field(value, key)?)
}

fn sequence<'a>(value: &'a YamlValue, key: &str) -> Result<&'a [YamlValue], TextSourceError> {
    field(value, key)?
        .as_sequence()
        .ok_or_else(|| invalid(format!("{key}는 sequence여야 합니다")))
}

fn optional_text(value: Option<&str>) -> YamlValue {
    line(value.unwrap_or("none"))
}

/// `none`이면 값 없음. 실제 값이 문자열 `none`인 field는 표현할 수 없어 쓰기 전에 거부합니다.
fn checked_optional(value: Option<&str>, what: &str) -> Result<YamlValue, TextSourceError> {
    if value == Some("none") {
        return Err(unsupported(format!(
            "{what} 값 none은 값 없음과 구분할 수 없습니다"
        )));
    }
    Ok(optional_text(value))
}

fn parse_optional_text(value: &str) -> Option<String> {
    (value != "none").then(|| value.to_owned())
}

// --- asset record ---

fn asset_record_fields(
    record: &AssetRecord,
) -> Result<Vec<(YamlValue, YamlValue)>, TextSourceError> {
    if !record.preserved_fields().is_empty() {
        return Err(unsupported(
            "asset record의 보존 unknown field는 텍스트로 쓰지 않습니다",
        ));
    }
    let source = match &record.source_metadata {
        None => "none".to_owned(),
        Some(source) => {
            if !source.preserved_fields().is_empty() {
                return Err(unsupported(
                    "출처 metadata의 보존 unknown field는 텍스트로 쓰지 않습니다",
                ));
            }
            match source.location() {
                SourceLocation::Project { relative_path } => format!("project {relative_path}"),
                SourceLocation::External { file_name, origin } => {
                    format!("external {origin} {file_name}")
                }
            }
        }
    };
    Ok(vec![
        (
            scalar("content-hash"),
            line(record.content_hash.to_string()),
        ),
        (scalar("media-type"), line(record.media_type.as_str())),
        (scalar("size"), line(record.size.to_string())),
        (scalar("source"), line(source)),
    ])
}

const ASSET_RECORD_KEYS: [&str; 4] = ["content-hash", "media-type", "size", "source"];

fn parse_asset_record(value: &YamlValue) -> Result<AssetRecord, TextSourceError> {
    let hash_text = get(value, "content-hash")?;
    let content_hash: ContentHash = hash_text
        .parse()
        .map_err(|_| invalid(format!("content hash 형식: {hash_text}")))?;
    if content_hash.to_string() != hash_text {
        return Err(invalid("content hash는 소문자 canonical 표기여야 합니다"));
    }
    let size_text = get(value, "size")?;
    let size: u64 = size_text
        .parse()
        .ok()
        .filter(|n: &u64| n.to_string() == size_text)
        .ok_or_else(|| invalid(format!("size 형식: {size_text}")))?;
    let source = match get(value, "source")? {
        "none" => None,
        other => {
            if let Some(path) = other.strip_prefix("project ") {
                Some(SourceMetadata::project(path).map_err(invalid)?)
            } else if let Some(rest) = other.strip_prefix("external ") {
                let (origin, file_name) = rest
                    .split_once(' ')
                    .ok_or_else(|| invalid("외부 출처는 external <지문> <파일 이름>입니다"))?;
                let origin: stellar_assets::origin::OriginFingerprint =
                    origin.parse().map_err(|_| invalid("외부 출처 지문 형식"))?;
                Some(SourceMetadata::external(file_name, origin).map_err(invalid)?)
            } else {
                return Err(unsupported(format!("asset 출처 {other}")));
            }
        }
    };
    Ok(AssetRecord::new(
        content_hash,
        MediaType::new(get(value, "media-type")?).map_err(invalid)?,
        size,
        source,
    ))
}

fn image(id: StableId, record: &AssetRecord) -> Result<YamlValue, TextSourceError> {
    let mut out = vec![(scalar("id"), line(css::id(id)))];
    out.extend(asset_record_fields(record)?);
    Ok(YamlValue::Mapping(out))
}

fn parse_image(value: &YamlValue) -> Result<(StableId, AssetRecord), TextSourceError> {
    let mut keys = vec!["id"];
    keys.extend(ASSET_RECORD_KEYS);
    fields(value, &keys)?;
    Ok((
        css::parse_id(get(value, "id")?)?,
        parse_asset_record(value)?,
    ))
}

// --- 글꼴 ---

const LICENSE_FIELDS: [LicenseField; 4] = [
    LicenseField::Name,
    LicenseField::Url,
    LicenseField::Copyright,
    LicenseField::Notes,
];

fn font(hash: FontFileHash, asset: &DocumentFontAsset) -> Result<YamlValue, TextSourceError> {
    let record = ProjectFontRecord::decode(&asset.font_record)
        .map_err(|e| unsupported(format!("해석할 수 없는 글꼴 record: {e}")))?;
    let asset_record = AssetRecord::decode(&asset.asset_record)
        .map_err(|e| unsupported(format!("해석할 수 없는 글꼴 asset record: {e}")))?;
    if record.file_hash() != hash {
        return Err(invalid("글꼴 record의 file hash가 표 key와 다릅니다"));
    }
    let faces = record
        .faces()
        .iter()
        .map(|face| -> Result<YamlValue, TextSourceError> {
            Ok(mapping([
                ("family", checked_optional(face.family(), "family")?),
                (
                    "subfamily",
                    checked_optional(face.subfamily(), "subfamily")?,
                ),
                (
                    "postscript",
                    checked_optional(face.postscript(), "postscript")?,
                ),
            ]))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = vec![
        (scalar("file-hash"), line(hash.to_string())),
        (
            scalar("asset-id"),
            line(css::id(record.asset_id().stable_id())),
        ),
        (scalar("format"), line(record.format().media_type())),
        (scalar("faces"), YamlValue::Sequence(faces)),
        (scalar("license-state"), line(record.license_state().name())),
    ];
    for field in LICENSE_FIELDS {
        out.push((
            scalar(field.name()),
            checked_optional(record.license().get(field), field.name())?,
        ));
    }
    out.extend(asset_record_fields(&asset_record)?);
    let value = YamlValue::Mapping(out);
    // 텍스트 값으로 다시 만든 record bytes가 원본과 같아야 합니다(손실 없음).
    let (_, back) = parse_font(&value)?;
    if &back != asset {
        return Err(unsupported(
            "글꼴 record를 텍스트 값으로 정확히 복원할 수 없습니다",
        ));
    }
    Ok(value)
}

fn parse_font(value: &YamlValue) -> Result<(FontFileHash, DocumentFontAsset), TextSourceError> {
    let mut keys = vec!["file-hash", "asset-id", "format", "faces", "license-state"];
    keys.extend(LICENSE_FIELDS.map(LicenseField::name));
    keys.extend(ASSET_RECORD_KEYS);
    fields(value, &keys)?;
    let hash_text = get(value, "file-hash")?;
    let hash: FontFileHash = hash_text
        .parse()
        .map_err(|_| invalid(format!("글꼴 file hash 형식: {hash_text}")))?;
    if hash.to_string() != hash_text {
        return Err(invalid("글꼴 file hash는 canonical 표기여야 합니다"));
    }
    let format_text = get(value, "format")?;
    let format = FontFileFormat::ALL
        .into_iter()
        .find(|format| format.media_type() == format_text)
        .ok_or_else(|| unsupported(format!("글꼴 형식 {format_text}")))?;
    let faces = sequence(value, "faces")?
        .iter()
        .map(|face| {
            fields(face, &["family", "subfamily", "postscript"])?;
            FontFaceNames::new(
                parse_optional_text(get(face, "family")?),
                parse_optional_text(get(face, "subfamily")?),
                parse_optional_text(get(face, "postscript")?),
            )
            .map_err(invalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut license = FontLicenseInfo::new();
    for field in LICENSE_FIELDS {
        license
            .set(field, parse_optional_text(get(value, field.name())?))
            .map_err(invalid)?;
    }
    let state_text = get(value, "license-state")?;
    let state = FontLicenseState::ALL
        .into_iter()
        .find(|state| state.name() == state_text)
        .ok_or_else(|| unsupported(format!("글꼴 라이선스 상태 {state_text}")))?;
    license
        .change_state(state, LicenseChangeOrigin::User)
        .map_err(invalid)?;
    let record = ProjectFontRecord::new(
        AssetId::from_stable_id(css::parse_id(get(value, "asset-id")?)?),
        hash,
        format,
        faces,
        license,
    )
    .map_err(invalid)?;
    let asset_record = parse_asset_record(value)?;
    Ok((
        hash,
        DocumentFontAsset {
            font_record: record.encode().map_err(invalid)?,
            asset_record: asset_record.encode().map_err(invalid)?,
        },
    ))
}

/// catalog를 canonical 텍스트로 씁니다. 문서 사이에 같은 file hash의 다른 글꼴 record가 있으면 거부합니다.
pub fn encode(catalog: &AssetCatalog) -> Result<String, TextSourceError> {
    let mut fonts: BTreeMap<FontFileHash, &DocumentFontAsset> = BTreeMap::new();
    for table in catalog.text_styles.values() {
        for (hash, asset) in &table.fonts {
            if fonts.insert(*hash, asset).is_some_and(|prev| prev != asset) {
                return Err(unsupported(format!(
                    "문서마다 다른 글꼴 record를 가진 file hash {hash}"
                )));
            }
        }
    }
    let value = mapping([
        ("format", line(CATALOG_FORMAT)),
        ("schema_version", line(CATALOG_VERSION)),
        (
            "images",
            YamlValue::Sequence(
                catalog
                    .images
                    .iter()
                    .map(|(id, record)| image(*id, record))
                    .collect::<Result<_, _>>()?,
            ),
        ),
        (
            "fonts",
            YamlValue::Sequence(
                fonts
                    .iter()
                    .map(|(hash, asset)| font(*hash, asset))
                    .collect::<Result<_, _>>()?,
            ),
        ),
        (
            "text-styles",
            YamlValue::Sequence(
                catalog
                    .text_styles
                    .iter()
                    .map(|(id, table)| styles::text_styles_value(*id, table))
                    .collect::<Result<_, _>>()?,
            ),
        ),
    ]);
    let encoded = write_canonical_checked(&value).map_err(invalid)?;
    if &decode(&encoded)? != catalog {
        return Err(unsupported("catalog를 텍스트로 정확히 복원할 수 없습니다"));
    }
    Ok(encoded)
}

/// catalog 텍스트를 읽습니다. 쓰지 않은 글꼴 항목과 순서가 어긋난 항목은 거부합니다.
pub fn decode(source: &str) -> Result<AssetCatalog, TextSourceError> {
    let value = load_restricted(source).map_err(invalid)?;
    fields(
        &value,
        &["format", "schema_version", "images", "fonts", "text-styles"],
    )?;
    if get(&value, "format")? != CATALOG_FORMAT {
        return Err(unsupported("asset catalog 형식"));
    }
    if get(&value, "schema_version")? != CATALOG_VERSION {
        return Err(unsupported("asset catalog 버전"));
    }
    let mut catalog = AssetCatalog::default();
    let mut last = None;
    for item in sequence(&value, "images")? {
        let (id, record) = parse_image(item)?;
        if last.is_some_and(|prev| prev >= id) {
            return Err(invalid("이미지는 id 오름차순, 중복 없이 씁니다"));
        }
        last = Some(id);
        catalog.images.insert(id, record);
    }
    let mut fonts = BTreeMap::new();
    for item in sequence(&value, "fonts")? {
        let (hash, asset) = parse_font(item)?;
        if fonts
            .last_key_value()
            .is_some_and(|(prev, _)| *prev >= hash)
        {
            return Err(invalid("글꼴은 file hash 오름차순, 중복 없이 씁니다"));
        }
        fonts.insert(hash, asset);
    }
    let mut used = BTreeSet::new();
    let mut last = None;
    for item in sequence(&value, "text-styles")? {
        let (document, table) = styles::parse_text_styles_value(item, &fonts)?;
        if last.is_some_and(|prev| prev >= document) {
            return Err(invalid(
                "Text style 표는 문서 id 오름차순, 중복 없이 씁니다",
            ));
        }
        last = Some(document);
        used.extend(table.fonts.keys().copied());
        catalog.text_styles.insert(document, table);
    }
    if used.len() != fonts.len() {
        return Err(invalid("어떤 문서도 등록하지 않은 글꼴 항목이 있습니다"));
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::license::LicenseField;
    use stellar_document::text_styles::DocumentTextStyles;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn font_asset(seed: u8) -> (FontFileHash, DocumentFontAsset) {
        let hash = FontFileHash::from_sha256_digest([seed; 32]);
        let license = FontLicenseInfo::new()
            .with(LicenseField::Name, "SIL Open Font License 1.1")
            .unwrap();
        let mut license = license;
        license
            .change_state(FontLicenseState::Allowed, LicenseChangeOrigin::User)
            .unwrap();
        let record = ProjectFontRecord::new(
            AssetId::from_stable_id(sid(u64::from(seed) + 500)),
            hash,
            FontFileFormat::TrueType,
            vec![
                FontFaceNames::new(Some("Pretendard".into()), Some("Regular".into()), None)
                    .unwrap(),
            ],
            license,
        )
        .unwrap();
        let asset = AssetRecord::new(
            ContentHash::compute(&[seed]),
            MediaType::new("font/ttf").unwrap(),
            1024,
            Some(SourceMetadata::project("fonts/pretendard.ttf").unwrap()),
        );
        (
            hash,
            DocumentFontAsset {
                font_record: record.encode().unwrap(),
                asset_record: asset.encode().unwrap(),
            },
        )
    }

    fn catalog() -> AssetCatalog {
        let mut catalog = AssetCatalog::default();
        catalog.images.insert(
            sid(80),
            AssetRecord::new(
                ContentHash::compute(b"png"),
                MediaType::new("image/png").unwrap(),
                3,
                Some(SourceMetadata::project("images/hero image.png").unwrap()),
            ),
        );
        catalog.images.insert(
            sid(81),
            AssetRecord::new(
                ContentHash::compute(b"jpg"),
                MediaType::new("image/jpeg").unwrap(),
                3,
                None,
            ),
        );
        let origin = format!("hmac-sha256:{}:{}", "0a".repeat(8), "1b".repeat(32))
            .parse()
            .unwrap();
        catalog.images.insert(
            sid(82),
            AssetRecord::new(
                ContentHash::compute(b"gif"),
                MediaType::new("image/gif").unwrap(),
                3,
                Some(SourceMetadata::external("외부 사진 1.gif", origin).unwrap()),
            ),
        );
        let (hash, font) = font_asset(7);
        let mut first = DocumentTextStyles::initial(sid(100));
        first.fonts.insert(hash, font.clone());
        first.paragraphs.insert(3, first.paragraphs[&0]);
        let mut second = DocumentTextStyles::initial(sid(101));
        second.fonts.insert(hash, font);
        catalog.text_styles.insert(sid(1), first);
        catalog.text_styles.insert(sid(2), second);
        catalog
    }

    #[test]
    fn catalog_round_trips_images_fonts_and_style_tables_exactly() {
        let catalog = catalog();
        let text = encode(&catalog).unwrap();
        assert!(text.starts_with("format: stellar-asset-catalog\nschema_version: \"4\"\n"));
        // 같은 글꼴을 두 문서가 등록해도 catalog 항목은 하나이고 문서는 file hash로 참조합니다.
        assert_eq!(text.matches("    file-hash:").count(), 1);
        assert!(
            text.contains("source: \"project images/hero image.png\""),
            "{text}"
        );
        assert!(text.contains("source: \"external hmac-sha256:"), "{text}");
        assert_eq!(decode(&text).unwrap(), catalog);
        assert_eq!(encode(&decode(&text).unwrap()).unwrap(), text);
    }

    #[test]
    fn catalog_rejects_unreferenced_fonts_disorder_and_unknown_fields() {
        let text = encode(&catalog()).unwrap();
        let first = sid(80).to_string();
        let second = sid(81).to_string();
        let swapped = text
            .replacen(&first, "SWAP", 1)
            .replacen(&second, &first, 1)
            .replacen("SWAP", &second, 1);
        let font_ref = format!(
            "      - \"{}\"\n",
            FontFileHash::from_sha256_digest([7; 32])
        );
        for bad in [
            text.replace(&font_ref, ""),
            swapped,
            text.replacen("    size: \"3\"\n", "    size: \"3\"\n    future: x\n", 1),
            text.replacen("license-state: allowed", "license-state: maybe", 1),
            text.replacen("source: none", "source: \"cloud x\"", 1),
            text.replacen("schema_version: \"4\"", "schema_version: \"5\"", 1),
        ] {
            assert_ne!(bad, text);
            assert!(decode(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn different_records_for_one_font_hash_are_refused() {
        let mut catalog = catalog();
        let (hash, mut other) = font_asset(7);
        other.asset_record = AssetRecord::new(
            ContentHash::compute(b"other"),
            MediaType::new("font/ttf").unwrap(),
            9,
            None,
        )
        .encode()
        .unwrap();
        catalog
            .text_styles
            .get_mut(&sid(2))
            .unwrap()
            .fonts
            .insert(hash, other);
        assert!(matches!(
            encode(&catalog),
            Err(TextSourceError::Unsupported(_))
        ));
    }
}
