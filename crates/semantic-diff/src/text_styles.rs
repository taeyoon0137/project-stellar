//! 문서 typography는 resource ID와 CSS longhand 단위로, sparse 문단/문자 표는 index 단위로 병합합니다.
use crate::{
    keys,
    records::{ProjectionError, ProjectionFailure},
    snapshot::{SemanticObject, SnapshotError},
    value::SemanticValue,
};
use std::collections::BTreeMap;
use stellar_core_types::{
    ids::StableId,
    property::{PathSegment, PropertyKey},
    typed_value::{TypedValue, TypographyValue},
    typography::Typography,
};
use stellar_document::text_styles::{keys::LONGHANDS, DocumentTextStyles};

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
        let single = DocumentTextStyles {
            paragraphs: BTreeMap::from([(*index, *style)]),
            ..Default::default()
        };
        doc.insert_property(
            doc.path(&[keys::TEXT_PARAGRAPH_STYLE]).with_index(*index),
            SemanticValue::Bytes(single.encode()),
        )?;
    }
    for (index, style) in &table.runs {
        let single = DocumentTextStyles {
            runs: BTreeMap::from([(*index, style.clone())]),
            ..Default::default()
        };
        doc.insert_property(
            doc.path(&[keys::TEXT_RUN_STYLE]).with_index(*index),
            SemanticValue::Bytes(single.encode()),
        )?;
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
    let bad = |key: PropertyKey| ProjectionError {
        object: doc.id(),
        reason: ProjectionFailure::BadProperty { key },
    };
    for (path, value) in doc.properties() {
        let [key] = path.keys() else { continue };
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
            (
                keys::TEXT_PARAGRAPH_STYLE | keys::TEXT_RUN_STYLE | keys::TEXT_FONT_ASSET,
                tail,
                SemanticValue::Bytes(bytes),
            ) => {
                let mut single = DocumentTextStyles::decode(bytes).map_err(|_| bad(*key))?;
                match (*key, tail) {
                    (keys::TEXT_PARAGRAPH_STYLE, Some(PathSegment::Index(i))) => {
                        table
                            .paragraphs
                            .insert(*i, single.paragraphs.remove(i).ok_or_else(|| bad(*key))?);
                    }
                    (keys::TEXT_RUN_STYLE, Some(PathSegment::Index(i))) => {
                        table
                            .runs
                            .insert(*i, single.runs.remove(i).ok_or_else(|| bad(*key))?);
                    }
                    (keys::TEXT_FONT_ASSET, Some(PathSegment::Key(s))) => {
                        let hash = s.parse().map_err(|_| bad(*key))?;
                        table
                            .fonts
                            .insert(hash, single.fonts.remove(&hash).ok_or_else(|| bad(*key))?);
                    }
                    _ => return Err(bad(*key)),
                }
                if !single.is_empty() {
                    return Err(bad(*key));
                }
            }
            (
                keys::TEXT_DEFAULT_TYPOGRAPHY
                | keys::TEXT_DEFAULT_FILL
                | keys::TEXT_OPAQUE_TYPOGRAPHY
                | keys::TEXT_PARAGRAPH_STYLE
                | keys::TEXT_RUN_STYLE
                | keys::TEXT_FONT_ASSET,
                _,
                _,
            ) => return Err(bad(*key)),
            _ => {}
        }
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
