//! stellar-git-porcelain — System Git porcelain 골격.
//!
//! document를 직접 수정하지 않고 snapshot 인터페이스(빈 trait)에만 의존합니다.

/// Stellar 프로젝트 snapshot 인터페이스(골격용 빈 trait).
pub trait SnapshotSource {}
