//! stellar-assets — 콘텐츠 주소 에셋 골격. 의존: storage → core-types.

/// `AssetId` 에셋 식별자 (XIII.5.1).
pub mod id;

/// 에셋 media type (XIII.5.1).
pub mod media_type;

/// `AssetRecord` 에셋 record와 canonical 인코딩 (XIII.5.1).
pub mod record;

/// 외부 원본 출처 지문 — 기기별 비밀 key의 HMAC (XIII.5.1 후속 수정).
pub mod origin;

/// streaming content hash (XIII.5.2).
pub mod hash_stream;

/// 에셋 content 파일 경로 (XIII.5.2).
pub mod path;

/// 에셋 저장소와 외부 파일 import (XIII.5.2), content dedup (XIII.5.3).
pub mod store;

/// `AssetId → AssetRecord` catalog (XIII.5.3).
pub mod catalog;

/// 에셋 streaming atomic write — temp+fsync+atomic rename (XIII.5.4).
pub mod atomic_write;

/// `Update from Source` — 원본 재import와 AssetRecord 참조 변경 값 (XIII.5.5).
pub mod update_source;

/// 프로젝트 내부 원본 경로 경계 — 고정 거부 영역과 호출자 정책 (XIII.5.5 후속 수정).
pub mod project_source;

/// asset content GC — generation/chunk GC와 별도 mark-sweep (XIII.5.6).
pub mod gc;

/// 에셋 import 정책 — 허용 media type·최대 크기·unknown type 처리 (XIII.5.7).
pub mod policy;
