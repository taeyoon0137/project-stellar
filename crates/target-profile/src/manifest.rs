//! `stellar.yaml` 프로젝트 Target Profile 목록과 기본 profile (IX.10.3).
//!
//! 매니페스트 최상위 매핑의 두 키를 다룹니다.
//!
//! | 키 | 필수 | 형식 |
//! |---|---|---|
//! | `target_profiles` | 예 | [`TargetProfile`] 매핑의 시퀀스(저장 순서 보존, 최대 [`MAX_PROFILES`]개) |
//! | `default_target_profile` | profile이 1개 이상이면 예 | 스칼라, `target_profiles`에 있는 id |
//!
//! 규칙(저장 전·열기 시 동일):
//!
//! - profile id는 목록 안에서 유일합니다([`SectionError::DuplicateId`]).
//! - profile이 있으면 기본 profile이 반드시 있어야 하고([`SectionError::MissingDefault`]), 목록에 없는 id를 기본으로
//!   지정하면 거부합니다([`SectionError::UnknownDefault`], 빈 목록 + 기본값 포함). 기본값을 암시적으로(예: 첫 항목)
//!   고르지 않습니다.
//!
//! 다른 최상위 키(`schema_version`, `project`, `dependencies` …)는 project crate의 매니페스트 모델이 소유합니다.
//! [`ProjectTargetProfiles::write_into_root`]는 두 키만 교체·삽입·삭제하고 나머지 키와 순서를 보존합니다. 새 기본
//! profile 키는 `target_profiles` 바로 뒤에 넣습니다.
//!
//! **미연결 초안**: 이 절 형식은 아직 `stellar.yaml` 파일 형식이 아닙니다. project crate 파서는 최상위 키를
//! `schema_version`·`project`·`target_profiles`·`dependencies`만 받아 `default_target_profile`을 `UnknownKey`로 거부하고,
//! `target_profiles[i]`도 다른 키 집합으로 읽습니다(`docs/spec/stellar-format.md` §14.5). 연결과 형식 통합은 ADR 결정
//! 뒤의 작업입니다(IX.10.3 `[~]`, crate 문서 참고).

use std::collections::BTreeSet;
use std::fmt;

use crate::node::{expect_mapping, expect_scalar, invalid, ManifestNode, SchemaError};
use crate::profile::TargetProfileId;
use crate::spec::TargetProfile;

/// 최상위 키: profile 목록.
pub const KEY_TARGET_PROFILES: &str = "target_profiles";
/// 최상위 키: 기본 profile id.
pub const KEY_DEFAULT_TARGET_PROFILE: &str = "default_target_profile";
/// 프로젝트당 최대 profile 수(입력 상한).
pub const MAX_PROFILES: usize = 64;

/// Target Profile 절 오류. 입력 값은 담지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionError {
    /// 노드 스키마 위반.
    Schema(SchemaError),
    /// 같은 id의 profile이 두 번 이상 나타남.
    DuplicateId {
        /// 두 번째로 나타난 entry 경로(예: `target_profiles[2].id`).
        path: String,
    },
    /// profile 수 상한 초과.
    TooManyProfiles {
        /// 상한.
        max: usize,
    },
    /// profile이 있는데 기본 profile이 없음.
    MissingDefault,
    /// 기본 profile id가 목록에 없음.
    UnknownDefault,
}

impl From<SchemaError> for SectionError {
    fn from(error: SchemaError) -> SectionError {
        SectionError::Schema(error)
    }
}

impl fmt::Display for SectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SectionError::Schema(error) => error.fmt(f),
            SectionError::DuplicateId { path } => write!(f, "profile id 중복: {path}"),
            SectionError::TooManyProfiles { max } => {
                write!(f, "target profile은 최대 {max}개입니다")
            }
            SectionError::MissingDefault => {
                write!(
                    f,
                    "{KEY_DEFAULT_TARGET_PROFILE}가 필요합니다(profile이 1개 이상)"
                )
            }
            SectionError::UnknownDefault => {
                write!(
                    f,
                    "{KEY_DEFAULT_TARGET_PROFILE}가 {KEY_TARGET_PROFILES}에 없는 id입니다"
                )
            }
        }
    }
}

impl std::error::Error for SectionError {}

/// 프로젝트 Target Profile 목록과 기본 profile. 생성 시 절 규칙을 검사하므로 항상 유효합니다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProjectTargetProfiles {
    profiles: Vec<TargetProfile>,
    default: Option<TargetProfileId>,
}

impl ProjectTargetProfiles {
    /// profile이 없는 절(새 프로젝트).
    pub fn empty() -> ProjectTargetProfiles {
        ProjectTargetProfiles {
            profiles: Vec::new(),
            default: None,
        }
    }

    /// 규칙을 검사해 만듭니다.
    pub fn new(
        profiles: Vec<TargetProfile>,
        default: Option<TargetProfileId>,
    ) -> Result<ProjectTargetProfiles, SectionError> {
        validate(&profiles, default.as_ref())?;
        Ok(ProjectTargetProfiles { profiles, default })
    }

    /// profile 목록(저장 순서).
    pub fn profiles(&self) -> &[TargetProfile] {
        &self.profiles
    }

    /// 기본 profile id.
    pub fn default_id(&self) -> Option<&TargetProfileId> {
        self.default.as_ref()
    }

    /// 기본 profile.
    pub fn default_profile(&self) -> Option<&TargetProfile> {
        self.default.as_ref().and_then(|id| self.get(id))
    }

    /// id로 profile을 찾습니다.
    pub fn get(&self, id: &TargetProfileId) -> Option<&TargetProfile> {
        self.profiles.iter().find(|p| &p.id == id)
    }

    /// 기본 profile을 바꿉니다. 목록에 없는 id는 거부하고 절은 그대로입니다.
    pub fn set_default(&mut self, id: TargetProfileId) -> Result<(), SectionError> {
        if self.get(&id).is_none() {
            return Err(SectionError::UnknownDefault);
        }
        self.default = Some(id);
        Ok(())
    }

    /// 같은 id의 profile을 교체합니다(목록 위치 보존). 없는 id면 `false`이고 절은 그대로입니다.
    pub fn replace_profile(&mut self, profile: TargetProfile) -> bool {
        match self.profiles.iter_mut().find(|p| p.id == profile.id) {
            Some(slot) => {
                *slot = profile;
                true
            }
            None => false,
        }
    }

    /// 두 최상위 키의 canonical (키, 값) 목록. 기본 profile이 없으면 `default_target_profile`을 쓰지 않습니다.
    pub fn to_entries(&self) -> Vec<(String, ManifestNode)> {
        let mut entries = vec![(
            KEY_TARGET_PROFILES.to_string(),
            ManifestNode::Sequence(self.profiles.iter().map(TargetProfile::to_node).collect()),
        )];
        if let Some(default) = &self.default {
            entries.push((
                KEY_DEFAULT_TARGET_PROFILE.to_string(),
                ManifestNode::scalar(default.as_str()),
            ));
        }
        entries
    }

    /// 매니페스트 최상위 매핑에서 절을 읽습니다. 다른 최상위 키는 검사하지 않습니다(project 소유).
    pub fn from_root(root: &ManifestNode) -> Result<ProjectTargetProfiles, SectionError> {
        expect_mapping(root, "<root>")?;
        let list = root
            .get(KEY_TARGET_PROFILES)
            .ok_or_else(|| SchemaError::MissingField {
                path: KEY_TARGET_PROFILES.to_string(),
            })?;
        let items = list.as_sequence().ok_or_else(|| SchemaError::WrongType {
            path: KEY_TARGET_PROFILES.to_string(),
            expected: "sequence",
        })?;
        if items.len() > MAX_PROFILES {
            return Err(SectionError::TooManyProfiles { max: MAX_PROFILES });
        }
        let mut profiles = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            profiles.push(TargetProfile::from_node(
                item,
                &format!("{KEY_TARGET_PROFILES}[{index}]"),
            )?);
        }
        let default = match root.get(KEY_DEFAULT_TARGET_PROFILE) {
            None => None,
            Some(node) => {
                let text = expect_scalar(node, KEY_DEFAULT_TARGET_PROFILE)?;
                Some(
                    TargetProfileId::new(text)
                        .map_err(|e| invalid(KEY_DEFAULT_TARGET_PROFILE, e))?,
                )
            }
        };
        ProjectTargetProfiles::new(profiles, default)
    }

    /// 최상위 매핑에 절을 기록합니다. 두 키만 교체·삽입·삭제하고 다른 키와 순서를 보존합니다.
    pub fn write_into_root(&self, root: &mut ManifestNode) -> Result<(), SchemaError> {
        let ManifestNode::Mapping(pairs) = root else {
            return Err(SchemaError::WrongType {
                path: "<root>".to_string(),
                expected: "mapping",
            });
        };
        let mut entries = self.to_entries().into_iter();
        if let Some((key, value)) = entries.next() {
            set_or_insert(pairs, key, value, None);
        }
        match entries.next() {
            Some((key, value)) => set_or_insert(pairs, key, value, Some(KEY_TARGET_PROFILES)),
            None => pairs.retain(|(key, _)| key != KEY_DEFAULT_TARGET_PROFILE),
        }
        Ok(())
    }
}

/// 키가 있으면 값을 교체하고, 없으면 `after` 키 바로 뒤(없으면 끝)에 넣습니다.
fn set_or_insert(
    pairs: &mut Vec<(String, ManifestNode)>,
    key: String,
    value: ManifestNode,
    after: Option<&str>,
) {
    if let Some(slot) = pairs.iter_mut().find(|(k, _)| *k == key) {
        slot.1 = value;
        return;
    }
    let position = after
        .and_then(|anchor| pairs.iter().position(|(k, _)| k == anchor))
        .map_or(pairs.len(), |index| index + 1);
    pairs.insert(position, (key, value));
}

fn validate(
    profiles: &[TargetProfile],
    default: Option<&TargetProfileId>,
) -> Result<(), SectionError> {
    if profiles.len() > MAX_PROFILES {
        return Err(SectionError::TooManyProfiles { max: MAX_PROFILES });
    }
    let mut seen = BTreeSet::new();
    for (index, profile) in profiles.iter().enumerate() {
        if !seen.insert(&profile.id) {
            return Err(SectionError::DuplicateId {
                path: format!("{KEY_TARGET_PROFILES}[{index}].id"),
            });
        }
    }
    match default {
        None if profiles.is_empty() => Ok(()),
        None => Err(SectionError::MissingDefault),
        Some(id) if seen.contains(id) => Ok(()),
        Some(_) => Err(SectionError::UnknownDefault),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::ValueError;
    use crate::registry::ProfileRegistryVersion;
    use crate::spec::{
        AndroidApiLevel, AndroidProfile, ApplePlatform, AppleProfile, CapabilitySetVersion,
        DottedVersion, ProfileSpec, WebBaseline, WebProfile,
    };

    fn id(text: &str) -> TargetProfileId {
        TargetProfileId::new(text).expect("유효한 id")
    }

    fn apple(name: &str) -> TargetProfile {
        TargetProfile::new(
            id(name),
            ProfileRegistryVersion::new(1).expect("v1"),
            ProfileSpec::Apple(AppleProfile {
                platform: ApplePlatform::Ios,
                minimum_os: DottedVersion::new(26, 0, 0),
            }),
        )
    }

    fn android(name: &str) -> TargetProfile {
        TargetProfile::new(
            id(name),
            ProfileRegistryVersion::new(1).expect("v1"),
            ProfileSpec::Android(AndroidProfile {
                api_level: AndroidApiLevel::new(35).expect("api"),
                ui_system_version: DottedVersion::new(1, 7, 0),
            }),
        )
    }

    fn web(name: &str) -> TargetProfile {
        TargetProfile::new(
            id(name),
            ProfileRegistryVersion::new(1).expect("v1"),
            ProfileSpec::Web(WebProfile {
                baseline: WebBaseline::new(2024).expect("baseline"),
                capability_set_version: CapabilitySetVersion::new(1).expect("set"),
            }),
        )
    }

    fn section() -> ProjectTargetProfiles {
        ProjectTargetProfiles::new(
            vec![
                apple("apple-phone"),
                android("android-phone"),
                web("web-modern"),
            ],
            Some(id("web-modern")),
        )
        .expect("유효한 절")
    }

    /// project가 소유하는 키를 포함한 매니페스트 최상위 노드.
    fn root_with(section_entries: Vec<(String, ManifestNode)>) -> ManifestNode {
        let mut pairs = vec![
            ("schema_version".to_string(), ManifestNode::scalar("1")),
            (
                "project".to_string(),
                ManifestNode::Mapping(vec![
                    ("id".to_string(), ManifestNode::scalar("0195c530")),
                    ("name".to_string(), ManifestNode::scalar("Demo")),
                ]),
            ),
        ];
        pairs.extend(section_entries);
        pairs.push((
            "dependencies".to_string(),
            ManifestNode::Mapping(Vec::new()),
        ));
        ManifestNode::Mapping(pairs)
    }

    #[test]
    fn manifest_round_trip_preserves_section_and_other_keys() {
        let section = section();
        let root = root_with(section.to_entries());
        let keys: Vec<&str> = root
            .as_mapping()
            .expect("매핑")
            .iter()
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(
            keys,
            vec![
                "schema_version",
                "project",
                "target_profiles",
                "default_target_profile",
                "dependencies"
            ]
        );
        // 노드 → 모델 → 노드: 최상위 전체가 bytes 단위로 같습니다.
        let restored = ProjectTargetProfiles::from_root(&root).expect("읽기");
        assert_eq!(restored, section);
        let mut rewritten = root.clone();
        restored.write_into_root(&mut rewritten).expect("기록");
        assert_eq!(rewritten.canonical_bytes(), root.canonical_bytes());
        assert_eq!(
            restored.default_profile().map(|p| p.id.as_str()),
            Some("web-modern")
        );
        // 모델 → 노드 → 모델.
        assert_eq!(
            ProjectTargetProfiles::from_root(&rewritten).expect("재읽기"),
            section
        );
    }

    #[test]
    fn write_inserts_default_after_profiles_and_removes_it_when_empty() {
        // 기본 profile 키가 없던 매니페스트(새 프로젝트)에 profile을 추가합니다.
        let mut root = root_with(vec![(
            KEY_TARGET_PROFILES.to_string(),
            ManifestNode::Sequence(Vec::new()),
        )]);
        assert_eq!(
            ProjectTargetProfiles::from_root(&root),
            Ok(ProjectTargetProfiles::empty())
        );
        let section = section();
        section.write_into_root(&mut root).expect("기록");
        assert_eq!(root, root_with(section.to_entries()));
        // 다시 비우면 기본 profile 키를 지웁니다.
        ProjectTargetProfiles::empty()
            .write_into_root(&mut root)
            .expect("기록");
        assert_eq!(
            root,
            root_with(vec![(
                KEY_TARGET_PROFILES.to_string(),
                ManifestNode::Sequence(Vec::new()),
            )])
        );
        assert!(matches!(
            ProjectTargetProfiles::empty().write_into_root(&mut ManifestNode::scalar("x")),
            Err(SchemaError::WrongType { .. })
        ));
    }

    #[test]
    fn invalid_default_fixtures_are_rejected() {
        let profiles = || {
            ManifestNode::Sequence(vec![
                apple("apple-phone").to_node(),
                web("web-modern").to_node(),
            ])
        };
        let root = |default: Option<ManifestNode>, list: ManifestNode| {
            let mut entries = vec![(KEY_TARGET_PROFILES.to_string(), list)];
            if let Some(default) = default {
                entries.push((KEY_DEFAULT_TARGET_PROFILE.to_string(), default));
            }
            root_with(entries)
        };
        // 존재하지 않는 기본 profile.
        assert_eq!(
            ProjectTargetProfiles::from_root(&root(
                Some(ManifestNode::scalar("android-phone")),
                profiles()
            )),
            Err(SectionError::UnknownDefault)
        );
        // 빈 목록 + 기본 profile.
        assert_eq!(
            ProjectTargetProfiles::from_root(&root(
                Some(ManifestNode::scalar("web-modern")),
                ManifestNode::Sequence(Vec::new())
            )),
            Err(SectionError::UnknownDefault)
        );
        // profile이 있는데 기본 profile 없음(첫 항목을 암시적으로 고르지 않음).
        assert_eq!(
            ProjectTargetProfiles::from_root(&root(None, profiles())),
            Err(SectionError::MissingDefault)
        );
        // 형식이 잘못된 기본 profile 값.
        assert_eq!(
            ProjectTargetProfiles::from_root(&root(Some(ManifestNode::scalar("")), profiles())),
            Err(SectionError::Schema(SchemaError::InvalidValue {
                path: KEY_DEFAULT_TARGET_PROFILE.to_string(),
                error: ValueError::Empty
            }))
        );
        assert_eq!(
            ProjectTargetProfiles::from_root(&root(
                Some(ManifestNode::Sequence(Vec::new())),
                profiles()
            )),
            Err(SectionError::Schema(SchemaError::WrongType {
                path: KEY_DEFAULT_TARGET_PROFILE.to_string(),
                expected: "scalar"
            }))
        );
        // 모델 API도 같은 규칙을 강제합니다.
        assert_eq!(
            ProjectTargetProfiles::new(vec![apple("a")], Some(id("b"))),
            Err(SectionError::UnknownDefault)
        );
        let mut section = section();
        assert_eq!(
            section.set_default(id("missing")),
            Err(SectionError::UnknownDefault)
        );
        assert_eq!(
            section.default_id().map(TargetProfileId::as_str),
            Some("web-modern")
        );
        assert_eq!(section.set_default(id("apple-phone")), Ok(()));
        assert_eq!(
            section.default_profile().map(|p| p.id.as_str()),
            Some("apple-phone")
        );
    }

    #[test]
    fn section_structure_violations_are_rejected() {
        let duplicate = root_with(vec![
            (
                KEY_TARGET_PROFILES.to_string(),
                ManifestNode::Sequence(vec![
                    apple("same").to_node(),
                    web("other").to_node(),
                    android("same").to_node(),
                ]),
            ),
            (
                KEY_DEFAULT_TARGET_PROFILE.to_string(),
                ManifestNode::scalar("same"),
            ),
        ]);
        assert_eq!(
            ProjectTargetProfiles::from_root(&duplicate),
            Err(SectionError::DuplicateId {
                path: "target_profiles[2].id".to_string()
            })
        );
        let too_many: Vec<ManifestNode> = (0..=MAX_PROFILES)
            .map(|n| web(&format!("w{n}")).to_node())
            .collect();
        assert_eq!(
            ProjectTargetProfiles::from_root(&root_with(vec![(
                KEY_TARGET_PROFILES.to_string(),
                ManifestNode::Sequence(too_many)
            )])),
            Err(SectionError::TooManyProfiles { max: MAX_PROFILES })
        );
        assert_eq!(
            ProjectTargetProfiles::from_root(&root_with(Vec::new())),
            Err(SectionError::Schema(SchemaError::MissingField {
                path: KEY_TARGET_PROFILES.to_string()
            }))
        );
        assert_eq!(
            ProjectTargetProfiles::from_root(&root_with(vec![(
                KEY_TARGET_PROFILES.to_string(),
                ManifestNode::Mapping(Vec::new())
            )])),
            Err(SectionError::Schema(SchemaError::WrongType {
                path: KEY_TARGET_PROFILES.to_string(),
                expected: "sequence"
            }))
        );
        // entry 오류는 index 경로를 포함합니다.
        let broken = root_with(vec![(
            KEY_TARGET_PROFILES.to_string(),
            ManifestNode::Sequence(vec![web("ok").to_node(), ManifestNode::scalar("x")]),
        )]);
        assert_eq!(
            ProjectTargetProfiles::from_root(&broken),
            Err(SectionError::Schema(SchemaError::WrongType {
                path: "target_profiles[1]".to_string(),
                expected: "mapping"
            }))
        );
    }

    #[test]
    fn replace_profile_keeps_position() {
        let mut section = section();
        let mut updated = android("android-phone");
        if let ProfileSpec::Android(fields) = &mut updated.spec {
            fields.api_level = AndroidApiLevel::new(36).expect("api");
        }
        assert!(section.replace_profile(updated.clone()));
        assert_eq!(section.profiles()[1], updated);
        assert!(!section.replace_profile(android("missing")));
        assert_eq!(section.profiles().len(), 3);
    }
}
