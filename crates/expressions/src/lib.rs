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
