//! 프로젝트 창별 터미널 세션 registry (XVIII.1.2).
//!
//! 프로젝트 창 하나([`WindowSessionId`])가 registry 하나를 가지고, 그 안에 여러 터미널 세션을
//! 둡니다(기술 아키텍처 32 "프로젝트 창별 다중 세션"). registry는 세션 메모리에만 있으며 창을 닫으면
//! 함께 사라집니다.
//!
//! - 세션 수는 [`RegistryLimits::max_sessions`]로 제한합니다.
//! - 식별자는 registry의 [`SessionIdAllocator`]가 발급하고 재사용하지 않습니다. 닫은 세션의
//!   식별자·handle은 다시 해석되지 않습니다.
//! - 세션은 만든 순서(발급 순번)로 나열됩니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::WindowSessionId;

use crate::id::{SessionIdAllocator, TerminalSessionId};
use crate::profile::TerminalProfile;
use crate::state::TerminalProcessState;

/// 창 하나의 기본 세션 상한.
pub const DEFAULT_MAX_SESSIONS: usize = 32;

/// registry 한도.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryLimits {
    /// 동시에 열어 둘 수 있는 세션 수.
    pub max_sessions: usize,
}

impl Default for RegistryLimits {
    fn default() -> Self {
        RegistryLimits {
            max_sessions: DEFAULT_MAX_SESSIONS,
        }
    }
}

/// registry 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryError {
    /// 세션 수 상한에 도달했습니다.
    LimitReached {
        /// 상한.
        limit: usize,
    },
    /// 이 registry에 없는 세션입니다(닫혔거나 다른 창의 세션).
    UnknownSession,
    /// 식별자 공간을 다 썼습니다.
    IdSpaceExhausted,
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::LimitReached { limit } => {
                write!(f, "터미널 세션은 창마다 {limit}개까지 열 수 있습니다")
            }
            RegistryError::UnknownSession => f.write_str("터미널 세션을 찾을 수 없습니다"),
            RegistryError::IdSpaceExhausted => {
                f.write_str("터미널 세션 식별자를 더 발급할 수 없습니다")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

/// registry 안의 세션 하나.
#[derive(Debug)]
pub struct TerminalSession {
    id: TerminalSessionId,
    profile: TerminalProfile,
    state: TerminalProcessState,
}

impl TerminalSession {
    /// 식별자.
    pub fn id(&self) -> TerminalSessionId {
        self.id
    }

    /// 세션을 만들 때 복사한 profile.
    pub fn profile(&self) -> &TerminalProfile {
        &self.profile
    }

    /// 현재 상태.
    pub fn state(&self) -> TerminalProcessState {
        self.state
    }
}

/// 프로젝트 창 하나의 터미널 세션 registry.
#[derive(Debug)]
pub struct TerminalRegistry {
    window: WindowSessionId,
    limits: RegistryLimits,
    ids: SessionIdAllocator,
    sessions: BTreeMap<TerminalSessionId, TerminalSession>,
}

impl TerminalRegistry {
    /// 기본 한도의 registry.
    pub fn new(window: WindowSessionId) -> TerminalRegistry {
        TerminalRegistry::with_limits(window, RegistryLimits::default())
    }

    /// 한도를 지정한 registry.
    pub fn with_limits(window: WindowSessionId, limits: RegistryLimits) -> TerminalRegistry {
        TerminalRegistry {
            window,
            limits,
            ids: SessionIdAllocator::new(),
            sessions: BTreeMap::new(),
        }
    }

    /// 이 registry가 속한 프로젝트 창.
    pub fn window(&self) -> WindowSessionId {
        self.window
    }

    /// 한도.
    pub fn limits(&self) -> RegistryLimits {
        self.limits
    }

    /// profile로 새 세션을 만듭니다(아직 시작하지 않음).
    pub fn create(&mut self, profile: TerminalProfile) -> Result<TerminalSessionId, RegistryError> {
        if self.sessions.len() >= self.limits.max_sessions {
            return Err(RegistryError::LimitReached {
                limit: self.limits.max_sessions,
            });
        }
        let id = self
            .ids
            .allocate()
            .map_err(|_| RegistryError::IdSpaceExhausted)?;
        self.sessions.insert(
            id,
            TerminalSession {
                id,
                profile,
                state: TerminalProcessState::NotStarted,
            },
        );
        Ok(id)
    }

    /// 세션을 닫고 registry에서 제거합니다. 제거 직전 상태를 돌려줍니다.
    pub fn close(&mut self, id: TerminalSessionId) -> Result<TerminalProcessState, RegistryError> {
        let session = self
            .sessions
            .remove(&id)
            .ok_or(RegistryError::UnknownSession)?;
        Ok(session.state)
    }

    /// 세션.
    pub fn session(&self, id: TerminalSessionId) -> Option<&TerminalSession> {
        self.sessions.get(&id)
    }

    /// 열린 세션 식별자(만든 순서).
    pub fn ids(&self) -> Vec<TerminalSessionId> {
        self.sessions.keys().copied().collect()
    }

    /// 열린 세션 수.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// 열린 세션이 없는지.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// FFI·UI handle을 이 registry의 **열린** 세션 식별자로 되돌립니다.
    pub fn resolve_handle(&self, handle: u64) -> Option<TerminalSessionId> {
        self.ids
            .decode(handle)
            .filter(|id| self.sessions.contains_key(id))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::profile::ProfileName;
    use stellar_core_types::ids::StableId;

    pub(crate) fn window(n: u8) -> WindowSessionId {
        WindowSessionId::from_stable_id(StableId::from_bytes([n; 16]))
    }

    pub(crate) fn profile(name: &str) -> TerminalProfile {
        TerminalProfile::default_shell(ProfileName::new(name).unwrap())
    }

    #[test]
    fn twenty_sessions_create_and_close_without_leaks() {
        let mut registry = TerminalRegistry::new(window(1));
        let mut all_ids = Vec::new();
        for round in 0..5 {
            let ids: Vec<TerminalSessionId> = (0..20)
                .map(|n| registry.create(profile(&format!("s{round}-{n}"))).unwrap())
                .collect();
            assert_eq!(registry.len(), 20);
            assert_eq!(registry.ids(), ids);
            for id in &ids {
                assert_eq!(registry.resolve_handle(id.to_handle()), Some(*id));
                assert_eq!(registry.close(*id), Ok(TerminalProcessState::NotStarted));
            }
            assert!(registry.is_empty());
            assert!(registry.sessions.is_empty());
            for id in &ids {
                // 닫은 세션은 다시 해석·조회·닫기되지 않습니다.
                assert_eq!(registry.resolve_handle(id.to_handle()), None);
                assert!(registry.session(*id).is_none());
                assert_eq!(registry.close(*id), Err(RegistryError::UnknownSession));
            }
            all_ids.extend(ids);
        }
        // 식별자는 재사용하지 않습니다.
        let mut unique = all_ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), all_ids.len());
    }

    #[test]
    fn session_limit_is_enforced_and_freed_by_close() {
        let mut registry =
            TerminalRegistry::with_limits(window(2), RegistryLimits { max_sessions: 2 });
        let a = registry.create(profile("a")).unwrap();
        let _b = registry.create(profile("b")).unwrap();
        assert_eq!(
            registry.create(profile("c")),
            Err(RegistryError::LimitReached { limit: 2 })
        );
        registry.close(a).unwrap();
        assert!(registry.create(profile("c")).is_ok());
    }

    #[test]
    fn registries_of_different_windows_do_not_share_sessions() {
        let mut first = TerminalRegistry::new(window(3));
        let mut second = TerminalRegistry::new(window(4));
        let id = first.create(profile("first")).unwrap();
        let _ = second.create(profile("second")).unwrap();
        assert_eq!(first.window(), window(3));
        assert_eq!(second.resolve_handle(id.to_handle()), None);
        assert!(second.session(id).is_none());
        assert_eq!(second.close(id), Err(RegistryError::UnknownSession));
        assert_eq!(first.session(id).unwrap().profile().name.as_str(), "first");
        assert_eq!(
            first.session(id).unwrap().state(),
            TerminalProcessState::NotStarted
        );
    }
}
