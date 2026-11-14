//! XIX.2 검증: MCP resource·tool catalog golden fixture.
//!
//! catalog 정의(`stellar_mcp::catalog`)를 JSON으로 쓴 결과가 `tests/fixtures/catalog.json`과 byte
//! 단위로 같아야 합니다. 정의를 바꾸면 이 golden이 실패하므로 의도한 변경일 때만
//! `STELLAR_MCP_BLESS=1`로 갱신합니다.

use stellar_mcp::catalog::catalog_snapshot;
use stellar_mcp::json::write;

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/catalog.json");

#[test]
fn catalog_matches_golden_fixture() {
    let mut text = write(&catalog_snapshot());
    text.push('\n');
    if std::env::var_os("STELLAR_MCP_BLESS").is_some() {
        std::fs::write(GOLDEN, &text).unwrap();
    }
    let golden = std::fs::read_to_string(GOLDEN).unwrap();
    assert_eq!(text, golden, "catalog golden 불일치");
}
