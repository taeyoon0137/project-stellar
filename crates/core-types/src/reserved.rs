//! 아직 구현하지 않은 저장 요소의 예약 번호 (IX.12.1, ADR-0075 결정 8).
//!
//! 예약 번호는 다른 용도로 쓰지 않습니다. 이 module은 번호만 고정하며 encoder·decoder 동작은 바꾸지 않습니다.
//! 실제 형식은 해당 구현 행(Component Context: 로드맵 XXX.7)이 같은 번호로 정의합니다. core-types에 두는 이유는
//! expressions·storage·document·semantic-diff가 모두 이 crate에 의존해 같은 상수를 읽을 수 있기 때문입니다.

/// Component Context(ADR-0025 결정 3) 저장 번호.
pub mod context {
    /// `ReferenceTarget`의 Context 참조 tag(`crates/expressions/src/ast.rs`, 기존 tag 0x01\~0x04 다음).
    pub const REFERENCE_TARGET_CONTEXT_TAG: u8 = 0x05;

    /// `ContextKey`·`ContextProvision` record를 담는 chunk kind code. required kind(최상위 bit)이며 번호는 7입니다
    /// (`crates/storage/src/chunk/kinds.rs`의 1\~6 다음). 이 kind를 모르는 reader는 V.2.7 규칙으로 오류를 냅니다.
    pub const CHUNK_KIND_CONTEXT_SET: u32 = 0x8000_0007;
    /// [`CHUNK_KIND_CONTEXT_SET`]의 registry 이름.
    pub const CHUNK_KIND_CONTEXT_SET_NAME: &str = "context-set";

    /// context-set chunk 안 record kind: `ContextKey` 정의(StableId, 이름, 값 타입, 기본값).
    pub const RECORD_CONTEXT_KEY: u8 = 0x01;
    /// context-set chunk 안 record kind: `ContextProvision`(공급 Frame StableId, key StableId, 값 expression).
    pub const RECORD_CONTEXT_PROVISION: u8 = 0x02;

    /// `ComponentDefinition` 공개 인터페이스 확장 section tag: `ContextRequirement` 목록. 정의 인코딩 끝(local
    /// token scope 뒤)의 tag 있는 확장 section 목록에 들어갑니다(지금 인코딩에는 확장 section이 없음).
    pub const COMPONENT_SECTION_CONTEXT_REQUIREMENTS: u8 = 0x01;
    /// Slot 정의 확장 section tag: 공개 Context key 목록(ADR-0025 결정 3.11).
    pub const SLOT_SECTION_CONTEXT_KEYS: u8 = 0x01;

    /// semantic snapshot 속성 key: 컴포넌트의 `ContextRequirement` 목록(`0xE7xx` 컴포넌트 대역의 빈 번호).
    pub const SNAPSHOT_COMPONENT_CONTEXT_REQUIREMENTS: u32 = 0xE705;
    /// semantic snapshot 속성 key: Slot의 공개 Context key 목록.
    pub const SNAPSHOT_SLOT_CONTEXT_KEYS: u32 = 0xE716;
    /// semantic snapshot 대역: `ContextKey`·`ContextProvision` 객체 속성(`0xECxx`).
    pub const SNAPSHOT_CONTEXT_BAND: u32 = 0xEC00;
}

#[cfg(test)]
mod tests {
    use super::context::*;

    #[test]
    fn context_numbers_do_not_overlap_existing_numbers() {
        // 검증 기준(IX.12.1): 예약 번호가 기존 번호와 겹치지 않는 golden. 기존 번호는 각 crate의 golden test가
        // 고정한 값입니다(ReferenceTarget tag: expressions `ast.rs`, chunk kind: storage `kinds.rs`, snapshot key:
        // semantic-diff `keys.rs`). 문서 crate test가 실제 ReferenceTarget tag와 다시 비교합니다.
        let reference_tags = [0x01u8, 0x02, 0x03, 0x04];
        assert!(!reference_tags.contains(&REFERENCE_TARGET_CONTEXT_TAG));
        assert_eq!(REFERENCE_TARGET_CONTEXT_TAG, 0x05);
        let chunk_kinds = [
            0x8000_0001u32,
            0x8000_0002,
            0x8000_0003,
            0x8000_0004,
            0x8000_0005,
            0x0000_0006,
        ];
        assert!(chunk_kinds
            .iter()
            .all(|code| code & 0x7FFF_FFFF != CHUNK_KIND_CONTEXT_SET & 0x7FFF_FFFF));
        assert_eq!(CHUNK_KIND_CONTEXT_SET >> 31, 1, "required kind");
        assert_ne!(RECORD_CONTEXT_KEY, RECORD_CONTEXT_PROVISION);
        let component_snapshot = [0xE701u32, 0xE702, 0xE703, 0xE704];
        let slot_snapshot = [0xE711u32, 0xE712, 0xE713, 0xE714, 0xE715];
        assert!(!component_snapshot.contains(&SNAPSHOT_COMPONENT_CONTEXT_REQUIREMENTS));
        assert!(!slot_snapshot.contains(&SNAPSHOT_SLOT_CONTEXT_KEYS));
        // 0xECxx는 snapshot 전용 대역(0xE0xx~0xEFxx)의 빈 대역입니다(0xE0~0xEB 사용 중).
        assert_eq!(SNAPSHOT_CONTEXT_BAND >> 8, 0xEC);
    }
}
