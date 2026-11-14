//! 캔버스 ViewPresentation 런타임 타입 (IX.7.9).
//!
//! 캔버스에는 같은 View를 여러 변수 할당(예: compact+error, regular+idle)으로 나란히 배치할 수 있습니다.
//! [`ViewPresentation`]은 그 배치 하나이며 **정의를 복제하지 않습니다** — View 정의(변수·Variant, IX.7.8
//! [`ViewVariantModel`])는 [`Arc`]로 공유하고, 배치는 할당([`Assignment`])과 캔버스 위치만 가집니다. resolve는
//! 공유 정의에 할당을 적용해 계산합니다(IX.7.4~IX.7.6 resolver).
//!
//! [`PresentationCanvas`]는 View마다 정의 하나를 보관하고 배치를 관리합니다. 정의가 바뀌면
//! ([`PresentationCanvas::update_definition`]) 그 View의 모든 배치가 새 정의 하나를 함께 가리킵니다.
//!
//! 런타임 전용입니다. 배치·할당은 프로젝트에 저장하지 않습니다(preview 상태는 세션 메모리, 기술 아키텍처 16장).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use stellar_core_types::ids::StableId;
use stellar_core_types::value::Vec2;

use super::commands::ViewVariantModel;
use super::condition::{Assignment, ConditionError};
use super::resolver::{resolve_variants, VariantResolution};

/// 캔버스 배치 식별자(런타임 전용, 저장하지 않음).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PresentationId(u64);

impl PresentationId {
    /// 숫자 값.
    pub const fn raw(&self) -> u64 {
        self.0
    }
}

/// 배치 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresentationError {
    /// 캔버스에 등록되지 않은 View입니다.
    UnknownView {
        /// View.
        view: StableId,
    },
    /// 배치가 없습니다.
    UnknownPresentation {
        /// 배치.
        presentation: PresentationId,
    },
    /// 할당이 View 변수에 맞지 않습니다.
    Assignment(ConditionError),
}

impl fmt::Display for PresentationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PresentationError::UnknownView { view } => {
                write!(f, "캔버스에 View {view} 정의가 없습니다")
            }
            PresentationError::UnknownPresentation { presentation } => {
                write!(f, "배치 {}가 없습니다", presentation.raw())
            }
            PresentationError::Assignment(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for PresentationError {}

impl From<ConditionError> for PresentationError {
    fn from(err: ConditionError) -> PresentationError {
        PresentationError::Assignment(err)
    }
}

/// 캔버스에 배치한 View 하나.
#[derive(Debug, Clone)]
pub struct ViewPresentation {
    id: PresentationId,
    definition: Arc<ViewVariantModel>,
    assignment: Assignment,
    origin: Vec2,
}

impl ViewPresentation {
    /// 식별자.
    pub fn id(&self) -> PresentationId {
        self.id
    }

    /// View StableId.
    pub fn view(&self) -> StableId {
        self.definition.variables().view()
    }

    /// 공유 정의.
    pub fn definition(&self) -> &Arc<ViewVariantModel> {
        &self.definition
    }

    /// 변수 할당.
    pub fn assignment(&self) -> &Assignment {
        &self.assignment
    }

    /// 캔버스 위치.
    pub fn origin(&self) -> Vec2 {
        self.origin
    }

    /// 다른 배치와 같은 정의 객체를 공유하는지(복제 없음 확인).
    pub fn shares_definition_with(&self, other: &ViewPresentation) -> bool {
        Arc::ptr_eq(&self.definition, &other.definition)
    }

    /// 할당을 정의의 Variant에 적용한 결과.
    pub fn resolve(&self) -> VariantResolution {
        resolve_variants(
            self.definition.variants(),
            &self.assignment,
            self.definition.variables(),
        )
    }
}

/// View 정의와 캔버스 배치 모음.
#[derive(Debug, Clone, Default)]
pub struct PresentationCanvas {
    definitions: BTreeMap<StableId, Arc<ViewVariantModel>>,
    presentations: BTreeMap<PresentationId, ViewPresentation>,
    next: u64,
}

impl PresentationCanvas {
    /// 빈 캔버스.
    pub fn new() -> PresentationCanvas {
        PresentationCanvas::default()
    }

    /// View 정의를 등록합니다(같은 View가 있으면 [`PresentationCanvas::update_definition`]과 같음).
    pub fn register(&mut self, definition: ViewVariantModel) -> Vec<PresentationId> {
        self.update_definition(definition)
    }

    /// View 정의를 교체하고 그 View의 모든 배치가 새 정의 하나를 공유하게 합니다. 새 정의에서 할당이 더 이상
    /// 유효하지 않은 배치 id를 돌려줍니다(할당은 그대로 두며, 없는 변수 term은 resolver 규칙대로 만족하지 않음).
    pub fn update_definition(&mut self, definition: ViewVariantModel) -> Vec<PresentationId> {
        let view = definition.variables().view();
        let shared = Arc::new(definition);
        self.definitions.insert(view, Arc::clone(&shared));
        let mut invalid = Vec::new();
        for presentation in self.presentations.values_mut() {
            if presentation.view() == view {
                presentation.definition = Arc::clone(&shared);
                if presentation
                    .assignment
                    .validate(shared.variables())
                    .is_err()
                {
                    invalid.push(presentation.id);
                }
            }
        }
        invalid
    }

    /// View 정의.
    pub fn definition(&self, view: StableId) -> Option<&Arc<ViewVariantModel>> {
        self.definitions.get(&view)
    }

    /// View를 할당 `assignment`로 `origin`에 배치합니다. 할당은 View 변수에 맞아야 합니다.
    pub fn place(
        &mut self,
        view: StableId,
        assignment: Assignment,
        origin: Vec2,
    ) -> Result<PresentationId, PresentationError> {
        let definition = self
            .definitions
            .get(&view)
            .ok_or(PresentationError::UnknownView { view })?;
        assignment.validate(definition.variables())?;
        let id = PresentationId(self.next);
        self.next += 1;
        let presentation = ViewPresentation {
            id,
            definition: Arc::clone(definition),
            assignment,
            origin,
        };
        self.presentations.insert(id, presentation);
        Ok(id)
    }

    /// 배치의 할당을 바꿉니다.
    pub fn set_assignment(
        &mut self,
        presentation: PresentationId,
        assignment: Assignment,
    ) -> Result<Assignment, PresentationError> {
        let entry = self
            .presentations
            .get_mut(&presentation)
            .ok_or(PresentationError::UnknownPresentation { presentation })?;
        assignment.validate(entry.definition.variables())?;
        Ok(std::mem::replace(&mut entry.assignment, assignment))
    }

    /// 배치를 옮깁니다.
    pub fn move_to(
        &mut self,
        presentation: PresentationId,
        origin: Vec2,
    ) -> Result<Vec2, PresentationError> {
        let entry = self
            .presentations
            .get_mut(&presentation)
            .ok_or(PresentationError::UnknownPresentation { presentation })?;
        Ok(std::mem::replace(&mut entry.origin, origin))
    }

    /// 배치를 지웁니다.
    pub fn remove(
        &mut self,
        presentation: PresentationId,
    ) -> Result<ViewPresentation, PresentationError> {
        self.presentations
            .remove(&presentation)
            .ok_or(PresentationError::UnknownPresentation { presentation })
    }

    /// 배치.
    pub fn get(&self, presentation: PresentationId) -> Option<&ViewPresentation> {
        self.presentations.get(&presentation)
    }

    /// View의 배치(id 오름차순).
    pub fn presentations_of(&self, view: StableId) -> impl Iterator<Item = &ViewPresentation> + '_ {
        self.presentations
            .values()
            .filter(move |presentation| presentation.view() == view)
    }

    /// 배치 수.
    pub fn len(&self) -> usize {
        self.presentations.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.presentations.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::super::commands::ViewVariantCommand;
    use super::super::resolver::fixtures::*;
    use super::super::variable::fixtures::*;
    use super::*;

    fn model() -> ViewVariantModel {
        let mut model = ViewVariantModel::new(stable(VIEW));
        for command in [
            ViewVariantCommand::CreateVariable(device_variable()),
            ViewVariantCommand::CreateVariable(payment_variable()),
            ViewVariantCommand::CreateVariable(dark_variable()),
            ViewVariantCommand::CreateVariant(variant(COMPACT, compact())),
            ViewVariantCommand::CreateVariant(variant(ERROR, error())),
            ViewVariantCommand::CreateVariant(variant(
                COMPACT_ERROR,
                compact().equals(vid(PAYMENT), payment(2)),
            )),
        ] {
            command.apply(&mut model).unwrap();
        }
        model
    }

    fn at(x: f32) -> Vec2 {
        Vec2::new(x, 0.0).unwrap()
    }

    #[test]
    fn hundred_presentations_share_one_definition() {
        // 검증 기준(IX.7.9): 100 presentations가 definition 1개 공유.
        let mut canvas = PresentationCanvas::new();
        canvas.register(model());
        let view = stable(VIEW);
        let mut ids = Vec::new();
        for i in 0..100u32 {
            let assignment = Assignment::new()
                .with(vid(DEVICE), device(i % 2))
                .with(vid(PAYMENT), payment(i % 4));
            ids.push(
                canvas
                    .place(view, assignment, at(i as f32 * 400.0))
                    .unwrap(),
            );
        }
        assert_eq!(canvas.len(), 100);
        let definition = canvas.definition(view).unwrap();
        // 캔버스 1 + 배치 100이 같은 정의 객체를 가리킵니다(복제 0).
        assert_eq!(Arc::strong_count(definition), 101);
        let first = canvas.get(ids[0]).unwrap();
        assert!(canvas
            .presentations_of(view)
            .all(|p| p.shares_definition_with(first) && Arc::ptr_eq(p.definition(), definition)));
        // 같은 정의라도 할당별로 다른 resolve 결과.
        let compact_error = canvas.get(ids[2]).unwrap(); // device 0, payment 2
        assert_eq!(
            compact_error.resolve().matched,
            vec![
                variant_id(COMPACT),
                variant_id(ERROR),
                variant_id(COMPACT_ERROR)
            ]
        );
        let regular_processing = canvas.get(ids[1]).unwrap(); // device 1, payment 1
        assert!(regular_processing.resolve().matched.is_empty());
        assert_eq!(regular_processing.origin(), at(400.0));
    }

    #[test]
    fn definition_update_rebinds_all_presentations_to_one_new_definition() {
        let mut canvas = PresentationCanvas::new();
        canvas.register(model());
        let view = stable(VIEW);
        let a = canvas
            .place(
                view,
                Assignment::new().with(vid(DEVICE), device(0)),
                at(0.0),
            )
            .unwrap();
        let b = canvas
            .place(
                view,
                Assignment::new().with(vid(PAYMENT), payment(2)),
                at(1.0),
            )
            .unwrap();
        // 변수 삭제 후 정의 교체: payment를 쓰는 배치 b의 할당이 무효가 됩니다.
        let mut updated = (**canvas.definition(view).unwrap()).clone();
        ViewVariantCommand::DeleteVariable(vid(PAYMENT))
            .apply(&mut updated)
            .unwrap();
        let invalid = canvas.update_definition(updated);
        assert_eq!(invalid, vec![b]);
        let definition = canvas.definition(view).unwrap();
        assert_eq!(Arc::strong_count(definition), 3);
        assert!(canvas
            .get(a)
            .unwrap()
            .shares_definition_with(canvas.get(b).unwrap()));
        // 할당·위치 변경, 잘못된 할당 거부, 삭제.
        assert!(matches!(
            canvas.set_assignment(a, Assignment::new().with(vid(PAYMENT), payment(1))),
            Err(PresentationError::Assignment(_))
        ));
        canvas
            .set_assignment(a, Assignment::new().with(vid(DEVICE), device(1)))
            .unwrap();
        assert_eq!(canvas.move_to(a, at(9.0)).unwrap(), at(0.0));
        canvas.remove(b).unwrap();
        assert_eq!(Arc::strong_count(canvas.definition(view).unwrap()), 2);
        assert!(matches!(
            canvas.remove(b),
            Err(PresentationError::UnknownPresentation { .. })
        ));
        assert!(matches!(
            canvas.place(stable(1), Assignment::new(), at(0.0)),
            Err(PresentationError::UnknownView { .. })
        ));
    }
}
