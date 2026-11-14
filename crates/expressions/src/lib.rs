//! stellar-expressions — Project Stellar 표현식 타입 시스템. core-types만 직접 의존합니다.

/// 표현식 타입과 nullable `Optional<T>` (VIII.1.1).
pub mod types;

/// type rule 공통 오류와 피연산자 위치 (VIII.1.2).
pub mod rules;

/// 산술 type rule table (VIII.1.2).
pub mod arithmetic;

/// 비교·논리·조건식 type rule (VIII.1.3).
pub mod logic;

/// 함수 signature registry (VIII.1.4).
pub mod functions;

/// 닫힌 표현식 연산 집합과 연산 직렬화 (VIII.1.5).
pub mod ops;

/// Expression AST node·payload·child 참조·구조 검증 (VIII.2.1, VIII.2.2).
pub mod ast;

/// AST node binary 직렬화와 canonical AST serializer (VIII.2.2, VIII.2.5).
pub mod codec;

/// AST cycle detector (VIII.2.3).
pub mod cycle;

/// AST type checker (VIII.2.4).
pub mod typecheck;

/// GUI 편집용 block·edge projection (VIII.3.1).
pub mod graph_view;

/// GUI socket 타입과 연결 가능 여부 (VIII.3.2).
pub mod sockets;

/// GUI block 추가·연결·삭제 command plan (VIII.3.3).
pub mod edit_plan;

/// AST 검증 오류의 GUI block·socket 매핑 (VIII.3.4).
pub mod gui_diagnostics;

/// 평가 공통 오류·참조 입력·값 의미 (VIII.4.1).
pub mod eval;

/// bytecode 내부 포맷과 compiler (VIII.4.1).
pub mod bytecode;

/// bytecode evaluator (VIII.4.1).
pub mod vm;

/// AST 기준 evaluator (VIII.4.1).
pub mod ast_eval;

/// color·string·수치 함수 evaluator (VIII.4.4).
pub mod function_eval;

/// 환경 입력 key registry (VIII.7.1).
pub mod environment;

/// 테스트 전용 지원 코드: 다른 crate가 따로 보관하는 상수의 대조용 재노출.
///
/// 이 crate의 unit test와, 기본값이 꺼진 `test-support` feature를 `[dev-dependencies]`에서 켠 crate의
/// 테스트에서만 빌드됩니다.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
