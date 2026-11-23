//! 생성 파일 목록 (ADR-0097 결정 1·6).
//!
//! 각 항목은 (저장소 루트 기준 경로, 내용)입니다. `stellar-plugin-bindgen`이 쓰고 `tests/schema_drift.rs`가
//! 저장소의 파일과 byte 단위로 비교합니다(drift 0).

/// 생성 파일 표.
pub fn generated_files() -> Vec<(&'static str, String)> {
    let idl = crate::idl::Idl::source();
    vec![
        (
            "schemas/plugin-manifest.schema.json",
            crate::manifest::manifest_json_schema(),
        ),
        (crate::bindgen::RUST_PATH, crate::bindgen::render_rust(&idl)),
        (crate::bindgen::TS_PATH, crate::bindgen::render_ts(&idl)),
    ]
}
