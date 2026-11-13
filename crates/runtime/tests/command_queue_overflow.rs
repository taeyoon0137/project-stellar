//! command queue overflow 정책 fixture (XII.1.2).
//!
//! `tests/fixtures/command_queue_overflow_v1.txt`의 각 case를 [`CommandQueue`]에 그대로 적용해
//! 접수 판정(enqueued/coalesced/evicted/full/closed)과 꺼내는 순서를 검사합니다.

use stellar_runtime::command_queue::{
    CoalesceKey, CommandPriority, CommandQueue, InputEventClass, Pushed, QueueCapacity,
    RejectReason, Rejected, SubmitOptions,
};

const FIXTURE: &str = include_str!("fixtures/command_queue_overflow_v1.txt");

/// fixture가 담은 case 수(파서가 case를 건너뛰지 않았는지 확인).
const EXPECTED_CASES: usize = 12;

#[derive(Debug, PartialEq, Eq)]
enum Expect {
    Enqueued,
    Coalesced(String),
    Evicted(String),
    Full,
    Closed,
}

fn parse_number<T: std::str::FromStr>(text: &str, line: usize) -> T {
    text.parse()
        .unwrap_or_else(|_| panic!("{line}행: 숫자가 아닙니다: {text}"))
}

fn parse_key(text: &str, line: usize) -> CoalesceKey {
    if text == "resize" {
        return CoalesceKey::Resize;
    }
    if let Some(device) = text.strip_prefix("pointer-move:") {
        return CoalesceKey::PointerMove {
            device: parse_number(device, line),
        };
    }
    if let Some(value) = text.strip_prefix("custom:") {
        return CoalesceKey::Custom(parse_number(value, line));
    }
    panic!("{line}행: 알 수 없는 병합 key: {text}");
}

fn parse_spec(text: &str, line: usize) -> SubmitOptions {
    if let Some(priority) = CommandPriority::from_name(text) {
        return SubmitOptions {
            priority,
            coalesce: None,
        };
    }
    let class = match text.strip_prefix("input:") {
        Some("resize") => InputEventClass::Resize,
        Some("wheel") => InputEventClass::Wheel,
        Some("discrete") => InputEventClass::Discrete,
        Some(rest) => match rest.strip_prefix("pointer-move:") {
            Some(device) => InputEventClass::PointerMove {
                device: parse_number(device, line),
            },
            None => panic!("{line}행: 알 수 없는 입력 분류: {text}"),
        },
        None => panic!("{line}행: 알 수 없는 spec: {text}"),
    };
    class.submit_options()
}

fn parse_expect(words: &[&str], line: usize) -> Expect {
    match words {
        ["enqueued"] => Expect::Enqueued,
        ["coalesced", label] => Expect::Coalesced((*label).to_string()),
        ["evicted", label] => Expect::Evicted((*label).to_string()),
        ["full"] => Expect::Full,
        ["closed"] => Expect::Closed,
        _ => panic!("{line}행: 알 수 없는 기대값: {words:?}"),
    }
}

fn actual(result: Result<Pushed<String>, Rejected<String>>, label: &str, line: usize) -> Expect {
    match result {
        Ok(Pushed::Enqueued) => Expect::Enqueued,
        Ok(Pushed::Coalesced { superseded }) => Expect::Coalesced(superseded),
        Ok(Pushed::EnqueuedEvicting { evicted }) => Expect::Evicted(evicted),
        Err(Rejected { reason, item }) => {
            // 거부된 항목은 호출자에게 그대로 돌아와야 합니다.
            assert_eq!(item, label, "{line}행: 거부 항목 반환");
            match reason {
                RejectReason::QueueFull { .. } => Expect::Full,
                RejectReason::Closed => Expect::Closed,
            }
        }
    }
}

/// fixture를 실행하고 (case 수, push 수)를 반환합니다.
fn run_fixture(source: &str) -> (usize, usize) {
    let mut queue: Option<CommandQueue<String>> = None;
    let mut case = String::new();
    let mut cases = 0;
    let mut pushes = 0;
    for (index, raw) in source.lines().enumerate() {
        let line = index + 1;
        let text = raw.trim();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = text.split_whitespace().collect();
        match words.as_slice() {
            ["case", name] => {
                assert!(queue.is_none(), "{line}행: 이전 case에 end가 없습니다");
                case = (*name).to_string();
            }
            ["capacity", input, command, background] => {
                let capacity = QueueCapacity::new(
                    parse_number(input, line),
                    parse_number(command, line),
                    parse_number(background, line),
                )
                .unwrap_or_else(|e| panic!("{line}행: {e}"));
                queue = Some(CommandQueue::new(capacity));
            }
            ["push", label, rest @ ..] => {
                let q = queue
                    .as_ref()
                    .unwrap_or_else(|| panic!("{line}행: capacity 없음"));
                let arrow = rest
                    .iter()
                    .position(|w| *w == "=>")
                    .unwrap_or_else(|| panic!("{line}행: => 없음"));
                let (spec_words, expect_words) = (&rest[..arrow], &rest[arrow + 1..]);
                let mut options = match spec_words.first() {
                    Some(spec) => parse_spec(spec, line),
                    None => panic!("{line}행: spec 없음"),
                };
                for extra in &spec_words[1..] {
                    match extra.strip_prefix("coalesce=") {
                        Some(key) => options = options.with_coalesce(parse_key(key, line)),
                        None => panic!("{line}행: 알 수 없는 옵션: {extra}"),
                    }
                }
                let expected = parse_expect(expect_words, line);
                let got = actual(q.try_push(options, (*label).to_string()), label, line);
                assert_eq!(got, expected, "case {case}, {line}행: push {label}");
                pushes += 1;
            }
            ["close"] => queue
                .as_ref()
                .unwrap_or_else(|| panic!("{line}행: capacity 없음"))
                .close(),
            ["drain", "=>", labels @ ..] => {
                let q = queue
                    .as_ref()
                    .unwrap_or_else(|| panic!("{line}행: capacity 없음"));
                let drained: Vec<String> = std::iter::from_fn(|| q.try_pop()).collect();
                let expected: Vec<String> = match labels {
                    ["-"] => Vec::new(),
                    _ => labels.iter().map(|l| (*l).to_string()).collect(),
                };
                assert_eq!(drained, expected, "case {case}, {line}행: drain 순서");
            }
            ["end"] => {
                let q = queue
                    .take()
                    .unwrap_or_else(|| panic!("{line}행: capacity 없음"));
                assert!(q.is_empty(), "case {case}: 끝에 대기 항목이 남았습니다");
                cases += 1;
            }
            _ => panic!("{line}행: 알 수 없는 지시: {text}"),
        }
    }
    assert!(queue.is_none(), "마지막 case에 end가 없습니다");
    (cases, pushes)
}

#[test]
fn overflow_policy_fixture_passes() {
    // 검증 기준(XII.1.2): queue overflow 정책 fixture 통과.
    let (cases, pushes) = run_fixture(FIXTURE);
    assert_eq!(cases, EXPECTED_CASES);
    assert_eq!(pushes, 56);
}

#[test]
fn fixture_runner_detects_a_wrong_expectation() {
    // 기대값이 틀리면 실패해야 합니다(러너가 판정을 무시하지 않음).
    let wrong =
        "case wrong\ncapacity 1 1 1\npush a input => enqueued\npush b input => enqueued\nend\n";
    let result = std::panic::catch_unwind(|| run_fixture(wrong));
    assert!(result.is_err());
}
