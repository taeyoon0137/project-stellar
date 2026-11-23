//! CSS 속성 체계 (IX.11, ADR-0024, ADR-0075).
//!
//! 저장은 바이너리·텍스트 원본의 숫자 `PropertyKey` 그대로이고, CSS 이름·값 문법·매핑 상태는 registry
//! metadata입니다. 이 module은 registry가 공유하는 CSS 대응 규칙을 모읍니다.
//!
//! - [`border`]: Frame 테두리 4변 longhand key·값·레이아웃 입력(IX.11.10).
//! - [`bundle`]: 묶음 key와 longhand key 대응(IX.11.22).
//! - [`grammar`]: 값 문법 종류(IX.11.1).
//! - [`mapping`]: registry 항목별 CSS 매핑 metadata와 longhand 이름 inventory(IX.11.1).
//! - [`registry`]: 종류별 registry의 통합 view, 표시 문자열, snapshot(IX.11.2·IX.11.3).
//! - [`value`]: registry 값 문법별 formatter·parser(IX.11.13·IX.11.14).
//! - [`shorthand`]: shorthand → longhand 확장(IX.11.14).
//! - [`custom_property`]: 토큰·ContextKey의 CSS custom property 이름과 선언 텍스트(IX.11.16).
//! - [`declaration`]: 선언 블록 텍스트 해석·정규 출력(IX.11.15).
//! - [`typography`]: Text style·Typography CSS longhand key와 값(IX.11.11).
//! - [`style_defaults`]: 명시하지 않은 layout longhand를 푸는 기본값 세트(ADR-0082, ADR-0086).

pub mod border;
pub mod bundle;
pub mod custom_property;
pub mod declaration;
pub mod grammar;
pub mod mapping;
pub mod registry;
pub mod shorthand;
pub mod style_defaults;
pub mod typography;
pub mod value;

/// 예약 번호(IX.12.1)와 실제 `ReferenceTarget` tag 비교. document가 expressions에 의존하므로 여기서 봅니다.
#[cfg(test)]
mod reserved_numbers {
    use stellar_core_types::ids::StableId;
    use stellar_core_types::property::PropertyPath;
    use stellar_core_types::reference::TokenRef;
    use stellar_core_types::reserved::context::REFERENCE_TARGET_CONTEXT_TAG;
    use stellar_expressions::ast::{EnvironmentKey, ReferenceTarget};

    #[test]
    fn context_tag_is_not_an_existing_reference_target_tag() {
        // 검증 기준(IX.12.1): 예약 번호가 기존 번호와 겹치지 않음. ReferenceTarget의 모든 variant를 봅니다(새 variant가
        // 생기면 이 match가 컴파일 실패로 갱신을 요구합니다).
        let id = StableId::new_v7(1, 0, 1);
        let targets = [
            ReferenceTarget::Token(TokenRef::new(id)),
            ReferenceTarget::Property(PropertyPath::new(id)),
            ReferenceTarget::ComponentProperty(id),
            ReferenceTarget::Environment(EnvironmentKey::new(1)),
        ];
        for target in &targets {
            match target {
                ReferenceTarget::Token(_)
                | ReferenceTarget::Property(_)
                | ReferenceTarget::ComponentProperty(_)
                | ReferenceTarget::Environment(_) => {}
            }
            assert_ne!(target.tag(), REFERENCE_TARGET_CONTEXT_TAG);
        }
        let max = targets.iter().map(ReferenceTarget::tag).max().unwrap();
        assert_eq!(REFERENCE_TARGET_CONTEXT_TAG, max + 1, "다음 빈 tag");
    }
}
