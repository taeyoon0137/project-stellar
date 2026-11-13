//! stellar-assets — 콘텐츠 주소 에셋 골격. 의존: storage → core-types.

/// `AssetId` 에셋 식별자 (XIII.5.1).
pub mod id;

/// 에셋 media type (XIII.5.1).
pub mod media_type;

/// `AssetRecord` 에셋 record와 canonical 인코딩 (XIII.5.1).
pub mod record;
