//! MotionSpec 모음과 canonical binary 형식 (XXX.1.1, ADR-0094 결정 5).
//!
//! [`MotionSet`]은 MotionSpec을 StableId 순서로 담습니다. canonical binary 형식은 다음과 같습니다(정수·f32 LE).
//!
//! ```text
//! 빈 모음                → 0 bytes
//! 비지 않은 모음         → version u16(=1) ++ count u32 ++ spec*  (id 오름차순)
//! spec                   → id[16] ++ owner tag u8 ++ owner id[16]
//!                          ++ target count u16 ++ (root[16] ++ key u32)*
//!                          ++ start tag u8 ++ variable[16] ++ (Becomes: value len u16 ++ TypedValue bytes)
//!                          ++ duration f32 ++ delay f32
//!                          ++ timing tag u8 ++ timing payload ++ reduced-motion tag u8
//! timing payload         → linear: 없음 | cubic-bezier: f32*4 | steps: count u32 ++ position tag u8
//!                          | spring: mass f32 ++ stiffness f32 ++ damping f32 ++ velocity f32
//! ```
//!
//! 읽기는 모르는 tag·버전, id 비오름차순, 남는 bytes, 검증 실패를 모두 거부합니다. 같은 모음은 언제나 같은 bytes입니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::TypedValue;

use super::spec::{
    MotionOwner, MotionSpec, MotionSpecError, MotionSpecId, MotionSpecInput, ReducedMotion,
    StartCondition, StepPosition, TimingFunction,
};
use crate::component::state_machine::StateVariableId;

/// binary 형식 버전.
pub const MOTION_FORMAT_VERSION: u16 = 1;

/// MotionSpec 모음(StableId 순).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MotionSet {
    specs: BTreeMap<MotionSpecId, MotionSpec>,
}

/// 모음 오류.
#[derive(Debug, Clone, PartialEq)]
pub enum MotionSetError {
    /// 같은 id가 이미 있습니다.
    DuplicateId(MotionSpecId),
    /// 없는 id입니다.
    UnknownId(MotionSpecId),
    /// binary 형식 오류.
    Malformed(&'static str),
    /// 모르는 버전.
    UnknownVersion(u16),
    /// 저장된 spec 검증 실패.
    Spec(MotionSpecError),
}

impl fmt::Display for MotionSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MotionSetError::DuplicateId(id) => write!(f, "MotionSpec id 중복: {id}"),
            MotionSetError::UnknownId(id) => write!(f, "없는 MotionSpec: {id}"),
            MotionSetError::Malformed(reason) => write!(f, "MotionSpec 형식 오류: {reason}"),
            MotionSetError::UnknownVersion(v) => write!(f, "모르는 MotionSpec 형식 버전: {v}"),
            MotionSetError::Spec(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for MotionSetError {}

impl From<MotionSpecError> for MotionSetError {
    fn from(error: MotionSpecError) -> MotionSetError {
        MotionSetError::Spec(error)
    }
}

impl MotionSet {
    /// 빈 모음.
    pub fn new() -> MotionSet {
        MotionSet::default()
    }

    /// spec 수.
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    /// 비었는지.
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// spec 하나.
    pub fn get(&self, id: MotionSpecId) -> Option<&MotionSpec> {
        self.specs.get(&id)
    }

    /// 모든 spec(id 순).
    pub fn iter(&self) -> impl Iterator<Item = &MotionSpec> + '_ {
        self.specs.values()
    }

    /// 소유자 `owner`의 spec(id 순).
    pub fn owned_by(&self, owner: MotionOwner) -> impl Iterator<Item = &MotionSpec> + '_ {
        self.specs.values().filter(move |s| s.owner() == owner)
    }

    /// 추가합니다.
    pub fn insert(&mut self, spec: MotionSpec) -> Result<(), MotionSetError> {
        if self.specs.contains_key(&spec.id()) {
            return Err(MotionSetError::DuplicateId(spec.id()));
        }
        self.specs.insert(spec.id(), spec);
        Ok(())
    }

    /// 같은 id의 spec을 바꾸고 이전 spec을 돌려줍니다.
    pub fn replace(&mut self, spec: MotionSpec) -> Result<MotionSpec, MotionSetError> {
        match self.specs.get_mut(&spec.id()) {
            Some(slot) => Ok(std::mem::replace(slot, spec)),
            None => Err(MotionSetError::UnknownId(spec.id())),
        }
    }

    /// 지웁니다.
    pub fn remove(&mut self, id: MotionSpecId) -> Result<MotionSpec, MotionSetError> {
        self.specs.remove(&id).ok_or(MotionSetError::UnknownId(id))
    }

    /// canonical binary.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.specs.is_empty() {
            return out;
        }
        out.extend_from_slice(&MOTION_FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&(self.specs.len() as u32).to_le_bytes());
        for spec in self.specs.values() {
            encode_spec(spec, &mut out);
        }
        out
    }

    /// canonical binary를 읽습니다.
    pub fn decode(bytes: &[u8]) -> Result<MotionSet, MotionSetError> {
        let mut set = MotionSet::new();
        if bytes.is_empty() {
            return Ok(set);
        }
        let mut r = Reader { bytes, at: 0 };
        let version = r.u16()?;
        if version != MOTION_FORMAT_VERSION {
            return Err(MotionSetError::UnknownVersion(version));
        }
        let count = r.u32()?;
        if count == 0 {
            return Err(MotionSetError::Malformed("빈 모음은 0 bytes입니다"));
        }
        let mut previous: Option<MotionSpecId> = None;
        for _ in 0..count {
            let spec = decode_spec(&mut r)?;
            if previous.is_some_and(|p| p >= spec.id()) {
                return Err(MotionSetError::Malformed("id 오름차순이 아님"));
            }
            previous = Some(spec.id());
            set.insert(spec)?;
        }
        if r.at != bytes.len() {
            return Err(MotionSetError::Malformed("남는 bytes"));
        }
        Ok(set)
    }
}

fn put_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&(value + 0.0).to_le_bytes());
}

fn encode_spec(spec: &MotionSpec, out: &mut Vec<u8>) {
    out.extend_from_slice(&spec.id().stable_id().to_bytes());
    out.push(spec.owner().tag());
    out.extend_from_slice(&spec.owner().id().to_bytes());
    out.extend_from_slice(&(spec.targets().len() as u16).to_le_bytes());
    for target in spec.targets() {
        out.extend_from_slice(&target.root().to_bytes());
        out.extend_from_slice(&target.keys()[0].0.to_le_bytes());
    }
    let start = spec.start();
    out.push(start.tag());
    out.extend_from_slice(&start.variable().stable_id().to_bytes());
    if let StartCondition::Becomes { value, .. } = start {
        let bytes = value.to_bytes();
        out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&bytes);
    }
    put_f32(out, spec.duration_ms());
    put_f32(out, spec.delay_ms());
    let timing = spec.timing();
    out.push(timing.tag());
    match timing {
        TimingFunction::Linear => {}
        TimingFunction::CubicBezier { x1, y1, x2, y2 } => {
            for v in [x1, y1, x2, y2] {
                put_f32(out, v);
            }
        }
        TimingFunction::Steps { count, position } => {
            out.extend_from_slice(&count.to_le_bytes());
            out.push(position.tag());
        }
        TimingFunction::Spring {
            mass,
            stiffness,
            damping,
            initial_velocity,
        } => {
            for v in [mass, stiffness, damping, initial_velocity] {
                put_f32(out, v);
            }
        }
    }
    out.push(spec.reduced_motion().tag());
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], MotionSetError> {
        let end = self
            .at
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(MotionSetError::Malformed("bytes가 모자람"))?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, MotionSetError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, MotionSetError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, MotionSetError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn f32(&mut self) -> Result<f32, MotionSetError> {
        let value = f32::from_bits(self.u32()?);
        if !value.is_finite() || value.to_bits() == (-0.0f32).to_bits() {
            return Err(MotionSetError::Malformed("f32가 유한 정규형이 아님"));
        }
        Ok(value)
    }

    fn id(&mut self) -> Result<StableId, MotionSetError> {
        let b = self.take(16)?;
        let mut raw = [0u8; 16];
        raw.copy_from_slice(b);
        Ok(StableId::from_bytes(raw))
    }
}

fn decode_spec(r: &mut Reader<'_>) -> Result<MotionSpec, MotionSetError> {
    let id = MotionSpecId::new(r.id()?);
    let owner = match r.u8()? {
        0x01 => MotionOwner::Component(ComponentRef::new(r.id()?)),
        0x02 => MotionOwner::View(r.id()?),
        _ => return Err(MotionSetError::Malformed("모르는 owner tag")),
    };
    let count = r.u16()?;
    let mut targets = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let root = r.id()?;
        let key = PropertyKey(r.u32()?);
        targets.push(PropertyPath::new(root).push(key));
    }
    let start_tag = r.u8()?;
    let variable = StateVariableId::new(r.id()?);
    let start = match start_tag {
        0x01 => {
            let len = r.u16()? as usize;
            let value = TypedValue::from_bytes(r.take(len)?)
                .map_err(|_| MotionSetError::Malformed("시작 조건 값"))?;
            StartCondition::Becomes { variable, value }
        }
        0x02 => StartCondition::Changes { variable },
        _ => return Err(MotionSetError::Malformed("모르는 시작 조건 tag")),
    };
    let duration_ms = r.f32()?;
    let delay_ms = r.f32()?;
    let timing = match r.u8()? {
        0x01 => TimingFunction::Linear,
        0x02 => TimingFunction::CubicBezier {
            x1: r.f32()?,
            y1: r.f32()?,
            x2: r.f32()?,
            y2: r.f32()?,
        },
        0x03 => TimingFunction::Steps {
            count: r.u32()?,
            position: StepPosition::from_tag(r.u8()?)
                .ok_or(MotionSetError::Malformed("모르는 step 위치"))?,
        },
        0x04 => TimingFunction::Spring {
            mass: r.f32()?,
            stiffness: r.f32()?,
            damping: r.f32()?,
            initial_velocity: r.f32()?,
        },
        _ => return Err(MotionSetError::Malformed("모르는 timing tag")),
    };
    let reduced_motion = ReducedMotion::from_tag(r.u8()?)
        .ok_or(MotionSetError::Malformed("모르는 reduced-motion tag"))?;
    Ok(MotionSpec::new(MotionSpecInput {
        id,
        owner,
        targets,
        start,
        duration_ms,
        delay_ms,
        timing,
        reduced_motion,
    })?)
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! MotionSpec 공용 fixture.

    use stellar_core_types::color::{Color, ColorSpace};

    use super::*;

    pub(crate) fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0x0abc, n)
    }

    /// timing function 네 종류와 두 시작 조건을 모두 담은 모음.
    pub(crate) fn sample_set() -> MotionSet {
        let mut set = MotionSet::new();
        let timings = [
            TimingFunction::Linear,
            TimingFunction::EASE,
            TimingFunction::Steps {
                count: 4,
                position: StepPosition::JumpBoth,
            },
            TimingFunction::Spring {
                mass: 1.0,
                stiffness: 170.0,
                damping: 26.0,
                initial_velocity: -0.5,
            },
        ];
        for (n, timing) in timings.into_iter().enumerate() {
            let n = n as u64;
            let start = if n.is_multiple_of(2) {
                StartCondition::Becomes {
                    variable: StateVariableId::new(sid(500)),
                    value: TypedValue::Color(
                        Color::new(ColorSpace::DisplayP3, 0.25, 0.5, 1.0, 0.75).expect("색"),
                    ),
                }
            } else {
                StartCondition::Changes {
                    variable: StateVariableId::new(sid(501)),
                }
            };
            let spec = MotionSpec::new(MotionSpecInput {
                id: MotionSpecId::new(sid(100 + n)),
                owner: if n == 3 {
                    MotionOwner::View(sid(7))
                } else {
                    MotionOwner::Component(ComponentRef::new(sid(8)))
                },
                targets: vec![
                    PropertyPath::new(sid(200 + n)).push(PropertyKey(0x0104)),
                    PropertyPath::new(sid(300 + n)).push(PropertyKey(0x0016)),
                ],
                start,
                duration_ms: 150.0 + n as f32 * 50.0,
                delay_ms: if n == 1 { -40.0 } else { 0.0 },
                timing,
                reduced_motion: if n == 2 {
                    ReducedMotion::Play
                } else {
                    ReducedMotion::Instant
                },
            })
            .expect("유효");
            set.insert(spec).expect("삽입");
        }
        set
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::sample_set;
    use super::*;

    #[test]
    fn binary_round_trip_is_canonical() {
        // 검증 기준(XXX.1.1): round-trip.
        let set = sample_set();
        let bytes = set.encode();
        let back = MotionSet::decode(&bytes).expect("읽기");
        assert_eq!(back, set);
        assert_eq!(back.encode(), bytes);
    }

    #[test]
    fn empty_set_is_zero_bytes() {
        assert!(MotionSet::new().encode().is_empty());
        assert_eq!(MotionSet::decode(&[]), Ok(MotionSet::new()));
    }

    #[test]
    fn malformed_bytes_are_rejected() {
        let bytes = sample_set().encode();
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(MotionSet::decode(&trailing).is_err());
        assert!(MotionSet::decode(&bytes[..bytes.len() - 1]).is_err());
        let mut version = bytes.clone();
        version[0] = 9;
        assert_eq!(
            MotionSet::decode(&version),
            Err(MotionSetError::UnknownVersion(9))
        );
        // 첫 spec의 timing tag 자리를 찾기는 번거로우므로 reduced-motion tag(첫 spec 끝)를 바꿔 봅니다.
        let set = sample_set();
        let mut single = MotionSet::new();
        let first = set.iter().next().expect("spec").clone();
        single.insert(first).expect("삽입");
        let mut one = single.encode();
        let last = one.len() - 1;
        one[last] = 0x7F;
        assert!(MotionSet::decode(&one).is_err());
    }
}
