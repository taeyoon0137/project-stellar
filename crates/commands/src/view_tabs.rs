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
//!
//! # camera (XII.4.3)
//!
//! viewport는 scene camera의 입력입니다. [`CanvasViewport::world_to_view`]는 화면(view, 캔버스 view 좌표: 좌상단 원점,
//! y 아래로 증가, point 단위) 크기를 받아 문서 좌표 → 화면 좌표 아핀 변환을 만듭니다:
//! `view = (world − center) × zoom + size / 2`. 이동([`CanvasViewport::panned`])과 확대([`CanvasViewport::zoomed`])는 이
//! 값만 바꾸고 문서 geometry를 바꾸지 않습니다. 메뉴 확대·축소 단계는 ×2·÷2([`ZOOM_STEP`])이고 화면 중심을 고정합니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::Transform2D;
use stellar_document::model::DocumentModel;

/// 최소 배율(2%).
pub const MIN_ZOOM: f64 = 0.02;

/// 최대 배율(25600%).
pub const MAX_ZOOM: f64 = 256.0;

/// 메뉴 확대·축소 한 단계 배율(Zoom In ×2, Zoom Out ÷2).
pub const ZOOM_STEP: f64 = 2.0;

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

impl CanvasViewport {
    /// 화면에서 `(dx, dy)` point만큼 내용을 옮긴 viewport(내용이 오른쪽·아래로 가면 중심은 왼쪽·위 문서 좌표로).
    pub fn panned(&self, dx: f64, dy: f64) -> Result<CanvasViewport, ViewportError> {
        CanvasViewport::new(
            self.center_x - dx / self.zoom,
            self.center_y - dy / self.zoom,
            self.zoom,
        )
    }

    /// 배율을 `factor`배 한 viewport. 화면 중심에서 `(anchor_x, anchor_y)` point 떨어진 화면 점이 가리키는 문서 점을
    /// 고정합니다(메뉴 명령은 `(0, 0)` = 화면 중심). 결과 배율은 [`MIN_ZOOM`]~[`MAX_ZOOM`]으로 맞춥니다.
    pub fn zoomed(
        &self,
        factor: f64,
        anchor_x: f64,
        anchor_y: f64,
    ) -> Result<CanvasViewport, ViewportError> {
        if !(factor.is_finite() && anchor_x.is_finite() && anchor_y.is_finite()) {
            return Err(ViewportError::NonFinite);
        }
        if factor <= 0.0 {
            return Err(ViewportError::NonPositiveZoom);
        }
        let zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        // anchor 화면 점의 문서 좌표: center + anchor / zoom. 확대 뒤에도 같은 문서 점이 같은 화면 점에 오게 합니다.
        let anchor_world_x = self.center_x + anchor_x / self.zoom;
        let anchor_world_y = self.center_y + anchor_y / self.zoom;
        CanvasViewport::new(
            anchor_world_x - anchor_x / zoom,
            anchor_world_y - anchor_y / zoom,
            zoom,
        )
    }

    /// 배율만 `zoom`으로 바꾼 viewport(화면 중심 고정).
    pub fn with_zoom(&self, zoom: f64) -> Result<CanvasViewport, ViewportError> {
        CanvasViewport::new(self.center_x, self.center_y, zoom)
    }

    /// 화면 크기 `width × height` point의 문서 좌표 → 화면 좌표 변환(scene camera 입력). 결과가 f32 범위를 넘으면
    /// `None`입니다.
    pub fn world_to_view(&self, width: f32, height: f32) -> Option<Transform2D> {
        let zoom = self.zoom as f32;
        let tx = f64::from(width) / 2.0 - self.center_x * self.zoom;
        let ty = f64::from(height) / 2.0 - self.center_y * self.zoom;
        Transform2D::new(zoom, 0.0, 0.0, zoom, tx as f32, ty as f32).ok()
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
    fn pan_and_zoom_move_the_camera_only() {
        let viewport = CanvasViewport::new(100.0, 50.0, 2.0).unwrap();
        // 화면 800×600의 중심(400, 300)이 문서 (100, 50)을 가리킵니다.
        let camera = viewport.world_to_view(800.0, 600.0).unwrap();
        assert_eq!(camera.components(), (2.0, 0.0, 0.0, 2.0, 200.0, 200.0));
        let center = camera
            .apply(stellar_core_types::value::Vec2::new(100.0, 50.0).unwrap())
            .unwrap();
        assert_eq!((center.x(), center.y()), (400.0, 300.0));

        // 내용을 화면 오른쪽으로 40pt 옮기면 중심 문서 좌표는 40 / 2 = 20만큼 왼쪽.
        let panned = viewport.panned(40.0, -10.0).unwrap();
        assert_eq!(
            (panned.center_x(), panned.center_y(), panned.zoom()),
            (80.0, 55.0, 2.0)
        );

        // 화면 중심 고정 확대·축소.
        let zoomed = viewport.zoomed(ZOOM_STEP, 0.0, 0.0).unwrap();
        assert_eq!(
            (zoomed.center_x(), zoomed.center_y(), zoomed.zoom()),
            (100.0, 50.0, 4.0)
        );
        assert_eq!(zoomed.zoomed(1.0 / ZOOM_STEP, 0.0, 0.0).unwrap(), viewport);
        // anchor 고정: 중심에서 (+100, 0)pt인 점이 가리키는 문서 x = 100 + 100 / 2 = 150은 확대 뒤에도 같은 화면 점.
        let anchored = viewport.zoomed(2.0, 100.0, 0.0).unwrap();
        assert_eq!(anchored.zoom(), 4.0);
        assert_eq!(anchored.center_x() + 100.0 / anchored.zoom(), 150.0);
        // 경계와 오류.
        assert_eq!(viewport.zoomed(1e9, 0.0, 0.0).unwrap().zoom(), MAX_ZOOM);
        assert_eq!(
            viewport.zoomed(0.0, 0.0, 0.0),
            Err(ViewportError::NonPositiveZoom)
        );
        assert_eq!(
            viewport.zoomed(f64::NAN, 0.0, 0.0),
            Err(ViewportError::NonFinite)
        );
        assert_eq!(viewport.with_zoom(1.0).unwrap().zoom(), 1.0);
        // f32 범위를 넘는 camera는 만들지 않습니다.
        let far = CanvasViewport::new(1e300, 0.0, 1.0).unwrap();
        assert_eq!(far.world_to_view(800.0, 600.0), None);
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
