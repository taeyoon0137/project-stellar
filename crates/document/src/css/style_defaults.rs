//! 기본값 세트(style defaults): 명시하지 않은 longhand의 값 (ADR-0082, ADR-0086).
//!
//! 원본에는 사용자가 명시한 longhand만 있습니다([`crate::layout_record`]). 명시하지 않은 longhand는 활성 기본값
//! 세트가 정합니다. 레이아웃, CLI·MCP 조회, codegen이 모두 [`resolve`] 하나로 값을 풉니다. 엔진은 풀린 값만
//! 입력으로 받습니다(ADR-0082 결정 1).
//!
//! - 내장 세트는 [`CSS`](`css`, 기본)와 [`REACT_NATIVE`](`react-native`) 두 개입니다. 표 원본은 이 파일의
//!   [`builtin_entries`]이고, 문서 표(`docs/spec/style-defaults.md`)는 [`snapshot_text`]와 같아야 합니다.
//! - 사용자 세트([`UserSetSource`])는 `extends`로 다른 세트를 물려받고 longhand 몇 개만 바꿉니다. 사슬은 내장
//!   세트에서 끝나야 하고, 순환·모르는 세트는 거부합니다([`flatten`]).
//! - 세트 값은 노드 종류 구획([`NodeClass`])마다 다를 수 있습니다. 구획 값이 없으면 공통 값을 씁니다.
//! - 세트는 초기값만 정합니다. 명시 값은 세트 값과 같아도 명시로 남습니다(결정 5).
//!
//! 이 표가 다루는 longhand는 명시·미설정 구분이 있는 layout longhand([`LayoutLonghand`])입니다. payload 속성
//! (`background-color`, 테두리, payload 크기 …)은 모델에서 언제나 값이 있는 명시 속성이라 세트가 끼어들지
//! 않습니다.

use std::collections::BTreeMap;
use std::fmt;

use crate::layout_record::{
    format_value, payload_size, LayoutLonghand, LayoutRecord, LayoutValue, LayoutValueError,
};
use crate::node_kind::NodeKind;
use crate::records::NodePayload;

/// `style_defaults` 키가 없을 때의 세트.
pub const DEFAULT_SET: &str = CSS;
/// 브라우저 기본값 세트 이름.
pub const CSS: &str = "css";
/// React Native(Yoga) 기본값 세트 이름.
pub const REACT_NATIVE: &str = "react-native";
/// 내장 세트 이름(표시 순).
pub const BUILTIN_SETS: [&str; 2] = [CSS, REACT_NATIVE];
/// `extends` 사슬 최대 길이(사용자 세트 수).
pub const MAX_EXTENDS_DEPTH: usize = 16;

/// 세트 값의 노드 종류 구획.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodeClass {
    /// Frame과 ComponentInstance(정의 root Frame을 그립니다).
    Frame,
    /// Text.
    Text,
    /// Image.
    Image,
    /// Shape와 Path.
    Shape,
}

impl NodeClass {
    /// 모든 구획(표시 순).
    pub const ALL: [NodeClass; 4] = [
        NodeClass::Frame,
        NodeClass::Text,
        NodeClass::Image,
        NodeClass::Shape,
    ];

    /// 파일·표의 구획 이름.
    pub fn name(self) -> &'static str {
        match self {
            NodeClass::Frame => "frame",
            NodeClass::Text => "text",
            NodeClass::Image => "image",
            NodeClass::Shape => "shape",
        }
    }

    /// 이름에서 찾습니다.
    pub fn from_name(name: &str) -> Option<NodeClass> {
        NodeClass::ALL.into_iter().find(|c| c.name() == name)
    }

    /// node 종류의 구획. Fragment는 상자가 없어 `None`입니다.
    pub fn of(kind: NodeKind) -> Option<NodeClass> {
        match kind {
            NodeKind::Frame | NodeKind::ComponentInstance => Some(NodeClass::Frame),
            NodeKind::Text => Some(NodeClass::Text),
            NodeKind::Image => Some(NodeClass::Image),
            NodeKind::Shape | NodeKind::Path => Some(NodeClass::Shape),
            NodeKind::Fragment => None,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// 내장 세트 값의 확인 근거.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// CSS initial value(명세).
    CssInitial,
    /// HTML UA stylesheet(`div` 등).
    UserAgent,
    /// Stellar 모델 대응(브라우저 기본과 같은 결과를 내는 값으로 정함).
    StellarMapping,
    /// React Native 공식 문서의 기본값.
    ReactNativeDocs,
    /// Yoga 동작(자동 최소 크기가 없음 등).
    YogaBehavior,
}

impl Basis {
    /// 표기 이름.
    pub fn name(self) -> &'static str {
        match self {
            Basis::CssInitial => "css-initial",
            Basis::UserAgent => "user-agent",
            Basis::StellarMapping => "stellar-mapping",
            Basis::ReactNativeDocs => "react-native-docs",
            Basis::YogaBehavior => "yoga-behavior",
        }
    }
}

/// 내장 세트 표 한 줄.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinEntry {
    /// 세트 이름.
    pub set: &'static str,
    /// 구획(`None`이면 모든 구획).
    pub class: Option<NodeClass>,
    /// longhand.
    pub longhand: LayoutLonghand,
    /// 정규 CSS 값 텍스트.
    pub value: &'static str,
    /// 확인 근거.
    pub basis: Basis,
}

const fn entry(
    set: &'static str,
    class: Option<NodeClass>,
    longhand: LayoutLonghand,
    value: &'static str,
    basis: Basis,
) -> BuiltinEntry {
    BuiltinEntry {
        set,
        class,
        longhand,
        value,
        basis,
    }
}

/// 내장 세트 표. `css`는 모든 longhand의 공통 값을 갖고, `react-native`는 `css`를 물려받아 다른 값만 둡니다.
/// 표에 없는 `react-native` 값은 `css` 값을 그대로 물려받습니다(확인하지 못한 항목 포함, ADR-0082 결정 2).
pub fn builtin_entries() -> &'static [BuiltinEntry] {
    use Basis::*;
    use LayoutLonghand as L;
    const C: &str = CSS;
    const R: &str = REACT_NATIVE;
    const FRAME: Option<NodeClass> = Some(NodeClass::Frame);
    const TABLE: &[BuiltinEntry] = &[
        entry(C, None, L::Position, "static", CssInitial),
        entry(C, None, L::BoxSizing, "content-box", CssInitial),
        entry(C, None, L::Top, "auto", CssInitial),
        entry(C, None, L::Right, "auto", CssInitial),
        entry(C, None, L::Bottom, "auto", CssInitial),
        entry(C, None, L::Left, "auto", CssInitial),
        entry(C, None, L::Width, "auto", CssInitial),
        entry(C, None, L::Height, "auto", CssInitial),
        entry(C, None, L::MinWidth, "auto", CssInitial),
        entry(C, None, L::MinHeight, "auto", CssInitial),
        entry(C, None, L::MaxWidth, "none", CssInitial),
        entry(C, None, L::MaxHeight, "none", CssInitial),
        entry(C, None, L::AspectRatio, "auto", CssInitial),
        entry(C, None, L::FlexGrow, "0", CssInitial),
        entry(C, None, L::FlexShrink, "1", CssInitial),
        entry(C, None, L::FlexBasis, "auto", CssInitial),
        entry(C, None, L::AlignSelf, "auto", CssInitial),
        entry(C, None, L::JustifySelf, "auto", CssInitial),
        entry(C, None, L::GridColumnStart, "auto", CssInitial),
        entry(C, None, L::GridColumnEnd, "auto", CssInitial),
        entry(C, None, L::GridRowStart, "auto", CssInitial),
        entry(C, None, L::GridRowEnd, "auto", CssInitial),
        // CSS initial은 `inline`이지만 Stellar에는 inline formatting context가 없고 Frame은 `div`입니다(ADR-0076
        // 결정 3). Text·Image·Shape도 상자 하나로 배치하므로 같은 값을 씁니다.
        entry(C, None, L::Display, "block", UserAgent),
        entry(C, None, L::PaddingTop, "0px", CssInitial),
        entry(C, None, L::PaddingRight, "0px", CssInitial),
        entry(C, None, L::PaddingBottom, "0px", CssInitial),
        entry(C, None, L::PaddingLeft, "0px", CssInitial),
        entry(C, None, L::FlexDirection, "row", CssInitial),
        entry(C, None, L::FlexWrap, "nowrap", CssInitial),
        entry(C, None, L::JustifyContent, "normal", CssInitial),
        entry(C, None, L::AlignContent, "normal", CssInitial),
        entry(C, None, L::AlignItems, "normal", CssInitial),
        // CSS initial은 `legacy`입니다. `legacy`는 정렬 keyword 없이 쓰면 `normal`과 같습니다.
        entry(C, None, L::JustifyItems, "normal", StellarMapping),
        entry(C, None, L::RowGap, "normal", CssInitial),
        entry(C, None, L::ColumnGap, "normal", CssInitial),
        entry(C, None, L::GridTemplateColumns, "none", CssInitial),
        entry(C, None, L::GridTemplateRows, "none", CssInitial),
        entry(C, None, L::GridAutoColumns, "auto", CssInitial),
        entry(C, None, L::GridAutoRows, "auto", CssInitial),
        entry(C, None, L::GridAutoFlow, "row", CssInitial),
        // React Native(Yoga): View·Text·Image 모두.
        entry(R, None, L::Position, "relative", ReactNativeDocs),
        entry(R, None, L::BoxSizing, "border-box", ReactNativeDocs),
        entry(R, None, L::FlexShrink, "0", ReactNativeDocs),
        entry(R, None, L::MinWidth, "0px", YogaBehavior),
        entry(R, None, L::MinHeight, "0px", YogaBehavior),
        // View(Frame): flex container, 세로 main 축.
        entry(R, FRAME, L::Display, "flex", ReactNativeDocs),
        entry(R, FRAME, L::FlexDirection, "column", ReactNativeDocs),
        entry(R, FRAME, L::AlignContent, "flex-start", ReactNativeDocs),
    ];
    TABLE
}

/// 내장 세트의 부모(`react-native` → `css`). `css`는 뿌리입니다.
fn builtin_parent(name: &str) -> Option<&'static str> {
    match name {
        REACT_NATIVE => Some(CSS),
        _ => None,
    }
}

/// 내장 세트인지.
pub fn is_builtin(name: &str) -> bool {
    BUILTIN_SETS.contains(&name)
}

/// 사용자 세트 원본(파일 `design/style-defaults/<name>.stellar.yaml`의 의미 모델).
#[derive(Debug, Clone, PartialEq)]
pub struct UserSetSource {
    /// 세트 이름(파일 이름 stem).
    pub name: String,
    /// 물려받는 세트.
    pub extends: String,
    /// 모든 구획 공통 값.
    pub common: Vec<(LayoutLonghand, LayoutValue)>,
    /// 구획별 값.
    pub sections: Vec<(NodeClass, Vec<(LayoutLonghand, LayoutValue)>)>,
}

/// 기본값 세트 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyleDefaultsError {
    /// 세트가 없습니다.
    UnknownSet {
        /// 이름.
        name: String,
    },
    /// `extends` 사슬이 순환합니다.
    Cycle {
        /// 사슬(첫 이름으로 다시 돌아옴).
        chain: Vec<String>,
    },
    /// `extends` 사슬이 너무 깁니다.
    TooDeep {
        /// 시작 세트.
        name: String,
    },
    /// 사용자 세트가 내장 세트 이름을 씁니다.
    ReservedName {
        /// 이름.
        name: String,
    },
    /// 세트 이름 형식이 아닙니다.
    InvalidName {
        /// 이름.
        name: String,
    },
    /// 같은 longhand를 한 구획에 두 번 적었습니다.
    DuplicateLonghand {
        /// 세트.
        set: String,
        /// longhand.
        longhand: LayoutLonghand,
    },
    /// 모르는 longhand 이름.
    UnknownLonghand {
        /// 세트.
        set: String,
        /// 이름.
        name: String,
    },
    /// 값이 longhand 문법에 맞지 않습니다(`var()` 포함).
    InvalidValue {
        /// 세트.
        set: String,
        /// longhand.
        longhand: LayoutLonghand,
        /// 원인.
        error: LayoutValueError,
    },
}

impl fmt::Display for StyleDefaultsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StyleDefaultsError::UnknownSet { name } => write!(f, "기본값 세트 {name}이 없습니다"),
            StyleDefaultsError::Cycle { chain } => {
                write!(f, "extends 사슬이 순환합니다: {}", chain.join(" → "))
            }
            StyleDefaultsError::TooDeep { name } => write!(
                f,
                "{name}의 extends 사슬이 {MAX_EXTENDS_DEPTH}단계를 넘습니다"
            ),
            StyleDefaultsError::ReservedName { name } => {
                write!(
                    f,
                    "{name}은 내장 세트 이름이라 사용자 세트에 쓸 수 없습니다"
                )
            }
            StyleDefaultsError::InvalidName { name } => write!(
                f,
                "세트 이름 {name}은 소문자·숫자·하이픈(첫 글자 소문자, 64자 이하)이어야 합니다"
            ),
            StyleDefaultsError::DuplicateLonghand { set, longhand } => {
                write!(f, "세트 {set}에 {longhand}가 두 번 있습니다")
            }
            StyleDefaultsError::UnknownLonghand { set, name } => {
                write!(
                    f,
                    "세트 {set}: {name}은 기본값을 둘 수 있는 longhand가 아닙니다"
                )
            }
            StyleDefaultsError::InvalidValue {
                set,
                longhand,
                error,
            } => write!(f, "세트 {set}의 {longhand}: {error}"),
        }
    }
}

impl std::error::Error for StyleDefaultsError {}

impl StyleDefaultsError {
    /// 안정 진단 code.
    pub fn code(&self) -> &'static str {
        match self {
            StyleDefaultsError::UnknownSet { .. } => "unknown-style-defaults",
            StyleDefaultsError::Cycle { .. } => "style-defaults-cycle",
            StyleDefaultsError::TooDeep { .. } => "style-defaults-too-deep",
            StyleDefaultsError::ReservedName { .. } => "style-defaults-reserved-name",
            StyleDefaultsError::InvalidName { .. } => "style-defaults-invalid-name",
            StyleDefaultsError::DuplicateLonghand { .. } => "style-defaults-duplicate-longhand",
            StyleDefaultsError::UnknownLonghand { .. } => "style-defaults-unknown-longhand",
            StyleDefaultsError::InvalidValue { .. } => "style-defaults-invalid-value",
        }
    }
}

/// 세트 이름 형식: `[a-z][a-z0-9-]{0,63}`.
pub fn is_valid_set_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// `extends` 사슬을 풀어 고정 배열로 평탄화한 세트.
#[derive(Debug, Clone, PartialEq)]
pub struct FlatSet {
    name: String,
    chain: Vec<String>,
    /// [구획][longhand 순번] 값.
    values: Vec<Vec<LayoutValue>>,
}

fn longhand_index(longhand: LayoutLonghand) -> usize {
    LayoutLonghand::ALL
        .iter()
        .position(|l| *l == longhand)
        .expect("ALL에 모든 longhand")
}

impl FlatSet {
    /// 세트 이름.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 자기부터 뿌리(`css`)까지의 사슬.
    pub fn chain(&self) -> &[String] {
        &self.chain
    }

    /// `class` 구획의 `longhand` 기본값.
    pub fn value(&self, class: NodeClass, longhand: LayoutLonghand) -> &LayoutValue {
        &self.values[class.index()][longhand_index(longhand)]
    }

    fn apply(&mut self, class: Option<NodeClass>, longhand: LayoutLonghand, value: LayoutValue) {
        let index = longhand_index(longhand);
        match class {
            Some(class) => self.values[class.index()][index] = value,
            None => {
                for row in &mut self.values {
                    row[index] = value.clone();
                }
            }
        }
    }
}

fn builtin_value(entry: &BuiltinEntry) -> LayoutValue {
    entry
        .longhand
        .parse(entry.value)
        .unwrap_or_else(|e| panic!("내장 세트 {} {}: {e}", entry.set, entry.longhand))
}

fn flatten_builtin(name: &str) -> FlatSet {
    let mut chain = vec![name.to_string()];
    let mut parent = builtin_parent(name);
    while let Some(p) = parent {
        chain.push(p.to_string());
        parent = builtin_parent(p);
    }
    let placeholder = LayoutValue::Keyword("auto");
    let mut set = FlatSet {
        name: name.to_string(),
        chain: chain.clone(),
        values: vec![vec![placeholder; LayoutLonghand::ALL.len()]; NodeClass::ALL.len()],
    };
    // 뿌리부터 적용합니다.
    for link in chain.iter().rev() {
        for entry in builtin_entries().iter().filter(|e| e.set == link) {
            set.apply(entry.class, entry.longhand, builtin_value(entry));
        }
    }
    set
}

/// 내장 세트를 평탄화합니다.
pub fn builtin(name: &str) -> Option<FlatSet> {
    is_builtin(name).then(|| flatten_builtin(name))
}

/// 사용자 세트 하나를 검사합니다(이름, longhand 중복). 값 문법은 원본을 읽을 때 이미 검사됩니다.
pub fn check_source(source: &UserSetSource) -> Result<(), StyleDefaultsError> {
    if is_builtin(&source.name) {
        return Err(StyleDefaultsError::ReservedName {
            name: source.name.clone(),
        });
    }
    if !is_valid_set_name(&source.name) {
        return Err(StyleDefaultsError::InvalidName {
            name: source.name.clone(),
        });
    }
    let groups = std::iter::once(&source.common).chain(source.sections.iter().map(|(_, v)| v));
    for group in groups {
        let mut seen = std::collections::BTreeSet::new();
        for (longhand, value) in group {
            if !seen.insert(*longhand) {
                return Err(StyleDefaultsError::DuplicateLonghand {
                    set: source.name.clone(),
                    longhand: *longhand,
                });
            }
            if !longhand.accepts(value) {
                return Err(StyleDefaultsError::InvalidValue {
                    set: source.name.clone(),
                    longhand: *longhand,
                    error: LayoutValueError {
                        code: "invalid-value",
                        message: format!("{value}은 정규 값이 아닙니다"),
                    },
                });
            }
        }
    }
    Ok(())
}

/// `name` 세트를 평탄화합니다. `user`는 사용자 세트 원본을 이름으로 찾습니다.
pub fn flatten<'a>(
    name: &str,
    user: impl Fn(&str) -> Option<&'a UserSetSource>,
) -> Result<FlatSet, StyleDefaultsError> {
    let mut sources: Vec<&UserSetSource> = Vec::new();
    let mut current = name.to_string();
    let mut visited: Vec<String> = Vec::new();
    loop {
        if is_builtin(&current) {
            break;
        }
        if visited.contains(&current) {
            visited.push(current);
            return Err(StyleDefaultsError::Cycle { chain: visited });
        }
        if sources.len() >= MAX_EXTENDS_DEPTH {
            return Err(StyleDefaultsError::TooDeep {
                name: name.to_string(),
            });
        }
        let source = user(&current).ok_or_else(|| StyleDefaultsError::UnknownSet {
            name: current.clone(),
        })?;
        check_source(source)?;
        visited.push(current.clone());
        sources.push(source);
        current = source.extends.clone();
    }
    let mut set = flatten_builtin(&current);
    let mut chain: Vec<String> = sources.iter().map(|s| s.name.clone()).collect();
    chain.append(&mut set.chain);
    for source in sources.iter().rev() {
        for (longhand, value) in &source.common {
            set.apply(None, *longhand, value.clone());
        }
        for (class, values) in &source.sections {
            for (longhand, value) in values {
                set.apply(Some(*class), *longhand, value.clone());
            }
        }
    }
    set.name = name.to_string();
    set.chain = chain;
    Ok(set)
}

/// 값의 출처(ADR-0082 결정 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueSource {
    /// 사용자가 명시한 값(layout record 또는 payload).
    Explicit,
    /// 활성 기본값 세트의 값.
    Default {
        /// 세트 이름.
        set: String,
    },
}

impl fmt::Display for ValueSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueSource::Explicit => f.write_str("explicit"),
            ValueSource::Default { set } => write!(f, "default({set})"),
        }
    }
}

/// 풀린 longhand 하나.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedValue {
    /// longhand.
    pub longhand: LayoutLonghand,
    /// 값.
    pub value: LayoutValue,
    /// 출처.
    pub source: ValueSource,
}

/// node 하나의 풀린 layout longhand 전체([`LayoutLonghand::ALL`] 순서).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLayout {
    /// 구획.
    pub class: NodeClass,
    /// 값(모든 longhand).
    pub values: Vec<ResolvedValue>,
}

impl ResolvedLayout {
    /// longhand 값.
    pub fn get(&self, longhand: LayoutLonghand) -> &ResolvedValue {
        &self.values[longhand_index(longhand)]
    }

    /// 엔진 입력용 `(longhand 이름, 정규 CSS 값)` 목록. 모든 longhand가 한 번씩 나옵니다.
    pub fn declarations(&self) -> Vec<(&'static str, String)> {
        self.values
            .iter()
            .map(|v| (v.longhand.name(), format_value(&v.value)))
            .collect()
    }
}

/// node의 layout longhand를 모두 풉니다(ADR-0082 결정 1). 명시 값(layout record, payload 크기)이 있으면 그
/// 값이고 출처는 `explicit`, 없으면 `set`의 구획 값이고 출처는 `default(<세트>)`입니다. node 종류에 붙지 않는
/// longhand(Frame이 아닌 node의 container longhand)는 언제나 세트 값입니다. Fragment는 상자가 없어 `None`입니다.
pub fn resolve(
    set: &FlatSet,
    payload: &NodePayload,
    record: Option<&LayoutRecord>,
) -> Option<ResolvedLayout> {
    let kind = payload.kind();
    let class = NodeClass::of(kind)?;
    let size = payload_size(payload);
    let values = LayoutLonghand::ALL
        .into_iter()
        .map(|longhand| {
            let payload_value = match (longhand, size) {
                (LayoutLonghand::Width, Some((w, _))) => Some(LayoutValue::Length(w + 0.0)),
                (LayoutLonghand::Height, Some((_, h))) => Some(LayoutValue::Length(h + 0.0)),
                _ => None,
            };
            let explicit = payload_value.or_else(|| {
                record
                    .filter(|_| longhand.applies_to(kind))
                    .and_then(|r| r.get(longhand).cloned())
            });
            match explicit {
                Some(value) => ResolvedValue {
                    longhand,
                    value,
                    source: ValueSource::Explicit,
                },
                None => ResolvedValue {
                    longhand,
                    value: set.value(class, longhand).clone(),
                    source: ValueSource::Default {
                        set: set.name.clone(),
                    },
                },
            }
        })
        .collect();
    Some(ResolvedLayout { class, values })
}

/// `active` 세트가 `target` 세트(대상 프레임워크 고유 기본값)와 다른 longhand와 `active` 값(구획별, longhand
/// 순서). codegen이 차이만 명시 선언으로 내는 데 씁니다(ADR-0082 결정 6). 두 세트가 같으면 빈 목록입니다.
pub fn differences(
    active: &FlatSet,
    target: &FlatSet,
    class: NodeClass,
) -> Vec<(LayoutLonghand, LayoutValue)> {
    LayoutLonghand::ALL
        .into_iter()
        .filter(|l| active.value(class, *l) != target.value(class, *l))
        .map(|l| (l, active.value(class, l).clone()))
        .collect()
}

/// 내장 세트 표 전체의 결정론적 텍스트(`docs/spec/style-defaults.md` 표와 비교).
///
/// 한 줄은 `| longhand | css | react-native |`이고 값은 `frame/text/image/shape` 구획 값이 같으면 하나, 다르면
/// `구획=값` 목록입니다. `react-native` 값이 `css`와 같으면 `=`(물려받음)입니다.
pub fn snapshot_text() -> String {
    let css = flatten_builtin(CSS);
    let rn = flatten_builtin(REACT_NATIVE);
    let cell = |set: &FlatSet, longhand: LayoutLonghand| -> String {
        let values: Vec<String> = NodeClass::ALL
            .iter()
            .map(|c| format_value(set.value(*c, longhand)))
            .collect();
        if values.iter().all(|v| *v == values[0]) {
            format!("`{}`", values[0])
        } else {
            NodeClass::ALL
                .iter()
                .zip(&values)
                .map(|(c, v)| format!("{}=`{v}`", c.name()))
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    let mut out = String::from("| longhand | css | react-native |\n|---|---|---|\n");
    for longhand in LayoutLonghand::ALL {
        let css_cell = cell(&css, longhand);
        let rn_cell = cell(&rn, longhand);
        let rn_cell = if rn_cell == css_cell {
            "=".to_string()
        } else {
            rn_cell
        };
        out.push_str(&format!(
            "| `{}` | {css_cell} | {rn_cell} |\n",
            longhand.name()
        ));
    }
    out
}

/// 사용자 세트 원본을 이름으로 모은 표.
pub type UserSets = BTreeMap<String, UserSetSource>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fragment::FragmentPayload;
    use crate::frame::FramePayload;
    use crate::text::TextPayload;
    use stellar_core_types::ids::StableId;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn user(name: &str, extends: &str, common: &[(LayoutLonghand, &str)]) -> UserSetSource {
        UserSetSource {
            name: name.into(),
            extends: extends.into(),
            common: common
                .iter()
                .map(|(l, v)| (*l, l.parse(v).unwrap()))
                .collect(),
            sections: Vec::new(),
        }
    }

    #[test]
    fn builtin_tables_cover_every_longhand_and_parse() {
        for longhand in LayoutLonghand::ALL {
            let rows: Vec<_> = builtin_entries()
                .iter()
                .filter(|e| e.set == CSS && e.longhand == longhand)
                .collect();
            assert_eq!(rows.len(), 1, "css 세트에 {longhand} 공통 값 하나");
            assert!(rows[0].class.is_none());
        }
        for entry in builtin_entries() {
            assert!(is_builtin(entry.set));
            builtin_value(entry);
        }
        let mut seen = std::collections::BTreeSet::new();
        for entry in builtin_entries() {
            assert!(
                seen.insert((entry.set, entry.class, entry.longhand)),
                "중복 {entry:?}"
            );
        }
    }

    #[test]
    fn css_set_is_css_initial_with_div_display_and_content_box() {
        let css = builtin(CSS).unwrap();
        for class in NodeClass::ALL {
            assert_eq!(
                css.value(class, LayoutLonghand::Display),
                &LayoutValue::Keyword("block")
            );
            assert_eq!(
                css.value(class, LayoutLonghand::BoxSizing),
                &LayoutValue::Keyword("content-box")
            );
            assert_eq!(
                css.value(class, LayoutLonghand::FlexShrink),
                &LayoutValue::Number(1.0)
            );
            assert_eq!(
                css.value(class, LayoutLonghand::MinWidth),
                &LayoutValue::Keyword("auto")
            );
        }
        assert_eq!(css.chain(), ["css"]);
    }

    #[test]
    fn react_native_set_inherits_css_and_overrides_view_defaults() {
        let rn = builtin(REACT_NATIVE).unwrap();
        assert_eq!(rn.chain(), ["react-native", "css"]);
        let frame = NodeClass::Frame;
        assert_eq!(
            rn.value(frame, LayoutLonghand::Display),
            &LayoutValue::Keyword("flex")
        );
        assert_eq!(
            rn.value(frame, LayoutLonghand::FlexDirection),
            &LayoutValue::Keyword("column")
        );
        assert_eq!(
            rn.value(frame, LayoutLonghand::AlignContent),
            &LayoutValue::Keyword("flex-start")
        );
        assert_eq!(
            rn.value(NodeClass::Text, LayoutLonghand::Display),
            &LayoutValue::Keyword("block"),
            "확인하지 못한 항목은 css 값을 물려받음"
        );
        for class in NodeClass::ALL {
            assert_eq!(
                rn.value(class, LayoutLonghand::FlexShrink),
                &LayoutValue::Number(0.0)
            );
            assert_eq!(
                rn.value(class, LayoutLonghand::MinWidth),
                &LayoutValue::Length(0.0)
            );
            assert_eq!(
                rn.value(class, LayoutLonghand::BoxSizing),
                &LayoutValue::Keyword("border-box")
            );
            assert_eq!(
                rn.value(class, LayoutLonghand::Position),
                &LayoutValue::Keyword("relative")
            );
        }
    }

    #[test]
    fn user_sets_extend_and_override_by_section() {
        let mut compact = user(
            "compact",
            REACT_NATIVE,
            &[(LayoutLonghand::FlexShrink, "1")],
        );
        compact.sections.push((
            NodeClass::Frame,
            vec![(LayoutLonghand::RowGap, LayoutValue::Length(8.0))],
        ));
        let dense = user("dense", "compact", &[(LayoutLonghand::RowGap, "4px")]);
        let sets: UserSets = [compact, dense]
            .into_iter()
            .map(|s| (s.name.clone(), s))
            .collect();
        let flat = flatten("dense", |n| sets.get(n)).unwrap();
        assert_eq!(flat.chain(), ["dense", "compact", "react-native", "css"]);
        assert_eq!(
            flat.value(NodeClass::Frame, LayoutLonghand::FlexShrink),
            &LayoutValue::Number(1.0)
        );
        // dense 공통 값이 compact frame 구획 값보다 나중(자식)이라 이깁니다.
        assert_eq!(
            flat.value(NodeClass::Frame, LayoutLonghand::RowGap),
            &LayoutValue::Length(4.0)
        );
        assert_eq!(
            flat.value(NodeClass::Text, LayoutLonghand::RowGap),
            &LayoutValue::Length(4.0)
        );
        let compact = flatten("compact", |n| sets.get(n)).unwrap();
        assert_eq!(
            compact.value(NodeClass::Frame, LayoutLonghand::RowGap),
            &LayoutValue::Length(8.0)
        );
        assert_eq!(
            compact.value(NodeClass::Text, LayoutLonghand::RowGap),
            &LayoutValue::Keyword("normal")
        );
    }

    #[test]
    fn cycles_unknown_sets_and_reserved_names_are_rejected() {
        let a = user("a", "b", &[]);
        let b = user("b", "a", &[]);
        let sets: UserSets = [a, b].into_iter().map(|s| (s.name.clone(), s)).collect();
        assert_eq!(
            flatten("a", |n| sets.get(n)).unwrap_err(),
            StyleDefaultsError::Cycle {
                chain: vec!["a".into(), "b".into(), "a".into()]
            }
        );
        assert_eq!(
            flatten("missing", |n| sets.get(n)).unwrap_err().code(),
            "unknown-style-defaults"
        );
        let orphan = user("orphan", "nowhere", &[]);
        let sets: UserSets = [(orphan.name.clone(), orphan)].into_iter().collect();
        assert_eq!(
            flatten("orphan", |n| sets.get(n)).unwrap_err(),
            StyleDefaultsError::UnknownSet {
                name: "nowhere".into()
            }
        );
        let reserved = user("css", "react-native", &[]);
        assert_eq!(
            check_source(&reserved).unwrap_err().code(),
            "style-defaults-reserved-name"
        );
        let bad = user("Bad_Name", "css", &[]);
        assert_eq!(
            check_source(&bad).unwrap_err().code(),
            "style-defaults-invalid-name"
        );
        let mut duplicate = user("dup", "css", &[(LayoutLonghand::Left, "1px")]);
        duplicate
            .common
            .push((LayoutLonghand::Left, LayoutValue::Length(2.0)));
        assert_eq!(
            check_source(&duplicate).unwrap_err().code(),
            "style-defaults-duplicate-longhand"
        );
        // 사슬 길이 상한.
        let chain: UserSets = (0..=MAX_EXTENDS_DEPTH)
            .map(|i| {
                let extends = if i == MAX_EXTENDS_DEPTH {
                    CSS.to_string()
                } else {
                    format!("s{}", i + 1)
                };
                let s = user(&format!("s{i}"), &extends, &[]);
                (s.name.clone(), s)
            })
            .collect();
        assert_eq!(
            flatten("s0", |n| chain.get(n)).unwrap_err().code(),
            "style-defaults-too-deep"
        );
    }

    #[test]
    fn resolve_marks_explicit_and_default_sources() {
        let css = builtin(CSS).unwrap();
        let rn = builtin(REACT_NATIVE).unwrap();
        let frame = NodePayload::Frame(FramePayload::default());
        let mut record = LayoutRecord::new(sid(1));
        record
            .set(
                LayoutLonghand::FlexDirection,
                LayoutValue::Keyword("column"),
            )
            .unwrap();
        let resolved = resolve(&rn, &frame, Some(&record)).unwrap();
        assert_eq!(resolved.declarations().len(), LayoutLonghand::ALL.len());
        let direction = resolved.get(LayoutLonghand::FlexDirection);
        assert_eq!(
            direction.source,
            ValueSource::Explicit,
            "세트 값과 같아도 명시"
        );
        assert_eq!(direction.source.to_string(), "explicit");
        let display = resolved.get(LayoutLonghand::Display);
        assert_eq!(display.value, LayoutValue::Keyword("flex"));
        assert_eq!(display.source.to_string(), "default(react-native)");
        // payload 크기는 명시 width·height.
        let width = resolved.get(LayoutLonghand::Width);
        assert_eq!(width.value, LayoutValue::Length(0.0));
        assert_eq!(width.source, ValueSource::Explicit);
        // Text의 container longhand는 언제나 세트 값.
        let text = NodePayload::Text(TextPayload::empty(0));
        let resolved = resolve(&css, &text, Some(&record)).unwrap();
        assert_eq!(resolved.class, NodeClass::Text);
        let direction = resolved.get(LayoutLonghand::FlexDirection);
        assert_eq!(direction.value, LayoutValue::Keyword("row"));
        assert_eq!(direction.source.to_string(), "default(css)");
        assert_eq!(
            resolved.get(LayoutLonghand::Width).value,
            LayoutValue::Keyword("auto")
        );
        assert!(resolve(&css, &NodePayload::Fragment(FragmentPayload), None).is_none());
    }

    #[test]
    fn differences_list_only_differing_values() {
        let css = builtin(CSS).unwrap();
        let rn = builtin(REACT_NATIVE).unwrap();
        assert!(differences(&css, &css, NodeClass::Frame).is_empty());
        let rn_over_css: Vec<String> = differences(&rn, &css, NodeClass::Frame)
            .iter()
            .map(|(l, v)| format!("{l}: {v}"))
            .collect();
        assert_eq!(
            rn_over_css,
            [
                "position: relative",
                "box-sizing: border-box",
                "min-width: 0px",
                "min-height: 0px",
                "flex-shrink: 0",
                "display: flex",
                "flex-direction: column",
                "align-content: flex-start",
            ]
        );
        let css_over_rn: Vec<String> = differences(&css, &rn, NodeClass::Text)
            .iter()
            .map(|(l, v)| format!("{l}: {v}"))
            .collect();
        assert_eq!(
            css_over_rn,
            [
                "position: static",
                "box-sizing: content-box",
                "min-width: auto",
                "min-height: auto",
                "flex-shrink: 1",
            ]
        );
    }

    #[test]
    fn spec_document_table_matches_snapshot() {
        let spec = include_str!("../../../../docs/spec/style-defaults.md");
        let table = snapshot_text();
        assert!(
            spec.contains(&table),
            "docs/spec/style-defaults.md 표가 snapshot_text()와 다릅니다:\n{table}"
        );
    }
}
