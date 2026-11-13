//! `stellar-bench-fixtures` — fixture를 필요할 때 생성해 요약을 JSON 한 줄로 출력합니다.
//!
//! ```text
//! stellar-bench-fixtures <fixture> [--seed <u64>]
//! ```
//!
//! 기준 장비·lab runner가 fixture를 저장소 밖에서 재현하고, 결과 content hash를 측정 결과와 함께
//! 기록하는 용도입니다. fixture 파일 자체는 쓰지 않습니다. 대형 tier는 `--release`로 실행합니다.

use std::process::ExitCode;
use std::time::Instant;

use stellar_bench_fixtures::cli::{parse_args, run, usage};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let request = match parse_args(&args) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("{message}\n{}", usage());
            return ExitCode::from(2);
        }
    };
    let started = Instant::now();
    match run(&request) {
        Ok(report) => {
            println!("{}", report.to_json(Some(started.elapsed().as_millis())));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("fixture 생성 실패: {error}");
            ExitCode::FAILURE
        }
    }
}
