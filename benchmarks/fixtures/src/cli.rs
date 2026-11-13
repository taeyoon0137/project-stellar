//! on-demand 생성 CLI의 인자 해석과 실행(바이너리는 얇은 wrapper).
//!
//! 인자는 `<fixture> [--seed <u64>]`뿐입니다. 알 수 없는 인자·중복·잘못된 숫자는 거부합니다.

use stellar_storage::hash::ContentHash;

use crate::error::FixtureError;
use crate::synthetic::{generate_synthetic, ScaleTier, SyntheticSpec};
use crate::GENERATOR_VERSION;

/// 기본 seed.
pub const DEFAULT_SEED: u64 = 1;

/// 생성 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// fixture 이름.
    pub fixture: String,
    /// seed.
    pub seed: u64,
}

/// 지원하는 fixture 이름.
pub fn fixture_names() -> Vec<&'static str> {
    ScaleTier::ALL.iter().map(|t| t.name()).collect()
}

/// 사용법 문자열.
pub fn usage() -> String {
    format!(
        "사용법: stellar-bench-fixtures <fixture> [--seed <u64>]\nfixture: {}",
        fixture_names().join(", ")
    )
}

/// 인자를 해석합니다.
pub fn parse_args(args: &[String]) -> Result<Request, String> {
    let mut fixture: Option<String> = None;
    let mut seed: Option<u64> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--seed" {
            if seed.is_some() {
                return Err("--seed가 두 번 있습니다".to_string());
            }
            let value = iter.next().ok_or("--seed 값이 없습니다")?;
            let parsed = value
                .parse::<u64>()
                .map_err(|_| format!("--seed 값이 u64가 아닙니다: {value:?}"))?;
            seed = Some(parsed);
        } else if arg.starts_with('-') {
            return Err(format!("알 수 없는 옵션: {arg:?}"));
        } else if fixture.is_some() {
            return Err(format!("fixture가 둘 이상입니다: {arg:?}"));
        } else if fixture_names().contains(&arg.as_str()) {
            fixture = Some(arg.clone());
        } else {
            return Err(format!("알 수 없는 fixture: {arg:?}"));
        }
    }
    Ok(Request {
        fixture: fixture.ok_or("fixture 이름이 없습니다")?,
        seed: seed.unwrap_or(DEFAULT_SEED),
    })
}

/// 생성 결과 요약.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// fixture 이름.
    pub fixture: String,
    /// seed.
    pub seed: u64,
    /// generator version.
    pub generator_version: u32,
    /// 숫자 지표(이름, 값). 이름은 정적 식별자입니다.
    pub metrics: Vec<(&'static str, u64)>,
    /// content hash.
    pub content_hash: ContentHash,
}

impl Report {
    /// JSON 한 줄. `elapsed_ms`가 있으면 마지막 필드로 넣습니다.
    pub fn to_json(&self, elapsed_ms: Option<u128>) -> String {
        let mut out = String::from("{");
        out.push_str(&format!("\"fixture\":{}", json_string(&self.fixture)));
        out.push_str(&format!(",\"seed\":{}", self.seed));
        out.push_str(&format!(
            ",\"generator_version\":{}",
            self.generator_version
        ));
        for (name, value) in &self.metrics {
            out.push_str(&format!(",{}:{value}", json_string(name)));
        }
        out.push_str(&format!(
            ",\"content_hash\":{}",
            json_string(&self.content_hash.to_string())
        ));
        if let Some(elapsed) = elapsed_ms {
            out.push_str(&format!(",\"elapsed_ms\":{elapsed}"));
        }
        out.push('}');
        out
    }
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 요청한 fixture를 생성합니다.
pub fn run(request: &Request) -> Result<Report, FixtureError> {
    let tier = ScaleTier::from_name(&request.fixture).ok_or(FixtureError::InvalidSpec {
        reason: "알 수 없는 fixture",
    })?;
    let summary = generate_synthetic(&SyntheticSpec::tier(tier, request.seed), |_| Ok(()))?;
    Ok(Report {
        fixture: summary.name,
        seed: summary.seed,
        generator_version: GENERATOR_VERSION,
        metrics: vec![
            ("documents", summary.documents as u64),
            ("nodes", summary.nodes as u64),
            ("chunks", summary.chunks),
            ("payload_bytes", summary.payload_bytes),
        ],
        content_hash: summary.content_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn arguments_are_parsed_strictly() {
        assert_eq!(
            parse_args(&args(&["nodes-10k"])),
            Ok(Request {
                fixture: "nodes-10k".into(),
                seed: DEFAULT_SEED
            })
        );
        assert_eq!(
            parse_args(&args(&["--seed", "42", "project-1m"])),
            Ok(Request {
                fixture: "project-1m".into(),
                seed: 42
            })
        );
        for bad in [
            &[][..],
            &["unknown"][..],
            &["nodes-10k", "nodes-200k"][..],
            &["nodes-10k", "--seed"][..],
            &["nodes-10k", "--seed", "-1"][..],
            &["nodes-10k", "--seed", "1", "--seed", "2"][..],
            &["nodes-10k", "--verbose"][..],
        ] {
            assert!(parse_args(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn report_json_is_escaped_and_ordered() {
        let report = Report {
            fixture: "a\"b".into(),
            seed: 3,
            generator_version: 1,
            metrics: vec![("nodes", 10)],
            content_hash: ContentHash::compute(b""),
        };
        let json = report.to_json(Some(5));
        assert!(json.starts_with("{\"fixture\":\"a\\\"b\",\"seed\":3,\"generator_version\":1,\"nodes\":10,\"content_hash\":\"sha256:"));
        assert!(json.ends_with(",\"elapsed_ms\":5}"));
        assert!(!report.to_json(None).contains("elapsed_ms"));
    }
}
