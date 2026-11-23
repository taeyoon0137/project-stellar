//! 속성 registry CSS 매핑 → Web profile capability matrix (IX.11.19, ADR-0025 결정 1.3).
//!
//! crate 그래프상 이 crate는 문서 registry를 읽지 못합니다. 그래서 matrix는 **유도 함수**입니다. 상위 crate가
//! registry 항목을 [`RegistryInput`]으로 넘기면 [`web_capability`]가 Web 상태와 판정 근거를 돌려줍니다.
//!
//! - 대응표(ADR-0025 결정 1.3): `exact`·`converted` → Native, `approximated` → Approximated, `stellar-only` →
//!   구조 변환·path 굽기·래스터화 규칙이 있으면 Approximated, 출력이 없으면 Unsupported. Deprecated는 나오지
//!   않습니다. `initial-differs`·`subset`은 상태를 바꾸지 않습니다.
//! - 속성 모델 14장 확장(E1–E10)은 그 장의 Web export 열로 정합니다([`EXTENSIONS`]).
//! - capability 조건이 붙은 `converted`(예: `clipping` FrameBounds의 `overflow-clip-margin: <visual-box>`)는 기준
//!   browser baseline의 지원 여부로 정합니다([`CAPABILITY_CONDITIONS`]).
//! - `stellar-only` 항목의 출력 규칙은 [`STELLAR_ONLY_OUTPUTS`] 표가 정합니다. 표에 없는 `stellar-only` 항목은
//!   Unsupported입니다.
//! - Apple·Android 항목은 이 matrix에 없습니다(XXII.1.4가 lowering 규칙으로 더함).

use crate::capability::CapabilityStatus;

/// registry CSS 매핑 상태(문서 `MappingStatus`의 지역 거울).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MappingInput {
    /// `exact`.
    Exact,
    /// `converted`.
    Converted,
    /// `approximated`.
    Approximated,
    /// `stellar-only`.
    StellarOnly,
}

/// registry 항목 하나(상위 crate가 문서 registry에서 만듭니다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryInput<'a> {
    /// 범위 이름(`node`, `frame`, …, `modifier`).
    pub scope: &'a str,
    /// registry 이름(CSS longhand 또는 Stellar 이름).
    pub name: &'a str,
    /// CSS 매핑 상태.
    pub status: MappingInput,
}

/// Web 상태 판정 근거.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// ADR-0025 결정 1.3 대응표.
    MappingTable,
    /// 속성 모델 14장 확장.
    Extension(&'static str),
    /// 기준 browser baseline의 기능 지원 여부.
    Capability(&'static str),
    /// `stellar-only` 출력 규칙(구조 변환·path 굽기·래스터화).
    StellarOnlyRule(&'static str),
    /// `stellar-only`이고 출력 규칙이 없음.
    NoOutput,
}

/// matrix 항목 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebCapability {
    /// Web 상태.
    pub status: CapabilityStatus,
    /// 근거.
    pub basis: Basis,
}

/// 속성 모델 14장 확장(범위, 이름) → (ID, Web export 열을 결정 1.3으로 바꾼 상태).
pub const EXTENSIONS: &[(&str, &str, &str, CapabilityStatus)] = &[
    // E1 선형 공간 합성(Blend, 겹친 Opacity, Blur, Shadow).
    (
        "modifier",
        "mix-blend-mode",
        "E1",
        CapabilityStatus::Approximated,
    ),
    ("modifier", "opacity", "E1", CapabilityStatus::Approximated),
    (
        "modifier",
        "blur-radius",
        "E1",
        CapabilityStatus::Approximated,
    ),
    (
        "modifier",
        "shadow-color",
        "E1",
        CapabilityStatus::Approximated,
    ),
    (
        "modifier",
        "shadow-offset",
        "E1",
        CapabilityStatus::Approximated,
    ),
    (
        "modifier",
        "shadow-offset-x",
        "E1",
        CapabilityStatus::Approximated,
    ),
    (
        "modifier",
        "shadow-offset-y",
        "E1",
        CapabilityStatus::Approximated,
    ),
    (
        "modifier",
        "shadow-blur",
        "E1",
        CapabilityStatus::Approximated,
    ),
    // E2 modifier stack의 자유 순서: wrapper 분할.
    (
        "modifier",
        "modifier-order",
        "E2",
        CapabilityStatus::Approximated,
    ),
    // E5 initial-differs 기본값(Frame clip, Image fit): 명시 출력(converted).
    ("image", "object-fit", "E5", CapabilityStatus::Native),
    // E7 Line·Polygon·Path·Boolean 등 SVG 기하: SVG 또는 path 굽기.
    ("shape", "x1", "E7", CapabilityStatus::Approximated),
    ("shape", "y1", "E7", CapabilityStatus::Approximated),
    ("shape", "x2", "E7", CapabilityStatus::Approximated),
    ("shape", "y2", "E7", CapabilityStatus::Approximated),
    (
        "shape",
        "polygon-sides",
        "E7",
        CapabilityStatus::Approximated,
    ),
    (
        "shape",
        "polygon-rotation",
        "E7",
        CapabilityStatus::Approximated,
    ),
    (
        "shape",
        "polygon-corner-radius",
        "E7",
        CapabilityStatus::Approximated,
    ),
    (
        "modifier",
        "boolean-operation",
        "E7",
        CapabilityStatus::Approximated,
    ),
    (
        "modifier",
        "boolean-operands",
        "E7",
        CapabilityStatus::Approximated,
    ),
    // E9 두 축 indefinite aspect-ratio 해석.
    ("node", "aspect-ratio", "E9", CapabilityStatus::Approximated),
    // E10 비율 축 min-*: auto를 0으로 해석: min-*: 0px 명시 출력(converted).
    ("node", "min-width", "E10", CapabilityStatus::Native),
    ("node", "min-height", "E10", CapabilityStatus::Native),
];

/// capability 조건이 붙은 `converted` 항목(범위, 이름) → (필요 기능, 기준 browser 지원 여부).
///
/// 지원 여부는 기준 browser(Chrome 157, IX.11.8 기준과 같음)에서 `CSS.supports()`로 확인했습니다(보고서 IX.11.19).
pub const CAPABILITY_CONDITIONS: &[(&str, &str, &str, bool)] = &[(
    "frame",
    "clipping",
    "overflow-clip-margin: <visual-box>",
    true,
)];

/// `stellar-only` 항목의 출력 규칙(범위, 이름) → 규칙.
pub const STELLAR_ONLY_OUTPUTS: &[(&str, &str, &str)] = &[
    ("text", "text-content", "구조 변환: 요소 텍스트 내용"),
    ("image", "image-asset", "구조 변환: img src"),
    (
        "image",
        "image-crop",
        "구조 변환: crop wrapper(overflow 상자 안 위치 이동)",
    ),
    ("image", "image-color-space", "래스터화: 색공간 재해석"),
    (
        "modifier",
        "enabled",
        "구조 변환: 꺼진 modifier는 출력에서 뺌",
    ),
    ("modifier", "stroke-expand-width", "path 굽기: 외곽선 확장"),
    ("modifier", "corner-radius", "path 굽기: 모서리 둥글리기"),
];

fn find<T: Copy>(table: &[(&str, &str, T)], scope: &str, name: &str) -> Option<T> {
    table
        .iter()
        .find(|(s, n, _)| *s == scope && *n == name)
        .map(|(_, _, v)| *v)
}

/// registry 항목 하나의 Web 상태.
pub fn web_capability(input: RegistryInput<'_>) -> WebCapability {
    if let Some((id, status)) = EXTENSIONS
        .iter()
        .find(|(s, n, _, _)| *s == input.scope && *n == input.name)
        .map(|(_, _, id, status)| (*id, *status))
    {
        return WebCapability {
            status,
            basis: Basis::Extension(id),
        };
    }
    if let Some((feature, supported)) = CAPABILITY_CONDITIONS
        .iter()
        .find(|(s, n, _, _)| *s == input.scope && *n == input.name)
        .map(|(_, _, feature, supported)| (*feature, *supported))
    {
        return WebCapability {
            status: if supported {
                CapabilityStatus::Native
            } else {
                CapabilityStatus::Approximated
            },
            basis: Basis::Capability(feature),
        };
    }
    match input.status {
        MappingInput::Exact | MappingInput::Converted => WebCapability {
            status: CapabilityStatus::Native,
            basis: Basis::MappingTable,
        },
        MappingInput::Approximated => WebCapability {
            status: CapabilityStatus::Approximated,
            basis: Basis::MappingTable,
        },
        MappingInput::StellarOnly => match find(STELLAR_ONLY_OUTPUTS, input.scope, input.name) {
            Some(rule) => WebCapability {
                status: CapabilityStatus::Approximated,
                basis: Basis::StellarOnlyRule(rule),
            },
            None => WebCapability {
                status: CapabilityStatus::Unsupported,
                basis: Basis::NoOutput,
            },
        },
    }
}

/// ADR-0025 결정 1.3 대응표만 적용한 상태(확장·capability 조건 전).
pub const fn mapping_table(status: MappingInput, has_output_rule: bool) -> CapabilityStatus {
    match status {
        MappingInput::Exact | MappingInput::Converted => CapabilityStatus::Native,
        MappingInput::Approximated => CapabilityStatus::Approximated,
        MappingInput::StellarOnly if has_output_rule => CapabilityStatus::Approximated,
        MappingInput::StellarOnly => CapabilityStatus::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(
        scope: &'static str,
        name: &'static str,
        status: MappingInput,
    ) -> RegistryInput<'static> {
        RegistryInput {
            scope,
            name,
            status,
        }
    }

    #[test]
    fn mapping_table_follows_adr_0025_decision_1_3() {
        assert_eq!(
            web_capability(input("frame", "background-color", MappingInput::Exact)).status,
            CapabilityStatus::Native
        );
        assert_eq!(
            web_capability(input("frame", "transform", MappingInput::Converted)).status,
            CapabilityStatus::Native
        );
        assert_eq!(
            web_capability(input("modifier", "mask-image", MappingInput::Approximated)).status,
            CapabilityStatus::Approximated
        );
        assert_eq!(
            web_capability(input("node", "locked", MappingInput::StellarOnly)),
            WebCapability {
                status: CapabilityStatus::Unsupported,
                basis: Basis::NoOutput
            }
        );
        assert_eq!(
            web_capability(input("image", "image-asset", MappingInput::StellarOnly)).status,
            CapabilityStatus::Approximated
        );
        for status in [
            MappingInput::Exact,
            MappingInput::Converted,
            MappingInput::Approximated,
            MappingInput::StellarOnly,
        ] {
            assert_ne!(mapping_table(status, true), CapabilityStatus::Deprecated);
            assert_ne!(mapping_table(status, false), CapabilityStatus::Deprecated);
        }
    }

    #[test]
    fn extensions_and_capability_conditions_override_the_table() {
        let opacity = web_capability(input("modifier", "opacity", MappingInput::Exact));
        assert_eq!(opacity.status, CapabilityStatus::Approximated);
        assert_eq!(opacity.basis, Basis::Extension("E1"));
        let clip = web_capability(input("frame", "clipping", MappingInput::Converted));
        assert_eq!(clip.status, CapabilityStatus::Native);
        assert!(matches!(clip.basis, Basis::Capability(_)));
        for (scope, name, id, status) in EXTENSIONS {
            assert!(id.starts_with('E'), "{scope} {name}");
            assert_ne!(*status, CapabilityStatus::Deprecated);
        }
    }
}
