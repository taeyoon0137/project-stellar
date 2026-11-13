//! stellar-assets — 콘텐츠 주소 에셋 골격. 의존: storage → core-types.

/// `AssetId` 에셋 식별자 (XIII.5.1).
pub mod id;

/// 에셋 media type (XIII.5.1).
pub mod media_type;

/// `AssetRecord` 에셋 record와 canonical 인코딩 (XIII.5.1).
pub mod record;

/// streaming content hash (XIII.5.2).
pub mod hash_stream;

/// 에셋 content 파일 경로 (XIII.5.2).
pub mod path;

/// 에셋 저장소와 외부 파일 import (XIII.5.2).
pub mod store;
