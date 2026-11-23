//! 생성 schema 산출물 (XIX.9.2, ADR-0085 결정 2).
//!
//! CommandTransaction 입력 JSON Schema는 `cargo xtask generate-command-schema`가 commands crate의 닫힌 enum과
//! ops 필드 종류 표에서 생성해 `schemas/command-transaction.schema.json`에 둡니다. 이 crate는 그 파일만
//! 포함하고 commands crate에 의존하지 않습니다. 수기 schema는 두지 않습니다. drift는
//! `cargo xtask generate-command-schema --check`(xtask 테스트 포함)가 확인합니다.

/// CommandTransaction 입력 schema 원문(`document.propose_commands`·`document.apply_commands`의 `inputSchema`).
pub const COMMAND_TRANSACTION_SCHEMA: &str =
    include_str!("../../../schemas/command-transaction.schema.json");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn included_schema_is_the_generated_transaction_object() {
        assert!(COMMAND_TRANSACTION_SCHEMA
            .contains("\"$id\": \"stellar://schemas/command-transaction\""));
        assert!(COMMAND_TRANSACTION_SCHEMA.contains("cargo xtask generate-command-schema"));
        assert!(COMMAND_TRANSACTION_SCHEMA.contains("\"oneOf\""));
        assert!(COMMAND_TRANSACTION_SCHEMA.ends_with("}\n"));
    }
}
