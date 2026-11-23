//! CSS 속성 체계 (IX.11, ADR-0024, ADR-0075).
//!
//! 저장은 바이너리·텍스트 원본의 숫자 `PropertyKey` 그대로이고, CSS 이름·값 문법·매핑 상태는 registry
//! metadata입니다. 이 module은 registry가 공유하는 CSS 대응 규칙을 모읍니다.
//!
//! - [`bundle`]: 묶음 key와 longhand key 대응(IX.11.22).
//! - [`grammar`]: 값 문법 종류(IX.11.1).
//! - [`mapping`]: registry 항목별 CSS 매핑 metadata와 longhand 이름 inventory(IX.11.1).
//! - [`registry`]: 종류별 registry의 통합 view, 표시 문자열, snapshot(IX.11.2·IX.11.3).
//! - [`value`]: registry 값 문법별 formatter·parser(IX.11.13·IX.11.14).
//! - [`shorthand`]: shorthand → longhand 확장(IX.11.14).
//! - [`custom_property`]: 토큰·ContextKey의 CSS custom property 이름과 선언 텍스트(IX.11.16).

pub mod bundle;
pub mod custom_property;
pub mod grammar;
pub mod mapping;
pub mod registry;
pub mod shorthand;
pub mod value;
