//! 내보내기 입력 고정 (XX.1.2).
//!
//! 내보내기는 시작 시점의 입력을 [`ExportCapture`]로 고정하고, 이후 단계(SVG 변환, PNG 래스터)는
//! capture만 읽습니다. 그래서 내보내기가 도는 동안 document가 편집되어도 결과에 섞이지 않습니다.
//!
//! - **scene**: [`SnapshotHandle`](`Arc<SceneSnapshot>`)을 붙잡습니다. snapshot은 불변이고, document
//!   thread가 편집하면 새 snapshot을 만들 뿐 이 snapshot을 바꾸지 않습니다(XI.1.4·XI.1.5). handle이
//!   살아 있는 동안 snapshot은 회수되지 않습니다.
//! - **font·asset revision**: [`ResourcePins`]로 capture 시점의 font catalog·asset store revision을
//!   고정합니다. asset 내용은 [`ExportCapture::encoded_image`]가 **고정 revision**으로만 조회하므로,
//!   host가 그 뒤 asset을 교체해도 내보내기는 이전 내용을 씁니다.
//! - **범위**: [`ExportSource`]를 snapshot에서 해석해 내보낼 최상위 node([`ExportRoot`])를 paint
//!   순서로 고정합니다. selection은 선택 node의 subtree이며, 선택 node의 조상이 건 clip·합성 효과는
//!   포함하지 않습니다([`ExportRoot::base_clip_depth`]).
//! - **영역**: 보이는 node의 world 그리기 영역(stroke·shadow·blur 포함, scene bounds XI.3.1)을
//!   내보내기 범위 안의 clip으로 자른 합집합입니다. 숨긴 node([`Visibility::Hidden`])와 그 subtree는
//!   제외합니다(scene builder 규칙상 숨긴 node의 자손도 숨김입니다).
//!
//! capture는 scene이 요청과 같은 View·TargetProfile에서 만든 것인지([`SceneSource`]) 확인합니다.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::revision::{DocumentRevision, SceneRevision};
use stellar_scene::bounds::Aabb;
use stellar_scene::handle::SnapshotHandle;
use stellar_scene::node::{SceneNode, SceneResourceRef, TextLayoutKey, Visibility};
use stellar_scene::snapshot::SceneSemanticHash;
use stellar_scene::visibility::clip_shape_world_bounds;
use stellar_scene::SceneSnapshot;

use crate::request::{ExportRequest, ExportSource, TargetProfileKey};
use crate::text::{
    ExportTextSource, GlyphOutline, GlyphOutlineError, GlyphOutlineKey, TextGlyphLayout,
};

/// font catalog revision(host가 발급하는 단조 번호).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontRevision(
    /// revision 번호.
    pub u64,
);

/// asset store revision(host가 발급하는 단조 번호).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetRevision(
    /// revision 번호.
    pub u64,
);

/// capture 시점에 고정한 resource revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResourcePins {
    /// font catalog revision(텍스트 outline 변환이 사용).
    pub fonts: FontRevision,
    /// asset store revision(이미지 내용 조회가 사용).
    pub assets: AssetRevision,
}

/// host가 넘기는 scene: 어느 View·TargetProfile에서 만든 snapshot인지 함께 밝힙니다.
#[derive(Debug, Clone)]
pub struct SceneSource {
    /// snapshot을 만든 ViewDefinition.
    pub view: StableId,
    /// snapshot을 평가한 target profile.
    pub target_profile: TargetProfileKey,
    /// snapshot.
    pub snapshot: SnapshotHandle,
}

/// 내보낼 최상위 node.
#[derive(Debug, Clone)]
pub struct ExportRoot {
    node: Arc<SceneNode>,
    base_clip_depth: usize,
}

impl ExportRoot {
    /// node(subtree 전체를 내보냅니다).
    pub fn node(&self) -> &Arc<SceneNode> {
        &self.node
    }

    /// 내보내기 범위 밖(선택 node의 조상)이 건 clip 개수. clip stack의 바깥쪽 이만큼은 적용하지
    /// 않습니다. View 전체 내보내기는 0입니다.
    pub const fn base_clip_depth(&self) -> usize {
        self.base_clip_depth
    }
}

/// capture 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureError {
    /// scene이 다른 View에서 만들어졌습니다.
    ViewMismatch {
        /// 요청 View.
        requested: StableId,
        /// scene View.
        scene: StableId,
    },
    /// scene이 다른 target profile로 평가되었습니다.
    ProfileMismatch {
        /// 요청 profile.
        requested: TargetProfileKey,
        /// scene profile.
        scene: TargetProfileKey,
    },
    /// 선택 node가 scene에 없습니다(정렬 순서상 첫 번째).
    SelectionNotInScene {
        /// 없는 node.
        id: StableId,
    },
    /// 보이는 내용이 없거나 영역의 넓이가 0입니다.
    NothingVisible,
    /// node world 영역 계산이 유한 범위를 넘었습니다.
    NonFiniteBounds {
        /// 문제 node.
        node: StableId,
    },
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CaptureError::ViewMismatch { requested, scene } => {
                write!(f, "요청 View {requested}와 scene View {scene}가 다릅니다")
            }
            CaptureError::ProfileMismatch { requested, scene } => write!(
                f,
                "요청 target profile {requested}와 scene target profile {scene}가 다릅니다"
            ),
            CaptureError::SelectionNotInScene { id } => {
                write!(f, "선택 node {id}가 scene에 없습니다")
            }
            CaptureError::NothingVisible => f.write_str("내보낼 보이는 내용이 없습니다"),
            CaptureError::NonFiniteBounds { node } => {
                write!(f, "node {node}의 영역이 유한 범위를 넘었습니다")
            }
        }
    }
}

impl std::error::Error for CaptureError {}

/// 이미지 encoded 형식(SVG data URI에 넣을 수 있는 형식).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageMedia {
    /// PNG.
    Png,
    /// JPEG.
    Jpeg,
    /// GIF.
    Gif,
    /// WebP.
    Webp,
}

impl ImageMedia {
    /// MIME type.
    pub const fn mime(self) -> &'static str {
        match self {
            ImageMedia::Png => "image/png",
            ImageMedia::Jpeg => "image/jpeg",
            ImageMedia::Gif => "image/gif",
            ImageMedia::Webp => "image/webp",
        }
    }
}

/// encoded image 생성 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodedImageError {
    /// bytes가 비어 있습니다.
    Empty,
    /// pixel 크기가 0입니다.
    ZeroSize,
}

impl fmt::Display for EncodedImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            EncodedImageError::Empty => "이미지 bytes가 비어 있습니다",
            EncodedImageError::ZeroSize => "이미지 pixel 크기가 0입니다",
        })
    }
}

impl std::error::Error for EncodedImageError {}

/// 고정 revision의 asset 내용(encoded bytes와 원본 pixel 크기).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedImage {
    media: ImageMedia,
    bytes: Arc<[u8]>,
    pixel_width: u32,
    pixel_height: u32,
}

impl EncodedImage {
    /// 비어 있지 않은 bytes와 0이 아닌 pixel 크기로 만듭니다. bytes 내용은 검사하지 않습니다.
    pub fn new(
        media: ImageMedia,
        bytes: Arc<[u8]>,
        pixel_width: u32,
        pixel_height: u32,
    ) -> Result<EncodedImage, EncodedImageError> {
        if bytes.is_empty() {
            return Err(EncodedImageError::Empty);
        }
        if pixel_width == 0 || pixel_height == 0 {
            return Err(EncodedImageError::ZeroSize);
        }
        Ok(EncodedImage {
            media,
            bytes,
            pixel_width,
            pixel_height,
        })
    }

    /// 형식.
    pub const fn media(&self) -> ImageMedia {
        self.media
    }

    /// encoded bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 원본 pixel 너비.
    pub const fn pixel_width(&self) -> u32 {
        self.pixel_width
    }

    /// 원본 pixel 높이.
    pub const fn pixel_height(&self) -> u32 {
        self.pixel_height
    }
}

/// asset 내용 조회. host(asset store)가 구현합니다.
///
/// `revision`은 capture가 고정한 값이며, 구현은 그 revision의 내용을 돌려주거나 없으면 `None`을
/// 돌려줘야 합니다(현재 revision으로 대체하지 않습니다).
pub trait ExportAssetSource {
    /// `revision` 시점의 `asset` 내용.
    fn encoded_image(&self, asset: AssetRef, revision: AssetRevision) -> Option<EncodedImage>;
}

/// 고정된 내보내기 입력.
#[derive(Debug, Clone)]
pub struct ExportCapture {
    request: ExportRequest,
    snapshot: SnapshotHandle,
    pins: ResourcePins,
    roots: Vec<ExportRoot>,
    resources: BTreeSet<SceneResourceRef>,
    bounds: Aabb,
    visible_nodes: u64,
    content_hash: SceneSemanticHash,
}

impl ExportCapture {
    /// 요청을 `scene`에서 해석해 입력을 고정합니다.
    ///
    /// scene의 View·target profile이 요청과 다르거나, 선택 node가 scene에 없거나, 보이는 내용이
    /// 없으면 거부합니다. tree 순회는 명시적 stack을 써서 깊이와 무관하게 호출 stack을 쓰지 않습니다.
    pub fn new(
        request: ExportRequest,
        scene: SceneSource,
        pins: ResourcePins,
    ) -> Result<ExportCapture, CaptureError> {
        let requested_view = request.source().view_id();
        if scene.view != requested_view {
            return Err(CaptureError::ViewMismatch {
                requested: requested_view,
                scene: scene.view,
            });
        }
        if &scene.target_profile != request.target_profile() {
            return Err(CaptureError::ProfileMismatch {
                requested: request.target_profile().clone(),
                scene: scene.target_profile,
            });
        }
        let snapshot = scene.snapshot;
        let roots = match request.source() {
            ExportSource::View { .. } => snapshot
                .roots()
                .map(|node| ExportRoot {
                    node: Arc::clone(node),
                    base_clip_depth: 0,
                })
                .collect(),
            ExportSource::Selection { nodes, .. } => resolve_selection(&snapshot, nodes)?,
        };
        let mut resources = BTreeSet::new();
        let mut bounds: Option<Aabb> = None;
        let mut visible_nodes = 0u64;
        for root in &roots {
            let mut stack: Vec<&SceneNode> = vec![&root.node];
            while let Some(node) = stack.pop() {
                if node.hit().visibility == Visibility::Hidden {
                    continue;
                }
                visible_nodes += 1;
                if let Some(resource) = node.content().resource() {
                    resources.insert(resource);
                }
                if let Some(area) = node_area(node, root.base_clip_depth)? {
                    bounds = Some(bounds.map_or(area, |acc| acc.union(&area)));
                }
                stack.extend(node.children().map(|child| &**child));
            }
        }
        let bounds = bounds
            .filter(|b| b.width() > 0.0 && b.height() > 0.0)
            .ok_or(CaptureError::NothingVisible)?;
        let content_hash = snapshot.semantic_hash();
        Ok(ExportCapture {
            request,
            snapshot,
            pins,
            roots,
            resources,
            bounds,
            visible_nodes,
            content_hash,
        })
    }

    /// 요청.
    pub const fn request(&self) -> &ExportRequest {
        &self.request
    }

    /// 고정한 snapshot.
    pub fn snapshot(&self) -> &SceneSnapshot {
        self.snapshot.snapshot()
    }

    /// 고정한 snapshot handle.
    pub const fn handle(&self) -> &SnapshotHandle {
        &self.snapshot
    }

    /// 고정한 resource revision.
    pub const fn pins(&self) -> ResourcePins {
        self.pins
    }

    /// 내보낼 최상위 node(paint 순서).
    pub fn roots(&self) -> &[ExportRoot] {
        &self.roots
    }

    /// 보이는 내보내기 node가 참조하는 resource(정렬, 중복 제거).
    pub const fn resources(&self) -> &BTreeSet<SceneResourceRef> {
        &self.resources
    }

    /// 내보내기 영역(world 좌표).
    pub const fn bounds(&self) -> Aabb {
        self.bounds
    }

    /// 보이는 내보내기 node 수.
    pub const fn visible_nodes(&self) -> u64 {
        self.visible_nodes
    }

    /// capture 시점 snapshot의 semantic hash(revision 제외 내용 hash).
    pub const fn content_hash(&self) -> SceneSemanticHash {
        self.content_hash
    }

    /// snapshot을 만든 document revision.
    pub fn document_revision(&self) -> DocumentRevision {
        self.snapshot.document_revision()
    }

    /// snapshot의 scene revision.
    pub fn scene_revision(&self) -> SceneRevision {
        self.snapshot.scene_revision()
    }

    /// `asset` 내용을 **고정한 asset revision**으로 조회합니다.
    pub fn encoded_image(
        &self,
        source: &dyn ExportAssetSource,
        asset: AssetRef,
    ) -> Option<EncodedImage> {
        source.encoded_image(asset, self.pins.assets)
    }

    /// text layout glyph run을 **고정한 font revision**으로 조회합니다(XX.3.2).
    pub fn glyph_layout(
        &self,
        source: &dyn ExportTextSource,
        layout: TextLayoutKey,
    ) -> Option<Arc<TextGlyphLayout>> {
        source.glyph_layout(layout, self.pins.fonts)
    }

    /// glyph outline을 **고정한 font revision**으로 조회합니다(XX.3.2).
    pub fn glyph_outline(
        &self,
        source: &dyn ExportTextSource,
        key: &GlyphOutlineKey,
    ) -> Result<GlyphOutline, GlyphOutlineError> {
        source.glyph_outline(key, self.pins.fonts)
    }
}

/// selection을 snapshot에서 해석합니다. 선택 node의 자손이 함께 선택되어도 최상위 하나만 남깁니다.
fn resolve_selection(
    snapshot: &SceneSnapshot,
    nodes: &BTreeSet<StableId>,
) -> Result<Vec<ExportRoot>, CaptureError> {
    let mut found: BTreeSet<StableId> = BTreeSet::new();
    let mut roots = Vec::new();
    // (node, 선택된 조상 아래인지). 자식을 역순으로 쌓아 paint 순서(pre-order)로 꺼냅니다.
    let mut stack: Vec<(&Arc<SceneNode>, bool)> = Vec::new();
    push_reversed(&mut stack, snapshot.roots(), false);
    while let Some((node, covered)) = stack.pop() {
        let selected = nodes.contains(&node.source());
        if selected {
            found.insert(node.source());
            if !covered {
                roots.push(ExportRoot {
                    node: Arc::clone(node),
                    base_clip_depth: node.clip().depth(),
                });
            }
        }
        push_reversed(&mut stack, node.children(), covered || selected);
    }
    match nodes.difference(&found).next() {
        Some(&id) => Err(CaptureError::SelectionNotInScene { id }),
        None => Ok(roots),
    }
}

fn push_reversed<'a>(
    stack: &mut Vec<(&'a Arc<SceneNode>, bool)>,
    items: impl Iterator<Item = &'a Arc<SceneNode>>,
    covered: bool,
) {
    let start = stack.len();
    stack.extend(items.map(|node| (node, covered)));
    stack[start..].reverse();
}

/// node의 world 그리기 영역을 내보내기 범위 안 clip(바깥쪽 `base`개 제외)으로 자른 값.
///
/// clip으로 완전히 가려지면 `None`입니다.
fn node_area(node: &SceneNode, base: usize) -> Result<Option<Aabb>, CaptureError> {
    let area = node
        .world_render_bounds()
        .ok_or(CaptureError::NonFiniteBounds {
            node: node.source(),
        })?;
    let in_scope = node.clip().depth().saturating_sub(base);
    let mut clipped = area;
    for shape in node.clip().iter_inner_first().take(in_scope) {
        let Some(clip) = clip_shape_world_bounds(shape) else {
            return Ok(None);
        };
        match clipped.intersection(&clip) {
            Some(inter) => clipped = inter,
            None => return Ok(None),
        }
    }
    Ok(Some(clipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{mpsc, Mutex};
    use std::thread;

    use stellar_core_types::revision::DocumentRevision;
    use stellar_scene::node::{HitTestMeta, PointerEvents};
    use stellar_scene::revision::SceneRevisionMap;
    use stellar_scene::stack::{PaintLayer, PaintStack, PaintStackRef};

    use crate::request::{ExportFormat, ExportSource};
    use crate::test_fixtures::{self as fx, Basic};

    const PINS: ResourcePins = ResourcePins {
        fonts: FontRevision(7),
        assets: AssetRevision(1),
    };

    fn scene(snapshot: SceneSnapshot) -> SceneSource {
        SceneSource {
            view: fx::view_id(),
            target_profile: fx::profile(),
            snapshot: SnapshotHandle::new(snapshot),
        }
    }

    fn capture_view(snapshot: SceneSnapshot) -> ExportCapture {
        ExportCapture::new(fx::view_request(ExportFormat::Svg), scene(snapshot), PINS).unwrap()
    }

    /// capture의 최상위 node만으로 다시 만든 snapshot의 hash(내보내기 단계가 읽는 내용).
    fn roots_hash(capture: &ExportCapture) -> SceneSemanticHash {
        let roots = capture
            .roots()
            .iter()
            .map(|r| Arc::clone(r.node()))
            .collect();
        SceneSnapshot::new(DocumentRevision(0), SceneRevision(0), roots)
            .unwrap()
            .semantic_hash()
    }

    fn bounds_tuple(b: Aabb) -> (f32, f32, f32, f32) {
        (b.min_x(), b.min_y(), b.max_x(), b.max_y())
    }

    /// `path` 위치 leaf의 fill을 바꾼 편집 node.
    fn repaint(snapshot: &SceneSnapshot, path: &[usize]) -> Arc<SceneNode> {
        let mut node = snapshot.root(path[0]).unwrap();
        for &index in &path[1..] {
            node = node.child(index).unwrap();
        }
        let mut spec = node.spec();
        spec.paint = PaintStackRef::new(PaintStack {
            layers: vec![PaintLayer::Fill(stellar_scene::stack::Paint::Solid(
                fx::srgb(0.2, 0.4, 0.6, 1.0),
            ))],
            ..PaintStack::empty()
        });
        Arc::new(SceneNode::leaf(spec))
    }

    #[test]
    fn capture_resolves_view_and_bounds() {
        let capture = capture_view(fx::basic_snapshot(&Basic::default()));
        assert_eq!(capture.roots().len(), 1);
        assert_eq!(capture.roots()[0].node().source(), fx::node_id(1));
        assert_eq!(capture.roots()[0].base_clip_depth(), 0);
        assert_eq!(capture.visible_nodes(), 4);
        // Frame(20,10)+100×80, Inside 테두리는 바깥으로 나가지 않고 자식은 frame clip 안입니다.
        assert_eq!(bounds_tuple(capture.bounds()), (20.0, 10.0, 120.0, 90.0));
        assert_eq!(
            capture.resources().iter().copied().collect::<Vec<_>>(),
            vec![SceneResourceRef::Image(fx::asset(1))]
        );
        assert_eq!(capture.pins(), PINS);
        assert_eq!(capture.document_revision(), DocumentRevision(1));
        assert_eq!(capture.scene_revision(), SceneRevision(1));
        assert_eq!(capture.content_hash(), capture.snapshot().semantic_hash());
        assert_eq!(capture.request().format(), ExportFormat::Svg);
        assert!(capture.handle().ptr_eq(&capture.clone().handle().clone()));
    }

    #[test]
    fn unclipped_overflow_extends_bounds_but_clipped_does_not() {
        let overflow = Basic {
            shape_at: (90.0, 70.0),
            ..Basic::default()
        };
        let clipped = capture_view(fx::basic_snapshot(&overflow));
        assert_eq!(bounds_tuple(clipped.bounds()), (20.0, 10.0, 120.0, 90.0));
        let unclipped = capture_view(fx::basic_snapshot(&Basic {
            clip: false,
            ..overflow
        }));
        // Shape(110,80) + 40×30, stroke 3 Center(1.5) + Round cap → (151.5, 111.5).
        assert_eq!(bounds_tuple(unclipped.bounds()), (20.0, 10.0, 151.5, 111.5));
    }

    #[test]
    fn edits_after_capture_do_not_reach_the_export() {
        // 검증 기준(XX.1.2): export 중 edit가 결과에 섞이지 않습니다.
        let mut map = SceneRevisionMap::new();
        let base = map
            .generate(DocumentRevision(1), fx::basic_roots(&Basic::default()))
            .unwrap();
        let mut current = SnapshotHandle::new(base);
        let capture = ExportCapture::new(
            fx::view_request(ExportFormat::Svg),
            SceneSource {
                view: fx::view_id(),
                target_profile: fx::profile(),
                snapshot: current.clone(),
            },
            PINS,
        )
        .unwrap();
        let captured_hash = capture.content_hash();
        let captured_roots = roots_hash(&capture);
        let captured_bounds = bounds_tuple(capture.bounds());

        // document thread: Shape를 다시 칠하고, 바깥으로 옮긴 snapshot을 연속 발행합니다.
        let edited = map
            .derive(
                &current,
                DocumentRevision(2),
                &[0, 0],
                repaint(&current, &[0, 0]),
            )
            .unwrap();
        current = SnapshotHandle::new(edited);
        assert_ne!(current.semantic_hash(), captured_hash);
        let moved_roots = fx::basic_roots(&Basic {
            clip: false,
            shape_at: (500.0, 500.0),
            image: fx::asset(2),
            ..Basic::default()
        });
        current = SnapshotHandle::new(map.generate(DocumentRevision(3), moved_roots).unwrap());
        let recaptured = capture_view(current.snapshot().clone());

        // 이전 capture는 그대로입니다.
        assert_eq!(capture.content_hash(), captured_hash);
        assert_eq!(capture.snapshot().semantic_hash(), captured_hash);
        assert_eq!(roots_hash(&capture), captured_roots);
        assert_eq!(bounds_tuple(capture.bounds()), captured_bounds);
        assert_eq!(capture.document_revision(), DocumentRevision(1));
        assert_eq!(
            capture.resources().iter().copied().collect::<Vec<_>>(),
            vec![SceneResourceRef::Image(fx::asset(1))]
        );
        // 새 capture는 편집 결과를 봅니다(편집이 실제로 내용을 바꿨음을 확인).
        assert_ne!(recaptured.content_hash(), captured_hash);
        assert_ne!(bounds_tuple(recaptured.bounds()), captured_bounds);
        assert_eq!(
            recaptured.resources().iter().copied().collect::<Vec<_>>(),
            vec![SceneResourceRef::Image(fx::asset(2))]
        );
        drop(current);
        // 마지막 document handle이 사라져도 capture가 snapshot을 붙잡고 있습니다.
        assert!(capture.handle().lifetime().is_alive());
    }

    #[test]
    fn concurrent_edits_never_mix_into_a_running_export() {
        // export worker가 capture를 읽는 동안 document thread가 200세대 편집을 발행합니다.
        const REVISIONS: u64 = 200;
        let mut map = SceneRevisionMap::new();
        let base = map
            .generate(DocumentRevision(1), fx::basic_roots(&Basic::default()))
            .unwrap();
        let capture = capture_view(base.clone());
        let expected = (roots_hash(&capture), bounds_tuple(capture.bounds()));
        let (tx, rx) = mpsc::channel::<()>();
        let worker = {
            let capture = capture.clone();
            thread::spawn(move || {
                let mut reads = 0u64;
                loop {
                    assert_eq!(
                        (roots_hash(&capture), bounds_tuple(capture.bounds())),
                        expected
                    );
                    reads += 1;
                    if rx.try_recv().is_ok() {
                        return reads;
                    }
                }
            })
        };
        let mut current = base;
        for revision in 0..REVISIONS {
            let path = [0, (revision % 3) as usize];
            let node = repaint(&current, &path);
            current = map
                .derive(&current, DocumentRevision(2 + revision), &path, node)
                .unwrap();
        }
        tx.send(()).unwrap();
        let reads = worker.join().expect("worker panic 없음");
        assert!(reads > 0);
        assert_ne!(current.semantic_hash(), capture.content_hash());
        assert_eq!(roots_hash(&capture), expected.0);
    }

    /// revision별 내용을 보관하는 asset store 흉내.
    struct VersionedAssets {
        revisions: Mutex<HashMap<(AssetRef, AssetRevision), EncodedImage>>,
    }

    impl ExportAssetSource for VersionedAssets {
        fn encoded_image(&self, asset: AssetRef, revision: AssetRevision) -> Option<EncodedImage> {
            self.revisions.lock().ok()?.get(&(asset, revision)).cloned()
        }
    }

    fn image(byte: u8) -> EncodedImage {
        EncodedImage::new(ImageMedia::Png, Arc::from(vec![byte; 4]), 3, 2).unwrap()
    }

    #[test]
    fn assets_are_read_at_the_pinned_revision() {
        let store = VersionedAssets {
            revisions: Mutex::new(HashMap::from([(
                (fx::asset(1), AssetRevision(1)),
                image(0xAA),
            )])),
        };
        let capture = capture_view(fx::basic_snapshot(&Basic::default()));
        // export 시작 뒤 host가 같은 asset을 교체해 revision 2를 만듭니다.
        store
            .revisions
            .lock()
            .unwrap()
            .insert((fx::asset(1), AssetRevision(2)), image(0xBB));
        let read = capture.encoded_image(&store, fx::asset(1)).unwrap();
        assert_eq!(read.bytes(), &[0xAA; 4]);
        // 고정 revision에 없는 asset은 현재 revision으로 대체하지 않습니다.
        assert_eq!(capture.encoded_image(&store, fx::asset(9)), None);
        assert_eq!(capture.pins().fonts, FontRevision(7));
    }

    #[test]
    fn scene_must_match_requested_view_and_profile() {
        let request = fx::view_request(ExportFormat::Svg);
        let snapshot = SnapshotHandle::new(fx::basic_snapshot(&Basic::default()));
        let other_view = fx::node_id(99);
        assert_eq!(
            ExportCapture::new(
                request.clone(),
                SceneSource {
                    view: other_view,
                    target_profile: fx::profile(),
                    snapshot: snapshot.clone(),
                },
                PINS
            )
            .unwrap_err(),
            CaptureError::ViewMismatch {
                requested: fx::view_id(),
                scene: other_view
            }
        );
        let apple = TargetProfileKey::new("apple-desktop").unwrap();
        assert_eq!(
            ExportCapture::new(
                request,
                SceneSource {
                    view: fx::view_id(),
                    target_profile: apple.clone(),
                    snapshot,
                },
                PINS
            )
            .unwrap_err(),
            CaptureError::ProfileMismatch {
                requested: fx::profile(),
                scene: apple
            }
        );
    }

    #[test]
    fn selection_keeps_topmost_nodes_and_drops_ancestor_clips() {
        let snapshot = fx::basic_snapshot(&Basic::default());
        let select = |ids: Vec<StableId>| {
            ExportCapture::new(
                fx::request(
                    ExportSource::selection(fx::view_id(), ids).unwrap(),
                    ExportFormat::Svg,
                ),
                scene(snapshot.clone()),
                PINS,
            )
        };
        // Image(4)와 Shape(2): paint 순서(Shape → Image)로 정렬되고 frame clip은 범위 밖입니다.
        let capture = select(vec![fx::node_id(4), fx::node_id(2)]).unwrap();
        let sources: Vec<StableId> = capture.roots().iter().map(|r| r.node().source()).collect();
        assert_eq!(sources, vec![fx::node_id(2), fx::node_id(4)]);
        assert!(capture.roots().iter().all(|r| r.base_clip_depth() == 1));
        assert_eq!(capture.visible_nodes(), 2);
        // Shape(30,20)+stroke outset 1.5 ~ Image(30,55)+30×20.
        assert_eq!(bounds_tuple(capture.bounds()), (28.5, 18.5, 71.5, 75.0));
        // Frame과 그 자손을 함께 선택하면 Frame 하나만 남습니다.
        let nested = select(vec![fx::node_id(1), fx::node_id(3)]).unwrap();
        assert_eq!(nested.roots().len(), 1);
        assert_eq!(nested.roots()[0].node().source(), fx::node_id(1));
        assert_eq!(nested.roots()[0].base_clip_depth(), 0);
        // scene에 없는 id는 거부합니다.
        assert_eq!(
            select(vec![fx::node_id(2), fx::node_id(50)]).unwrap_err(),
            CaptureError::SelectionNotInScene {
                id: fx::node_id(50)
            }
        );
    }

    #[test]
    fn hidden_content_is_not_exported() {
        let snapshot = fx::basic_snapshot(&Basic::default());
        // Frame을 숨기면(자손도 숨김) 보이는 내용이 없습니다.
        let frame = snapshot.root(0).unwrap();
        let mut spec = frame.spec();
        spec.hit = HitTestMeta {
            visibility: Visibility::Hidden,
            pointer_events: PointerEvents::Auto,
        };
        let hidden = snapshot
            .replace_node(
                DocumentRevision(2),
                SceneRevision(2),
                &[0],
                Arc::new(frame.with_spec(spec).unwrap()),
            )
            .unwrap();
        assert_eq!(
            ExportCapture::new(fx::view_request(ExportFormat::Svg), scene(hidden), PINS)
                .unwrap_err(),
            CaptureError::NothingVisible
        );
        let empty = SceneSnapshot::new(DocumentRevision(1), SceneRevision(1), Vec::new()).unwrap();
        assert_eq!(
            ExportCapture::new(fx::view_request(ExportFormat::Svg), scene(empty), PINS)
                .unwrap_err(),
            CaptureError::NothingVisible
        );
        for error in [
            CaptureError::NothingVisible,
            CaptureError::NonFiniteBounds {
                node: fx::node_id(1),
            },
            CaptureError::SelectionNotInScene { id: fx::node_id(1) },
        ] {
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn deep_trees_are_captured_without_recursion() {
        // 깊이 1000 frame chain을 64 KiB stack thread에서 capture합니다(재귀 순회라면 넘칩니다).
        let depth = 1_000u64;
        let mut builder = stellar_scene::builder::SceneBuilder::new();
        let mut root = builder.root_scope();
        let mut frames: Vec<stellar_scene::builder::FrameScope> = Vec::new();
        for n in 0..depth {
            let input = stellar_scene::builder::FrameInput {
                node: stellar_scene::builder::NodeInput::new(
                    fx::node_id(n),
                    fx::translate(0.0, 0.0),
                ),
                size: stellar_core_types::value::Size2::new(10.0, 10.0).unwrap(),
                corner_radii: stellar_scene::node::CornerRadii::ZERO,
                background: None,
                border: None,
                clip: stellar_scene::builder::FrameClipInput::None,
            };
            let parent = frames.last().map_or(&root, |frame| frame.scope());
            let frame = builder.begin_frame(parent, &input).unwrap();
            frames.push(frame);
        }
        while let Some(frame) = frames.pop() {
            let parent = match frames.last_mut() {
                Some(parent) => parent.scope_mut(),
                None => &mut root,
            };
            builder.finish_frame(parent, frame).unwrap();
        }
        let snapshot =
            SceneSnapshot::new(DocumentRevision(1), SceneRevision(1), builder.finish(root))
                .unwrap();
        assert_eq!(snapshot.node_count(), depth);
        let handle = SnapshotHandle::new(snapshot);
        let worker = {
            let handle = handle.clone();
            thread::Builder::new()
                .stack_size(64 * 1024)
                .spawn(move || {
                    let capture = ExportCapture::new(
                        fx::view_request(ExportFormat::Svg),
                        SceneSource {
                            view: fx::view_id(),
                            target_profile: fx::profile(),
                            snapshot: handle,
                        },
                        PINS,
                    )
                    .unwrap();
                    (capture.visible_nodes(), bounds_tuple(capture.bounds()))
                })
                .unwrap()
        };
        let (visible, bounds) = worker.join().expect("작은 stack에서도 panic 없음");
        assert_eq!(visible, depth);
        assert_eq!(bounds, (0.0, 0.0, 10.0, 10.0));
        drop(handle);
    }

    #[test]
    fn encoded_image_rejects_empty_input() {
        assert_eq!(
            EncodedImage::new(ImageMedia::Jpeg, Arc::from(Vec::new()), 1, 1),
            Err(EncodedImageError::Empty)
        );
        assert_eq!(
            EncodedImage::new(ImageMedia::Gif, Arc::from(vec![1u8]), 0, 1),
            Err(EncodedImageError::ZeroSize)
        );
        let ok = EncodedImage::new(ImageMedia::Webp, Arc::from(vec![1u8]), 2, 3).unwrap();
        assert_eq!(
            (ok.media(), ok.pixel_width(), ok.pixel_height()),
            (ImageMedia::Webp, 2, 3)
        );
        assert_eq!(ImageMedia::Png.mime(), "image/png");
        assert_eq!(ImageMedia::Jpeg.mime(), "image/jpeg");
        assert_eq!(ImageMedia::Gif.mime(), "image/gif");
        assert_eq!(ImageMedia::Webp.mime(), "image/webp");
    }
}
