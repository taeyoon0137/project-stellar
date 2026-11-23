//! 프로토타입 텍스트 원본 v1: 소유자 하나의 MotionSpec과 배치 hint (ADR-0094 결정 5).
//!
//! 텍스트 원본 v4(ADR-0077)와 같은 규칙을 씁니다. 줄 하나가 영속 값 하나이고, 값은 문서 CSS 값 formatter·parser
//! (`crate::css::value`)의 정규형이며, 읽은 값을 다시 쓴 결과가 입력과 byte 단위로 같아야 합니다. 모르는 키·빠진
//! 키·중복·다른 표기는 거부합니다. MotionSpec 키는 CSS `transition-*` longhand 이름을 그대로 씁니다.
//!
//! ```text
//! stellar-prototype: 1
//! owner: component <StableId> | view <StableId>
//! motion <StableId>
//!   transition-property: <root>/<key hex4> …
//!   transition-duration: <time>
//!   transition-delay: <time>
//!   transition-timing-function: linear | cubic-bezier(a, b, c, d) | steps(n, jump-*) | spring(m k c v)
//!   start: <StateVariable> changes | <StateVariable> becomes <value>
//!   reduced-motion: instant | play
//! hint <StableId> <x> <y>
//! ```
//!
//! motion은 id 순, 그 뒤 hint가 id 순입니다. MotionSpec과 hint가 모두 없으면 원본 파일을 쓰지 않습니다
//! ([`PrototypeSource::write`]가 `None`).

use std::fmt;
use std::str::FromStr;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::{EnumValue, TypedValue};
use stellar_core_types::value::{Angle, Duration, FiniteF32, Length, Percentage};

use super::layout_hint::{HintPoint, LayoutHints};
use crate::component::state_machine::StateVariableId;
use crate::css::grammar::{CssGrammar, LENGTH_SIGNED};
use crate::css::value::{
    self, format_angle, format_color, format_duration, format_length, format_number,
    format_percentage, parse_color, CssValue,
};
use crate::motion::spec::{
    MotionOwner, MotionSpec, MotionSpecId, MotionSpecInput, ReducedMotion, StartCondition,
    StepPosition, TimingFunction,
};

/// 형식 머리줄.
pub const HEADER: &str = "stellar-prototype: 1";

/// 소유자 하나의 프로토타입 원본.
#[derive(Debug, Clone, PartialEq)]
pub struct PrototypeSource {
    /// 소유자.
    pub owner: MotionOwner,
    /// MotionSpec(id 순).
    pub motions: Vec<MotionSpec>,
    /// 배치 hint.
    pub hints: LayoutHints,
}

/// 텍스트 원본 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceError {
    /// 1부터 센 줄 번호.
    pub line: usize,
    /// 이유.
    pub reason: String,
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}행: {}", self.line, self.reason)
    }
}

impl std::error::Error for SourceError {}

fn err(line: usize, reason: impl Into<String>) -> SourceError {
    SourceError {
        line,
        reason: reason.into(),
    }
}

/// 텍스트 원본 파일 크기 상한(bytes).
pub const MAX_SOURCE_BYTES: usize = 1 << 20;

fn format_owner(owner: MotionOwner) -> String {
    match owner {
        MotionOwner::Component(c) => format!("component {}", c.id()),
        MotionOwner::View(v) => format!("view {v}"),
    }
}

fn format_target(path: &PropertyPath) -> String {
    format!("{}/{:04x}", path.root(), path.keys()[0].0)
}

fn format_timing(timing: TimingFunction) -> String {
    match timing {
        TimingFunction::Linear => "linear".into(),
        TimingFunction::CubicBezier { x1, y1, x2, y2 } => format!(
            "cubic-bezier({}, {}, {}, {})",
            format_number(x1),
            format_number(y1),
            format_number(x2),
            format_number(y2)
        ),
        TimingFunction::Steps { count, position } => {
            format!("steps({count}, {})", position.css_name())
        }
        TimingFunction::Spring {
            mass,
            stiffness,
            damping,
            initial_velocity,
        } => format!(
            "spring({} {} {} {})",
            format_number(mass),
            format_number(stiffness),
            format_number(damping),
            format_number(initial_velocity)
        ),
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// 시작 조건 값의 텍스트. 표기할 수 없는 타입(gradient·shadow·typography·resource-ref)은 `None`입니다.
pub fn format_condition_value(value: &TypedValue) -> Option<String> {
    Some(match value {
        TypedValue::Boolean(b) => b.to_string(),
        TypedValue::Number(n) => format_number(n.get()),
        TypedValue::String(s) => escape(s),
        TypedValue::Enum(e) => format!("enum({} {})", e.enum_type(), e.variant()),
        TypedValue::Length(v) => format_length(v.get()),
        TypedValue::Percentage(v) => format_percentage(v.get()),
        TypedValue::Angle(v) => format_angle(v.get()),
        TypedValue::Duration(v) => format_duration(v.get()),
        TypedValue::Color(c) => format_color(*c),
        TypedValue::Gradient(_)
        | TypedValue::Shadow(_)
        | TypedValue::Typography(_)
        | TypedValue::ResourceRef(_) => return None,
    })
}

impl PrototypeSource {
    /// canonical 텍스트. MotionSpec과 hint가 모두 없으면 `None`(파일을 쓰지 않음). 표기할 수 없는 시작 조건 값이
    /// 있으면 오류입니다.
    pub fn write(&self) -> Result<Option<String>, SourceError> {
        if self.motions.is_empty() && self.hints.is_empty() {
            return Ok(None);
        }
        let mut out = String::new();
        out.push_str(HEADER);
        out.push('\n');
        out.push_str(&format!("owner: {}\n", format_owner(self.owner)));
        let mut motions: Vec<&MotionSpec> = self.motions.iter().collect();
        motions.sort_by_key(|m| m.id());
        for spec in motions {
            if spec.owner() != self.owner {
                return Err(err(0, format!("다른 소유자의 MotionSpec: {}", spec.id())));
            }
            out.push_str(&format!("motion {}\n", spec.id()));
            let targets: Vec<String> = spec.targets().iter().map(format_target).collect();
            out.push_str(&format!("  transition-property: {}\n", targets.join(" ")));
            out.push_str(&format!(
                "  transition-duration: {}\n",
                format_duration(spec.duration_ms())
            ));
            out.push_str(&format!(
                "  transition-delay: {}\n",
                format_duration(spec.delay_ms())
            ));
            out.push_str(&format!(
                "  transition-timing-function: {}\n",
                format_timing(spec.timing())
            ));
            let start = match spec.start() {
                StartCondition::Changes { variable } => {
                    format!("{} changes", variable.stable_id())
                }
                StartCondition::Becomes { variable, value } => {
                    let text = format_condition_value(value)
                        .ok_or_else(|| err(0, "시작 조건 값을 텍스트로 쓸 수 없습니다"))?;
                    format!("{} becomes {text}", variable.stable_id())
                }
            };
            out.push_str(&format!("  start: {start}\n"));
            out.push_str(&format!(
                "  reduced-motion: {}\n",
                spec.reduced_motion().name()
            ));
        }
        for (id, point) in self.hints.iter() {
            out.push_str(&format!(
                "hint {id} {} {}\n",
                format_number(point.x()),
                format_number(point.y())
            ));
        }
        Ok(Some(out))
    }

    /// canonical 텍스트를 읽습니다. 다시 쓴 결과가 입력과 같지 않으면 거부합니다.
    pub fn read(text: &str) -> Result<PrototypeSource, SourceError> {
        if text.len() > MAX_SOURCE_BYTES {
            return Err(err(0, "파일이 너무 큽니다"));
        }
        if !text.ends_with('\n') {
            return Err(err(0, "마지막 줄이 줄바꿈으로 끝나야 합니다"));
        }
        let mut lines = Lines {
            lines: text[..text.len() - 1].split('\n').collect(),
            at: 0,
        };
        let (n, header) = lines.next("머리")?;
        if header != HEADER {
            return Err(err(n, "모르는 형식 머리줄"));
        }
        let (n, owner_line) = lines.next("owner")?;
        let owner = parse_owner(n, field(n, owner_line, "owner: ")?)?;
        let mut source = PrototypeSource {
            owner,
            motions: Vec::new(),
            hints: LayoutHints::new(),
        };
        while !lines.done() {
            let (n, line) = lines.next("본문")?;
            if let Some(rest) = line.strip_prefix("motion ") {
                let id = MotionSpecId::new(parse_id(n, rest)?);
                let (n1, l) = lines.next("transition-property")?;
                let targets = field(n1, l, "  transition-property: ")?
                    .split(' ')
                    .map(|t| parse_target(n1, t))
                    .collect::<Result<Vec<_>, _>>()?;
                let (n2, l) = lines.next("transition-duration")?;
                let duration_ms = parse_time(n2, field(n2, l, "  transition-duration: ")?)?;
                let (n3, l) = lines.next("transition-delay")?;
                let delay_ms = parse_time(n3, field(n3, l, "  transition-delay: ")?)?;
                let (n4, l) = lines.next("transition-timing-function")?;
                let timing = parse_timing(n4, field(n4, l, "  transition-timing-function: ")?)?;
                let (n5, l) = lines.next("start")?;
                let start = parse_start(n5, field(n5, l, "  start: ")?)?;
                let (n6, l) = lines.next("reduced-motion")?;
                let reduced_motion = match field(n6, l, "  reduced-motion: ")? {
                    "instant" => ReducedMotion::Instant,
                    "play" => ReducedMotion::Play,
                    other => return Err(err(n6, format!("모르는 reduced-motion: {other}"))),
                };
                if !source.hints.is_empty() {
                    return Err(err(n, "motion은 hint보다 앞에 둡니다"));
                }
                if source.motions.last().is_some_and(|m| m.id() >= id) {
                    return Err(err(n, "motion이 id 오름차순이 아닙니다"));
                }
                let spec = MotionSpec::new(MotionSpecInput {
                    id,
                    owner,
                    targets,
                    start,
                    duration_ms,
                    delay_ms,
                    timing,
                    reduced_motion,
                })
                .map_err(|e| err(n, e.to_string()))?;
                source.motions.push(spec);
            } else if let Some(rest) = line.strip_prefix("hint ") {
                let parts: Vec<&str> = rest.split(' ').collect();
                let [id, x, y] = parts.as_slice() else {
                    return Err(err(n, "hint는 `hint <id> <x> <y>`입니다"));
                };
                let id = parse_id(n, id)?;
                if source
                    .hints
                    .iter()
                    .last()
                    .is_some_and(|(last, _)| last >= id)
                {
                    return Err(err(n, "hint가 id 오름차순이 아닙니다"));
                }
                let point = HintPoint::new(parse_number(n, x)?, parse_number(n, y)?)
                    .map_err(|e| err(n, e.to_string()))?;
                source.hints.pin(id, point);
            } else {
                return Err(err(n, format!("모르는 줄: {line}")));
            }
        }
        // 정규형 확인: 다시 쓴 결과가 같아야 합니다.
        let written = source.write()?.unwrap_or_default();
        if written != text {
            return Err(err(0, "정규형이 아닙니다"));
        }
        Ok(source)
    }
}

struct Lines<'a> {
    lines: Vec<&'a str>,
    at: usize,
}

impl<'a> Lines<'a> {
    fn next(&mut self, expect: &str) -> Result<(usize, &'a str), SourceError> {
        let line = *self
            .lines
            .get(self.at)
            .ok_or_else(|| err(self.at + 1, format!("{expect} 줄이 없습니다")))?;
        self.at += 1;
        Ok((self.at, line))
    }

    fn done(&self) -> bool {
        self.at >= self.lines.len()
    }
}

fn field<'a>(line: usize, text: &'a str, prefix: &str) -> Result<&'a str, SourceError> {
    text.strip_prefix(prefix)
        .ok_or_else(|| err(line, format!("`{}` 줄이어야 합니다", prefix.trim())))
}

fn parse_id(line: usize, text: &str) -> Result<StableId, SourceError> {
    let id =
        StableId::from_str(text).map_err(|_| err(line, format!("StableId가 아닙니다: {text}")))?;
    if id.to_string() != text {
        return Err(err(line, "StableId 정규형이 아닙니다"));
    }
    Ok(id)
}

fn parse_owner(line: usize, text: &str) -> Result<MotionOwner, SourceError> {
    if let Some(rest) = text.strip_prefix("component ") {
        return Ok(MotionOwner::Component(ComponentRef::new(parse_id(
            line, rest,
        )?)));
    }
    if let Some(rest) = text.strip_prefix("view ") {
        return Ok(MotionOwner::View(parse_id(line, rest)?));
    }
    Err(err(line, "owner는 `component <id>` 또는 `view <id>`입니다"))
}

fn parse_target(line: usize, text: &str) -> Result<PropertyPath, SourceError> {
    let (root, key) = text
        .split_once('/')
        .ok_or_else(|| err(line, "대상은 `<id>/<key>`입니다"))?;
    if key.len() != 4 || key.chars().any(|c| !matches!(c, '0'..='9' | 'a'..='f')) {
        return Err(err(line, "key는 소문자 hex 네 자리입니다"));
    }
    let key = u32::from_str_radix(key, 16).map_err(|_| err(line, "key"))?;
    Ok(PropertyPath::new(parse_id(line, root)?).push(PropertyKey(key)))
}

fn parse_with(line: usize, grammar: CssGrammar, text: &str) -> Result<CssValue, SourceError> {
    value::parse(grammar, text).map_err(|e| err(line, e.message.to_string()))
}

fn parse_number(line: usize, text: &str) -> Result<f32, SourceError> {
    match parse_with(line, CssGrammar::Number, text)? {
        CssValue::Number(v) if format_number(v) == text => Ok(v),
        _ => Err(err(line, format!("수 정규형이 아닙니다: {text}"))),
    }
}

fn parse_time(line: usize, text: &str) -> Result<f32, SourceError> {
    match parse_with(line, CssGrammar::Duration, text)? {
        CssValue::Duration(v) if format_duration(v) == text => Ok(v),
        _ => Err(err(line, format!("시간 정규형이 아닙니다: {text}"))),
    }
}

fn function_args<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.strip_prefix(name)?
        .strip_prefix('(')?
        .strip_suffix(')')
}

fn parse_timing(line: usize, text: &str) -> Result<TimingFunction, SourceError> {
    if text == "linear" {
        return Ok(TimingFunction::Linear);
    }
    if let Some(args) = function_args(text, "cubic-bezier") {
        let v: Vec<f32> = args
            .split(", ")
            .map(|a| parse_number(line, a))
            .collect::<Result<_, _>>()?;
        let [x1, y1, x2, y2] = v.as_slice() else {
            return Err(err(line, "cubic-bezier는 인자 네 개입니다"));
        };
        return Ok(TimingFunction::CubicBezier {
            x1: *x1,
            y1: *y1,
            x2: *x2,
            y2: *y2,
        });
    }
    if let Some(args) = function_args(text, "steps") {
        let (count, position) = args
            .split_once(", ")
            .ok_or_else(|| err(line, "steps는 `steps(n, jump-*)`입니다"))?;
        let count = match parse_with(
            line,
            CssGrammar::Count {
                min: 1,
                max: u32::MAX,
            },
            count,
        )? {
            CssValue::Count(n) if n.to_string() == count => n,
            _ => return Err(err(line, "steps 개수")),
        };
        let position = StepPosition::ALL
            .into_iter()
            .find(|p| p.css_name() == position)
            .ok_or_else(|| err(line, format!("모르는 step 위치: {position}")))?;
        return Ok(TimingFunction::Steps { count, position });
    }
    if let Some(args) = function_args(text, "spring") {
        let v: Vec<f32> = args
            .split(' ')
            .map(|a| parse_number(line, a))
            .collect::<Result<_, _>>()?;
        let [mass, stiffness, damping, initial_velocity] = v.as_slice() else {
            return Err(err(line, "spring은 인자 네 개입니다"));
        };
        return Ok(TimingFunction::Spring {
            mass: *mass,
            stiffness: *stiffness,
            damping: *damping,
            initial_velocity: *initial_velocity,
        });
    }
    Err(err(line, format!("모르는 timing function: {text}")))
}

fn parse_start(line: usize, text: &str) -> Result<StartCondition, SourceError> {
    let (id, rest) = text
        .split_once(' ')
        .ok_or_else(|| err(line, "start는 `<id> changes|becomes <값>`입니다"))?;
    let variable = StateVariableId::new(parse_id(line, id)?);
    if rest == "changes" {
        return Ok(StartCondition::Changes { variable });
    }
    let value = rest
        .strip_prefix("becomes ")
        .ok_or_else(|| err(line, "start는 `changes` 또는 `becomes <값>`입니다"))?;
    Ok(StartCondition::Becomes {
        variable,
        value: parse_condition_value(line, value)?,
    })
}

fn finite(line: usize, v: f32) -> Result<FiniteF32, SourceError> {
    FiniteF32::new(v).map_err(|_| err(line, "유한한 수가 아닙니다"))
}

/// 시작 조건 값 텍스트를 읽습니다. 정규형만 받습니다.
fn parse_condition_value(line: usize, text: &str) -> Result<TypedValue, SourceError> {
    let value = match text {
        "true" => TypedValue::Boolean(true),
        "false" => TypedValue::Boolean(false),
        _ if text.starts_with('"') => {
            let inner = text
                .strip_prefix('"')
                .and_then(|t| t.strip_suffix('"'))
                .ok_or_else(|| err(line, "문자열 따옴표"))?;
            let mut out = String::new();
            let mut chars = inner.chars();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => match chars.next() {
                        Some('"') => out.push('"'),
                        Some('\\') => out.push('\\'),
                        Some('n') => out.push('\n'),
                        _ => return Err(err(line, "모르는 escape")),
                    },
                    '"' => return Err(err(line, "escape하지 않은 따옴표")),
                    other => out.push(other),
                }
            }
            TypedValue::String(out)
        }
        _ if text.starts_with("enum(") => {
            let args = function_args(text, "enum").ok_or_else(|| err(line, "enum(…)"))?;
            let (t, v) = args
                .split_once(' ')
                .ok_or_else(|| err(line, "enum(<type> <variant>)"))?;
            let t: u32 = t.parse().map_err(|_| err(line, "enum type"))?;
            let v: u32 = v.parse().map_err(|_| err(line, "enum variant"))?;
            TypedValue::Enum(EnumValue::new(t, v))
        }
        _ if text.ends_with("px") => match parse_with(line, LENGTH_SIGNED, text)? {
            CssValue::Length(v) => TypedValue::Length(Length::from_raw(finite(line, v)?)),
            _ => return Err(err(line, "길이")),
        },
        _ if text.ends_with("deg") => match parse_with(line, CssGrammar::Angle, text)? {
            CssValue::Angle(v) => TypedValue::Angle(Angle::from_raw(finite(line, v)?)),
            _ => return Err(err(line, "각도")),
        },
        _ if text.ends_with("ms") => {
            TypedValue::Duration(Duration::from_raw(finite(line, parse_time(line, text)?)?))
        }
        _ if text.ends_with('%') => match parse_with(line, CssGrammar::Percentage, text)? {
            CssValue::Percentage(v) => {
                TypedValue::Percentage(Percentage::from_raw(finite(line, v)?))
            }
            _ => return Err(err(line, "백분율")),
        },
        _ if text.starts_with('#') || text.starts_with("color(") => {
            TypedValue::Color(parse_color(text).map_err(|e| err(line, e.message.to_string()))?)
        }
        _ => TypedValue::Number(finite(line, parse_number(line, text)?)?),
    };
    if format_condition_value(&value).as_deref() != Some(text) {
        return Err(err(line, format!("값 정규형이 아닙니다: {text}")));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::set::fixtures::{sample_set, sid};

    fn source_for(owner: MotionOwner) -> PrototypeSource {
        let set = sample_set();
        let mut hints = LayoutHints::new();
        hints.pin(sid(900), HintPoint::new(12.5, -3.0).expect("좌표"));
        hints.pin(sid(901), HintPoint::new(0.0, 40.0).expect("좌표"));
        PrototypeSource {
            owner,
            motions: set.owned_by(owner).cloned().collect(),
            hints,
        }
    }

    #[test]
    fn text_round_trip_is_byte_identical() {
        // 검증 기준(XXX.1.1): round-trip(텍스트 원본).
        for owner in [
            MotionOwner::Component(ComponentRef::new(sid(8))),
            MotionOwner::View(sid(7)),
        ] {
            let source = source_for(owner);
            let text = source.write().expect("쓰기").expect("내용 있음");
            let back = PrototypeSource::read(&text).expect("읽기");
            assert_eq!(back, source);
            assert_eq!(back.write().expect("쓰기"), Some(text));
        }
    }

    #[test]
    fn golden_text() {
        let text = source_for(MotionOwner::Component(ComponentRef::new(sid(8))))
            .write()
            .expect("쓰기")
            .expect("내용");
        let golden = include_str!("source_golden.txt");
        if text != golden {
            panic!("프로토타입 원본 golden이 다릅니다. 새 값:\n{text}");
        }
    }

    #[test]
    fn empty_owner_writes_no_file() {
        let source = PrototypeSource {
            owner: MotionOwner::View(sid(1)),
            motions: Vec::new(),
            hints: LayoutHints::new(),
        };
        assert_eq!(source.write(), Ok(None));
    }

    #[test]
    fn non_canonical_text_is_rejected() {
        let text = source_for(MotionOwner::Component(ComponentRef::new(sid(8))))
            .write()
            .expect("쓰기")
            .expect("내용");
        let variants = [
            text.replace("200ms", "200.0ms"),
            text.replace("cubic-bezier(0.25, 0.1, 0.25, 1)", "ease"),
            text.replace("reduced-motion: instant", "reduced-motion: none"),
            text.replace("jump-both", "both"),
            text.replace("  transition-delay", "  transition-delays"),
            text.trim_end().to_string(),
            format!("{text}unknown line\n"),
        ];
        for variant in variants {
            assert!(PrototypeSource::read(&variant).is_err(), "{variant}");
        }
    }

    #[test]
    fn condition_values_round_trip() {
        for text in [
            "true",
            "3.5",
            "\"on \\\"x\\\"\"",
            "enum(7 2)",
            "12px",
            "-4deg",
            "250ms",
            "50%",
            "#ff0000",
        ] {
            let value = parse_condition_value(1, text).expect(text);
            assert_eq!(format_condition_value(&value).as_deref(), Some(text));
        }
        for text in ["12.0px", "TRUE", "+3", "#FF0000"] {
            assert!(parse_condition_value(1, text).is_err(), "{text}");
        }
    }
}
