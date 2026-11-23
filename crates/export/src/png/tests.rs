//! PNG 내보내기 테스트 (XX.2.2~XX.2.4).

use std::collections::HashMap;
use std::sync::Arc;

use stellar_core_types::color::ColorSpace;
use stellar_scene::builder::{FrameClipInput, StrokeInput};
use stellar_scene::node::{CornerRadii, ImageFit, ImageSampling};
use stellar_scene::stack::{Opacity, Paint};

use super::*;
use crate::preflight::ConfirmedExport;
use crate::raster::software::SoftwareTarget;
use crate::raster::tests::{
    capture_of, frame, leaf_frame, quad_png, request, scene, srgb, text_layout, Assets, Glyphs,
    LAYOUT,
};
use crate::raster::{CancelToken, OffscreenError, OutputProfile, RgbaImage};
use crate::request::{ExportBackground, ExportFormat};
use crate::test_fixtures as fx;

/// encode·해석·색 정보·encoder 옵션 테스트 (XX.2.2).
mod codec;

/// worker 실행 테스트 (XX.2.3).
mod job;

/// golden 비교 테스트 (XX.2.4).
mod golden;

/// 2색 4×3 image.
fn sample_image(profile: OutputProfile) -> RgbaImage {
    let mut pixels = Vec::new();
    for y in 0..3u8 {
        for x in 0..4u8 {
            pixels.extend_from_slice(&[x * 60, y * 100, 255 - x * 30, 255 - y * 60]);
        }
    }
    RgbaImage::new(4, 3, profile, pixels).unwrap()
}

fn assets() -> Assets {
    let mut map = HashMap::new();
    map.insert(fx::asset(1), quad_png());
    Assets(map)
}

fn software() -> Arc<SoftwareTarget> {
    Arc::new(SoftwareTarget::new(
        Arc::new(assets()),
        Arc::new(Glyphs::default()),
    ))
}

/// golden fixture scene: 둥근 clip Frame 안 opacity 그룹, 테두리, path stroke, image, text.
pub(crate) fn showcase_roots() -> Vec<Arc<stellar_scene::node::SceneNode>> {
    let mut root = frame(1, 0.0, 0.0, 48.0, 32.0, srgb(0.95, 0.95, 0.9, 1.0));
    root.corner_radii = CornerRadii::new(6.0, 6.0, 6.0, 6.0).unwrap();
    root.clip = FrameClipInput::Bounds;
    root.border = Some(stellar_scene::builder::BorderInput {
        color: srgb(0.1, 0.1, 0.3, 1.0),
        thickness: 1.5,
    });
    scene(&root, |b, s| {
        let mut group = frame(2, 4.0, 4.0, 20.0, 12.0, srgb(0.2, 0.6, 1.0, 1.0));
        group.node.paint.opacity = Opacity::new(0.6).unwrap();
        let mut overlap = frame(3, 8.0, 4.0, 18.0, 10.0, srgb(1.0, 0.3, 0.2, 1.0));
        overlap.corner_radii = CornerRadii::new(3.0, 0.0, 3.0, 0.0).unwrap();
        let mut group_scope = b.begin_frame(s, &group).unwrap();
        leaf_frame(b, group_scope.scope_mut(), &overlap);
        b.finish_frame(s, group_scope).unwrap();
        let mut node =
            stellar_scene::builder::NodeInput::new(fx::node_id(4), fx::translate(28.0, 4.0));
        node.paint
            .fills
            .push(Paint::Solid(srgb(0.0, 0.5, 0.2, 0.8)));
        node.paint.strokes.push(StrokeInput {
            paint: Paint::Solid(srgb(0.0, 0.0, 0.0, 1.0)),
            width: 1.25,
        });
        b.build_path(
            s,
            &stellar_scene::builder::PathInput {
                node,
                layout_rect: fx::rect(0.0, 0.0, 16.0, 12.0),
                geometry: stellar_scene::geometry::PathGeometry::new(vec![
                    stellar_scene::geometry::PathCommand::MoveTo(fx::p(0.0, 0.0)),
                    stellar_scene::geometry::PathCommand::LineTo(fx::p(16.0, 0.0)),
                    stellar_scene::geometry::PathCommand::CubicTo {
                        c1: fx::p(16.0, 8.0),
                        c2: fx::p(12.0, 12.0),
                        to: fx::p(4.0, 12.0),
                    },
                    stellar_scene::geometry::PathCommand::Close,
                ])
                .unwrap(),
                fill_rule: stellar_scene::geometry::FillRule::NonZero,
            },
        )
        .unwrap();
        b.build_image(
            s,
            &stellar_scene::builder::ImageInput {
                node: stellar_scene::builder::NodeInput::new(
                    fx::node_id(5),
                    fx::translate(4.0, 19.0),
                ),
                asset: fx::asset(1),
                size: stellar_core_types::value::Size2::new(10.0, 10.0).unwrap(),
                fit: ImageFit::Cover,
                crop: stellar_scene::node::ImageCrop::FULL,
                sampling: ImageSampling::Linear,
                color_space: None,
            },
        )
        .unwrap();
        b.build_text(
            s,
            &stellar_scene::builder::TextInput {
                node: stellar_scene::builder::NodeInput::new(
                    fx::node_id(6),
                    fx::translate(18.0, 18.0),
                ),
                layout: LAYOUT,
                size: stellar_core_types::value::Size2::new(26.0, 10.0).unwrap(),
            },
        )
        .unwrap();
    })
}

/// showcase scene을 그리는 target(image asset과 glyph layout 포함).
pub(crate) fn showcase_target() -> SoftwareTarget {
    let mut layouts = HashMap::new();
    layouts.insert(
        LAYOUT,
        Arc::new(text_layout(&[1, 1, 3, 1], Some(srgb(0.3, 0.1, 0.5, 1.0)))),
    );
    SoftwareTarget::new(Arc::new(assets()), Arc::new(Glyphs(layouts)))
}
