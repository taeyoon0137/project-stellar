//! 증분 레이아웃 단일 leaf 변경 benchmark 실행기 (X.7.4).
//!
//! ```text
//! cargo run --release -p stellar-layout --example incremental_leaf_bench -- \
//!     [--out <path>] [--edits <n>] [--sections <n>] [--rows <n>]
//! ```
//!
//! 기본 합성 문서(200,201 node)에서 단일 leaf 너비 변경을 반복하고 결과 JSON(visited·cache hit·duration)을
//! 표준 출력 또는 `--out` 파일에 씁니다. 실패하면 오류를 표준 오류에 쓰고 종료 코드 1을 돌려줍니다.

use std::process::ExitCode;

use stellar_layout::bench::{run_leaf_edit_benchmark, LeafEditConfig};

/// 명령행 인자.
struct Args {
    out: Option<String>,
    config: LeafEditConfig,
}

/// `flag` 뒤의 수 값.
fn number(flag: &str, value: Option<String>) -> Result<usize, String> {
    let value = value.ok_or(format!("{flag} 뒤에 수가 없습니다"))?;
    value
        .parse()
        .map_err(|_| format!("{flag} 값이 수가 아닙니다: {value}"))
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        out: None,
        config: LeafEditConfig::DEFAULT,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--out" => args.out = Some(iter.next().ok_or("--out 뒤에 경로가 없습니다")?),
            "--edits" => args.config.edits = number("--edits", iter.next())?,
            "--sections" => args.config.sections = number("--sections", iter.next())?,
            "--rows" => args.config.rows = number("--rows", iter.next())?,
            other => return Err(format!("알 수 없는 인자: {other}")),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("incremental_leaf_bench: {error}");
            return ExitCode::FAILURE;
        }
    };
    let report = match run_leaf_edit_benchmark(&args.config) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("incremental_leaf_bench: benchmark 실패: {error}");
            return ExitCode::FAILURE;
        }
    };
    let json = report.to_json();
    match args.out {
        Some(path) => {
            if let Err(error) = std::fs::write(&path, json) {
                eprintln!("incremental_leaf_bench: {path} 쓰기 실패: {error}");
                return ExitCode::FAILURE;
            }
        }
        None => print!("{json}"),
    }
    ExitCode::SUCCESS
}
