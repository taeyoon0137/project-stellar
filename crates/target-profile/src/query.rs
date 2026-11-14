//! 선택한 Target Profile의 element·property 생성 가능 여부와 비활성화 사유 조회 (IX.10.6).
//!
//! 편집기 palette·inspector(Phase XII)가 쓰는 조회 API입니다. 결과는 profile이 pin한 registry release
//! ([`ResolvedTargetProfile::registry`])의 capability 표로 계산하므로 앱 업데이트로 바뀌지 않습니다(IX.10.4).
//!
//! 생성 정책:
//!
//! | 상태 | 생성 | 비활성화 사유 |
//! |---|---|---|
//! | Native | 가능 | 없음 |
//! | Approximated | 가능(근사 표시는 상태로 전달) | 없음 |
//! | Unsupported | 불가 | [`DisabledReason::Unsupported`] — 더 높은 profile 수준에서 가능하면 그 최소 수준 |
//! | Deprecated | 불가(기존 사용은 IX.10.8 진단) | [`DisabledReason::Deprecated`] — 대체 subject |
//! | 미등록 | 불가 | [`DisabledReason::NotRegistered`] |

use crate::capability::{
    CapabilityPropertyId, CapabilityStatus, CapabilitySubject, ContractCode, ElementKind,
    ProfileLevel,
};
use crate::profile::TargetProfileKind;
use crate::registry::ResolvedTargetProfile;

/// 생성 비활성화 사유.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DisabledReason {
    /// 현재 profile 수준에서 표현할 수 없음.
    Unsupported {
        /// profile 종류.
        kind: TargetProfileKind,
        /// 같은 종류의 더 높은 수준에서 Native·Approximated가 되면 그 최소 수준.
        available_from: Option<ProfileLevel>,
    },
    /// 플랫폼이 사용 중단을 권고함.
    Deprecated {
        /// 권장 대체 subject.
        replacement: Option<CapabilitySubject>,
    },
    /// pin한 registry에 없는 subject.
    NotRegistered,
}

impl DisabledReason {
    /// 사람이 읽는 설명(한국어).
    pub fn describe(&self) -> String {
        match self {
            DisabledReason::Unsupported {
                kind,
                available_from: Some(level),
            } => format!(
                "이 {kind} profile에서 지원하지 않습니다({} 이상에서 지원)",
                level.describe(*kind)
            ),
            DisabledReason::Unsupported {
                kind,
                available_from: None,
            } => format!("{kind} profile에서 지원하지 않습니다"),
            DisabledReason::Deprecated {
                replacement: Some(subject),
            } => format!("사용 중단되었습니다(대체: {subject})"),
            DisabledReason::Deprecated { replacement: None } => "사용 중단되었습니다".to_string(),
            DisabledReason::NotRegistered => "capability registry에 없습니다".to_string(),
        }
    }
}

/// subject 하나의 생성 가능 여부.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CreationAvailability {
    /// 대상.
    pub subject: CapabilitySubject,
    /// 상태(미등록이면 `None`).
    pub status: Option<CapabilityStatus>,
    /// 새로 만들 수 있는지.
    pub creatable: bool,
    /// 만들 수 없을 때 사유.
    pub disabled_reason: Option<DisabledReason>,
}

/// 선택한 profile 기준 조회기.
#[derive(Debug, Clone, Copy)]
pub struct CapabilityQuery<'p, 'c> {
    resolved: &'p ResolvedTargetProfile<'c>,
}

impl<'p, 'c> CapabilityQuery<'p, 'c> {
    /// 해석한 profile(pin한 release 포함)로 조회기를 만듭니다.
    pub fn new(resolved: &'p ResolvedTargetProfile<'c>) -> CapabilityQuery<'p, 'c> {
        CapabilityQuery { resolved }
    }

    /// subject의 생성 가능 여부.
    pub fn subject(&self, subject: CapabilitySubject) -> CreationAvailability {
        let profile = self.resolved.profile();
        let table = self.resolved.registry().capabilities();
        let Some(rule) = table.rule(subject) else {
            return CreationAvailability {
                subject,
                status: None,
                creatable: false,
                disabled_reason: Some(DisabledReason::NotRegistered),
            };
        };
        let ladder = rule.ladder_for(profile);
        let level = ProfileLevel::of(profile);
        let status = ladder.status_at(level);
        let disabled_reason = match status {
            Some(CapabilityStatus::Native | CapabilityStatus::Approximated) => None,
            Some(CapabilityStatus::Deprecated) => Some(DisabledReason::Deprecated {
                replacement: rule.replacement,
            }),
            // 구조 검사를 통과한 계단은 항상 상태가 있습니다. 없으면 보수적으로 Unsupported로 막습니다.
            Some(CapabilityStatus::Unsupported) | None => Some(DisabledReason::Unsupported {
                kind: profile.kind(),
                available_from: ladder
                    .as_steps()
                    .iter()
                    .find(|step| {
                        step.from > level
                            && matches!(
                                step.status,
                                CapabilityStatus::Native | CapabilityStatus::Approximated
                            )
                    })
                    .map(|step| step.from),
            }),
        };
        CreationAvailability {
            subject,
            status: status.or(Some(CapabilityStatus::Unsupported)),
            creatable: disabled_reason.is_none(),
            disabled_reason,
        }
    }

    /// element 종류.
    pub fn element(&self, kind: ElementKind) -> CreationAvailability {
        self.subject(CapabilitySubject::Element(kind))
    }

    /// 속성.
    pub fn property(&self, id: CapabilityPropertyId) -> CreationAvailability {
        self.subject(CapabilitySubject::Property(id))
    }

    /// semantic contract.
    pub fn contract(&self, code: ContractCode) -> CreationAvailability {
        self.subject(CapabilitySubject::Contract(code))
    }

    /// 여러 subject를 입력 순서대로 조회합니다(palette filter).
    pub fn filter(
        &self,
        subjects: impl IntoIterator<Item = CapabilitySubject>,
    ) -> Vec<CreationAvailability> {
        subjects.into_iter().map(|s| self.subject(s)).collect()
    }

    /// 만들 수 있는 subject만 입력 순서대로 돌려줍니다.
    pub fn creatable(
        &self,
        subjects: impl IntoIterator<Item = CapabilitySubject>,
    ) -> Vec<CapabilitySubject> {
        subjects
            .into_iter()
            .filter(|s| self.subject(*s).creatable)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::fixtures::{android, apple, web};
    use crate::builtin::{catalog, properties as p};
    use crate::spec::ApplePlatform;
    use CapabilityStatus::{Approximated as A, Deprecated as D, Native as N, Unsupported as U};

    fn ok(subject: CapabilitySubject, status: CapabilityStatus) -> CreationAvailability {
        CreationAvailability {
            subject,
            status: Some(status),
            creatable: true,
            disabled_reason: None,
        }
    }

    fn blocked(
        subject: CapabilitySubject,
        status: CapabilityStatus,
        reason: DisabledReason,
    ) -> CreationAvailability {
        CreationAvailability {
            subject,
            status: Some(status),
            creatable: false,
            disabled_reason: Some(reason),
        }
    }

    #[test]
    fn controls_and_representative_properties_per_profile() {
        let catalog = catalog().expect("내장 catalog");
        let profiles = [
            web("web-2021", 2021, 1),
            web("web-2022", 2022, 1),
            apple("ios-26", ApplePlatform::Ios, 26, 1),
            apple("macos-26", ApplePlatform::MacOs, 26, 1),
            android("android-30", 30, 1),
            android("android-35", 35, 1),
        ];
        let resolved: Vec<_> = profiles
            .iter()
            .map(|profile| catalog.resolve(profile).expect("해석"))
            .collect();
        let query = |index: usize| CapabilityQuery::new(&resolved[index]);
        let (web21, web22, ios, macos, droid30, droid35) = (0, 1, 2, 3, 4, 5);

        // Button/TextInput/List는 모든 profile에서 Native로 생성 가능합니다.
        for code in [
            ContractCode::BUTTON,
            ContractCode::TEXT_INPUT,
            ContractCode::LIST,
        ] {
            for index in 0..resolved.len() {
                assert_eq!(
                    query(index).contract(code),
                    ok(CapabilitySubject::Contract(code), N),
                    "{code:?} @ {index}"
                );
            }
        }
        // Modal: Baseline 2022 이전 Web은 근사(생성 가능), 이후 Native.
        let modal = CapabilitySubject::Contract(ContractCode::MODAL);
        assert_eq!(query(web21).contract(ContractCode::MODAL), ok(modal, A));
        assert_eq!(query(web22).contract(ContractCode::MODAL), ok(modal, N));
        assert_eq!(query(ios).contract(ContractCode::MODAL), ok(modal, N));
        assert_eq!(query(droid30).contract(ContractCode::MODAL), ok(modal, N));

        let prop = CapabilitySubject::Property;
        // 대표 CSS 속성.
        assert_eq!(
            query(web22).property(p::CSS_SUBGRID),
            blocked(
                prop(p::CSS_SUBGRID),
                U,
                DisabledReason::Unsupported {
                    kind: TargetProfileKind::Web,
                    available_from: Some(ProfileLevel::web(2023)),
                }
            )
        );
        assert_eq!(
            query(web22).property(p::CSS_BACKDROP_FILTER),
            ok(prop(p::CSS_BACKDROP_FILTER), A)
        );
        assert_eq!(
            query(droid35).property(p::CSS_BACKDROP_FILTER),
            blocked(
                prop(p::CSS_BACKDROP_FILTER),
                U,
                DisabledReason::Unsupported {
                    kind: TargetProfileKind::Android,
                    available_from: None,
                }
            )
        );
        assert_eq!(
            query(web22).property(p::CSS_CLIP_RECT),
            blocked(
                prop(p::CSS_CLIP_RECT),
                D,
                DisabledReason::Deprecated {
                    replacement: Some(prop(p::CLIP_PATH)),
                }
            )
        );
        // 대표 Apple 속성.
        assert_eq!(
            query(ios).property(p::APPLE_SF_SYMBOL),
            ok(prop(p::APPLE_SF_SYMBOL), N)
        );
        assert_eq!(
            query(droid30).property(p::APPLE_SF_SYMBOL),
            ok(prop(p::APPLE_SF_SYMBOL), A)
        );
        assert_eq!(
            query(droid30).property(p::APPLE_MATERIAL_VIBRANCY),
            blocked(
                prop(p::APPLE_MATERIAL_VIBRANCY),
                U,
                DisabledReason::Unsupported {
                    kind: TargetProfileKind::Android,
                    available_from: None,
                }
            )
        );
        // 대표 Android 속성.
        assert_eq!(
            query(droid30).property(p::ANDROID_DYNAMIC_COLOR),
            blocked(
                prop(p::ANDROID_DYNAMIC_COLOR),
                U,
                DisabledReason::Unsupported {
                    kind: TargetProfileKind::Android,
                    available_from: Some(ProfileLevel::android(31)),
                }
            )
        );
        assert_eq!(
            query(droid35).property(p::ANDROID_DYNAMIC_COLOR),
            ok(prop(p::ANDROID_DYNAMIC_COLOR), N)
        );
        assert_eq!(
            query(droid35).property(p::ANDROID_STATUS_BAR_COLOR),
            blocked(
                prop(p::ANDROID_STATUS_BAR_COLOR),
                D,
                DisabledReason::Deprecated { replacement: None }
            )
        );
        assert_eq!(
            query(droid30).property(p::ANDROID_STATUS_BAR_COLOR),
            ok(prop(p::ANDROID_STATUS_BAR_COLOR), N)
        );
        assert_eq!(
            query(macos).property(p::ANDROID_RIPPLE),
            ok(prop(p::ANDROID_RIPPLE), A)
        );
        // element.
        assert_eq!(
            query(macos).element(ElementKind::TEXT),
            ok(CapabilitySubject::Element(ElementKind::TEXT), N)
        );
        // 미등록.
        let unknown = CapabilityPropertyId(0x0009_0001);
        assert_eq!(
            query(ios).property(unknown),
            CreationAvailability {
                subject: prop(unknown),
                status: None,
                creatable: false,
                disabled_reason: Some(DisabledReason::NotRegistered),
            }
        );
    }

    #[test]
    fn filter_keeps_input_order_and_creatable_drops_disabled() {
        let catalog = catalog().expect("내장 catalog");
        let profile = android("android-30", 30, 1);
        let resolved = catalog.resolve(&profile).expect("해석");
        let query = CapabilityQuery::new(&resolved);
        let subjects = [
            CapabilitySubject::Property(p::ANDROID_RIPPLE),
            CapabilitySubject::Property(p::CSS_SUBGRID),
            CapabilitySubject::Contract(ContractCode::MODAL),
            CapabilitySubject::Property(p::LAYER_BLUR),
        ];
        let rows = query.filter(subjects);
        assert_eq!(
            rows.iter().map(|r| r.subject).collect::<Vec<_>>(),
            subjects.to_vec()
        );
        assert_eq!(
            query.creatable(subjects),
            vec![
                CapabilitySubject::Property(p::ANDROID_RIPPLE),
                CapabilitySubject::Contract(ContractCode::MODAL),
            ]
        );
        // 사유 설명.
        let reasons: Vec<String> = rows
            .iter()
            .filter_map(|r| r.disabled_reason.map(|reason| reason.describe()))
            .collect();
        assert_eq!(
            reasons,
            vec![
                "android profile에서 지원하지 않습니다".to_string(),
                "이 android profile에서 지원하지 않습니다(API 31 이상에서 지원)".to_string(),
            ]
        );
        assert_eq!(
            DisabledReason::Deprecated {
                replacement: Some(CapabilitySubject::Property(p::CLIP_PATH))
            }
            .describe(),
            "사용 중단되었습니다(대체: property:0x00000005)"
        );
    }
}
