//! 문서 style 표에 등록된 project 글꼴 record를 검증하고 같은 bytes만 다시 엽니다.
use super::project_font::{ProjectFaceError, ProjectFontRecord};
use std::{path::Path, sync::OnceLock};
use stellar_assets::{
    catalog::AssetCatalog, policy::AssetImportPolicy, record::AssetRecord, store::AssetStore,
};
use stellar_core_types::font::FontFaceRef;
use stellar_document::text_styles::DocumentTextStyles;

fn import_policy() -> &'static AssetImportPolicy {
    static POLICY: OnceLock<AssetImportPolicy> = OnceLock::new();
    POLICY.get_or_init(|| {
        AssetImportPolicy::from_config_str(include_str!("../../../../config/asset-import.yaml"))
            .expect("출하된 asset import 정책")
    })
}

/// 원본을 export할 때 각 등록된 폰트 객체의 hash와 byte 수를 사용자 경로에 접근하지 않고 확인합니다.
pub fn verify_document_font_assets(
    root: &Path,
    styles: &DocumentTextStyles,
) -> Result<(), ProjectFaceError> {
    let store = AssetStore::new(root);
    for (hash, entry) in &styles.fonts {
        let asset = AssetRecord::decode(&entry.asset_record)
            .map_err(|_| ProjectFaceError::ReferenceMismatch)?;
        if asset.content_hash.digest() != hash.sha256_digest()
            || asset.size > import_policy().max_file_size_bytes()
        {
            return Err(ProjectFaceError::ReferenceMismatch);
        }
        store
            .locate_content(&asset.content_hash, asset.size)
            .map_err(ProjectFaceError::Store)?
            .ok_or(ProjectFaceError::MissingAsset)?;
    }
    Ok(())
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
