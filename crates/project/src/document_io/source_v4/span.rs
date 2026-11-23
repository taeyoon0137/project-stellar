//! node → 파일·줄 범위 표(TS-MAP, ADR-0077 결정 6).
//!
//! codec이 쓴 텍스트에서 `nodes` 목록 항목의 줄 범위를 찾습니다. canonical writer는 최상위 `nodes:` 아래 항목을
//! `  -` 한 줄로 시작하고 첫 키 `    id: "<StableId>"`를 쓰며, 문자열은 항상 한 줄로 escape합니다. 그래서 같은
//! 규칙으로 쓴 파일은 줄 단위 스캔으로 정확한 범위를 얻습니다. 읽을 때는 디스크 bytes를 이 스캔에 그대로 넣으므로
//! 사람이 고친 줄 번호도 그 파일 기준입니다.

use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;

/// 한 node의 원본 위치. 줄 번호는 1부터, 끝 줄을 포함합니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceSpan {
    /// `design/` 기준 상대 경로(`/` 구분).
    pub file: String,
    /// 항목 시작 줄(`  -`).
    pub start_line: u32,
    /// 항목 마지막 줄.
    pub end_line: u32,
}

/// 프로젝트 전체의 node 위치 표.
pub type SpanTable = BTreeMap<StableId, SourceSpan>;

/// `text`의 최상위 `nodes:` 목록에서 node 항목의 줄 범위를 찾습니다. `$ref` 항목은 node가 아니라 건너뜁니다.
pub fn node_spans(file: &str, text: &str) -> Vec<(StableId, SourceSpan)> {
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|line| *line == "nodes:") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut index = start + 1;
    while index < lines.len() {
        let line = lines[index];
        if !line.starts_with("  ") {
            break;
        }
        if line != "  -" {
            index += 1;
            continue;
        }
        let begin = index;
        let mut end = index;
        index += 1;
        while index < lines.len() && lines[index].starts_with("    ") && lines[index] != "  -" {
            end = index;
            index += 1;
        }
        let id = lines
            .get(begin + 1)
            .and_then(|line| line.strip_prefix("    id: \""))
            .and_then(|rest| rest.strip_suffix('"'))
            .and_then(|id| id.parse::<StableId>().ok());
        if let Some(id) = id {
            out.push((
                id,
                SourceSpan {
                    file: file.to_owned(),
                    start_line: begin as u32 + 1,
                    end_line: end as u32 + 1,
                },
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_cover_each_node_item_and_skip_refs() {
        let a = StableId::new_v7(1, 0, 1);
        let b = StableId::new_v7(2, 0, 2);
        let text = format!(
            "format: x\nnodes:\n  -\n    id: \"{a}\"\n    kind: frame\n    payload:\n      width: 1px\n  -\n    \"$ref\": \"a/b.stellar.yaml\"\n  -\n    id: \"{b}\"\n    kind: fragment\ntrailer: none\n"
        );
        let spans = node_spans("v.stellar.yaml", &text);
        assert_eq!(
            spans,
            vec![
                (
                    a,
                    SourceSpan {
                        file: "v.stellar.yaml".into(),
                        start_line: 3,
                        end_line: 7
                    }
                ),
                (
                    b,
                    SourceSpan {
                        file: "v.stellar.yaml".into(),
                        start_line: 10,
                        end_line: 12
                    }
                ),
            ]
        );
        assert!(node_spans("x", "nodes: []\n").is_empty());
    }
}
