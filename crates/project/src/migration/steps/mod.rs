//! 제품 schema 1→2 묶음 step의 레코드 변환 (IX.11.9 등, ADR-0077 결정 4, ADR-0086).
//!
//! 각 변환은 [`DocumentRecords`](stellar_document::records::DocumentRecords)를 받아 새 레코드와 보고서를 돌려주는
//! 순수 함수입니다. 아직 [`super::PRODUCT_STEPS`]에 등록하지 않습니다. IX.11.6·IX.11.9·IX.11.10·IX.11.11의 변경을
//! 1→2 한 번으로 묶고, scene이 layout 결과 위치로 그리게 된 뒤 등록합니다(등록 전에 켜면 위치를 layout record로
//! 옮긴 Frame이 transform 이동 성분 없이 그려집니다).

/// Frame 위치와 transform 분리 (IX.11.9).
pub mod frame_position;
