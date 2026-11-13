//! 테스트 전용 scene fixture (renderer-core 테스트 공용).
//!
//! renderer-core는 crate 의존 정책(II.2.8)상 core-types 타입(`Rect`, `Transform2D`, `StableId`,
//! `SceneRevision` 등)을 이름으로 만들 수 없습니다. 그래서 fixture를 scene 공개 codec
//! (`stellar_scene::codec`, XI.1.1) canonical bytes로 적고 `decode`해 실제 [`SceneSnapshot`]을
//! 얻습니다. decode가 모든 값 규칙·z-order 불변식을 검사하므로 fixture도 실제 scene과 같은 검증을
//! 거칩니다. z-order는 pre-order 순서로 1부터 자동 배정합니다.

use std::sync::Arc;

use stellar_scene::codec::decode;
use stellar_scene::handle::SnapshotHandle;
use stellar_scene::{SceneNode, SceneSnapshot};

/// 단색 RGBA(sRGB, straight alpha).
pub(crate) type Rgba = [f32; 4];

/// node 내용.
#[derive(Debug, Clone)]
pub(crate) enum ContentFx {
    /// 사각형(x, y, w, h)과 모서리 radius.
    Rect { rect: [f32; 4], radii: [f32; 4] },
    /// text layout key와 bounds.
    Text { key: u8, bounds: [f32; 4] },
    /// 이미지 asset과 bounds.
    Image { asset: u8, bounds: [f32; 4] },
}

/// clip 하나(rect clip만).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ClipFx {
    pub rect: [f32; 4],
    pub radii: [f32; 4],
    /// clip local → world (a, b, c, d, tx, ty).
    pub transform: [f32; 6],
}

/// drop shadow(offset x, y, blur, 색).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ShadowFx {
    pub offset: [f32; 2],
    pub blur: f32,
    pub color: Rgba,
}

/// paint stack.
#[derive(Debug, Clone)]
pub(crate) struct PaintFx {
    pub fills: Vec<Rgba>,
    /// (색, 폭).
    pub strokes: Vec<(Rgba, f32)>,
    pub shadows: Vec<ShadowFx>,
    pub opacity: f32,
    /// `SceneBlendMode::tag()`.
    pub blend: u8,
    pub blur: f32,
}

impl Default for PaintFx {
    fn default() -> PaintFx {
        PaintFx {
            fills: Vec::new(),
            strokes: Vec::new(),
            shadows: Vec::new(),
            opacity: 1.0,
            blend: 0,
            blur: 0.0,
        }
    }
}

/// node fixture.
#[derive(Debug, Clone)]
pub(crate) struct NodeFx {
    pub id: u8,
    pub content: ContentFx,
    pub transform: [f32; 6],
    pub clips: Vec<ClipFx>,
    pub paint: PaintFx,
    pub hidden: bool,
    pub children: Vec<NodeFx>,
}

pub(crate) const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// 이동 변환.
pub(crate) fn translate(x: f32, y: f32) -> [f32; 6] {
    [1.0, 0.0, 0.0, 1.0, x, y]
}

/// 사각형 node(fill 하나).
pub(crate) fn rect(id: u8, rect: [f32; 4], fill: Rgba) -> NodeFx {
    NodeFx {
        id,
        content: ContentFx::Rect {
            rect,
            radii: [0.0; 4],
        },
        transform: IDENTITY,
        clips: Vec::new(),
        paint: PaintFx {
            fills: vec![fill],
            ..PaintFx::default()
        },
        hidden: false,
        children: Vec::new(),
    }
}

/// 칠하지 않는 group 사각형 node.
pub(crate) fn group(id: u8, bounds: [f32; 4], children: Vec<NodeFx>) -> NodeFx {
    NodeFx {
        paint: PaintFx::default(),
        children,
        ..rect(id, bounds, [0.0; 4])
    }
}

/// 불투명 색.
pub(crate) fn opaque(r: f32, g: f32, b: f32) -> Rgba {
    [r, g, b, 1.0]
}

impl NodeFx {
    pub(crate) fn with_opacity(mut self, opacity: f32) -> NodeFx {
        self.paint.opacity = opacity;
        self
    }

    pub(crate) fn with_blend(mut self, blend: u8) -> NodeFx {
        self.paint.blend = blend;
        self
    }

    pub(crate) fn with_blur(mut self, blur: f32) -> NodeFx {
        self.paint.blur = blur;
        self
    }

    pub(crate) fn with_shadow(mut self, shadow: ShadowFx) -> NodeFx {
        self.paint.shadows.push(shadow);
        self
    }

    pub(crate) fn with_clip(mut self, clip: ClipFx) -> NodeFx {
        self.clips.push(clip);
        self
    }

    pub(crate) fn with_radii(mut self, radii: [f32; 4]) -> NodeFx {
        if let ContentFx::Rect { radii: r, .. } = &mut self.content {
            *r = radii;
        }
        self
    }

    pub(crate) fn with_transform(mut self, transform: [f32; 6]) -> NodeFx {
        self.transform = transform;
        self
    }

    pub(crate) fn with_content(mut self, content: ContentFx) -> NodeFx {
        self.content = content;
        self
    }

    pub(crate) fn with_stroke(mut self, color: Rgba, width: f32) -> NodeFx {
        self.paint.strokes.push((color, width));
        self
    }

    pub(crate) fn hidden(mut self) -> NodeFx {
        self.hidden = true;
        self
    }
}

/// 축 정렬 rect clip.
pub(crate) fn rect_clip(rect: [f32; 4]) -> ClipFx {
    ClipFx {
        rect,
        radii: [0.0; 4],
        transform: IDENTITY,
    }
}

fn put_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_f32s(out: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        put_f32(out, *value);
    }
}

fn put_color(out: &mut Vec<u8>, color: Rgba) {
    out.push(0x01); // sRGB
    put_f32s(out, &color);
}

fn write_node(node: &NodeFx, z: &mut u64, out: &mut Vec<u8>) {
    out.extend_from_slice(&[node.id; 16]);
    let layout = match &node.content {
        ContentFx::Rect { rect, radii } => {
            out.push(1);
            put_f32s(out, rect);
            put_f32s(out, radii);
            *rect
        }
        ContentFx::Text { key, bounds } => {
            out.push(3);
            out.extend_from_slice(&[*key; 16]);
            put_f32s(out, bounds);
            *bounds
        }
        ContentFx::Image { asset, bounds } => {
            out.push(4);
            out.extend_from_slice(&[*asset; 16]);
            put_f32s(out, bounds);
            out.push(0); // fit
            put_f32s(out, &[0.0, 0.0, 1.0, 1.0]); // crop
            out.push(0); // sampling
            out.push(0); // color space 없음
            *bounds
        }
    };
    put_f32s(out, &node.transform);
    put_f32s(out, &layout);
    out.extend_from_slice(&(node.clips.len() as u32).to_le_bytes());
    for clip in &node.clips {
        out.push(1);
        put_f32s(out, &clip.rect);
        put_f32s(out, &clip.radii);
        put_f32s(out, &clip.transform);
    }
    let layers = node.paint.fills.len() + node.paint.strokes.len();
    out.extend_from_slice(&(layers as u32).to_le_bytes());
    for fill in &node.paint.fills {
        out.push(1);
        out.push(1);
        put_color(out, *fill);
    }
    for (color, width) in &node.paint.strokes {
        out.push(2);
        out.push(1);
        put_color(out, *color);
        put_f32(out, *width);
        out.push(0); // align center
        out.push(0); // cap butt
    }
    out.extend_from_slice(&(node.paint.shadows.len() as u32).to_le_bytes());
    for shadow in &node.paint.shadows {
        put_f32s(out, &shadow.offset);
        put_f32(out, shadow.blur);
        put_color(out, shadow.color);
    }
    put_f32(out, node.paint.opacity);
    out.push(node.paint.blend);
    put_f32(out, node.paint.blur);
    *z += 1;
    out.extend_from_slice(&z.to_le_bytes());
    out.push(u8::from(node.hidden));
    out.push(0); // pointer events auto
    out.extend_from_slice(&(node.children.len() as u32).to_le_bytes());
}

/// pre-order로 node를 씁니다(재귀 없이).
fn write_tree(roots: &[NodeFx], z: &mut u64, out: &mut Vec<u8>) {
    let mut stack: Vec<&NodeFx> = roots.iter().rev().collect();
    while let Some(node) = stack.pop() {
        write_node(node, z, out);
        stack.extend(node.children.iter().rev());
    }
}

/// revision과 최상위 node로 snapshot을 만듭니다.
pub(crate) fn snapshot_with(document: u64, scene: u64, roots: &[NodeFx]) -> SceneSnapshot {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"STSN");
    bytes.push(1);
    bytes.extend_from_slice(&document.to_le_bytes());
    bytes.extend_from_slice(&scene.to_le_bytes());
    bytes.extend_from_slice(&(roots.len() as u32).to_le_bytes());
    let mut z = 0;
    write_tree(roots, &mut z, &mut bytes);
    match decode(&bytes) {
        Ok(snapshot) => snapshot,
        Err(error) => panic!("fixture decode 실패: {error}"),
    }
}

/// revision 1의 snapshot.
pub(crate) fn snapshot(roots: &[NodeFx]) -> SceneSnapshot {
    snapshot_with(1, 1, roots)
}

/// node가 없는 snapshot handle(scene revision 지정).
pub(crate) fn empty_handle(scene_revision: u64) -> SnapshotHandle {
    SnapshotHandle::new(snapshot_with(scene_revision, scene_revision, &[]))
}

/// snapshot의 node를 pre-order로 모읍니다.
pub(crate) fn nodes(snapshot: &SceneSnapshot) -> Vec<Arc<SceneNode>> {
    let mut out = Vec::new();
    let mut stack: Vec<&Arc<SceneNode>> = snapshot.roots().collect();
    stack.reverse();
    while let Some(node) = stack.pop() {
        out.push(Arc::clone(node));
        let start = stack.len();
        stack.extend(node.children());
        stack[start..].reverse();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_decode_into_real_scene_nodes() {
        let scene = snapshot(&[
            rect(1, [0.0, 0.0, 10.0, 10.0], opaque(1.0, 0.0, 0.0)),
            group(
                2,
                [0.0, 0.0, 50.0, 50.0],
                vec![
                    rect(3, [5.0, 5.0, 5.0, 5.0], [0.0, 1.0, 0.0, 0.5])
                        .with_clip(rect_clip([0.0, 0.0, 8.0, 8.0])),
                    rect(4, [1.0, 1.0, 2.0, 2.0], opaque(0.0, 0.0, 1.0))
                        .with_shadow(ShadowFx {
                            offset: [1.0, 1.0],
                            blur: 2.0,
                            color: [0.0, 0.0, 0.0, 0.5],
                        })
                        .with_stroke(opaque(0.0, 0.0, 0.0), 1.0),
                ],
            )
            .with_opacity(0.5)
            .with_blend(1)
            .with_blur(3.0)
            .with_transform(translate(5.0, 5.0)),
            rect(5, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 1.0, 1.0))
                .with_content(ContentFx::Text {
                    key: 9,
                    bounds: [0.0, 0.0, 4.0, 4.0],
                })
                .hidden(),
            rect(6, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 1.0, 1.0))
                .with_content(ContentFx::Image {
                    asset: 7,
                    bounds: [0.0, 0.0, 4.0, 4.0],
                })
                .with_radii([1.0; 4]),
        ]);
        assert_eq!(scene.node_count(), 6);
        assert_eq!(scene.scene_revision().0, 1);
        let all = nodes(&scene);
        let z: Vec<u64> = all.iter().map(|node| node.z().0).collect();
        assert_eq!(z, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(all[1].paint().get().opacity.get(), 0.5);
        assert_eq!(all[2].clip().depth(), 1);
        assert_eq!(all[3].paint().get().shadows.len(), 1);
        assert_eq!(empty_handle(7).scene_revision().0, 7);
    }
}
