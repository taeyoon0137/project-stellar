//! 창별 열린 탭·active view·viewport (XII.2.5).
//!
//! 창 하나는 작업 문서의 View를 탭으로 열고 그중 하나를 active view로 보여 줍니다(기술 아키텍처 §22.3: 열린
//! 문서와 탭, 창별 캔버스 위치와 확대율). 탭마다 [`CanvasViewport`](화면 중심의 문서 좌표와 배율)를 따로
//! 가집니다. 이 상태는 선택과 같이 창의 [`WindowViewState`](crate::selection::WindowViewState)에 있고
//! 문서 모델·저장 bytes·history에 들어가지 않습니다(§22.1 선택·view 상태 비저장, §23.2 history는 문서 편집과
//! 선택만).
//!
//! 규칙:
//! - 탭은 작업 문서에 있는 View만 엽니다([`ViewTabs::open`]). 같은 View를 다시 열면 그 탭을 active로 바꿀 뿐
//!   탭을 늘리지 않습니다. 새 탭은 기본 viewport([`CanvasViewport::DEFAULT`])로 엽니다.
//! - active 탭을 닫으면 오른쪽 이웃이, 없으면 왼쪽 이웃이 active가 됩니다. 마지막 탭을 닫으면 active가 없습니다.
//! - viewport는 유한 값이고 배율은 [`MIN_ZOOM`]~[`MAX_ZOOM`]입니다. 범위 밖 배율은 경계로 맞춥니다.
//! - 문서에서 View가 사라지면(재로드·외부 변경) [`ViewTabs::prune_missing`]으로 그 탭을 닫습니다.

use stellar_core_types::ids::StableId;
use stellar_document::model::DocumentModel;

/// 최소 배율(2%).
pub const MIN_ZOOM: f64 = 0.02;

/// 최대 배율(25600%).
pub const MAX_ZOOM: f64 = 256.0;

/// 캔버스 viewport: 화면 중심이 가리키는 문서 좌표(View root 좌표계)와 배율(1.0 = 100%).
///
/// 값은 항상 유한하고 배율은 [`MIN_ZOOM`]~[`MAX_ZOOM`]이므로 `Eq`가 성립합니다(NaN 없음).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasViewport {
    center_x: f64,
    center_y: f64,
    zoom: f64,
}

impl Eq for CanvasViewport {}

/// viewport 값 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportError {
    /// 좌표나 배율이 유한하지 않습니다.
    NonFinite,
    /// 배율이 0 이하입니다.
    NonPositiveZoom,
}

impl std::fmt::Display for ViewportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ViewportError::NonFinite => f.write_str("viewport 값이 유한하지 않습니다"),
            ViewportError::NonPositiveZoom => f.write_str("viewport 배율은 0보다 커야 합니다"),
        }
    }
}

impl std::error::Error for ViewportError {}

impl CanvasViewport {
    /// 원점 중심, 100%.
    pub const DEFAULT: CanvasViewport = CanvasViewport {
        center_x: 0.0,
        center_y: 0.0,
        zoom: 1.0,
    };

    /// viewport를 만듭니다. 값이 유한해야 하고 배율은 0보다 커야 하며, [`MIN_ZOOM`]~[`MAX_ZOOM`] 밖이면 경계로
    /// 맞춥니다.
    pub fn new(center_x: f64, center_y: f64, zoom: f64) -> Result<CanvasViewport, ViewportError> {
        if !(center_x.is_finite() && center_y.is_finite() && zoom.is_finite()) {
            return Err(ViewportError::NonFinite);
        }
        if zoom <= 0.0 {
            return Err(ViewportError::NonPositiveZoom);
        }
        Ok(CanvasViewport {
            // -0.0을 0.0으로 정규화해 같은 위치가 같은 값이 되게 합니다.
            center_x: center_x + 0.0,
            center_y: center_y + 0.0,
            zoom: zoom.clamp(MIN_ZOOM, MAX_ZOOM),
        })
    }

    /// 화면 중심 x(문서 좌표).
    pub const fn center_x(&self) -> f64 {
        self.center_x
    }

    /// 화면 중심 y(문서 좌표).
    pub const fn center_y(&self) -> f64 {
        self.center_y
    }

    /// 배율.
    pub const fn zoom(&self) -> f64 {
        self.zoom
    }
}

impl Default for CanvasViewport {
    fn default() -> CanvasViewport {
        CanvasViewport::DEFAULT
    }
}

/// 열린 탭 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewTab {
    /// 탭이 보여 주는 View.
    pub view: StableId,
    /// 탭의 viewport.
    pub viewport: CanvasViewport,
}

/// 탭 조작 오류. 상태는 바뀌지 않았습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewTabsError {
    /// 작업 문서에 없는 View입니다.
    UnknownView {
        /// 요청 View.
        view: StableId,
    },
    /// 그 View의 탭이 열려 있지 않습니다.
    NotOpen {
        /// 요청 View.
        view: StableId,
    },
    /// active 탭이 없습니다.
    NoActiveTab,
}

impl std::fmt::Display for ViewTabsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ViewTabsError::UnknownView { view } => write!(f, "문서에 없는 View입니다: {view}"),
            ViewTabsError::NotOpen { view } => write!(f, "열려 있지 않은 탭입니다: {view}"),
            ViewTabsError::NoActiveTab => f.write_str("active 탭이 없습니다"),
        }
    }
}

impl std::error::Error for ViewTabsError {}

/// 창의 열린 탭 목록(표시 순)과 active 탭.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewTabs {
    tabs: Vec<ViewTab>,
    active: Option<usize>,
}

/// `model`에 View `view`가 있는지.
pub fn document_has_view(model: &DocumentModel, view: StableId) -> bool {
    model
        .definition()
        .pages
        .iter()
        .any(|page| page.views.iter().any(|candidate| candidate.id == view))
}

/// 문서의 첫 View(첫 페이지의 첫 View, 표시 순). View가 없으면 `None`.
pub fn first_view(model: &DocumentModel) -> Option<StableId> {
    model
        .definition()
        .pages
        .iter()
        .flat_map(|page| page.views.iter())
        .map(|view| view.id)
        .next()
}

impl ViewTabs {
    /// 빈 탭 목록.
    pub fn new() -> ViewTabs {
        ViewTabs::default()
    }

    /// 열린 탭(표시 순).
    pub fn tabs(&self) -> &[ViewTab] {
        &self.tabs
    }

    /// 열린 탭 수.
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    /// 열린 탭이 없는지.
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// active 탭.
    pub fn active(&self) -> Option<&ViewTab> {
        self.active.and_then(|index| self.tabs.get(index))
    }

    /// active view.
    pub fn active_view(&self) -> Option<StableId> {
        self.active().map(|tab| tab.view)
    }

    /// active 탭의 viewport.
    pub fn active_viewport(&self) -> Option<CanvasViewport> {
        self.active().map(|tab| tab.viewport)
    }

    fn index_of(&self, view: StableId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.view == view)
    }

    /// View 탭을 열고 active로 만듭니다. 이미 열려 있으면 active만 바꾸고 `false`, 새로 열었으면 `true`.
    pub fn open(&mut self, model: &DocumentModel, view: StableId) -> Result<bool, ViewTabsError> {
        if let Some(index) = self.index_of(view) {
            self.active = Some(index);
            return Ok(false);
        }
        if !document_has_view(model, view) {
            return Err(ViewTabsError::UnknownView { view });
        }
        self.tabs.push(ViewTab {
            view,
            viewport: CanvasViewport::DEFAULT,
        });
        self.active = Some(self.tabs.len() - 1);
        Ok(true)
    }

    /// 열린 View 탭을 active로 만듭니다.
    pub fn activate(&mut self, view: StableId) -> Result<(), ViewTabsError> {
        let index = self.index_of(view).ok_or(ViewTabsError::NotOpen { view })?;
        self.active = Some(index);
        Ok(())
    }

    /// View 탭을 닫습니다. active 탭이면 오른쪽(없으면 왼쪽) 이웃이 active가 됩니다.
    pub fn close(&mut self, view: StableId) -> Result<(), ViewTabsError> {
        let index = self.index_of(view).ok_or(ViewTabsError::NotOpen { view })?;
        self.remove_at(index);
        Ok(())
    }

    fn remove_at(&mut self, index: usize) {
        self.tabs.remove(index);
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(active) if active == index => Some(index.min(self.tabs.len() - 1)),
            Some(active) if active > index => Some(active - 1),
            other => other,
        };
    }

    /// active 탭의 viewport를 바꾸고 이전 값을 돌려줍니다.
    pub fn set_active_viewport(
        &mut self,
        viewport: CanvasViewport,
    ) -> Result<CanvasViewport, ViewTabsError> {
        let index = self.active.ok_or(ViewTabsError::NoActiveTab)?;
        Ok(std::mem::replace(&mut self.tabs[index].viewport, viewport))
    }

    /// 문서에 더 없는 View의 탭을 닫습니다. 닫은 View(표시 순)를 돌려줍니다.
    pub fn prune_missing(&mut self, model: &DocumentModel) -> Vec<StableId> {
        let mut removed = Vec::new();
        let mut index = 0;
        while index < self.tabs.len() {
            let view = self.tabs[index].view;
            if document_has_view(model, view) {
                index += 1;
            } else {
                removed.push(view);
                self.remove_at(index);
            }
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    /// 페이지 2개: 페이지 10에 View 1·2, 페이지 20에 View 3.
    fn model() -> DocumentModel {
        let mut definition = DocumentDefinition::new(sid(100), String::new());
        let mut first = PageDefinition::new(sid(10), String::new());
        first
            .add_view(ViewDefinition::new(sid(1), String::new(), sid(1_001)))
            .unwrap();
        first
            .add_view(ViewDefinition::new(sid(2), String::new(), sid(1_002)))
            .unwrap();
        let mut second = PageDefinition::new(sid(20), String::new());
        second
            .add_view(ViewDefinition::new(sid(3), String::new(), sid(1_003)))
            .unwrap();
        definition.add_page(first).unwrap();
        definition.add_page(second).unwrap();
        DocumentModel::new(definition)
    }

    #[test]
    fn viewport_values_are_finite_and_zoom_is_clamped() {
        let viewport = CanvasViewport::new(12.5, -3.0, 2.0).unwrap();
        assert_eq!(
            (viewport.center_x(), viewport.center_y(), viewport.zoom()),
            (12.5, -3.0, 2.0)
        );
        assert_eq!(CanvasViewport::new(0.0, 0.0, 1e9).unwrap().zoom(), MAX_ZOOM);
        assert_eq!(
            CanvasViewport::new(0.0, 0.0, 1e-9).unwrap().zoom(),
            MIN_ZOOM
        );
        assert_eq!(
            CanvasViewport::new(f64::NAN, 0.0, 1.0),
            Err(ViewportError::NonFinite)
        );
        assert_eq!(
            CanvasViewport::new(0.0, f64::INFINITY, 1.0),
            Err(ViewportError::NonFinite)
        );
        assert_eq!(
            CanvasViewport::new(0.0, 0.0, 0.0),
            Err(ViewportError::NonPositiveZoom)
        );
        assert_eq!(
            CanvasViewport::new(-0.0, -0.0, 1.0).unwrap(),
            CanvasViewport::DEFAULT
        );
        assert!(CanvasViewport::new(-0.0, 0.0, 1.0)
            .unwrap()
            .center_x()
            .is_sign_positive());
    }

    #[test]
    fn opening_tabs_activates_and_never_duplicates() {
        let model = model();
        let mut tabs = ViewTabs::new();
        assert_eq!(first_view(&model), Some(sid(1)));
        assert_eq!(tabs.open(&model, sid(1)), Ok(true));
        assert_eq!(tabs.open(&model, sid(3)), Ok(true));
        assert_eq!(tabs.active_view(), Some(sid(3)));
        assert_eq!(tabs.open(&model, sid(1)), Ok(false), "이미 열린 탭");
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.active_view(), Some(sid(1)));
        assert_eq!(
            tabs.open(&model, sid(99)),
            Err(ViewTabsError::UnknownView { view: sid(99) })
        );
        assert_eq!(tabs.len(), 2, "실패는 상태를 바꾸지 않음");
        assert_eq!(tabs.active_viewport(), Some(CanvasViewport::DEFAULT));
    }

    #[test]
    fn each_tab_keeps_its_own_viewport() {
        let model = model();
        let mut tabs = ViewTabs::new();
        assert_eq!(
            tabs.set_active_viewport(CanvasViewport::DEFAULT),
            Err(ViewTabsError::NoActiveTab)
        );
        tabs.open(&model, sid(1)).unwrap();
        let zoomed = CanvasViewport::new(100.0, 50.0, 4.0).unwrap();
        assert_eq!(
            tabs.set_active_viewport(zoomed),
            Ok(CanvasViewport::DEFAULT)
        );
        tabs.open(&model, sid(2)).unwrap();
        assert_eq!(tabs.active_viewport(), Some(CanvasViewport::DEFAULT));
        tabs.activate(sid(1)).unwrap();
        assert_eq!(tabs.active_viewport(), Some(zoomed));
        assert_eq!(
            tabs.activate(sid(3)),
            Err(ViewTabsError::NotOpen { view: sid(3) })
        );
    }

    #[test]
    fn closing_moves_active_to_a_neighbor() {
        let model = model();
        let mut tabs = ViewTabs::new();
        for view in [1, 2, 3] {
            tabs.open(&model, sid(view)).unwrap();
        }
        tabs.activate(sid(2)).unwrap();
        tabs.close(sid(2)).unwrap();
        assert_eq!(tabs.active_view(), Some(sid(3)), "오른쪽 이웃");
        tabs.close(sid(3)).unwrap();
        assert_eq!(tabs.active_view(), Some(sid(1)), "오른쪽이 없으면 왼쪽");
        tabs.open(&model, sid(2)).unwrap();
        tabs.close(sid(1)).unwrap();
        assert_eq!(
            tabs.active_view(),
            Some(sid(2)),
            "active 앞 탭을 닫아도 active 유지"
        );
        tabs.close(sid(2)).unwrap();
        assert!(tabs.is_empty());
        assert_eq!(tabs.active(), None);
        assert_eq!(
            tabs.close(sid(2)),
            Err(ViewTabsError::NotOpen { view: sid(2) })
        );
    }

    #[test]
    fn tabs_of_removed_views_are_pruned() {
        let full = model();
        let mut tabs = ViewTabs::new();
        for view in [1, 2, 3] {
            tabs.open(&full, sid(view)).unwrap();
        }
        tabs.activate(sid(2)).unwrap();
        // View 2가 없는 문서.
        let mut definition = DocumentDefinition::new(sid(100), String::new());
        let mut page = PageDefinition::new(sid(10), String::new());
        page.add_view(ViewDefinition::new(sid(1), String::new(), sid(1_001)))
            .unwrap();
        page.add_view(ViewDefinition::new(sid(3), String::new(), sid(1_003)))
            .unwrap();
        definition.add_page(page).unwrap();
        let reduced = DocumentModel::new(definition);
        assert_eq!(tabs.prune_missing(&reduced), vec![sid(2)]);
        assert_eq!(
            tabs.tabs().iter().map(|tab| tab.view).collect::<Vec<_>>(),
            [sid(1), sid(3)]
        );
        assert_eq!(tabs.active_view(), Some(sid(3)));
        assert!(tabs.prune_missing(&reduced).is_empty());
    }
}
