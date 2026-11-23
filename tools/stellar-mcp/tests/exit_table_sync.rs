//! XIX.9.9 검증: 결과 종류 ↔ 종료 코드 대응표 golden과 `stellar`·`stellar-mcp` 종료 코드 표 일치.
//!
//! - `tests/fixtures/result-exit-map.json`: `stellar_agent_protocol::outcome` 대응표(결과 종류, 종료 코드, 이름,
//!   재시도 가능, protocol 오류 여부). 의도한 변경이면 `STELLAR_MCP_BLESS=1`로 갱신합니다.
//! - `stellar-mcp`의 종료 코드 표(`stellar_mcp::exit::ALL`)가 `stellar`의 XXIV.1.2 표
//!   (`tools/stellar-cli/src/exit.rs`)와 값·이름이 같습니다. crate 의존이 아니라 소스 파일 비교입니다.

use std::collections::BTreeMap;

use stellar_mcp::agent_protocol::outcome::ResultKind;
use stellar_mcp::exit;
use stellar_mcp::json::{write, JsonValue as J};

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/result-exit-map.json"
);

#[test]
fn result_exit_map_matches_golden() {
    let rows = ResultKind::ALL
        .iter()
        .map(|kind| {
            J::object(vec![
                ("kind", J::string(kind.name())),
                ("exit_code", J::int(i64::from(kind.exit().code))),
                ("exit_name", J::string(kind.exit().name)),
                ("retryable", J::Bool(kind.retryable())),
                ("protocol_error", J::Bool(kind.is_protocol_error())),
            ])
        })
        .collect();
    let mut text = write(&J::Array(rows));
    text.push('\n');
    if std::env::var_os("STELLAR_MCP_BLESS").is_some() {
        std::fs::write(GOLDEN, &text).unwrap();
    }
    assert_eq!(
        text,
        std::fs::read_to_string(GOLDEN).unwrap(),
        "대응표 golden 불일치"
    );
    // 모든 대응 종료 코드는 stellar-mcp 표에 있습니다.
    for kind in ResultKind::ALL {
        assert_eq!(exit::for_kind(kind).name, kind.exit().name);
    }
}

/// `ExitStatus::<Variant> => <값>,` 줄에서 variant별 값을 모읍니다.
fn arms(source: &str, quoted: bool) -> Vec<(&str, &str)> {
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("ExitStatus::")?;
            let (variant, value) = rest.split_once(" => ")?;
            let value = value.strip_suffix(',')?;
            let value = if quoted {
                value.strip_prefix('"')?.strip_suffix('"')?
            } else {
                value.parse::<u8>().ok().map(|_| value)?
            };
            Some((variant, value))
        })
        .collect()
}

#[test]
fn bridge_exit_table_equals_the_cli_table() {
    let cli = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../stellar-cli/src/exit.rs"
    ));
    let codes: BTreeMap<&str, &str> = arms(cli, false).into_iter().collect();
    // 첫 match는 `exit_name`(ASCII 이름), 둘째는 설명 문구입니다.
    let names = arms(cli, true);
    assert_eq!(codes.len(), 13, "XXIV.1.2 표 13행");
    let mut table: Vec<(u8, &str)> = codes
        .iter()
        .map(|(variant, code)| {
            let name = names
                .iter()
                .find(|(v, n)| v == variant && n.is_ascii())
                .map(|(_, n)| *n)
                .unwrap();
            (code.parse().unwrap(), name)
        })
        .collect();
    table.sort_unstable();
    let bridge: Vec<(u8, &str)> = exit::ALL.iter().map(|e| (e.code, e.name)).collect();
    assert_eq!(bridge, table);
}
