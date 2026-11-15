//! 문서 typography는 resource ID와 CSS longhand 단위로, sparse 문단/문자 표는 index와 필드 단위로
//! 병합합니다. 같은 style entry의 서로 다른 필드를 두 branch가 바꿔도 충돌하지 않습니다.
use crate::{
    keys,
    records::{ProjectionError, ProjectionFailure},
    snapshot::{SemanticObject, SnapshotError},
    value::SemanticValue,
};
use std::collections::BTreeMap;
use stellar_core_types::{
    color::Color,
    ids::StableId,
    property::{PathSegment, PropertyKey, PropertyPath},
    typed_value::{TypedValue, TypographyValue},
    typography::{LetterSpacing, Typography},
    value::FiniteF32,
};
use stellar_document::text::{
    LineHeight, OpenTypeTag, ParagraphStyle, TextAlignment, TextStyle, WrappingMode,
};
use stellar_document::text_styles::{keys::LONGHANDS, DocumentTextStyles};

fn style_path(
    doc: &SemanticObject,
    entry: PropertyKey,
    field: Option<PropertyKey>,
    index: u32,
) -> PropertyPath {
    match field {
        Some(field) => doc.path(&[entry, field]),
        None => doc.path(&[entry]),
    }
    .with_index(index)
}

fn line_height_bytes(value: LineHeight) -> Vec<u8> {
    match value {
        LineHeight::Auto => vec![0],
        LineHeight::Multiplier(v) => [&[1][..], &v.to_le_bytes()].concat(),
        LineHeight::Fixed(v) => [&[2][..], &v.to_le_bytes()].concat(),
    }
}

fn letter_spacing_bytes(value: LetterSpacing) -> Vec<u8> {
    match value {
        LetterSpacing::Normal => vec![1],
        LetterSpacing::Px(v) => [&[2][..], &v.to_le_bytes()].concat(),
        LetterSpacing::Em(v) => [&[3][..], &v.to_le_bytes()].concat(),
    }
}

fn tagged_bytes<V>(items: &[(OpenTypeTag, V)], value: impl Fn(&V) -> [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(items.len() * 8);
    for (tag, v) in items {
        out.extend(tag.bytes());
        out.extend(value(v));
    }
    out
}

fn finite(bytes: &[u8]) -> Option<FiniteF32> {
    FiniteF32::from_le_bytes(bytes.try_into().ok()?).ok()
}

fn parse_line_height(bytes: &[u8]) -> Option<LineHeight> {
    match bytes.split_first()? {
        (0, []) => Some(LineHeight::Auto),
        (1, v) => Some(LineHeight::Multiplier(finite(v)?)),
        (2, v) => Some(LineHeight::Fixed(finite(v)?)),
        _ => None,
    }
}

fn parse_letter_spacing(bytes: &[u8]) -> Option<LetterSpacing> {
    match bytes.split_first()? {
        (1, []) => Some(LetterSpacing::Normal),
        (2, v) => Some(LetterSpacing::Px(finite(v)?)),
        (3, v) => Some(LetterSpacing::Em(finite(v)?)),
        _ => None,
    }
}

fn parse_tagged<V>(
    bytes: &[u8],
    value: impl Fn([u8; 4]) -> Option<V>,
) -> Option<Vec<(OpenTypeTag, V)>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(8) {
        return None;
    }
    bytes
        .chunks_exact(8)
        .map(|c| {
            Some((
                OpenTypeTag::new(c[..4].try_into().ok()?)?,
                value(c[4..].try_into().ok()?)?,
            ))
        })
        .collect()
}

#[derive(Default)]
struct ParagraphFields {
    marker: bool,
    typography: Option<StableId>,
    alignment: Option<u64>,
    wrapping: Option<u64>,
    line_height: Option<Vec<u8>>,
}

#[derive(Default)]
struct RunFields {
    marker: bool,
    typography: Option<StableId>,
    fill: Option<Color>,
    letter_spacing: Option<Vec<u8>>,
    features: Option<Vec<u8>>,
    variations: Option<Vec<u8>>,
}

pub(crate) fn project(
    table: &DocumentTextStyles,
    doc: &mut SemanticObject,
) -> Result<(), SnapshotError> {
    if let Some(id) = table.default_typography {
        doc.insert_property(
            doc.path(&[keys::TEXT_DEFAULT_TYPOGRAPHY]),
            SemanticValue::Id(id),
        )?;
    }
    if let Some(fill) = table.default_fill {
        doc.insert_property(
            doc.path(&[keys::TEXT_DEFAULT_FILL]),
            SemanticValue::Typed(TypedValue::Color(fill)),
        )?;
    }
    for (id, value) in &table.typography {
        let id = id.to_string();
        match value.record().and_then(|r| r.longhand_bytes()) {
            Ok(fields) => {
                for (key, bytes) in LONGHANDS.into_iter().zip(fields) {
                    doc.insert_property(
                        doc.path(&[key]).with_key_segment(&id),
                        SemanticValue::Bytes(bytes),
                    )?;
                }
            }
            Err(_) => doc.insert_property(
                doc.path(&[keys::TEXT_OPAQUE_TYPOGRAPHY])
                    .with_key_segment(&id),
                SemanticValue::Bytes(value.as_bytes().to_vec()),
            )?,
        }
    }
    for (index, style) in &table.paragraphs {
        let entry = keys::TEXT_PARAGRAPH_STYLE;
        let mut fields = vec![
            (None, SemanticValue::Bool(true)),
            (
                Some(keys::TEXT_STYLE_ALIGNMENT),
                SemanticValue::Uint(style.alignment as u64),
            ),
            (
                Some(keys::TEXT_STYLE_WRAPPING),
                SemanticValue::Uint(style.wrapping as u64),
            ),
            (
                Some(keys::TEXT_STYLE_LINE_HEIGHT),
                SemanticValue::Bytes(line_height_bytes(style.line_height)),
            ),
        ];
        if let Some(id) = style.typography {
            fields.push((Some(keys::TEXT_STYLE_TYPOGRAPHY), SemanticValue::Id(id)));
        }
        for (field, value) in fields {
            doc.insert_property(style_path(doc, entry, field, *index), value)?;
        }
    }
    for (index, style) in &table.runs {
        let entry = keys::TEXT_RUN_STYLE;
        let mut fields = vec![(None, SemanticValue::Bool(true))];
        if let Some(id) = style.typography {
            fields.push((Some(keys::TEXT_STYLE_TYPOGRAPHY), SemanticValue::Id(id)));
        }
        if let Some(fill) = style.fill {
            fields.push((
                Some(keys::TEXT_STYLE_FILL),
                SemanticValue::Typed(TypedValue::Color(fill)),
            ));
        }
        if let Some(spacing) = style.letter_spacing {
            fields.push((
                Some(keys::TEXT_STYLE_LETTER_SPACING),
                SemanticValue::Bytes(letter_spacing_bytes(spacing)),
            ));
        }
        if !style.features().is_empty() {
            fields.push((
                Some(keys::TEXT_STYLE_FEATURES),
                SemanticValue::Bytes(tagged_bytes(style.features(), |v| v.to_le_bytes())),
            ));
        }
        if !style.variations().is_empty() {
            fields.push((
                Some(keys::TEXT_STYLE_VARIATIONS),
                SemanticValue::Bytes(tagged_bytes(style.variations(), |v| v.to_le_bytes())),
            ));
        }
        for (field, value) in fields {
            doc.insert_property(style_path(doc, entry, field, *index), value)?;
        }
    }
    for (hash, asset) in &table.fonts {
        let single = DocumentTextStyles {
            fonts: BTreeMap::from([(*hash, asset.clone())]),
            ..Default::default()
        };
        doc.insert_property(
            doc.path(&[keys::TEXT_FONT_ASSET])
                .with_key_segment(&hash.to_string()),
            SemanticValue::Bytes(single.encode()),
        )?;
    }
    Ok(())
}

pub(crate) fn restore(doc: &SemanticObject) -> Result<DocumentTextStyles, ProjectionError> {
    let mut table = DocumentTextStyles::default();
    let mut resources: BTreeMap<StableId, [Vec<u8>; 7]> = BTreeMap::new();
    let mut paragraphs: BTreeMap<u32, ParagraphFields> = BTreeMap::new();
    let mut runs: BTreeMap<u32, RunFields> = BTreeMap::new();
    let bad = |key: PropertyKey| ProjectionError {
        object: doc.id(),
        reason: ProjectionFailure::BadProperty { key },
    };
    for (path, value) in doc.properties() {
        let (entry, field) = match path.keys() {
            [entry] => (*entry, None),
            [entry, field] => (*entry, Some(*field)),
            _ => continue,
        };
        if matches!(entry, keys::TEXT_PARAGRAPH_STYLE | keys::TEXT_RUN_STYLE) {
            let Some(PathSegment::Index(index)) = path.tail() else {
                return Err(bad(entry));
            };
            if entry == keys::TEXT_PARAGRAPH_STYLE {
                let p = paragraphs.entry(*index).or_default();
                match (field, value) {
                    (None, SemanticValue::Bool(true)) => p.marker = true,
                    (Some(keys::TEXT_STYLE_TYPOGRAPHY), SemanticValue::Id(id)) => {
                        p.typography = Some(*id)
                    }
                    (Some(keys::TEXT_STYLE_ALIGNMENT), SemanticValue::Uint(v)) => {
                        p.alignment = Some(*v)
                    }
                    (Some(keys::TEXT_STYLE_WRAPPING), SemanticValue::Uint(v)) => {
                        p.wrapping = Some(*v)
                    }
                    (Some(keys::TEXT_STYLE_LINE_HEIGHT), SemanticValue::Bytes(v)) => {
                        p.line_height = Some(v.clone())
                    }
                    _ => return Err(bad(entry)),
                }
            } else {
                let r = runs.entry(*index).or_default();
                match (field, value) {
                    (None, SemanticValue::Bool(true)) => r.marker = true,
                    (Some(keys::TEXT_STYLE_TYPOGRAPHY), SemanticValue::Id(id)) => {
                        r.typography = Some(*id)
                    }
                    (
                        Some(keys::TEXT_STYLE_FILL),
                        SemanticValue::Typed(TypedValue::Color(color)),
                    ) => r.fill = Some(*color),
                    (Some(keys::TEXT_STYLE_LETTER_SPACING), SemanticValue::Bytes(v)) => {
                        r.letter_spacing = Some(v.clone())
                    }
                    (Some(keys::TEXT_STYLE_FEATURES), SemanticValue::Bytes(v)) => {
                        r.features = Some(v.clone())
                    }
                    (Some(keys::TEXT_STYLE_VARIATIONS), SemanticValue::Bytes(v)) => {
                        r.variations = Some(v.clone())
                    }
                    _ => return Err(bad(entry)),
                }
            }
            continue;
        }
        if field.is_some() {
            continue;
        }
        let key = &entry;
        if let Some(i) = LONGHANDS.iter().position(|k| k == key) {
            let (Some(PathSegment::Key(id)), SemanticValue::Bytes(bytes)) = (path.tail(), value)
            else {
                return Err(bad(*key));
            };
            let id = id.parse().map_err(|_| bad(*key))?;
            resources.entry(id).or_default()[i] = bytes.clone();
            continue;
        }
        match (*key, path.tail(), value) {
            (keys::TEXT_DEFAULT_TYPOGRAPHY, None, SemanticValue::Id(id)) => {
                table.default_typography = Some(*id)
            }
            (keys::TEXT_DEFAULT_FILL, None, SemanticValue::Typed(TypedValue::Color(c))) => {
                table.default_fill = Some(*c)
            }
            (
                keys::TEXT_OPAQUE_TYPOGRAPHY,
                Some(PathSegment::Key(id)),
                SemanticValue::Bytes(bytes),
            ) => {
                let value = TypographyValue::from_slice(bytes);
                if value.record().is_ok() {
                    return Err(bad(*key));
                }
                table
                    .typography
                    .insert(id.parse().map_err(|_| bad(*key))?, value);
            }
            (keys::TEXT_FONT_ASSET, Some(PathSegment::Key(s)), SemanticValue::Bytes(bytes)) => {
                let mut single = DocumentTextStyles::decode(bytes).map_err(|_| bad(*key))?;
                let hash = s.parse().map_err(|_| bad(*key))?;
                table
                    .fonts
                    .insert(hash, single.fonts.remove(&hash).ok_or_else(|| bad(*key))?);
                if !single.is_empty() {
                    return Err(bad(*key));
                }
            }
            (
                keys::TEXT_DEFAULT_TYPOGRAPHY
                | keys::TEXT_DEFAULT_FILL
                | keys::TEXT_OPAQUE_TYPOGRAPHY
                | keys::TEXT_FONT_ASSET,
                _,
                _,
            ) => return Err(bad(*key)),
            _ => {}
        }
    }
    let paragraph_bad = || bad(keys::TEXT_PARAGRAPH_STYLE);
    for (index, p) in paragraphs {
        let style = ParagraphStyle {
            typography: p.typography,
            alignment: match p.alignment {
                Some(0) => TextAlignment::Start,
                Some(1) => TextAlignment::Center,
                Some(2) => TextAlignment::End,
                Some(3) => TextAlignment::Justify,
                _ => return Err(paragraph_bad()),
            },
            wrapping: match p.wrapping {
                Some(0) => WrappingMode::Wrap,
                Some(1) => WrappingMode::NoWrap,
                _ => return Err(paragraph_bad()),
            },
            line_height: p
                .line_height
                .as_deref()
                .and_then(parse_line_height)
                .ok_or_else(paragraph_bad)?,
        };
        if !p.marker || style.validate().is_err() {
            return Err(paragraph_bad());
        }
        table.paragraphs.insert(index, style);
    }
    let run_bad = || bad(keys::TEXT_RUN_STYLE);
    for (index, r) in runs {
        let features = match &r.features {
            Some(bytes) => {
                parse_tagged(bytes, |v| Some(u32::from_le_bytes(v))).ok_or_else(run_bad)?
            }
            None => Vec::new(),
        };
        let variations = match &r.variations {
            Some(bytes) => {
                parse_tagged(bytes, |v| FiniteF32::from_le_bytes(v).ok()).ok_or_else(run_bad)?
            }
            None => Vec::new(),
        };
        let mut style = TextStyle::new(r.typography, r.fill, features, variations);
        style.letter_spacing = match &r.letter_spacing {
            Some(bytes) => Some(parse_letter_spacing(bytes).ok_or_else(run_bad)?),
            None => None,
        };
        // 정렬·중복 없는 canonical 목록만 받습니다. TextStyle::new가 바꾼 목록은 원래 bytes와 다릅니다.
        let canonical = r.features.as_deref().unwrap_or_default()
            == tagged_bytes(style.features(), |v| v.to_le_bytes())
            && r.variations.as_deref().unwrap_or_default()
                == tagged_bytes(style.variations(), |v| v.to_le_bytes());
        if !r.marker || !canonical {
            return Err(run_bad());
        }
        table.runs.insert(index, style);
    }
    for (id, fields) in resources {
        let typography = Typography::from_longhand_bytes(&fields)
            .and_then(|r| TypographyValue::from_record(&r))
            .map_err(|_| bad(LONGHANDS[0]))?;
        if table.typography.insert(id, typography).is_some() {
            return Err(bad(keys::TEXT_OPAQUE_TYPOGRAPHY));
        }
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::tests::{build, head_records, sid};
    #[test]
    fn independently_addressed_font_size_and_spacing_reconstruct_a_document() {
        use stellar_core_types::{
            typography::{FontSize, LetterSpacing},
            value::FiniteF32,
        };
        let mut records = head_records();
        let resource = sid(222);
        records.definition.text_styles = DocumentTextStyles::initial(resource);
        records
            .definition
            .text_styles
            .typography
            .insert(sid(223), TypographyValue::new(vec![0xFF]));
        let base = build(&records);
        let doc = records.definition.id;
        let object = base.get(doc).unwrap();
        let mut merged = object.clone();
        for (index, value) in [
            (
                1,
                Typography {
                    size: FontSize::Subpixels(1509),
                    ..Default::default()
                },
            ),
            (
                6,
                Typography {
                    letter_spacing: LetterSpacing::Px(FiniteF32::new(2.).unwrap()),
                    ..Default::default()
                },
            ),
        ] {
            let mut changed = records.clone();
            changed
                .definition
                .text_styles
                .typography
                .insert(resource, TypographyValue::from_record(&value).unwrap());
            let snapshot = build(&changed);
            let target = snapshot.get(doc).unwrap();
            let paths: Vec<_> = target
                .properties()
                .iter()
                .filter(|(k, v)| object.property(k) != Some(v))
                .collect();
            assert_eq!(paths.len(), 1);
            assert_eq!(paths[0].0.keys(), &[LONGHANDS[index]]);
            merged
                .set_property(paths[0].0.clone(), paths[0].1.clone())
                .unwrap();
        }
        let table = restore(&merged).unwrap();
        let joined = table.typography[&resource].record().unwrap();
        assert_eq!(joined.size, FontSize::Subpixels(1509));
        assert_eq!(
            joined.letter_spacing,
            LetterSpacing::Px(FiniteF32::new(2.).unwrap())
        );
        assert_eq!(table.typography[&sid(223)].as_bytes(), [0xFF]);
        assert_eq!(restore(object).unwrap(), records.definition.text_styles);
    }
}
