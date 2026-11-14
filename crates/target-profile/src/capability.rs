//! capability 상태와 element·property·semantic contract별 registry 표 (IX.10.5).
//!
//! - [`CapabilityStatus`]: Native·Approximated·Unsupported·Deprecated(기술 아키텍처 17.2). code는 저장·지문 계약입니다.
//! - [`CapabilitySubject`]: 검사 대상. 문서 계층 타입에 의존하지 않도록 지역 numeric code로 표현합니다.
//!   - [`ElementKind`]: 문서 `NodeKind` 값과 같은 번호(Frame=1 … ComponentInstance=7).
//!   - [`ContractCode`]: 문서 control `ContractKind` 값과 같은 번호(Button=1 … SingleSelectionGroup=5, IX.6.7).
//!   - [`CapabilityPropertyId`]: capability가 플랫폼마다 다른 속성. 상위 16bit가 출처 namespace(0 공통, 1 CSS,
//!     2 Apple, 3 Android)입니다.
//! - [`StatusLadder`]: profile 수준([`ProfileLevel`])이 올라갈 때 상태가 바뀌는 계단입니다. 첫 계단은
//!   [`ProfileLevel::MIN`]에서 시작하고 수준은 엄격히 오름차순입니다.
//! - [`CapabilityRule`]: subject 하나의 종류별(Web·Apple·Android) 계단과 Apple 플랫폼별 override, Deprecated일 때의
//!   대체 subject입니다. 종류별 계단은 모두 필수입니다(누락을 Unsupported로 추정하지 않음).
//! - [`CapabilityTable`]: subject 오름차순 rule 표. 생성 시 구조를 검사합니다([`CapabilityTableError`]).
//!
//! 등록되지 않은 subject의 상태는 `None`입니다. 호출자(IX.10.6·IX.10.8)가 "등록되지 않음"으로 보고합니다.

use std::fmt;

use crate::profile::TargetProfileKind;
use crate::spec::{ApplePlatform, ProfileSpec, TargetProfile};

/// capability 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum CapabilityStatus {
    /// 플랫폼 기본 구성 요소로 그대로 표현됩니다.
    Native = 1,
    /// 근사 표현됩니다(결과가 달라질 수 있음).
    Approximated = 2,
    /// 표현할 수 없습니다.
    Unsupported = 3,
    /// 표현되지만 플랫폼이 사용 중단을 권고합니다.
    Deprecated = 4,
}

impl CapabilityStatus {
    /// 모든 상태(code 오름차순).
    pub const ALL: [CapabilityStatus; 4] = [
        CapabilityStatus::Native,
        CapabilityStatus::Approximated,
        CapabilityStatus::Unsupported,
        CapabilityStatus::Deprecated,
    ];

    /// 안정 numeric code.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// code의 역.
    pub const fn from_code(code: u8) -> Option<CapabilityStatus> {
        match code {
            1 => Some(CapabilityStatus::Native),
            2 => Some(CapabilityStatus::Approximated),
            3 => Some(CapabilityStatus::Unsupported),
            4 => Some(CapabilityStatus::Deprecated),
            _ => None,
        }
    }

    /// 안정 이름(snake_case).
    pub const fn name(self) -> &'static str {
        match self {
            CapabilityStatus::Native => "native",
            CapabilityStatus::Approximated => "approximated",
            CapabilityStatus::Unsupported => "unsupported",
            CapabilityStatus::Deprecated => "deprecated",
        }
    }
}

impl fmt::Display for CapabilityStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// element 종류(문서 `NodeKind` 번호와 같음).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ElementKind(pub u16);

impl ElementKind {
    /// Frame.
    pub const FRAME: ElementKind = ElementKind(1);
    /// Fragment.
    pub const FRAGMENT: ElementKind = ElementKind(2);
    /// Shape.
    pub const SHAPE: ElementKind = ElementKind(3);
    /// Path.
    pub const PATH: ElementKind = ElementKind(4);
    /// Text.
    pub const TEXT: ElementKind = ElementKind(5);
    /// Image.
    pub const IMAGE: ElementKind = ElementKind(6);
    /// ComponentInstance.
    pub const COMPONENT_INSTANCE: ElementKind = ElementKind(7);

    /// 알려진 모든 종류(번호 오름차순).
    pub const ALL: [ElementKind; 7] = [
        ElementKind::FRAME,
        ElementKind::FRAGMENT,
        ElementKind::SHAPE,
        ElementKind::PATH,
        ElementKind::TEXT,
        ElementKind::IMAGE,
        ElementKind::COMPONENT_INSTANCE,
    ];
}

/// semantic contract 종류(문서 control `ContractKind` 번호와 같음).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContractCode(pub u16);

impl ContractCode {
    /// Button.
    pub const BUTTON: ContractCode = ContractCode(1);
    /// TextInput.
    pub const TEXT_INPUT: ContractCode = ContractCode(2);
    /// Modal.
    pub const MODAL: ContractCode = ContractCode(3);
    /// List.
    pub const LIST: ContractCode = ContractCode(4);
    /// SingleSelectionGroup.
    pub const SINGLE_SELECTION_GROUP: ContractCode = ContractCode(5);

    /// 알려진 모든 종류(번호 오름차순).
    pub const ALL: [ContractCode; 5] = [
        ContractCode::BUTTON,
        ContractCode::TEXT_INPUT,
        ContractCode::MODAL,
        ContractCode::LIST,
        ContractCode::SINGLE_SELECTION_GROUP,
    ];
}

/// capability 속성 식별자. 상위 16bit는 출처 namespace입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CapabilityPropertyId(pub u32);

impl CapabilityPropertyId {
    /// 출처 namespace: 공통 시각 속성.
    pub const NAMESPACE_COMMON: u16 = 0;
    /// 출처 namespace: CSS 기원.
    pub const NAMESPACE_CSS: u16 = 1;
    /// 출처 namespace: Apple 기원.
    pub const NAMESPACE_APPLE: u16 = 2;
    /// 출처 namespace: Android 기원.
    pub const NAMESPACE_ANDROID: u16 = 3;

    /// 출처 namespace.
    pub const fn namespace(self) -> u16 {
        (self.0 >> 16) as u16
    }
}

/// capability 검사 대상.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CapabilitySubject {
    /// element 종류.
    Element(ElementKind),
    /// 속성.
    Property(CapabilityPropertyId),
    /// semantic contract.
    Contract(ContractCode),
}

impl CapabilitySubject {
    /// subject 분류 code(Element=1, Property=2, Contract=3).
    pub const fn class_code(self) -> u8 {
        match self {
            CapabilitySubject::Element(_) => 1,
            CapabilitySubject::Property(_) => 2,
            CapabilitySubject::Contract(_) => 3,
        }
    }

    /// 분류 안 번호.
    pub const fn raw(self) -> u32 {
        match self {
            CapabilitySubject::Element(kind) => kind.0 as u32,
            CapabilitySubject::Property(id) => id.0,
            CapabilitySubject::Contract(code) => code.0 as u32,
        }
    }
}

impl fmt::Display for CapabilitySubject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CapabilitySubject::Element(kind) => write!(f, "element:{}", kind.0),
            CapabilitySubject::Property(id) => write!(f, "property:{:#010x}", id.0),
            CapabilitySubject::Contract(code) => write!(f, "contract:{}", code.0),
        }
    }
}

/// 종류 안에서 비교하는 profile 수준. Apple은 최소 OS(major·minor·patch), Android는 API level, Web은 Baseline 연도.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileLevel(u64);

impl ProfileLevel {
    /// 가장 낮은 수준(모든 계단의 시작).
    pub const MIN: ProfileLevel = ProfileLevel(0);

    /// Apple OS 버전 수준.
    pub const fn apple(major: u16, minor: u16, patch: u16) -> ProfileLevel {
        ProfileLevel(((major as u64) << 32) | ((minor as u64) << 16) | patch as u64)
    }

    /// Android API level 수준.
    pub const fn android(api_level: u16) -> ProfileLevel {
        ProfileLevel(api_level as u64)
    }

    /// Web Baseline 연도 수준.
    pub const fn web(year: u16) -> ProfileLevel {
        ProfileLevel(year as u64)
    }

    /// profile의 수준.
    pub fn of(profile: &TargetProfile) -> ProfileLevel {
        match profile.spec {
            ProfileSpec::Apple(apple) => ProfileLevel::apple(
                apple.minimum_os.major(),
                apple.minimum_os.minor(),
                apple.minimum_os.patch(),
            ),
            ProfileSpec::Android(android) => ProfileLevel::android(android.api_level.get()),
            ProfileSpec::Web(web) => ProfileLevel::web(web.baseline.year()),
        }
    }

    /// 원시 값.
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// 종류에 맞춘 사람이 읽는 표기(`26.0`, `API 31`, `Baseline 2024`).
    pub fn describe(self, kind: TargetProfileKind) -> String {
        match kind {
            TargetProfileKind::Apple => {
                let major = (self.0 >> 32) & 0xFFFF;
                let minor = (self.0 >> 16) & 0xFFFF;
                let patch = self.0 & 0xFFFF;
                if patch == 0 {
                    format!("{major}.{minor}")
                } else {
                    format!("{major}.{minor}.{patch}")
                }
            }
            TargetProfileKind::Android => format!("API {}", self.0),
            TargetProfileKind::Web => format!("Baseline {}", self.0),
        }
    }
}

/// 상태 계단 한 칸.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StatusStep {
    /// 이 수준 이상에서.
    pub from: ProfileLevel,
    /// 상태.
    pub status: CapabilityStatus,
}

/// 수준별 상태 계단.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StatusLadder(Vec<StatusStep>);

impl StatusLadder {
    /// 모든 수준에서 같은 상태.
    pub fn constant(status: CapabilityStatus) -> StatusLadder {
        StatusLadder(vec![StatusStep {
            from: ProfileLevel::MIN,
            status,
        }])
    }

    /// 계단 목록으로 만듭니다(검사는 [`CapabilityTable::new`]).
    pub fn steps(steps: Vec<StatusStep>) -> StatusLadder {
        StatusLadder(steps)
    }

    /// 계단 목록.
    pub fn as_steps(&self) -> &[StatusStep] {
        &self.0
    }

    /// 수준의 상태(구조 검사를 통과한 계단 기준, 계단이 비면 `None`).
    pub fn status_at(&self, level: ProfileLevel) -> Option<CapabilityStatus> {
        self.0
            .iter()
            .take_while(|step| step.from <= level)
            .last()
            .map(|step| step.status)
    }

    /// `status`가 처음 나타나는 수준(없으면 `None`).
    pub fn first_level_with(
        &self,
        accept: impl Fn(CapabilityStatus) -> bool,
    ) -> Option<ProfileLevel> {
        self.0
            .iter()
            .find(|step| accept(step.status))
            .map(|step| step.from)
    }

    fn check(&self) -> Result<(), LadderProblem> {
        let Some(first) = self.0.first() else {
            return Err(LadderProblem::Empty);
        };
        if first.from != ProfileLevel::MIN {
            return Err(LadderProblem::DoesNotStartAtMin);
        }
        if self.0.windows(2).any(|pair| pair[0].from >= pair[1].from) {
            return Err(LadderProblem::NotAscending);
        }
        Ok(())
    }
}

/// subject 하나의 capability 규칙.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapabilityRule {
    /// 대상.
    pub subject: CapabilitySubject,
    /// Web 계단.
    pub web: StatusLadder,
    /// Apple 계단(플랫폼 override가 없을 때).
    pub apple: StatusLadder,
    /// Apple 플랫폼별 override 계단.
    pub apple_platforms: Vec<(ApplePlatform, StatusLadder)>,
    /// Android 계단.
    pub android: StatusLadder,
    /// Deprecated일 때 권장 대체 subject.
    pub replacement: Option<CapabilitySubject>,
}

impl CapabilityRule {
    /// 모든 종류에서 같은 상태인 규칙.
    pub fn uniform(subject: CapabilitySubject, status: CapabilityStatus) -> CapabilityRule {
        CapabilityRule {
            subject,
            web: StatusLadder::constant(status),
            apple: StatusLadder::constant(status),
            apple_platforms: Vec::new(),
            android: StatusLadder::constant(status),
            replacement: None,
        }
    }

    /// profile에 적용되는 계단.
    pub fn ladder_for(&self, profile: &TargetProfile) -> &StatusLadder {
        match profile.spec {
            ProfileSpec::Web(_) => &self.web,
            ProfileSpec::Android(_) => &self.android,
            ProfileSpec::Apple(apple) => self
                .apple_platforms
                .iter()
                .find(|(platform, _)| *platform == apple.platform)
                .map_or(&self.apple, |(_, ladder)| ladder),
        }
    }

    /// profile의 상태.
    pub fn status_for(&self, profile: &TargetProfile) -> Option<CapabilityStatus> {
        self.ladder_for(profile)
            .status_at(ProfileLevel::of(profile))
    }
}

/// 계단 구조 문제.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LadderProblem {
    /// 계단이 비었음.
    Empty,
    /// 첫 계단이 [`ProfileLevel::MIN`]이 아님.
    DoesNotStartAtMin,
    /// 수준이 엄격히 오름차순이 아님.
    NotAscending,
}

/// capability 표 구성 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityTableError {
    /// 같은 subject가 두 번.
    DuplicateSubject(CapabilitySubject),
    /// 계단 구조 위반.
    InvalidLadder {
        /// subject.
        subject: CapabilitySubject,
        /// 종류(Apple 플랫폼 override면 `Apple`).
        kind: TargetProfileKind,
        /// 문제.
        problem: LadderProblem,
    },
    /// 같은 Apple 플랫폼 override가 두 번.
    DuplicateApplePlatform {
        /// subject.
        subject: CapabilitySubject,
        /// 플랫폼.
        platform: ApplePlatform,
    },
    /// 대체 subject가 표에 없거나 자기 자신.
    InvalidReplacement {
        /// subject.
        subject: CapabilitySubject,
    },
}

impl fmt::Display for CapabilityTableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CapabilityTableError::DuplicateSubject(subject) => {
                write!(f, "capability subject 중복: {subject}")
            }
            CapabilityTableError::InvalidLadder {
                subject,
                kind,
                problem,
            } => write!(
                f,
                "{subject}의 {kind} 상태 계단이 잘못되었습니다: {problem:?}"
            ),
            CapabilityTableError::DuplicateApplePlatform { subject, platform } => {
                write!(f, "{subject}의 Apple {} override 중복", platform.name())
            }
            CapabilityTableError::InvalidReplacement { subject } => {
                write!(f, "{subject}의 대체 subject가 표에 없거나 자기 자신입니다")
            }
        }
    }
}

impl std::error::Error for CapabilityTableError {}

/// subject 오름차순 capability 규칙 표.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct CapabilityTable {
    rules: Vec<CapabilityRule>,
}

impl CapabilityTable {
    /// 빈 표.
    pub fn empty() -> CapabilityTable {
        CapabilityTable::default()
    }

    /// 규칙 목록으로 만듭니다(순서 무관). 구조를 검사합니다.
    pub fn new(mut rules: Vec<CapabilityRule>) -> Result<CapabilityTable, CapabilityTableError> {
        rules.sort_by_key(|rule| rule.subject);
        if let Some(pair) = rules
            .windows(2)
            .find(|pair| pair[0].subject == pair[1].subject)
        {
            return Err(CapabilityTableError::DuplicateSubject(pair[1].subject));
        }
        for rule in &rules {
            let ladders = [
                (TargetProfileKind::Web, &rule.web),
                (TargetProfileKind::Apple, &rule.apple),
                (TargetProfileKind::Android, &rule.android),
            ]
            .into_iter()
            .chain(
                rule.apple_platforms
                    .iter()
                    .map(|(_, ladder)| (TargetProfileKind::Apple, ladder)),
            );
            for (kind, ladder) in ladders {
                ladder
                    .check()
                    .map_err(|problem| CapabilityTableError::InvalidLadder {
                        subject: rule.subject,
                        kind,
                        problem,
                    })?;
            }
            for (index, (platform, _)) in rule.apple_platforms.iter().enumerate() {
                if rule.apple_platforms[..index]
                    .iter()
                    .any(|(other, _)| other == platform)
                {
                    return Err(CapabilityTableError::DuplicateApplePlatform {
                        subject: rule.subject,
                        platform: *platform,
                    });
                }
            }
        }
        let table = CapabilityTable { rules };
        for rule in &table.rules {
            if let Some(replacement) = rule.replacement {
                if replacement == rule.subject || table.rule(replacement).is_none() {
                    return Err(CapabilityTableError::InvalidReplacement {
                        subject: rule.subject,
                    });
                }
            }
        }
        Ok(table)
    }

    /// 규칙(subject 오름차순).
    pub fn rules(&self) -> &[CapabilityRule] {
        &self.rules
    }

    /// subject의 규칙.
    pub fn rule(&self, subject: CapabilitySubject) -> Option<&CapabilityRule> {
        self.rules
            .binary_search_by_key(&subject, |rule| rule.subject)
            .ok()
            .map(|index| &self.rules[index])
    }

    /// profile에서 subject의 상태. 등록되지 않은 subject는 `None`.
    pub fn status(
        &self,
        profile: &TargetProfile,
        subject: CapabilitySubject,
    ) -> Option<CapabilityStatus> {
        self.rule(subject)?.status_for(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::TargetProfileId;
    use crate::registry::ProfileRegistryVersion;
    use crate::spec::{AppleProfile, DottedVersion};

    fn apple(platform: ApplePlatform, major: u16) -> TargetProfile {
        TargetProfile::new(
            TargetProfileId::new("a").expect("id"),
            ProfileRegistryVersion::new(1).expect("v"),
            ProfileSpec::Apple(AppleProfile {
                platform,
                minimum_os: DottedVersion::new(major, 0, 0),
            }),
        )
    }

    #[test]
    fn status_codes_are_golden() {
        let golden = [
            (CapabilityStatus::Native, 1, "native"),
            (CapabilityStatus::Approximated, 2, "approximated"),
            (CapabilityStatus::Unsupported, 3, "unsupported"),
            (CapabilityStatus::Deprecated, 4, "deprecated"),
        ];
        for (status, code, name) in golden {
            assert_eq!(status.code(), code);
            assert_eq!(CapabilityStatus::from_code(code), Some(status));
            assert_eq!(status.name(), name);
        }
        assert_eq!(CapabilityStatus::from_code(0), None);
        assert_eq!(CapabilityStatus::from_code(5), None);
        // 문서 계층 번호와 같은 값(NodeKind·ContractKind).
        assert_eq!(ElementKind::ALL.map(|k| k.0), [1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(ContractCode::ALL.map(|c| c.0), [1, 2, 3, 4, 5]);
        assert_eq!(CapabilityPropertyId(0x0002_0001).namespace(), 2);
    }

    #[test]
    fn ladder_lookup_and_levels() {
        let ladder = StatusLadder::steps(vec![
            StatusStep {
                from: ProfileLevel::MIN,
                status: CapabilityStatus::Unsupported,
            },
            StatusStep {
                from: ProfileLevel::android(31),
                status: CapabilityStatus::Native,
            },
            StatusStep {
                from: ProfileLevel::android(35),
                status: CapabilityStatus::Deprecated,
            },
        ]);
        assert_eq!(ladder.check(), Ok(()));
        assert_eq!(
            ladder.status_at(ProfileLevel::android(30)),
            Some(CapabilityStatus::Unsupported)
        );
        assert_eq!(
            ladder.status_at(ProfileLevel::android(31)),
            Some(CapabilityStatus::Native)
        );
        assert_eq!(
            ladder.status_at(ProfileLevel::android(36)),
            Some(CapabilityStatus::Deprecated)
        );
        assert!(ProfileLevel::apple(17, 10, 0) > ProfileLevel::apple(17, 4, 0));
        assert!(ProfileLevel::apple(18, 0, 0) > ProfileLevel::apple(17, 65535, 65535));
        assert_eq!(
            ProfileLevel::apple(26, 1, 0).describe(TargetProfileKind::Apple),
            "26.1"
        );
        assert_eq!(
            ProfileLevel::android(31).describe(TargetProfileKind::Android),
            "API 31"
        );
        assert_eq!(
            ProfileLevel::web(2024).describe(TargetProfileKind::Web),
            "Baseline 2024"
        );
    }

    #[test]
    fn table_structure_is_checked() {
        let subject = CapabilitySubject::Element(ElementKind::FRAME);
        let bad = |ladder: StatusLadder| {
            let mut rule = CapabilityRule::uniform(subject, CapabilityStatus::Native);
            rule.android = ladder;
            CapabilityTable::new(vec![rule])
        };
        assert_eq!(
            bad(StatusLadder::steps(Vec::new())),
            Err(CapabilityTableError::InvalidLadder {
                subject,
                kind: TargetProfileKind::Android,
                problem: LadderProblem::Empty
            })
        );
        let step = |level: u16, status| StatusStep {
            from: ProfileLevel::android(level),
            status,
        };
        assert_eq!(
            bad(StatusLadder::steps(vec![step(
                21,
                CapabilityStatus::Native
            )])),
            Err(CapabilityTableError::InvalidLadder {
                subject,
                kind: TargetProfileKind::Android,
                problem: LadderProblem::DoesNotStartAtMin
            })
        );
        assert_eq!(
            bad(StatusLadder::steps(vec![
                step(0, CapabilityStatus::Native),
                step(30, CapabilityStatus::Deprecated),
                step(30, CapabilityStatus::Unsupported),
            ])),
            Err(CapabilityTableError::InvalidLadder {
                subject,
                kind: TargetProfileKind::Android,
                problem: LadderProblem::NotAscending
            })
        );
        let rule = CapabilityRule::uniform(subject, CapabilityStatus::Native);
        assert_eq!(
            CapabilityTable::new(vec![rule.clone(), rule.clone()]),
            Err(CapabilityTableError::DuplicateSubject(subject))
        );
        let mut doubled = rule.clone();
        doubled.apple_platforms = vec![
            (
                ApplePlatform::MacOs,
                StatusLadder::constant(CapabilityStatus::Native),
            ),
            (
                ApplePlatform::MacOs,
                StatusLadder::constant(CapabilityStatus::Native),
            ),
        ];
        assert_eq!(
            CapabilityTable::new(vec![doubled]),
            Err(CapabilityTableError::DuplicateApplePlatform {
                subject,
                platform: ApplePlatform::MacOs
            })
        );
        for replacement in [subject, CapabilitySubject::Element(ElementKind::TEXT)] {
            let mut self_replaced = rule.clone();
            self_replaced.replacement = Some(replacement);
            assert_eq!(
                CapabilityTable::new(vec![self_replaced]),
                Err(CapabilityTableError::InvalidReplacement { subject })
            );
        }
    }

    #[test]
    fn apple_platform_override_wins() {
        let subject = CapabilitySubject::Contract(ContractCode::SINGLE_SELECTION_GROUP);
        let mut rule = CapabilityRule::uniform(subject, CapabilityStatus::Approximated);
        rule.apple_platforms = vec![(
            ApplePlatform::MacOs,
            StatusLadder::constant(CapabilityStatus::Native),
        )];
        let table = CapabilityTable::new(vec![rule]).expect("표");
        assert_eq!(
            table.status(&apple(ApplePlatform::MacOs, 26), subject),
            Some(CapabilityStatus::Native)
        );
        assert_eq!(
            table.status(&apple(ApplePlatform::Ios, 26), subject),
            Some(CapabilityStatus::Approximated)
        );
        assert_eq!(
            table.status(
                &apple(ApplePlatform::Ios, 26),
                CapabilitySubject::Element(ElementKind::TEXT)
            ),
            None
        );
    }
}
