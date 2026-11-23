//! CSS 속성 체계 (IX.11, ADR-0024, ADR-0075).
//!
//! 저장은 바이너리·텍스트 원본의 숫자 `PropertyKey` 그대로이고, CSS 이름·값 문법·매핑 상태는 registry
//! metadata입니다. 이 module은 registry가 공유하는 CSS 대응 규칙을 모읍니다.
//!
//! - [`bundle`]: 묶음 key와 longhand key 대응(IX.11.22).

pub mod bundle;
