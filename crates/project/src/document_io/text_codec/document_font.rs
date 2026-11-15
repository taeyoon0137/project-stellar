//! 단일 View v3. 문서가 소유하는 기본 문단/문자 0과 명시적으로 등록한 프로젝트 글꼴.
use super::*;
use crate::fonts::{
    license::FontLicenseInfo,
    project_font::ProjectFontRecord,
    sfnt::{FontFaceNames, FontFileFormat},
};
use stellar_assets::{
    id::AssetId,
    media_type::MediaType,
    record::{AssetRecord, SourceLocation, SourceMetadata},
};
use stellar_core_types::{
    font::{FontFaceRef, FontFileHash, FontSource},
    typography::Family,
};
use stellar_document::text_styles::DocumentFontAsset;
use stellar_storage::hash::ContentHash;

fn unsupported() -> TextSourceError {
    TextSourceError::Unsupported(
        "문서 글꼴은 기본 문단·문자 0 및 명시적으로 등록한 프로젝트 face를 지원합니다".into(),
    )
}

pub(super) fn check_text(payload: &TextPayload) -> Result<(), TextSourceError> {
    if payload.paragraphs().iter().any(|p| p.style != 0)
        || payload.style_runs().iter().any(|r| r.style != 0)
    {
        return Err(unsupported());
    }
    Ok(())
}

pub(super) fn encode(styles: &DocumentTextStyles) -> Result<YamlValue, TextSourceError> {
    let id = styles.default_typography.ok_or_else(unsupported)?;
    let typography = styles
        .typography
        .get(&id)
        .ok_or_else(unsupported)?
        .record()
        .map_err(|_| unsupported())?;
    let [family] = typography.families.as_slice() else {
        return Err(unsupported());
    };
    let face = family.face.as_ref().ok_or_else(unsupported)?;
    let FontSize::Subpixels(size) = typography.size else {
        return Err(unsupported());
    };
    let fonts = styles
        .fonts
        .iter()
        .map(|(hash, entry)| encode_font(*hash, entry))
        .collect::<Result<_, _>>()?;
    let value = mapping([
        ("id", scalar(id)),
        (
            "font-family",
            mapping([
                ("name", scalar(&family.name)),
                ("file_hash", scalar(face.file_hash())),
                ("face_index", scalar(face.face_index())),
            ]),
        ),
        ("font-size", scalar(format!("{}px", f64::from(size) / 64.0))),
        ("font-weight", scalar(typography.weight.get())),
        ("font-style", scalar("normal")),
        (
            "font-stretch",
            scalar(format!("{}%", typography.stretch_percent.get())),
        ),
        ("line-height", scalar("normal")),
        ("letter-spacing", scalar("normal")),
        ("color", color(styles.default_fill.ok_or_else(unsupported)?)),
        ("project_fonts", YamlValue::Sequence(fonts)),
    ]);
    // 재구성한 표 전체를 비교하여 다른 resource/문단/run, 미지원 axis·라이선스나
    // binary decoder가 건너뛰는 향후 optional field도 유실되지 않게 합니다.
    if &decode(&value)? != styles {
        return Err(unsupported());
    }
    Ok(value)
}

fn encode_font(
    hash: FontFileHash,
    entry: &DocumentFontAsset,
) -> Result<YamlValue, TextSourceError> {
    let font = ProjectFontRecord::decode(&entry.font_record).map_err(invalid)?;
    let asset = AssetRecord::decode(&entry.asset_record).map_err(invalid)?;
    if hash != font.file_hash()
        || asset.content_hash.digest() != hash.sha256_digest()
        || asset.media_type.as_str() != font.format().media_type()
    {
        return Err(invalid("프로젝트 글꼴의 hash 또는 media type 불일치"));
    }
    let faces = font
        .faces()
        .iter()
        .map(|names| {
            let names = [
                ("family", names.family()),
                ("subfamily", names.subfamily()),
                ("postscript", names.postscript()),
            ];
            YamlValue::Mapping(
                names
                    .into_iter()
                    .filter_map(|(key, value)| value.map(|v| (scalar(key), scalar(v))))
                    .collect(),
            )
        })
        .collect();
    let source = optional(asset.source_metadata.as_ref(), |metadata| {
        match metadata.location() {
            SourceLocation::Project { relative_path } => {
                mapping([("kind", scalar("project")), ("path", scalar(relative_path))])
            }
            SourceLocation::External { file_name, origin } => mapping([
                ("kind", scalar("external")),
                ("filename", scalar(file_name)),
                ("origin", scalar(origin)),
            ]),
        }
    });
    Ok(mapping([
        ("content_hash", scalar(hash)),
        ("asset_id", scalar(font.asset_id())),
        ("media_type", scalar(asset.media_type)),
        ("byte_size", scalar(asset.size)),
        ("source", source),
        ("faces", YamlValue::Sequence(faces)),
    ]))
}

fn parse_source(value: &YamlValue) -> Result<SourceMetadata, TextSourceError> {
    match text(field(value, "kind")?)? {
        "project" => {
            fields(value, &["kind", "path"])?;
            SourceMetadata::project(text(field(value, "path")?)?).map_err(invalid)
        }
        "external" => {
            fields(value, &["kind", "filename", "origin"])?;
            SourceMetadata::external(
                text(field(value, "filename")?)?,
                parse(field(value, "origin")?)?,
            )
            .map_err(invalid)
        }
        _ => Err(unsupported()),
    }
}

fn parse_face(value: &YamlValue) -> Result<FontFaceNames, TextSourceError> {
    let names = value
        .as_mapping()
        .ok_or_else(|| invalid("face 이름 mapping이 필요합니다"))?;
    for (key, _) in names {
        if !matches!(text(key)?, "family" | "subfamily" | "postscript") {
            return Err(unsupported());
        }
    }
    let name = |key| {
        value
            .get(key)
            .map(text)
            .transpose()
            .map(|opt| opt.map(str::to_owned))
    };
    FontFaceNames::new(name("family")?, name("subfamily")?, name("postscript")?).map_err(invalid)
}

fn parse_font(value: &YamlValue) -> Result<(ProjectFontRecord, AssetRecord), TextSourceError> {
    fields(
        value,
        &[
            "content_hash",
            "asset_id",
            "media_type",
            "byte_size",
            "source",
            "faces",
        ],
    )?;
    let hash: ContentHash = parse(field(value, "content_hash")?)?;
    let id: AssetId = parse(field(value, "asset_id")?)?;
    let format = text(field(value, "media_type")?)?;
    let format = FontFileFormat::ALL
        .into_iter()
        .find(|f| f.media_type() == format)
        .ok_or_else(unsupported)?;
    let faces = field(value, "faces")?
        .as_sequence()
        .ok_or_else(|| invalid("faces sequence가 필요합니다"))?;
    let size = parse(field(value, "byte_size")?)?;
    if id == AssetId::NIL
        || size == 0
        || faces.len() > crate::fonts::sfnt::MAX_FACES_PER_FILE as usize
    {
        return Err(invalid("프로젝트 글꼴 ID, 크기 또는 face 수 오류"));
    }
    let font = ProjectFontRecord::new(
        id,
        FontFileHash::from_sha256_digest(*hash.digest()),
        format,
        faces.iter().map(parse_face).collect::<Result<_, _>>()?,
        FontLicenseInfo::default(),
    )
    .map_err(invalid)?;
    let asset = AssetRecord::new(
        hash,
        MediaType::new(format.media_type()).map_err(invalid)?,
        size,
        parse_optional(field(value, "source")?, parse_source)?,
    );
    Ok((font, asset))
}

/// CSS px를 문서의 1/64 px 정수로 정확히 변환합니다. 실수 반올림으로 다른 입력을 허용하지 않습니다.
fn parse_size(css: &str) -> Result<u32, TextSourceError> {
    let error = || invalid("font-size는 양의 1/64 px 단위여야 합니다");
    let px = css.strip_suffix("px").ok_or_else(error)?;
    let (whole, fraction) = px.split_once('.').unwrap_or((px, ""));
    let fraction = fraction.trim_end_matches('0');
    if !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 6
    {
        return Err(error());
    }
    let denominator = 10u32.pow(fraction.len() as u32);
    let remainder = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u32>().map_err(|_| error())? * 64
    };
    if remainder % denominator != 0 {
        return Err(error());
    }
    whole
        .parse::<u32>()
        .ok()
        .and_then(|n| n.checked_mul(64))
        .and_then(|n| n.checked_add(remainder / denominator))
        .filter(|n| (1..=i32::MAX as u32).contains(n))
        .ok_or_else(error)
}

pub(super) fn decode(value: &YamlValue) -> Result<DocumentTextStyles, TextSourceError> {
    fields(
        value,
        &[
            "id",
            "font-family",
            "font-size",
            "font-weight",
            "font-style",
            "font-stretch",
            "line-height",
            "letter-spacing",
            "color",
            "project_fonts",
        ],
    )?;
    let id = parse(field(value, "id")?)?;
    let family = field(value, "font-family")?;
    fields(family, &["name", "file_hash", "face_index"])?;
    let name = text(field(family, "name")?)?;
    let hash: FontFileHash = parse(field(family, "file_hash")?)?;
    let index = parse(field(family, "face_index")?)?;
    let number = |key, suffix| -> Result<&str, TextSourceError> {
        text(field(value, key)?)?
            .strip_suffix(suffix)
            .ok_or_else(|| invalid(format!("{key} 단위 오류")))
    };
    let size = parse_size(text(field(value, "font-size")?)?)?;
    for key in ["font-style", "line-height", "letter-spacing"] {
        if text(field(value, key)?)? != "normal" {
            return Err(unsupported());
        }
    }
    let typography = Typography {
        families: vec![Family {
            name: name.to_owned(),
            face: Some(FontFaceRef::new(FontSource::Project, hash, index, vec![])),
        }],
        size: FontSize::Subpixels(size),
        weight: FiniteF32::new(parse(field(value, "font-weight")?)?).map_err(invalid)?,
        style: FontStyle::Normal,
        stretch_percent: FiniteF32::new(number("font-stretch", "%")?.parse().map_err(invalid)?)
            .map_err(invalid)?,
        line_height: LineHeight::Normal,
        letter_spacing: LetterSpacing::Normal,
    };
    let mut styles = v2_text_styles(id);
    styles.typography.insert(
        id,
        TypographyValue::from_record(&typography).map_err(invalid)?,
    );
    styles.default_fill = Some(parse_color(field(value, "color")?)?);
    let mut ids = std::collections::BTreeSet::new();
    for font in field(value, "project_fonts")?
        .as_sequence()
        .ok_or_else(|| invalid("project_fonts sequence가 필요합니다"))?
    {
        let (font, asset) = parse_font(font)?;
        if !ids.insert(font.asset_id()) || styles.fonts.contains_key(&font.file_hash()) {
            return Err(invalid("중복 글꼴 hash 또는 asset ID"));
        }
        if hash == font.file_hash()
            && font.face_names(index).and_then(FontFaceNames::family) != Some(name)
        {
            return Err(invalid("font-family가 등록한 face와 일치하지 않습니다"));
        }
        styles.fonts.insert(
            font.file_hash(),
            DocumentFontAsset {
                font_record: font.encode().map_err(invalid)?,
                asset_record: asset.encode().map_err(invalid)?,
            },
        );
    }
    if !styles.fonts.contains_key(&hash) {
        return Err(invalid("font-family의 프로젝트 글꼴이 없습니다"));
    }
    Ok(styles)
}
