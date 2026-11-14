//! 작성자가 지정한 파일·face·절대 크기·색으로 문서 기본 글꼴을 등록하고 같은 bytes만 다시 엽니다.
use super::{
    license::FontLicenseInfo,
    project_font::{import_project_font, ProjectFaceError, ProjectFontRecord},
};
use std::{io, path::Path, sync::OnceLock};
use stellar_assets::{
    catalog::AssetCatalog, id::AssetId, origin::OriginKeyFile, policy::AssetImportPolicy,
    record::AssetRecord, store::AssetStore,
};
use stellar_core_types::{
    color::Color,
    font::FontFaceRef,
    ids::StableId,
    typed_value::TypographyValue,
    typography::{Family, FontSize, Typography},
};
use stellar_document::text_styles::{DocumentFontAsset, DocumentTextStyles};
use stellar_text::shaping::FontFace;

fn import_policy() -> &'static AssetImportPolicy {
    static POLICY: OnceLock<AssetImportPolicy> = OnceLock::new();
    POLICY.get_or_init(|| {
        AssetImportPolicy::from_config_str(include_str!("../../../../config/asset-import.yaml"))
            .expect("출하된 asset import 정책")
    })
}

/// 사용자가 지정한 파일, collection face, 절대 크기(1/64px), 색.
pub struct DocumentFontSelection {
    /// 절대 font 파일 경로.
    pub file: std::path::PathBuf,
    /// 파일 안의 face index.
    pub index: u32,
    /// 글자 크기(1/64 px).
    pub size: u32,
    /// 새 기본 색.
    pub fill: Color,
}

/// 검증한 face와 새 스타일 표. 한 번 읽은 face를 첫 장면에서도 사용합니다.
pub struct ImportedDocumentFont {
    /// 적용할 문서 style 표.
    pub styles: DocumentTextStyles,
    /// face의 canonical identity.
    pub reference: FontFaceRef,
    /// 검증한 bytes의 동일 face.
    pub face: FontFace,
}

/// 호출자가 현재 문서와 충돌하지 않는 두 ID를 제공하며, 파일 작업은 worker에서 실행합니다.
pub fn import_default_font(
    root: &Path,
    data_dir: &Path,
    current: &DocumentTextStyles,
    [new_resource, new_asset]: [StableId; 2],
    input: &DocumentFontSelection,
) -> Result<ImportedDocumentFont, io::Error> {
    if input.size == 0 || input.size > i32::MAX as u32 {
        return Err(io::Error::other("글꼴 크기 범위 위반"));
    }
    let resource = current.default_typography.unwrap_or(new_resource);
    let mut typography = match current.default_typography {
        Some(id) => current
            .typography
            .get(&id)
            .ok_or_else(|| io::Error::other("기본 typography 참조 누락"))?
            .record()
            .map_err(io::Error::other)?,
        None => Typography::default(),
    };
    let probe = super::project_font::probe_file(&input.file)?.map_err(io::Error::other)?;
    if probe
        .faces
        .get(input.index as usize)
        .is_none_or(|f| f.names.family().is_none())
    {
        return Err(io::Error::other(
            "선택한 face index 또는 family 이름이 파일에 없습니다",
        ));
    }
    let key = OriginKeyFile::in_data_dir(data_dir).map_err(io::Error::other)?;
    let store = AssetStore::new(root).with_origin_key_file(key);
    let imported = import_project_font(
        &store,
        import_policy(),
        AssetId::from_stable_id(new_asset),
        &input.file,
        FontLicenseInfo::default(),
    )
    .map_err(io::Error::other)?;
    let reference = imported
        .font
        .face_ref(input.index)
        .ok_or_else(|| io::Error::other("선택한 face index가 파일에 없습니다"))?;
    let family = imported.font.faces()[input.index as usize]
        .family()
        .ok_or_else(|| io::Error::other("선택한 face의 family 이름이 없습니다"))?;
    // 기존 사용자가 명시한 fallback 순서는 유지하고 primary만 교체합니다.
    typography.families.splice(
        ..typography.families.len().min(1),
        [Family {
            name: family.into(),
            face: Some(reference.clone()),
        }],
    );
    typography.size = FontSize::Subpixels(input.size);
    let value = TypographyValue::from_record(&typography).map_err(io::Error::other)?;
    let mut next = if current.is_empty() {
        DocumentTextStyles::initial(resource)
    } else {
        current.clone()
    };
    if let std::collections::btree_map::Entry::Vacant(entry) =
        next.fonts.entry(reference.file_hash())
    {
        entry.insert(DocumentFontAsset {
            font_record: imported.font.encode().map_err(io::Error::other)?,
            asset_record: imported.asset.record.encode().map_err(io::Error::other)?,
        });
    }
    // 선택 파일이 실제 shaping 가능한 face인지 먼저 검사합니다(지원하지 않는 bitmap/container 등).
    let face = load_document_face(root, &next, &reference).map_err(io::Error::other)?;
    next.default_typography = Some(resource);
    next.default_fill = Some(input.fill);
    next.typography.insert(resource, value);
    Ok(ImportedDocumentFont {
        styles: next,
        reference,
        face,
    })
}

/// project 글꼴의 저장된 기록을 사용합니다. 경로·family 이름으로 시스템 파일을 검색하지 않습니다.
pub fn load_document_face(
    root: &Path,
    styles: &DocumentTextStyles,
    face: &FontFaceRef,
) -> Result<stellar_text::shaping::FontFace, ProjectFaceError> {
    let entry = styles
        .fonts
        .get(&face.file_hash())
        .ok_or(ProjectFaceError::MissingAsset)?;
    let font = ProjectFontRecord::decode(&entry.font_record)
        .map_err(|_| ProjectFaceError::ReferenceMismatch)?;
    let asset = AssetRecord::decode(&entry.asset_record)
        .map_err(|_| ProjectFaceError::ReferenceMismatch)?;
    let mut catalog = AssetCatalog::new();
    catalog
        .insert(font.asset_id(), asset)
        .map_err(|_| ProjectFaceError::ReferenceMismatch)?;
    font.load_face(face, &catalog, &AssetStore::new(root), import_policy())
}
