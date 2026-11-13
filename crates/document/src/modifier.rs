//! Modifier stack (VI.9).
//!
//! modifier는 node payload와 **분리 저장**되는 순서 있는 목록입니다. 각 modifier는
//! 고유 [`ModifierId`](StableId newtype)를 갖고, 순서는 [`OrderKey`](III.5)로
//! 정합니다 — 삽입·재정렬이 다른 modifier의 id·key를 바꾸지 않아 semantic merge에
//! 유리합니다.

use std::fmt;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::NodeRef;
use stellar_core_types::value::{FiniteF32, Transform2D, Vec2};

/// modifier의 안정 식별자. node StableId와 타입이 섞이지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModifierId(StableId);

impl ModifierId {
    /// `StableId`로부터 만듭니다.
    pub const fn new(id: StableId) -> ModifierId {
        ModifierId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }
}

/// modifier stack 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModifierError {
    /// 같은 ModifierId가 이미 있습니다.
    DuplicateId {
        /// 중복 id.
        id: ModifierId,
    },
    /// 같은 OrderKey가 이미 있습니다(순서 모호).
    DuplicateOrderKey,
    /// 대상 modifier가 없습니다.
    NotFound {
        /// 요청 id.
        id: ModifierId,
    },
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
    /// 항목 kind와 payload 종류가 다릅니다(VI.9.3).
    KindPayloadMismatch {
        /// 항목 kind.
        kind: ModifierKind,
    },
    /// payload 값이 범위를 벗어납니다(VI.9.3).
    InvalidPayload {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for ModifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModifierError::DuplicateId { id } => write!(f, "modifier id 중복: {}", id.0),
            ModifierError::DuplicateOrderKey => f.write_str("modifier OrderKey 중복"),
            ModifierError::NotFound { id } => write!(f, "modifier가 없습니다: {}", id.0),
            ModifierError::Malformed { reason } => {
                write!(f, "Modifier binary 형식 위반: {reason}")
            }
            ModifierError::KindPayloadMismatch { kind } => {
                write!(f, "{} modifier의 payload 종류가 다릅니다", kind.name())
            }
            ModifierError::InvalidPayload { reason } => write!(f, "modifier 값 오류: {reason}"),
        }
    }
}

impl std::error::Error for ModifierError {}

/// modifier 종류. 숫자 값은 저장 포맷의 일부입니다(golden test로 고정).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum ModifierKind {
    /// 추가 변환.
    Transform = 1,
    /// boolean 연산(다른 node 참조).
    Boolean = 2,
    /// stroke를 fill 외곽선으로 확장.
    StrokeExpand = 3,
    /// 모서리 둥글리기.
    Corner = 4,
    /// mask(다른 node 참조).
    Mask = 5,
    /// clip(다른 node 참조).
    Clip = 6,
    /// 채우기.
    Fill = 7,
    /// 선.
    Stroke = 8,
    /// 그림자.
    Shadow = 9,
    /// 흐림.
    Blur = 10,
    /// blend mode.
    Blend = 11,
    /// 불투명도.
    Opacity = 12,
}

impl ModifierKind {
    /// 정의된 모든 종류.
    pub const ALL: [ModifierKind; 12] = [
        ModifierKind::Transform,
        ModifierKind::Boolean,
        ModifierKind::StrokeExpand,
        ModifierKind::Corner,
        ModifierKind::Mask,
        ModifierKind::Clip,
        ModifierKind::Fill,
        ModifierKind::Stroke,
        ModifierKind::Shadow,
        ModifierKind::Blur,
        ModifierKind::Blend,
        ModifierKind::Opacity,
    ];

    /// 안정 numeric 값.
    pub fn as_u16(self) -> u16 {
        self as u16
    }

    /// 숫자에서 복원합니다.
    pub fn from_u16(value: u16) -> Option<ModifierKind> {
        ModifierKind::ALL.into_iter().find(|k| k.as_u16() == value)
    }

    /// 진단용 이름.
    pub fn name(self) -> &'static str {
        match self {
            ModifierKind::Transform => "transform",
            ModifierKind::Boolean => "boolean",
            ModifierKind::StrokeExpand => "stroke-expand",
            ModifierKind::Corner => "corner",
            ModifierKind::Mask => "mask",
            ModifierKind::Clip => "clip",
            ModifierKind::Fill => "fill",
            ModifierKind::Stroke => "stroke",
            ModifierKind::Shadow => "shadow",
            ModifierKind::Blur => "blur",
            ModifierKind::Blend => "blend",
            ModifierKind::Opacity => "opacity",
        }
    }
}

// --- typed payload (VI.9.3) ---

/// boolean 연산. tag는 저장 포맷의 일부입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BooleanOp {
    /// 합집합.
    Union = 0,
    /// 차집합.
    Subtract = 1,
    /// 교집합.
    Intersect = 2,
    /// 배타적 합.
    Exclude = 3,
}

/// blend mode. tag는 저장 포맷의 일부입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BlendMode {
    /// 일반.
    Normal = 0,
    /// 곱하기.
    Multiply = 1,
    /// 스크린.
    Screen = 2,
    /// 오버레이.
    Overlay = 3,
    /// 어둡게.
    Darken = 4,
    /// 밝게.
    Lighten = 5,
}

/// modifier 종류별 typed payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModifierPayload {
    /// 추가 변환.
    Transform(Transform2D),
    /// boolean 연산과 피연산 node들.
    Boolean {
        /// 연산.
        op: BooleanOp,
        /// 피연산 node 참조.
        operands: Vec<NodeRef>,
    },
    /// stroke 확장 폭(비음수).
    StrokeExpand {
        /// 폭.
        width: FiniteF32,
    },
    /// 모서리 radius(비음수).
    Corner {
        /// radius.
        radius: FiniteF32,
    },
    /// mask 원본 node.
    Mask {
        /// 원본.
        source: NodeRef,
    },
    /// clip 원본 node.
    Clip {
        /// 원본.
        source: NodeRef,
    },
    /// 채우기 색.
    Fill {
        /// 색.
        color: Color,
    },
    /// 선 색과 두께(비음수).
    Stroke {
        /// 색.
        color: Color,
        /// 두께.
        width: FiniteF32,
    },
    /// 그림자.
    Shadow {
        /// 색.
        color: Color,
        /// 오프셋.
        offset: Vec2,
        /// blur 반경(비음수).
        blur: FiniteF32,
    },
    /// 흐림 반경(비음수).
    Blur {
        /// 반경.
        radius: FiniteF32,
    },
    /// blend mode.
    Blend {
        /// mode.
        mode: BlendMode,
    },
    /// 불투명도(0..=1).
    Opacity {
        /// 값.
        value: FiniteF32,
    },
}

fn black() -> Color {
    Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).expect("유효 색")
}

impl ModifierPayload {
    /// payload의 종류.
    pub fn kind(&self) -> ModifierKind {
        match self {
            ModifierPayload::Transform(_) => ModifierKind::Transform,
            ModifierPayload::Boolean { .. } => ModifierKind::Boolean,
            ModifierPayload::StrokeExpand { .. } => ModifierKind::StrokeExpand,
            ModifierPayload::Corner { .. } => ModifierKind::Corner,
            ModifierPayload::Mask { .. } => ModifierKind::Mask,
            ModifierPayload::Clip { .. } => ModifierKind::Clip,
            ModifierPayload::Fill { .. } => ModifierKind::Fill,
            ModifierPayload::Stroke { .. } => ModifierKind::Stroke,
            ModifierPayload::Shadow { .. } => ModifierKind::Shadow,
            ModifierPayload::Blur { .. } => ModifierKind::Blur,
            ModifierPayload::Blend { .. } => ModifierKind::Blend,
            ModifierPayload::Opacity { .. } => ModifierKind::Opacity,
        }
    }

    /// 종류의 기본 payload(새 modifier 추가용). 참조형 kind는 `NIL` 참조를 담아
    /// 사용자가 대상을 지정할 때까지 미해결 상태입니다.
    pub fn default_for(kind: ModifierKind) -> ModifierPayload {
        let nil = NodeRef::new(StableId::NIL);
        match kind {
            ModifierKind::Transform => ModifierPayload::Transform(Transform2D::identity()),
            ModifierKind::Boolean => ModifierPayload::Boolean {
                op: BooleanOp::Union,
                operands: Vec::new(),
            },
            ModifierKind::StrokeExpand => ModifierPayload::StrokeExpand {
                width: FiniteF32::ONE,
            },
            ModifierKind::Corner => ModifierPayload::Corner {
                radius: FiniteF32::ZERO,
            },
            ModifierKind::Mask => ModifierPayload::Mask { source: nil },
            ModifierKind::Clip => ModifierPayload::Clip { source: nil },
            ModifierKind::Fill => ModifierPayload::Fill { color: black() },
            ModifierKind::Stroke => ModifierPayload::Stroke {
                color: black(),
                width: FiniteF32::ONE,
            },
            ModifierKind::Shadow => ModifierPayload::Shadow {
                color: black(),
                offset: Vec2::ZERO,
                blur: FiniteF32::ZERO,
            },
            ModifierKind::Blur => ModifierPayload::Blur {
                radius: FiniteF32::ZERO,
            },
            ModifierKind::Blend => ModifierPayload::Blend {
                mode: BlendMode::Normal,
            },
            ModifierKind::Opacity => ModifierPayload::Opacity {
                value: FiniteF32::ONE,
            },
        }
    }

    /// 값 범위를 검증합니다.
    pub fn validate(&self) -> Result<(), ModifierError> {
        let non_negative = |v: FiniteF32, reason| {
            if v.get() < 0.0 {
                Err(ModifierError::InvalidPayload { reason })
            } else {
                Ok(())
            }
        };
        match self {
            ModifierPayload::StrokeExpand { width } => non_negative(*width, "폭은 비음수"),
            ModifierPayload::Corner { radius } => non_negative(*radius, "radius는 비음수"),
            ModifierPayload::Stroke { width, .. } => non_negative(*width, "두께는 비음수"),
            ModifierPayload::Shadow { blur, .. } => non_negative(*blur, "blur는 비음수"),
            ModifierPayload::Blur { radius } => non_negative(*radius, "반경은 비음수"),
            ModifierPayload::Opacity { value } => {
                if (0.0..=1.0).contains(&value.get()) {
                    Ok(())
                } else {
                    Err(ModifierError::InvalidPayload {
                        reason: "불투명도는 0..=1",
                    })
                }
            }
            _ => Ok(()),
        }
    }

    fn encode_into(&self, buf: &mut Vec<u8>) {
        let f = |buf: &mut Vec<u8>, v: FiniteF32| buf.extend_from_slice(&v.to_le_bytes());
        match self {
            ModifierPayload::Transform(m) => {
                let (a, b, c, d, tx, ty) = m.components();
                for v in [a, b, c, d, tx, ty] {
                    buf.extend_from_slice(&v.to_le_bytes());
                }
            }
            ModifierPayload::Boolean { op, operands } => {
                buf.push(*op as u8);
                buf.extend_from_slice(&(operands.len() as u32).to_le_bytes());
                for operand in operands {
                    buf.extend_from_slice(&operand.id().to_bytes());
                }
            }
            ModifierPayload::StrokeExpand { width } => f(buf, *width),
            ModifierPayload::Corner { radius } => f(buf, *radius),
            ModifierPayload::Mask { source } | ModifierPayload::Clip { source } => {
                buf.extend_from_slice(&source.id().to_bytes())
            }
            ModifierPayload::Fill { color } => buf.extend_from_slice(&color.to_bytes()),
            ModifierPayload::Stroke { color, width } => {
                buf.extend_from_slice(&color.to_bytes());
                f(buf, *width);
            }
            ModifierPayload::Shadow {
                color,
                offset,
                blur,
            } => {
                buf.extend_from_slice(&color.to_bytes());
                buf.extend_from_slice(&offset.x().to_le_bytes());
                buf.extend_from_slice(&offset.y().to_le_bytes());
                f(buf, *blur);
            }
            ModifierPayload::Blur { radius } => f(buf, *radius),
            ModifierPayload::Blend { mode } => buf.push(*mode as u8),
            ModifierPayload::Opacity { value } => f(buf, *value),
        }
    }

    fn decode(kind: ModifierKind, bytes: &[u8]) -> Result<ModifierPayload, ModifierError> {
        let bad = |reason| ModifierError::Malformed { reason };
        let finite = |i: usize| -> Result<FiniteF32, ModifierError> {
            let raw: [u8; 4] = bytes
                .get(i..i + 4)
                .ok_or(bad("payload 부족"))?
                .try_into()
                .expect("4바이트");
            FiniteF32::from_le_bytes(raw).map_err(|_| bad("비유한 값"))
        };
        let node = |i: usize| -> Result<NodeRef, ModifierError> {
            let raw: [u8; 16] = bytes
                .get(i..i + 16)
                .ok_or(bad("payload 부족"))?
                .try_into()
                .expect("16바이트");
            Ok(NodeRef::new(StableId::from_bytes(raw)))
        };
        let color = |i: usize| -> Result<Color, ModifierError> {
            let raw: [u8; Color::BYTE_LEN] = bytes
                .get(i..i + Color::BYTE_LEN)
                .ok_or(bad("payload 부족"))?
                .try_into()
                .expect("색 길이");
            Color::from_bytes(raw).map_err(|_| bad("잘못된 색"))
        };
        let (payload, used) = match kind {
            ModifierKind::Transform => {
                let v: Vec<f32> = (0..6)
                    .map(|k| finite(k * 4).map(|x| x.get()))
                    .collect::<Result<_, _>>()?;
                (
                    ModifierPayload::Transform(
                        Transform2D::new(v[0], v[1], v[2], v[3], v[4], v[5])
                            .map_err(|_| bad("비유한 값"))?,
                    ),
                    24,
                )
            }
            ModifierKind::Boolean => {
                let op = match bytes.first().copied() {
                    Some(0) => BooleanOp::Union,
                    Some(1) => BooleanOp::Subtract,
                    Some(2) => BooleanOp::Intersect,
                    Some(3) => BooleanOp::Exclude,
                    _ => return Err(bad("알 수 없는 boolean op")),
                };
                let count_raw: [u8; 4] = bytes
                    .get(1..5)
                    .ok_or(bad("payload 부족"))?
                    .try_into()
                    .expect("4바이트");
                let count = u32::from_le_bytes(count_raw) as usize;
                let operands = (0..count)
                    .map(|k| node(5 + k * 16))
                    .collect::<Result<_, _>>()?;
                (ModifierPayload::Boolean { op, operands }, 5 + count * 16)
            }
            ModifierKind::StrokeExpand => (ModifierPayload::StrokeExpand { width: finite(0)? }, 4),
            ModifierKind::Corner => (ModifierPayload::Corner { radius: finite(0)? }, 4),
            ModifierKind::Mask => (ModifierPayload::Mask { source: node(0)? }, 16),
            ModifierKind::Clip => (ModifierPayload::Clip { source: node(0)? }, 16),
            ModifierKind::Fill => (ModifierPayload::Fill { color: color(0)? }, Color::BYTE_LEN),
            ModifierKind::Stroke => (
                ModifierPayload::Stroke {
                    color: color(0)?,
                    width: finite(Color::BYTE_LEN)?,
                },
                Color::BYTE_LEN + 4,
            ),
            ModifierKind::Shadow => {
                let x = finite(Color::BYTE_LEN)?;
                let y = finite(Color::BYTE_LEN + 4)?;
                (
                    ModifierPayload::Shadow {
                        color: color(0)?,
                        offset: Vec2::new(x.get(), y.get()).map_err(|_| bad("비유한 값"))?,
                        blur: finite(Color::BYTE_LEN + 8)?,
                    },
                    Color::BYTE_LEN + 12,
                )
            }
            ModifierKind::Blur => (ModifierPayload::Blur { radius: finite(0)? }, 4),
            ModifierKind::Blend => {
                let mode = match bytes.first().copied() {
                    Some(0) => BlendMode::Normal,
                    Some(1) => BlendMode::Multiply,
                    Some(2) => BlendMode::Screen,
                    Some(3) => BlendMode::Overlay,
                    Some(4) => BlendMode::Darken,
                    Some(5) => BlendMode::Lighten,
                    _ => return Err(bad("알 수 없는 blend mode")),
                };
                (ModifierPayload::Blend { mode }, 1)
            }
            ModifierKind::Opacity => (ModifierPayload::Opacity { value: finite(0)? }, 4),
        };
        if used != bytes.len() {
            return Err(bad("payload 길이 불일치"));
        }
        payload.validate()?;
        Ok(payload)
    }
}

/// stack의 한 항목(id + 순서 key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModifierEntry {
    /// 고유 id.
    pub id: ModifierId,
    /// 순서 key.
    pub order: OrderKey,
    /// 종류(`payload.kind()`와 같아야 함).
    pub kind: ModifierKind,
    /// 활성 여부. 비활성 modifier는 평가에서 건너뛰지만 저장에는 유지됩니다.
    pub enabled: bool,
    /// 종류별 값.
    pub payload: ModifierPayload,
}

/// node 하나의 modifier stack. node payload와 별도로 저장됩니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModifierStack {
    /// 소유 node.
    pub node: StableId,
    /// OrderKey 오름차순으로 유지되는 항목.
    entries: Vec<ModifierEntry>,
}

impl ModifierStack {
    /// 빈 stack.
    pub fn new(node: StableId) -> ModifierStack {
        ModifierStack {
            node,
            entries: Vec::new(),
        }
    }

    /// 순서대로 정렬된 항목.
    pub fn entries(&self) -> &[ModifierEntry] {
        &self.entries
    }

    /// 순서대로 id 목록.
    pub fn ids_in_order(&self) -> Vec<ModifierId> {
        self.entries.iter().map(|e| e.id).collect()
    }

    /// 항목을 추가합니다. id·OrderKey 중복은 거부합니다. 입력 순서와 무관하게
    /// OrderKey 순으로 유지합니다.
    pub fn insert(&mut self, entry: ModifierEntry) -> Result<(), ModifierError> {
        if entry.payload.kind() != entry.kind {
            return Err(ModifierError::KindPayloadMismatch { kind: entry.kind });
        }
        entry.payload.validate()?;
        if self.entries.iter().any(|e| e.id == entry.id) {
            return Err(ModifierError::DuplicateId { id: entry.id });
        }
        if self.entries.iter().any(|e| e.order == entry.order) {
            return Err(ModifierError::DuplicateOrderKey);
        }
        let at = self.entries.partition_point(|e| e.order < entry.order);
        self.entries.insert(at, entry);
        Ok(())
    }

    /// 맨 뒤(가장 나중 적용)에 새 modifier를 추가하고 그 key를 반환합니다.
    pub fn push_back(
        &mut self,
        id: ModifierId,
        kind: ModifierKind,
    ) -> Result<OrderKey, ModifierError> {
        let order = OrderKey::between(self.entries.last().map(|e| &e.order), None);
        self.insert(ModifierEntry {
            id,
            order: order.clone(),
            kind,
            enabled: true,
            payload: ModifierPayload::default_for(kind),
        })?;
        Ok(order)
    }

    /// 활성 여부를 바꿉니다(저장에서 제거하지 않음).
    pub fn set_enabled(&mut self, id: ModifierId, enabled: bool) -> Result<(), ModifierError> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or(ModifierError::NotFound { id })?;
        entry.enabled = enabled;
        Ok(())
    }

    /// payload를 바꿉니다(종류는 유지해야 함).
    pub fn set_payload(
        &mut self,
        id: ModifierId,
        payload: ModifierPayload,
    ) -> Result<(), ModifierError> {
        payload.validate()?;
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or(ModifierError::NotFound { id })?;
        if payload.kind() != entry.kind {
            return Err(ModifierError::KindPayloadMismatch { kind: entry.kind });
        }
        entry.payload = payload;
        Ok(())
    }

    /// 평가 대상(활성) 항목을 순서대로 반환합니다. 비활성은 건너뜁니다.
    pub fn active_entries(&self) -> impl Iterator<Item = &ModifierEntry> {
        self.entries.iter().filter(|e| e.enabled)
    }

    /// 활성 Opacity modifier들의 곱(평가 예시 — 비활성은 기여하지 않음).
    pub fn effective_opacity(&self) -> f32 {
        self.active_entries()
            .filter_map(|e| match e.payload {
                ModifierPayload::Opacity { value } => Some(value.get()),
                _ => None,
            })
            .product()
    }

    /// 항목을 제거합니다.
    pub fn remove(&mut self, id: ModifierId) -> Result<ModifierEntry, ModifierError> {
        let index = self
            .entries
            .iter()
            .position(|e| e.id == id)
            .ok_or(ModifierError::NotFound { id })?;
        Ok(self.entries.remove(index))
    }

    /// canonical bytes: node id + 항목 수 + (id 16 + key 길이 u32 + key bytes + kind u16 + enabled u8 + payload 길이 u32 + payload)*.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.node.to_bytes());
        buf.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for entry in &self.entries {
            buf.extend_from_slice(&entry.id.0.to_bytes());
            let key = entry.order.as_bytes();
            buf.extend_from_slice(&(key.len() as u32).to_le_bytes());
            buf.extend_from_slice(key);
            buf.extend_from_slice(&entry.kind.as_u16().to_le_bytes());
            buf.push(u8::from(entry.enabled));
            let mut payload = Vec::new();
            entry.payload.encode_into(&mut payload);
            buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            buf.extend_from_slice(&payload);
        }
        buf
    }

    /// bytes에서 복원합니다. 중복·정렬 위반은 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<ModifierStack, ModifierError> {
        let mut pos = 0usize;
        let mut take = |n: usize| -> Result<&[u8], ModifierError> {
            if bytes.len() - pos < n {
                return Err(ModifierError::Malformed {
                    reason: "입력 부족",
                });
            }
            let slice = &bytes[pos..pos + n];
            pos += n;
            Ok(slice)
        };
        let id_of = |s: &[u8]| {
            let mut b = [0u8; 16];
            b.copy_from_slice(s);
            StableId::from_bytes(b)
        };
        let node = id_of(take(16)?);
        let count = u32::from_le_bytes(take(4)?.try_into().expect("4바이트")) as usize;
        let mut stack = ModifierStack::new(node);
        let mut previous: Option<OrderKey> = None;
        for _ in 0..count {
            let id = ModifierId(id_of(take(16)?));
            let key_len = u32::from_le_bytes(take(4)?.try_into().expect("4바이트")) as usize;
            let order = OrderKey::from_bytes(take(key_len)?.to_vec()).map_err(|_| {
                ModifierError::Malformed {
                    reason: "잘못된 OrderKey",
                }
            })?;
            if previous.as_ref().is_some_and(|p| *p >= order) {
                return Err(ModifierError::Malformed {
                    reason: "OrderKey 정렬 위반",
                });
            }
            let kind_raw = u16::from_le_bytes(take(2)?.try_into().expect("2바이트"));
            let kind = ModifierKind::from_u16(kind_raw).ok_or(ModifierError::Malformed {
                reason: "알 수 없는 modifier kind",
            })?;
            let enabled = match take(1)?[0] {
                0 => false,
                1 => true,
                _ => {
                    return Err(ModifierError::Malformed {
                        reason: "enabled flag",
                    })
                }
            };
            let payload_len = u32::from_le_bytes(take(4)?.try_into().expect("4바이트")) as usize;
            let payload = ModifierPayload::decode(kind, take(payload_len)?)?;
            previous = Some(order.clone());
            stack.insert(ModifierEntry {
                id,
                order,
                kind,
                enabled,
                payload,
            })?;
        }
        if pos != bytes.len() {
            return Err(ModifierError::Malformed {
                reason: "trailing bytes",
            });
        }
        Ok(stack)
    }
}

// --- 참조 의존성 등록 (VI.9.4) ---

impl ModifierPayload {
    /// 이 payload가 참조하는 다른 node들(Boolean 피연산, Mask/Clip 원본).
    /// NIL 참조(대상 미지정)는 제외합니다.
    pub fn referenced_nodes(&self) -> Vec<StableId> {
        let ids: Vec<StableId> = match self {
            ModifierPayload::Boolean { operands, .. } => operands.iter().map(|r| r.id()).collect(),
            ModifierPayload::Mask { source } | ModifierPayload::Clip { source } => {
                vec![source.id()]
            }
            _ => Vec::new(),
        };
        ids.into_iter().filter(|id| *id != StableId::NIL).collect()
    }
}

impl ModifierStack {
    /// stack의 모든 modifier 참조를 소유 node 기준으로 ReferenceIndex에 맞춥니다.
    ///
    /// 소유 node의 기존 outbound edge를 전부 지우고 현재 참조로 다시 등록합니다 —
    /// 참조 추가·제거·modifier 삭제 후 호출하면 inbound index가 정확해집니다.
    /// 비활성 modifier의 참조도 등록합니다(저장에 남아 있어 대상 삭제 시 영향이
    /// 있으므로).
    pub fn sync_references(&self, index: &mut crate::reference_index::ReferenceIndex) {
        index.remove_all_from(self.node);
        for entry in &self.entries {
            for target in entry.payload.referenced_nodes() {
                index.add_reference(self.node, target);
            }
        }
    }
}

// --- 참조 cycle 검출 (VI.9.5) ---

/// modifier 참조 graph의 cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModifierCycle {
    /// cycle을 이루는 node 경로. 첫 node가 마지막에 다시 나옵니다
    /// (예: `[a, b, a]`, 자기 참조는 `[a, a]`).
    pub path: Vec<StableId>,
}

impl fmt::Display for ModifierCycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<String> = self.path.iter().map(|id| id.to_string()).collect();
        write!(f, "modifier 참조 cycle: {}", names.join(" → "))
    }
}

impl std::error::Error for ModifierCycle {}

/// 모든 stack의 modifier 참조(소유 node → 참조 node)로 graph를 만들고 cycle을
/// 찾습니다. self·direct·indirect cycle을 모두 검출하며, 저장 전 검증(VI.10.1)이
/// 이 결과로 저장을 거부합니다. 비활성 modifier 참조도 포함합니다(다시 켜는 순간
/// cycle이 되므로).
///
/// 결정론: node·간선을 StableId 순으로 방문하므로 같은 입력은 같은 cycle을
/// 보고합니다.
pub fn find_modifier_cycle(stacks: &[ModifierStack]) -> Option<ModifierCycle> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut graph: BTreeMap<StableId, BTreeSet<StableId>> = BTreeMap::new();
    for stack in stacks {
        let targets = graph.entry(stack.node).or_default();
        for entry in &stack.entries {
            targets.extend(entry.payload.referenced_nodes());
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Visiting,
        Done,
    }
    let mut marks: BTreeMap<StableId, Mark> = BTreeMap::new();

    // 반복 DFS(재귀 깊이 제한 회피): (node, 다음 간선 index) 스택 + 현재 경로.
    let roots: Vec<StableId> = graph.keys().copied().collect();
    for root in roots {
        if marks.contains_key(&root) {
            continue;
        }
        let mut path: Vec<StableId> = vec![root];
        let mut iters: Vec<Vec<StableId>> = vec![graph
            .get(&root)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()];
        marks.insert(root, Mark::Visiting);
        while let Some(next_list) = iters.last_mut() {
            if let Some(next) = next_list.first().copied() {
                next_list.remove(0);
                match marks.get(&next) {
                    Some(Mark::Visiting) => {
                        let start = path.iter().position(|n| *n == next).expect("경로 위");
                        let mut cycle = path[start..].to_vec();
                        cycle.push(next);
                        return Some(ModifierCycle { path: cycle });
                    }
                    Some(Mark::Done) => {}
                    None => {
                        marks.insert(next, Mark::Visiting);
                        path.push(next);
                        iters.push(
                            graph
                                .get(&next)
                                .map(|s| s.iter().copied().collect())
                                .unwrap_or_default(),
                        );
                    }
                }
            } else {
                iters.pop();
                let done = path.pop().expect("경로와 반복자 길이 일치");
                marks.insert(done, Mark::Done);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(seed: u64) -> ModifierId {
        ModifierId::new(StableId::new_v7(seed, 0, seed))
    }

    #[test]
    fn id_and_order_round_trip() {
        // 검증 기준(VI.9.1): ID와 순서 round-trip.
        let mut stack = ModifierStack::new(StableId::new_v7(1, 0, 1));
        stack.push_back(id(10), ModifierKind::Fill).unwrap();
        stack.push_back(id(11), ModifierKind::Stroke).unwrap();
        stack.push_back(id(12), ModifierKind::Shadow).unwrap();
        // 맨 앞에 삽입(다른 항목의 key 불변).
        let first_key = OrderKey::between(None, Some(&stack.entries()[0].order));
        let keys_before: Vec<_> = stack.entries().iter().map(|e| e.order.clone()).collect();
        stack
            .insert(ModifierEntry {
                id: id(9),
                order: first_key,
                kind: ModifierKind::Transform,
                enabled: true,
                payload: ModifierPayload::default_for(ModifierKind::Transform),
            })
            .unwrap();
        assert_eq!(stack.ids_in_order(), vec![id(9), id(10), id(11), id(12)]);
        let keys_after: Vec<_> = stack.entries()[1..]
            .iter()
            .map(|e| e.order.clone())
            .collect();
        assert_eq!(keys_after, keys_before, "기존 key 불변");

        let bytes = stack.encode();
        let decoded = ModifierStack::decode(&bytes).unwrap();
        assert_eq!(decoded, stack);
        assert_eq!(decoded.encode(), bytes);
    }

    #[test]
    fn stack_is_stored_apart_from_node_payload() {
        // stack은 소유 node id만 참조하고 node payload bytes를 포함하지 않습니다.
        let stack = ModifierStack::new(StableId::new_v7(1, 0, 1));
        assert_eq!(stack.encode().len(), 16 + 4);
    }

    #[test]
    fn duplicates_and_malformed_are_rejected() {
        let mut stack = ModifierStack::new(StableId::new_v7(1, 0, 1));
        let key = stack.push_back(id(10), ModifierKind::Fill).unwrap();
        assert_eq!(
            stack.push_back(id(10), ModifierKind::Fill).unwrap_err(),
            ModifierError::DuplicateId { id: id(10) }
        );
        assert_eq!(
            stack
                .insert(ModifierEntry {
                    id: id(11),
                    order: key,
                    kind: ModifierKind::Blur,
                    enabled: true,
                    payload: ModifierPayload::default_for(ModifierKind::Blur),
                })
                .unwrap_err(),
            ModifierError::DuplicateOrderKey
        );
        assert_eq!(stack.remove(id(10)).unwrap().id, id(10));
        assert!(matches!(
            stack.remove(id(10)),
            Err(ModifierError::NotFound { .. })
        ));
        let mut bytes = stack.encode();
        bytes.push(0);
        assert!(ModifierStack::decode(&bytes).is_err());
        assert!(ModifierStack::decode(&[0; 5]).is_err());
    }

    #[test]
    fn golden_numeric_modifier_kinds() {
        // 검증 기준(VI.9.2): numeric kind golden test.
        let golden: [(ModifierKind, u16, &str); 12] = [
            (ModifierKind::Transform, 1, "transform"),
            (ModifierKind::Boolean, 2, "boolean"),
            (ModifierKind::StrokeExpand, 3, "stroke-expand"),
            (ModifierKind::Corner, 4, "corner"),
            (ModifierKind::Mask, 5, "mask"),
            (ModifierKind::Clip, 6, "clip"),
            (ModifierKind::Fill, 7, "fill"),
            (ModifierKind::Stroke, 8, "stroke"),
            (ModifierKind::Shadow, 9, "shadow"),
            (ModifierKind::Blur, 10, "blur"),
            (ModifierKind::Blend, 11, "blend"),
            (ModifierKind::Opacity, 12, "opacity"),
        ];
        assert_eq!(ModifierKind::ALL.len(), golden.len());
        for (kind, value, name) in golden {
            assert_eq!(kind.as_u16(), value, "{name}");
            assert_eq!(kind.name(), name);
            assert_eq!(ModifierKind::from_u16(value), Some(kind));
        }
        assert_eq!(ModifierKind::from_u16(0), None);
        assert_eq!(ModifierKind::from_u16(13), None);
    }

    #[test]
    fn unknown_kind_in_bytes_is_rejected() {
        let mut stack = ModifierStack::new(StableId::new_v7(1, 0, 1));
        stack.push_back(id(10), ModifierKind::Opacity).unwrap();
        let bytes = stack.encode();
        // kind 위치: node 16 + count 4 + id 16 + key len 4 + key.
        let key_len = stack.entries()[0].order.byte_len();
        let kind_at = 16 + 4 + 16 + 4 + key_len;
        let mut bad = bytes.clone();
        bad[kind_at..kind_at + 2].copy_from_slice(&99u16.to_le_bytes());
        assert!(ModifierStack::decode(&bad).is_err());
    }

    #[test]
    fn disabled_modifier_is_skipped_but_kept() {
        // 검증 기준(VI.9.3): enable/disable 결과 fixture.
        let mut stack = ModifierStack::new(StableId::new_v7(1, 0, 1));
        stack.push_back(id(10), ModifierKind::Opacity).unwrap();
        stack.push_back(id(11), ModifierKind::Opacity).unwrap();
        stack.push_back(id(12), ModifierKind::Fill).unwrap();
        stack
            .set_payload(
                id(10),
                ModifierPayload::Opacity {
                    value: FiniteF32::new(0.5).unwrap(),
                },
            )
            .unwrap();
        stack
            .set_payload(
                id(11),
                ModifierPayload::Opacity {
                    value: FiniteF32::new(0.4).unwrap(),
                },
            )
            .unwrap();
        assert!((stack.effective_opacity() - 0.2).abs() < 1e-6);

        // 비활성화 → 평가에서 제외.
        stack.set_enabled(id(11), false).unwrap();
        assert!((stack.effective_opacity() - 0.5).abs() < 1e-6);
        assert_eq!(stack.active_entries().count(), 2);
        // 저장에는 유지(enabled=false 그대로 round-trip).
        let decoded = ModifierStack::decode(&stack.encode()).unwrap();
        assert_eq!(decoded, stack);
        assert_eq!(decoded.entries().len(), 3);
        assert!(!decoded.entries()[1].enabled);
        // 다시 활성화하면 원래 결과.
        stack.set_enabled(id(11), true).unwrap();
        assert!((stack.effective_opacity() - 0.2).abs() < 1e-6);
    }

    #[test]
    fn every_kind_payload_round_trips() {
        let mut stack = ModifierStack::new(StableId::new_v7(1, 0, 1));
        for (i, kind) in ModifierKind::ALL.into_iter().enumerate() {
            stack.push_back(id(100 + i as u64), kind).unwrap();
        }
        stack
            .set_payload(
                id(101),
                ModifierPayload::Boolean {
                    op: BooleanOp::Subtract,
                    operands: vec![
                        NodeRef::new(StableId::new_v7(5, 0, 5)),
                        NodeRef::new(StableId::new_v7(6, 0, 6)),
                    ],
                },
            )
            .unwrap();
        let bytes = stack.encode();
        let decoded = ModifierStack::decode(&bytes).unwrap();
        assert_eq!(decoded, stack);
        assert_eq!(decoded.encode(), bytes);
    }

    #[test]
    fn payload_validation_and_kind_mismatch() {
        let mut stack = ModifierStack::new(StableId::new_v7(1, 0, 1));
        stack.push_back(id(10), ModifierKind::Opacity).unwrap();
        assert!(matches!(
            stack.set_payload(
                id(10),
                ModifierPayload::Opacity {
                    value: FiniteF32::new(1.5).unwrap()
                }
            ),
            Err(ModifierError::InvalidPayload { .. })
        ));
        assert_eq!(
            stack
                .set_payload(
                    id(10),
                    ModifierPayload::Blur {
                        radius: FiniteF32::ONE
                    }
                )
                .unwrap_err(),
            ModifierError::KindPayloadMismatch {
                kind: ModifierKind::Opacity
            }
        );
        assert!(matches!(
            stack.set_payload(
                id(10),
                ModifierPayload::Opacity {
                    value: FiniteF32::new(-0.1).unwrap()
                }
            ),
            Err(ModifierError::InvalidPayload { .. })
        ));
    }

    #[test]
    fn reference_index_tracks_modifier_dependencies() {
        // 검증 기준(VI.9.4): 참조 추가·제거 시 inbound index 정확.
        use crate::reference_index::ReferenceIndex;
        let owner = StableId::new_v7(1, 0, 1);
        let a = StableId::new_v7(50, 0, 50);
        let b = StableId::new_v7(51, 0, 51);
        let mask_src = StableId::new_v7(52, 0, 52);
        let mut index = ReferenceIndex::new();
        let mut stack = ModifierStack::new(owner);

        // Boolean(a, b) + Mask(mask_src) 추가.
        stack.push_back(id(10), ModifierKind::Boolean).unwrap();
        stack
            .set_payload(
                id(10),
                ModifierPayload::Boolean {
                    op: BooleanOp::Union,
                    operands: vec![NodeRef::new(a), NodeRef::new(b)],
                },
            )
            .unwrap();
        stack.push_back(id(11), ModifierKind::Mask).unwrap();
        stack.sync_references(&mut index);
        // Mask는 아직 NIL → 등록 안 됨.
        assert_eq!(index.inbound(a), vec![owner]);
        assert_eq!(index.inbound(b), vec![owner]);
        assert!(!index.has_inbound(mask_src));

        stack
            .set_payload(
                id(11),
                ModifierPayload::Mask {
                    source: NodeRef::new(mask_src),
                },
            )
            .unwrap();
        stack.sync_references(&mut index);
        assert_eq!(index.inbound(mask_src), vec![owner]);

        // 피연산 b 제거 → b inbound 사라짐.
        stack
            .set_payload(
                id(10),
                ModifierPayload::Boolean {
                    op: BooleanOp::Union,
                    operands: vec![NodeRef::new(a)],
                },
            )
            .unwrap();
        stack.sync_references(&mut index);
        assert!(!index.has_inbound(b));
        assert_eq!(index.inbound(a), vec![owner]);

        // 비활성 modifier 참조도 유지.
        stack.set_enabled(id(11), false).unwrap();
        stack.sync_references(&mut index);
        assert_eq!(index.inbound(mask_src), vec![owner]);

        // modifier 삭제 → 참조 제거, 다른 node의 참조는 유지.
        index.add_reference(StableId::new_v7(2, 0, 2), a);
        stack.remove(id(10)).unwrap();
        stack.sync_references(&mut index);
        assert_eq!(index.inbound(a), vec![StableId::new_v7(2, 0, 2)]);
        assert_eq!(index.edge_count(), 2);
    }

    fn mask_stack(owner: u64, source: u64, modifier: u64) -> ModifierStack {
        let mut stack = ModifierStack::new(StableId::new_v7(owner, 0, owner));
        stack.push_back(id(modifier), ModifierKind::Mask).unwrap();
        stack
            .set_payload(
                id(modifier),
                ModifierPayload::Mask {
                    source: NodeRef::new(StableId::new_v7(source, 0, source)),
                },
            )
            .unwrap();
        stack
    }

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    #[test]
    fn self_direct_and_indirect_cycles_are_detected() {
        // 검증 기준(VI.9.5): cycle fixture 전부 실패.
        // self: A mask = A.
        let cycle = find_modifier_cycle(&[mask_stack(1, 1, 10)]).expect("self cycle");
        assert_eq!(cycle.path, vec![sid(1), sid(1)]);

        // direct: A → B, B → A.
        let cycle = find_modifier_cycle(&[mask_stack(1, 2, 10), mask_stack(2, 1, 11)])
            .expect("direct cycle");
        assert_eq!(cycle.path, vec![sid(1), sid(2), sid(1)]);

        // indirect: A → B → C → A (Boolean 경유 포함).
        let mut c = ModifierStack::new(sid(3));
        c.push_back(id(12), ModifierKind::Boolean).unwrap();
        c.set_payload(
            id(12),
            ModifierPayload::Boolean {
                op: BooleanOp::Intersect,
                operands: vec![NodeRef::new(sid(9)), NodeRef::new(sid(1))],
            },
        )
        .unwrap();
        let cycle = find_modifier_cycle(&[mask_stack(1, 2, 10), mask_stack(2, 3, 11), c])
            .expect("indirect cycle");
        assert_eq!(cycle.path, vec![sid(1), sid(2), sid(3), sid(1)]);
        assert!(cycle.to_string().contains("→"));
    }

    #[test]
    fn acyclic_graphs_and_disabled_modifiers() {
        // chain A → B → C, D → C (공유 대상) — cycle 없음.
        let stacks = [
            mask_stack(1, 2, 10),
            mask_stack(2, 3, 11),
            mask_stack(4, 3, 12),
        ];
        assert!(find_modifier_cycle(&stacks).is_none());
        // 비활성 modifier가 만드는 cycle도 거부(다시 켜면 cycle이 되므로).
        let mut back = mask_stack(3, 1, 13);
        back.set_enabled(id(13), false).unwrap();
        let stacks = [mask_stack(1, 2, 10), mask_stack(2, 3, 11), back];
        assert!(find_modifier_cycle(&stacks).is_some());
        // 결정론.
        assert_eq!(find_modifier_cycle(&stacks), find_modifier_cycle(&stacks));
    }
}
