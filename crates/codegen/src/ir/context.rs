//! Context 선언·provision·소비의 IR (XXII.1.6, ADR-0106 결정 1.4, ADR-0025 결정 3·4.1).
//!
//! 해석은 XXX.7.4 Context 해석기가 합니다. IR은 해석 결과(소비 지점마다 값의 출처와 건넌 slot 경계)를 그대로
//! 보존하고, adapter는 다시 해석하지 않습니다. IR은 참조 무결성(선언된 key, View node, 출처 provision 존재)만
//! 검사합니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;

use super::behavior::IrExpr;

/// `ContextKey` 선언.
#[derive(Debug, Clone, PartialEq)]
pub struct IrContextKey {
    /// StableId.
    pub id: StableId,
    /// 이름(비어 있으면 안 됨). CSS 이름은 IX.11.16 파생 규칙으로 adapter 상위가 만듭니다.
    pub name: String,
    /// default 값 식(닫힌 식).
    pub default: IrExpr,
}

/// node에 붙은 `ContextProvision`.
#[derive(Debug, Clone, PartialEq)]
pub struct IrContextProvision {
    /// provider node.
    pub node: StableId,
    /// key.
    pub key: StableId,
    /// 값 식. 같은 key 읽기는 바깥(조상) 값입니다(ADR-0025 결정 3.5).
    pub value: IrExpr,
    /// 펼친 트리 위치마다의 provision 깊이 d(자기 포함 조상 provision 수, 1 이상). 오름차순, 중복 없음.
    pub depths: Vec<u32>,
}

/// 소비 지점 값의 출처(해석기 결과).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IrContextSource {
    /// 가장 가까운 조상 provision의 provider node.
    Provision(StableId),
    /// key default.
    Default,
}

/// 소비 지점이 건넌 slot 경계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IrSlotCrossing {
    /// slot host node.
    pub host: StableId,
    /// instance node.
    pub instance: StableId,
}

/// Context 소비 지점.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrContextConsumption {
    /// 소비 node.
    pub node: StableId,
    /// binding 대상 속성.
    pub path: PropertyPath,
    /// key.
    pub key: StableId,
    /// 값의 출처.
    pub source: IrContextSource,
    /// 안쪽부터 건넌 slot 경계.
    pub slots: Vec<IrSlotCrossing>,
}

/// slot host와 Slot의 공개 Context key 목록(ADR-0025 결정 3.11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrSlotHost {
    /// slot host node.
    pub host: StableId,
    /// instance node.
    pub instance: StableId,
    /// Slot이 공개한 key(오름차순).
    pub public_keys: Vec<StableId>,
}

/// Context 해석 결과 입력이자 IR 값.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IrContexts {
    /// key(id 오름차순).
    pub keys: Vec<IrContextKey>,
    /// provision((node, key) 오름차순).
    pub provisions: Vec<IrContextProvision>,
    /// 소비((node, path, key) 오름차순).
    pub consumptions: Vec<IrContextConsumption>,
    /// slot host(host 오름차순).
    pub slots: Vec<IrSlotHost>,
}

impl IrContexts {
    /// key 선언.
    pub fn key(&self, id: StableId) -> Option<&IrContextKey> {
        self.keys
            .binary_search_by(|k| k.id.cmp(&id))
            .ok()
            .map(|index| &self.keys[index])
    }

    /// (node, key)의 provision.
    pub fn provision(&self, node: StableId, key: StableId) -> Option<&IrContextProvision> {
        self.provisions
            .binary_search_by(|p| (p.node, p.key).cmp(&(node, key)))
            .ok()
            .map(|index| &self.provisions[index])
    }

    /// 속성 경로를 소비하는 지점(있으면). 해석기 결과를 그대로 돌려줍니다.
    pub fn consumption_of(&self, path: &PropertyPath) -> Option<&IrContextConsumption> {
        self.consumptions
            .iter()
            .find(|c| c.node == path.root() && &c.path == path)
    }

    /// key의 provision(provider node 오름차순).
    pub fn provisions_of(&self, key: StableId) -> impl Iterator<Item = &IrContextProvision> + '_ {
        self.provisions.iter().filter(move |p| p.key == key)
    }
}
