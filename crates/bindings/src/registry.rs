//! 프로젝트·라이브러리·문서 scope 토큰 registry (VIII.5.2).
//!
//! 토큰은 세 종류의 scope 중 하나에 속하는 [`TokenSet`]에 정의됩니다.
//!
//! | scope | 볼 수 있는 scope(우선순위 순) |
//! |---|---|
//! | [`TokenScope::Document`] | 같은 문서 → 프로젝트 → 모든 라이브러리 |
//! | [`TokenScope::Project`] | 프로젝트 → 모든 라이브러리 |
//! | [`TokenScope::Library`] | 같은 라이브러리만(외부 라이브러리는 사용 프로젝트를 모릅니다) |
//!
//! **resolution 규칙**
//!
//! 1. **ID 우선**: 표현식의 토큰 참조는 [`TokenRef`]이며 [`TokenRegistry::resolve`]로 scope·이름과 무관하게
//!    정확히 한 정의로 해석됩니다. 하나의 `TokenRef`는 registry 전체에서 한 scope에만 존재합니다.
//! 2. **같은 이름 허용 여부**: 한 scope(TokenSet) 안에서는 이름이 유일해야 합니다(VIII.5.1). 서로 다른 scope는
//!    같은 이름을 가질 수 있습니다.
//! 3. **이름 resolution(shadowing)**: 이름은 GUI 검색·작성 보조에만 씁니다. [`TokenRegistry::resolve_name`]은 위
//!    표의 우선순위대로 scope 계층을 보고 이름이 있는 첫 계층의 토큰을 고릅니다. 문서가 프로젝트를, 프로젝트가
//!    라이브러리를 가립니다. 라이브러리들은 같은 계층이라 두 라이브러리에 같은 이름이 있으면
//!    [`NameResolution::Ambiguous`]입니다. 이름 resolution 결과를 저장할 때는 ID로 기록합니다.
//! 4. **참조 가시성**: 토큰 default expression이 참조하는 토큰은 그 토큰 scope에서 볼 수 있는 scope에 있어야
//!    합니다([`TokenRegistry::reference_issues`]).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::TokenRef;
use stellar_expressions::ast::ExpressionAst;

use crate::token::{TokenDefinition, TokenName, TokenSet, TokenSetError, TokenType, TokenTypes};

/// 토큰이 속한 scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TokenScope {
    /// 프로젝트 공통 토큰.
    Project,
    /// 외부 라이브러리 토큰. 값은 dependencies의 LibraryId와 같은 `StableId`입니다.
    Library(StableId),
    /// 문서 전용 토큰. 값은 문서 `StableId`입니다.
    Document(StableId),
}

impl TokenScope {
    /// 이름 resolution 계층(작을수록 우선, 라이브러리들은 같은 계층).
    const fn tier(&self) -> u8 {
        match self {
            TokenScope::Document(_) => 0,
            TokenScope::Project => 1,
            TokenScope::Library(_) => 2,
        }
    }
}

impl fmt::Display for TokenScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenScope::Project => f.write_str("project"),
            TokenScope::Library(id) => write!(f, "library {id}"),
            TokenScope::Document(id) => write!(f, "document {id}"),
        }
    }
}

/// registry 편집 오류.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RegistryError {
    /// 같은 scope가 이미 등록되어 있습니다.
    DuplicateScope(TokenScope),
    /// scope가 등록되어 있지 않습니다.
    UnknownScope(TokenScope),
    /// 같은 토큰 ID가 다른 scope에 이미 있습니다(ID는 registry 전체에서 유일).
    IdInOtherScope {
        /// 토큰.
        token: TokenRef,
        /// 이미 정의된 scope.
        existing: TokenScope,
    },
    /// scope 안 TokenSet 편집 오류.
    Set(TokenSetError),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::DuplicateScope(scope) => write!(f, "scope가 이미 있습니다: {scope}"),
            RegistryError::UnknownScope(scope) => write!(f, "scope가 없습니다: {scope}"),
            RegistryError::IdInOtherScope { token, existing } => {
                write!(f, "토큰 {}가 이미 {existing}에 있습니다", token.id())
            }
            RegistryError::Set(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for RegistryError {}

impl From<TokenSetError> for RegistryError {
    fn from(error: TokenSetError) -> RegistryError {
        RegistryError::Set(error)
    }
}

/// ID로 해석한 토큰.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedToken<'a> {
    /// 정의된 scope.
    pub scope: TokenScope,
    /// 정의.
    pub definition: &'a TokenDefinition,
}

/// 이름 resolution 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameResolution {
    /// 우선순위가 가장 높은 계층의 유일한 토큰.
    Found {
        /// 토큰.
        token: TokenRef,
        /// 정의된 scope.
        scope: TokenScope,
    },
    /// 같은 계층(라이브러리들)에 같은 이름이 둘 이상 있습니다(scope 순).
    Ambiguous(Vec<(TokenScope, TokenRef)>),
    /// 볼 수 있는 scope에 그 이름이 없습니다.
    NotFound,
}

/// 토큰 참조 검사 결과 하나.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReferenceIssue {
    /// 참조한 토큰이 registry에 없습니다.
    Dangling {
        /// 참조하는 토큰.
        token: TokenRef,
        /// 없는 대상.
        target: TokenRef,
    },
    /// 참조한 토큰이 참조하는 토큰의 scope에서 보이지 않습니다.
    NotVisible {
        /// 참조하는 토큰.
        token: TokenRef,
        /// 참조하는 토큰의 scope.
        from: TokenScope,
        /// 대상 토큰.
        target: TokenRef,
        /// 대상 토큰의 scope.
        target_scope: TokenScope,
    },
    /// 참조한 토큰이 orphan plan(VIII.5.5)으로 삭제되어 참조가 orphan 상태입니다. 의도한 상태이므로 저장을
    /// 막지 않습니다([`crate::token_graph::validate_for_save`]).
    Orphaned {
        /// 참조하는 토큰.
        token: TokenRef,
        /// 삭제된 대상.
        target: TokenRef,
    },
}

/// orphan plan으로 삭제된 토큰의 기록(VIII.5.5). 참조를 지우지 않고 원래 ID·scope·이름·마지막 유효 타입을
/// 보존합니다(아키텍처 §14.4). 참조하는 토큰은 [`TokenRegistry::referrers`]로 언제든 다시 구합니다. 같은 ID를 같은
/// scope·이름·타입으로 다시 만들면 기록이 사라지고 참조가 다시 연결됩니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenTombstone {
    /// 삭제된 토큰.
    pub token: TokenRef,
    /// 정의되어 있던 scope.
    pub scope: TokenScope,
    /// 마지막 이름.
    pub name: TokenName,
    /// 마지막 유효 타입. 참조하는 표현식은 계속 이 타입으로 type 검사됩니다.
    pub token_type: TokenType,
}

/// scope별 TokenSet registry (VIII.5.2).
///
/// 토큰마다 값 revision(VIII.5.4)을 둡니다. 정의 추가와 default expression 교체는 그 토큰의 revision을 새 값으로
/// 올리고, rename은 값에 영향이 없으므로 올리지 않습니다. revision은 registry 안에서 단조 증가하며 저장하지 않습니다.
/// 동등성(`PartialEq`)은 revision과 무관하게 scope별 TokenSet 내용과 orphan 기록(tombstone)만 비교합니다.
#[derive(Debug, Clone, Default)]
pub struct TokenRegistry {
    sets: BTreeMap<TokenScope, TokenSet>,
    owners: BTreeMap<TokenRef, TokenScope>,
    tombstones: BTreeMap<TokenRef, TokenTombstone>,
    revisions: BTreeMap<TokenRef, u64>,
    last_revision: u64,
}

impl PartialEq for TokenRegistry {
    fn eq(&self, other: &TokenRegistry) -> bool {
        self.sets == other.sets && self.tombstones == other.tombstones
    }
}

impl Eq for TokenRegistry {}

impl TokenRegistry {
    /// 빈 registry.
    pub fn new() -> TokenRegistry {
        TokenRegistry::default()
    }

    /// scope에 TokenSet을 등록합니다. 같은 scope가 있거나, set의 토큰 ID가 다른 scope에 있으면 거부합니다.
    pub fn add_scope(&mut self, scope: TokenScope, set: TokenSet) -> Result<(), RegistryError> {
        if self.sets.contains_key(&scope) {
            return Err(RegistryError::DuplicateScope(scope));
        }
        for definition in set.iter() {
            if let Some(existing) = self.owners.get(&definition.id()) {
                return Err(RegistryError::IdInOtherScope {
                    token: definition.id(),
                    existing: *existing,
                });
            }
        }
        for definition in set.iter() {
            self.owners.insert(definition.id(), scope);
            self.bump(definition.id());
        }
        self.sets.insert(scope, set);
        Ok(())
    }

    /// 토큰 값 revision을 새 값으로 올립니다.
    fn bump(&mut self, token: TokenRef) {
        self.last_revision += 1;
        self.revisions.insert(token, self.last_revision);
    }

    /// 토큰 값 revision(VIII.5.4). 정의 추가·default expression 교체 때 바뀌고 rename에는 그대로입니다.
    pub fn value_revision(&self, token: TokenRef) -> Option<u64> {
        self.revisions.get(&token).copied()
    }

    /// scope를 제거하고 TokenSet을 돌려줍니다.
    pub fn remove_scope(&mut self, scope: TokenScope) -> Option<TokenSet> {
        let set = self.sets.remove(&scope)?;
        for definition in set.iter() {
            self.owners.remove(&definition.id());
            self.revisions.remove(&definition.id());
        }
        Some(set)
    }

    /// 등록된 scope(정렬).
    pub fn scopes(&self) -> impl Iterator<Item = TokenScope> + '_ {
        self.sets.keys().copied()
    }

    /// scope의 TokenSet.
    pub fn set(&self, scope: TokenScope) -> Option<&TokenSet> {
        self.sets.get(&scope)
    }

    /// 토큰이 정의된 scope.
    pub fn scope_of(&self, token: TokenRef) -> Option<TokenScope> {
        self.owners.get(&token).copied()
    }

    /// ID로 토큰을 해석합니다(이름·shadowing과 무관).
    pub fn resolve(&self, token: TokenRef) -> Option<ResolvedToken<'_>> {
        let scope = self.scope_of(token)?;
        let definition = self.sets.get(&scope)?.get(token)?;
        Some(ResolvedToken { scope, definition })
    }

    /// 모든 토큰(scope 순, scope 안에서는 ID 순).
    pub fn tokens(&self) -> impl Iterator<Item = (TokenScope, &TokenDefinition)> {
        self.sets
            .iter()
            .flat_map(|(scope, set)| set.iter().map(move |definition| (*scope, definition)))
    }

    /// 토큰 수.
    pub fn len(&self) -> usize {
        self.owners.len()
    }

    /// 토큰이 없으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.owners.is_empty()
    }

    /// `from` scope에서 볼 수 있는 등록된 scope(우선순위 순, 같은 계층은 scope 순).
    pub fn visible_from(&self, from: TokenScope) -> Vec<TokenScope> {
        let mut visible = Vec::new();
        match from {
            TokenScope::Library(_) => {
                if self.sets.contains_key(&from) {
                    visible.push(from);
                }
            }
            TokenScope::Document(_) | TokenScope::Project => {
                if matches!(from, TokenScope::Document(_)) && self.sets.contains_key(&from) {
                    visible.push(from);
                }
                if self.sets.contains_key(&TokenScope::Project) {
                    visible.push(TokenScope::Project);
                }
                visible.extend(
                    self.sets
                        .keys()
                        .filter(|scope| matches!(scope, TokenScope::Library(_)))
                        .copied(),
                );
            }
        }
        visible
    }

    /// `from` scope에서 `target` scope의 토큰이 보이는지 여부.
    pub fn is_visible(&self, from: TokenScope, target: TokenScope) -> bool {
        match (from, target) {
            (TokenScope::Library(a), TokenScope::Library(b)) => a == b,
            (TokenScope::Library(_), _) => false,
            (TokenScope::Project, TokenScope::Document(_)) => false,
            (TokenScope::Project, _) => true,
            (TokenScope::Document(a), TokenScope::Document(b)) => a == b,
            (TokenScope::Document(_), _) => true,
        }
    }

    /// 이름으로 토큰을 찾습니다(shadowing 규칙은 모듈 문서 3번).
    pub fn resolve_name(&self, from: TokenScope, name: &str) -> NameResolution {
        let Ok(name) = TokenName::new(name) else {
            return NameResolution::NotFound;
        };
        let mut tier: Option<u8> = None;
        let mut found: Vec<(TokenScope, TokenRef)> = Vec::new();
        for scope in self.visible_from(from) {
            if tier.is_some_and(|tier| tier != scope.tier()) {
                break;
            }
            let hit = self
                .sets
                .get(&scope)
                .and_then(|set| set.by_name(name.as_str()));
            if let Some(definition) = hit {
                tier = Some(scope.tier());
                found.push((scope, definition.id()));
            }
        }
        match found.as_slice() {
            [] => NameResolution::NotFound,
            [(scope, token)] => NameResolution::Found {
                token: *token,
                scope: *scope,
            },
            _ => NameResolution::Ambiguous(found),
        }
    }

    /// 모든 토큰의 토큰 참조를 검사합니다(없는 대상, 보이지 않는 scope). 결과는 정렬되어 있습니다.
    pub fn reference_issues(&self) -> BTreeSet<ReferenceIssue> {
        let mut issues = BTreeSet::new();
        for (from, definition) in self.tokens() {
            for target in definition.referenced_tokens() {
                match self.scope_of(target) {
                    None if self.tombstones.contains_key(&target) => {
                        issues.insert(ReferenceIssue::Orphaned {
                            token: definition.id(),
                            target,
                        });
                    }
                    None => {
                        issues.insert(ReferenceIssue::Dangling {
                            token: definition.id(),
                            target,
                        });
                    }
                    Some(target_scope) if !self.is_visible(from, target_scope) => {
                        issues.insert(ReferenceIssue::NotVisible {
                            token: definition.id(),
                            from,
                            target,
                            target_scope,
                        });
                    }
                    Some(_) => {}
                }
            }
        }
        issues
    }

    /// scope에 토큰을 추가합니다. ID는 registry 전체에서 유일해야 합니다.
    pub fn insert(
        &mut self,
        scope: TokenScope,
        definition: TokenDefinition,
    ) -> Result<(), RegistryError> {
        if let Some(existing) = self.owners.get(&definition.id()) {
            return Err(RegistryError::IdInOtherScope {
                token: definition.id(),
                existing: *existing,
            });
        }
        let set = self
            .sets
            .get_mut(&scope)
            .ok_or(RegistryError::UnknownScope(scope))?;
        let id = definition.id();
        set.insert(definition)?;
        self.owners.insert(id, scope);
        self.bump(id);
        Ok(())
    }

    /// 토큰을 제거하고 scope와 정의를 돌려줍니다.
    pub fn remove(&mut self, token: TokenRef) -> Option<(TokenScope, TokenDefinition)> {
        let scope = self.owners.remove(&token)?;
        self.revisions.remove(&token);
        let definition = self.sets.get_mut(&scope)?.remove(token)?;
        Some((scope, definition))
    }

    /// 토큰 이름을 바꾸고 이전 이름을 돌려줍니다(같은 scope 안 이름 유일성 검사).
    pub fn rename(&mut self, token: TokenRef, name: TokenName) -> Result<TokenName, RegistryError> {
        let set = self.set_of_mut(token)?;
        Ok(set.rename(token, name)?)
    }

    /// default expression을 바꾸고 이전 표현식을 돌려줍니다.
    pub fn replace_expression(
        &mut self,
        token: TokenRef,
        expression: ExpressionAst,
    ) -> Result<ExpressionAst, RegistryError> {
        let set = self.set_of_mut(token)?;
        let previous = set.replace_expression(token, expression)?;
        self.bump(token);
        Ok(previous)
    }

    /// `token`을 default expression에서 참조하는 모든 토큰(ID 순).
    pub fn referrers(&self, token: TokenRef) -> BTreeSet<TokenRef> {
        self.sets
            .values()
            .flat_map(|set| set.referrers(token))
            .collect()
    }

    /// orphan plan으로 삭제된 토큰의 기록.
    pub fn tombstone(&self, token: TokenRef) -> Option<&TokenTombstone> {
        self.tombstones.get(&token)
    }

    /// 모든 orphan 기록(ID 순).
    pub fn tombstones(&self) -> impl Iterator<Item = &TokenTombstone> {
        self.tombstones.values()
    }

    pub(crate) fn insert_tombstone(&mut self, tombstone: TokenTombstone) {
        self.tombstones.insert(tombstone.token, tombstone);
    }

    pub(crate) fn remove_tombstone(&mut self, token: TokenRef) -> Option<TokenTombstone> {
        self.tombstones.remove(&token)
    }

    fn set_of_mut(&mut self, token: TokenRef) -> Result<&mut TokenSet, RegistryError> {
        let scope = self
            .scope_of(token)
            .ok_or(RegistryError::Set(TokenSetError::UnknownToken(token)))?;
        self.sets
            .get_mut(&scope)
            .ok_or(RegistryError::UnknownScope(scope))
    }
}

impl TokenTypes for TokenRegistry {
    /// 정의된 토큰의 타입. orphan 기록만 남은 토큰은 마지막 유효 타입입니다.
    fn token_type_of(&self, id: TokenRef) -> Option<TokenType> {
        self.resolve(id)
            .map(|resolved| resolved.definition.token_type())
            .or_else(|| {
                self.tombstones
                    .get(&id)
                    .map(|tombstone| tombstone.token_type)
            })
    }
}

#[cfg(test)]
mod tests {
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::typed_value::TypedValue;
    use stellar_expressions::ast::{ExprNode, ExprNodeId, ReferenceTarget};
    use stellar_expressions::types::ExpressionType;

    use super::*;
    use crate::token::TokenSetId;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn token(n: u64) -> TokenRef {
        TokenRef::new(sid(1_000 + n))
    }

    fn color(r: f32) -> TypedValue {
        TypedValue::Color(Color::new(ColorSpace::Srgb, r, 0.0, 0.0, 1.0).expect("유효"))
    }

    fn literal_token(n: u64, name: &str) -> TokenDefinition {
        let node = ExprNodeId::new(sid(10_000 + n));
        TokenDefinition::new(
            token(n),
            TokenName::new(name).expect("이름"),
            TokenType::new(ExpressionType::Color),
            ExpressionAst::new(node, vec![ExprNode::literal(node, color(0.1))]).expect("유효"),
        )
    }

    fn alias_token(n: u64, name: &str, target: TokenRef) -> TokenDefinition {
        let node = ExprNodeId::new(sid(10_000 + n));
        TokenDefinition::new(
            token(n),
            TokenName::new(name).expect("이름"),
            TokenType::new(ExpressionType::Color),
            ExpressionAst::new(
                node,
                vec![ExprNode::reference(node, ReferenceTarget::Token(target))],
            )
            .expect("유효"),
        )
    }

    fn set(n: u64, tokens: Vec<TokenDefinition>) -> TokenSet {
        let mut set = TokenSet::new(TokenSetId::new(sid(n)));
        for token in tokens {
            set.insert(token).expect("추가");
        }
        set
    }

    const LIB_L: u64 = 501;
    const LIB_M: u64 = 502;
    const DOC_D: u64 = 601;
    const DOC_E: u64 = 602;

    fn library(n: u64) -> TokenScope {
        TokenScope::Library(sid(n))
    }

    fn document(n: u64) -> TokenScope {
        TokenScope::Document(sid(n))
    }

    /// 같은 이름 `color.primary`가 프로젝트·라이브러리 L·문서 D에, `color.accent`가 라이브러리 L·M에 있습니다.
    fn fixture() -> TokenRegistry {
        let mut registry = TokenRegistry::new();
        registry
            .add_scope(
                library(LIB_L),
                set(
                    1,
                    vec![
                        literal_token(1, "color.primary"),
                        literal_token(2, "color.accent"),
                    ],
                ),
            )
            .expect("L");
        registry
            .add_scope(
                library(LIB_M),
                set(2, vec![literal_token(3, "color.accent")]),
            )
            .expect("M");
        registry
            .add_scope(
                TokenScope::Project,
                set(3, vec![alias_token(4, "color.primary", token(1))]),
            )
            .expect("project");
        registry
            .add_scope(
                document(DOC_D),
                set(4, vec![alias_token(5, "color.primary", token(4))]),
            )
            .expect("D");
        registry
            .add_scope(document(DOC_E), set(5, vec![literal_token(6, "only.e")]))
            .expect("E");
        registry
    }

    #[test]
    fn scope_shadowing_resolves_names_by_tier() {
        let registry = fixture();
        let found = |token, scope| NameResolution::Found { token, scope };
        // 문서 D는 프로젝트·라이브러리 이름을 가립니다.
        assert_eq!(
            registry.resolve_name(document(DOC_D), "color.primary"),
            found(token(5), document(DOC_D))
        );
        // 다른 문서 E는 D를 보지 못하고 프로젝트 토큰을 봅니다.
        assert_eq!(
            registry.resolve_name(document(DOC_E), "color.primary"),
            found(token(4), TokenScope::Project)
        );
        assert_eq!(
            registry.resolve_name(TokenScope::Project, "color.primary"),
            found(token(4), TokenScope::Project)
        );
        // 라이브러리는 자기 토큰만 봅니다.
        assert_eq!(
            registry.resolve_name(library(LIB_L), "color.primary"),
            found(token(1), library(LIB_L))
        );
        assert_eq!(
            registry.resolve_name(library(LIB_M), "color.primary"),
            NameResolution::NotFound
        );
        // 두 라이브러리의 같은 이름은 같은 계층이라 모호합니다.
        assert_eq!(
            registry.resolve_name(document(DOC_D), "color.accent"),
            NameResolution::Ambiguous(vec![(library(LIB_L), token(2)), (library(LIB_M), token(3))])
        );
        assert_eq!(
            registry.resolve_name(TokenScope::Project, "only.e"),
            NameResolution::NotFound
        );
        assert_eq!(
            registry.resolve_name(document(DOC_D), " bad"),
            NameResolution::NotFound
        );
    }

    #[test]
    fn id_resolution_ignores_names_and_shadowing() {
        let registry = fixture();
        // 이름이 가려진 라이브러리 토큰도 ID로는 정확히 해석됩니다.
        let resolved = registry.resolve(token(1)).expect("ID 해석");
        assert_eq!(resolved.scope, library(LIB_L));
        assert_eq!(resolved.definition.name().as_str(), "color.primary");
        assert_eq!(
            registry.resolve(token(5)).map(|r| r.scope),
            Some(document(DOC_D))
        );
        assert!(registry.resolve(token(99)).is_none());
        assert_eq!(registry.len(), 6);
        assert_eq!(
            registry.token_type_of(token(4)),
            Some(TokenType::new(ExpressionType::Color))
        );
        assert_eq!(registry.referrers(token(1)), BTreeSet::from([token(4)]));
        // 가시성: 문서 → 프로젝트 → 라이브러리 참조는 정상.
        assert!(registry.reference_issues().is_empty());
    }

    #[test]
    fn ids_are_unique_across_scopes_and_visibility_is_checked() {
        let mut registry = fixture();
        // 다른 scope에 같은 ID.
        assert_eq!(
            registry.add_scope(document(700), set(6, vec![literal_token(1, "dup")])),
            Err(RegistryError::IdInOtherScope {
                token: token(1),
                existing: library(LIB_L)
            })
        );
        assert_eq!(
            registry.insert(TokenScope::Project, literal_token(3, "dup")),
            Err(RegistryError::IdInOtherScope {
                token: token(3),
                existing: library(LIB_M)
            })
        );
        assert_eq!(
            registry.add_scope(TokenScope::Project, set(7, Vec::new())),
            Err(RegistryError::DuplicateScope(TokenScope::Project))
        );
        assert_eq!(
            registry.insert(document(999), literal_token(20, "x")),
            Err(RegistryError::UnknownScope(document(999)))
        );
        // 라이브러리가 프로젝트를, 프로젝트가 문서를 참조하면 보이지 않는 참조입니다. 없는 토큰은 dangling.
        registry
            .insert(library(LIB_M), alias_token(10, "lib.to.project", token(4)))
            .expect("구조상 추가");
        registry
            .insert(
                TokenScope::Project,
                alias_token(11, "project.to.doc", token(5)),
            )
            .expect("구조상 추가");
        registry
            .insert(document(DOC_E), alias_token(12, "e.to.d", token(5)))
            .expect("구조상 추가");
        registry
            .insert(document(DOC_E), alias_token(13, "e.to.missing", token(77)))
            .expect("구조상 추가");
        assert_eq!(
            registry.reference_issues().into_iter().collect::<Vec<_>>(),
            vec![
                ReferenceIssue::Dangling {
                    token: token(13),
                    target: token(77)
                },
                ReferenceIssue::NotVisible {
                    token: token(10),
                    from: library(LIB_M),
                    target: token(4),
                    target_scope: TokenScope::Project
                },
                ReferenceIssue::NotVisible {
                    token: token(11),
                    from: TokenScope::Project,
                    target: token(5),
                    target_scope: document(DOC_D)
                },
                ReferenceIssue::NotVisible {
                    token: token(12),
                    from: document(DOC_E),
                    target: token(5),
                    target_scope: document(DOC_D)
                },
            ]
        );
        // 제거하면 ID 색인도 정리됩니다.
        let (scope, definition) = registry.remove(token(13)).expect("제거");
        assert_eq!((scope, definition.id()), (document(DOC_E), token(13)));
        assert!(registry.scope_of(token(13)).is_none());
        let removed = registry.remove_scope(library(LIB_M)).expect("scope 제거");
        assert_eq!(removed.len(), 2);
        assert!(registry.resolve(token(3)).is_none());
        // rename은 scope 안 이름 유일성만 봅니다(다른 scope와 같은 이름 허용).
        assert_eq!(
            registry.rename(token(6), TokenName::new("color.primary").expect("이름")),
            Ok(TokenName::new("only.e").expect("이름"))
        );
    }
}
