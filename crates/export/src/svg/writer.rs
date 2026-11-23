//! SVG 문서 writer (XX.3.3).
//!
//! [`write_svg`]는 사전 진단을 통과했거나 사용자가 진단을 확인한 capture([`ConfirmedExport`], XX.1.3)만 읽어
//! SVG 문서 하나를 만듭니다. 출력은 입력의 **값**에만 의존합니다: 같은 snapshot 내용·요청·고정 asset 내용이면
//! `Arc` 공유 구조, 생성 순서, scene revision과 무관하게 byte 단위로 같은 문서가 됩니다.
//!
//! - missing resource: 사용자가 확인한 missing image·text layout·font만 생략합니다(이미지 pixel, Text glyph,
//!   그 폰트 glyph). 확인하지 않은 resource가 없으면(진단 뒤 host가 내용을 잃음) 오류입니다.
//! - 순회: 최상위 node를 paint 순서로, 각 subtree를 명시적 stack pre-order로 걷습니다(재귀 없음).
//!   숨긴 node는 subtree째 생략합니다.
//! - 구조: node마다 `<g id>` 하나(world 좌표, 변환 없음). 그 안에 content 고유 모습(image 픽셀, text run
//!   색으로 칠한 glyph outline) → 칠하기 layer(아래 → 위) → 자식 그룹 순서로 씁니다([`crate::svg::mapping`]
//!   좌표계 규칙).
//! - text: 고정 font revision의 glyph run을 outline `<path>`로 씁니다([`crate::svg::text`], XX.3.2).
//!   `<text>` 요소와 font는 쓰지 않습니다. paint layer는 모든 glyph outline을 이은 기하를 칠합니다.
//! - clip: node clip stack이 부모 그룹이 이미 건 clip을 앞부분으로 가져야 합니다(아니면
//!   [`SvgExportError::ClipNotNested`]). 조상 그룹의 clip은 SVG 상속으로 이미 적용되므로 node
//!   그룹에는 **새로 늘어난 clip만** 겁니다. 늘어난 clip이 둘 이상이면 안쪽 `<clipPath>`가 바로
//!   바깥 정의를 `clip-path`로 참조하는 chain이 됩니다. 같은 (chain 안 바로 바깥 정의, clip 값)
//!   조합은 문서 안에서 한 정의를 공유하며, 정의는 `<defs>`에 처음 쓰인 순서대로 모읍니다.
//! - id: [`SvgIdAllocator`](StableId 기반, 중복 발급 거부).
//! - 지원 상태가 vector가 아닌 기능을 만나면 그 node에서 typed error로 멈춥니다(조용히 빼지 않음).
//! - 출력 크기는 [`SvgWriteOptions::max_output_bytes`]를 넘으면 중단합니다(이미지 base64는 쓰기 전에
//!   길이를 계산해 검사합니다).

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::AssetRef;
use stellar_scene::geometry::{FillRule, PathGeometry};
use stellar_scene::node::{SceneContent, SceneNode, TextLayoutKey, TextPrimitive, Visibility};
use stellar_scene::stack::{ClipShape, Paint, PaintLayer, StrokeAlign};

use crate::capture::{ExportAssetSource, ExportCapture, ExportRoot};
use crate::preflight::ConfirmedExport;
use crate::request::{ExportBackground, ExportFormat};
use crate::svg::element::{
    write_background, write_clip_def, write_fill, write_geometry_clip_def, write_group_close,
    write_group_open, write_image, write_stroke, SvgGeometry,
};
use crate::svg::format::{base64_len, write_number, write_numbers, SvgValueError};
use crate::svg::id::{SvgId, SvgIdAllocator, SvgIdError};
use crate::svg::mapping::{first_unsupported, node_features, SvgFeature, SvgSupport};
use crate::svg::text::{text_outlines, TextOutlineError, MAX_TEXT_OUTLINE_COMMANDS};
use crate::text::{ExportTextSource, GlyphOutline, GlyphOutlineError, GlyphOutlineKey};

/// 기본 최대 출력 크기(256 MiB).
pub const DEFAULT_MAX_SVG_BYTES: usize = 256 * 1024 * 1024;

/// writer 옵션.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SvgWriteOptions {
    /// 최대 출력 크기(byte).
    pub max_output_bytes: usize,
}

impl Default for SvgWriteOptions {
    fn default() -> SvgWriteOptions {
        SvgWriteOptions {
            max_output_bytes: DEFAULT_MAX_SVG_BYTES,
        }
    }
}

/// 완성된 SVG 문서.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvgDocument {
    text: String,
    groups: u64,
    clip_definitions: usize,
}

impl SvgDocument {
    /// 문서 문자열(UTF-8).
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// 문서 bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.text.as_bytes()
    }

    /// 문서 문자열을 꺼냅니다.
    pub fn into_string(self) -> String {
        self.text
    }

    /// 쓴 node 그룹 수.
    pub const fn groups(&self) -> u64 {
        self.groups
    }

    /// 쓴 `<clipPath>` 정의 수.
    pub const fn clip_definitions(&self) -> usize {
        self.clip_definitions
    }
}

/// SVG 내보내기 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SvgExportError {
    /// 요청 format이 SVG가 아닙니다.
    FormatMismatch {
        /// 요청 format.
        requested: ExportFormat,
    },
    /// node가 지금 기록할 수 없는 기능을 씁니다.
    Unsupported {
        /// 문제 node.
        node: StableId,
        /// 첫 미지원 기능.
        feature: SvgFeature,
        /// 지원 상태.
        support: SvgSupport,
    },
    /// node clip stack이 부모 clip stack으로 시작하지 않습니다.
    ClipNotNested {
        /// 문제 node.
        node: StableId,
    },
    /// 고정 asset revision에 이미지 내용이 없습니다.
    MissingImage {
        /// 문제 node.
        node: StableId,
        /// asset.
        asset: AssetRef,
    },
    /// 고정 font revision에 text layout glyph run이 없습니다.
    MissingTextLayout {
        /// 문제 node.
        node: StableId,
        /// layout key.
        layout: TextLayoutKey,
    },
    /// text glyph outline을 만들 수 없습니다.
    Text {
        /// 문제 node.
        node: StableId,
        /// 원인.
        error: TextOutlineError,
    },
    /// node 값 표기 실패(비유한 계산 결과 등).
    Value {
        /// 문제 node.
        node: StableId,
        /// 원인.
        error: SvgValueError,
    },
    /// 문서 수준 값(크기·배경) 표기 실패.
    Document(SvgValueError),
    /// id 발급 실패.
    Id(SvgIdError),
    /// 출력이 상한을 넘습니다.
    OutputTooLarge {
        /// 상한(byte).
        limit: usize,
    },
}

impl fmt::Display for SvgExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SvgExportError::FormatMismatch { requested } => {
                write!(f, "요청 형식이 SVG가 아닙니다({requested})")
            }
            SvgExportError::Unsupported {
                node,
                feature,
                support,
            } => write!(
                f,
                "node {node}의 {feature} 기능은 아직 SVG로 기록할 수 없습니다: {support}"
            ),
            SvgExportError::ClipNotNested { node } => {
                write!(f, "node {node}의 clip이 부모 clip 안에 있지 않습니다")
            }
            SvgExportError::MissingImage { node, asset } => write!(
                f,
                "node {node}의 이미지 {}를 고정 revision에서 찾을 수 없습니다",
                asset.id()
            ),
            SvgExportError::MissingTextLayout { node, .. } => write!(
                f,
                "node {node}의 text layout을 고정 font revision에서 찾을 수 없습니다"
            ),
            SvgExportError::Text { node, error } => write!(f, "node {node}: {error}"),
            SvgExportError::Value { node, error } => write!(f, "node {node}: {error}"),
            SvgExportError::Document(error) => write!(f, "SVG 문서: {error}"),
            SvgExportError::Id(error) => write!(f, "{error}"),
            SvgExportError::OutputTooLarge { limit } => {
                write!(f, "SVG 출력이 {limit} byte 상한을 넘습니다")
            }
        }
    }
}

impl std::error::Error for SvgExportError {}

impl From<SvgIdError> for SvgExportError {
    fn from(error: SvgIdError) -> SvgExportError {
        SvgExportError::Id(error)
    }
}

/// 진행 허가를 받은 capture를 SVG 문서로 씁니다. 이미지 내용은 `assets`에서 고정 asset revision으로, text glyph는
/// `text`에서 고정 font revision으로 읽습니다.
pub fn write_svg(
    export: &ConfirmedExport,
    assets: &dyn ExportAssetSource,
    text: &dyn ExportTextSource,
    options: &SvgWriteOptions,
) -> Result<SvgDocument, SvgExportError> {
    let capture = export.capture();
    let requested = capture.request().format();
    if requested != ExportFormat::Svg {
        return Err(SvgExportError::FormatMismatch { requested });
    }
    let mut writer = Writer {
        export,
        capture,
        assets,
        text,
        outlines: HashMap::new(),
        limit: options.max_output_bytes,
        ids: SvgIdAllocator::new(),
        clip_defs: HashMap::new(),
        defs: String::new(),
        body: String::new(),
        groups: 0,
        clip_definitions: 0,
    };
    if let ExportBackground::Solid(color) = capture.request().background() {
        write_background(&mut writer.body, capture.bounds(), color)
            .map_err(SvgExportError::Document)?;
    }
    for root in capture.roots() {
        writer.write_root(root)?;
    }
    let mut text = String::new();
    text.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    text.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\"");
    let bounds = capture.bounds();
    let scale = capture.request().scale().get();
    let document = |text: &mut String, name: &str, value: f32| -> Result<(), SvgExportError> {
        text.push(' ');
        text.push_str(name);
        text.push_str("=\"");
        write_number(text, value).map_err(SvgExportError::Document)?;
        text.push('"');
        Ok(())
    };
    document(&mut text, "width", bounds.width() * scale)?;
    document(&mut text, "height", bounds.height() * scale)?;
    text.push_str(" viewBox=\"");
    write_numbers(
        &mut text,
        &[
            bounds.min_x(),
            bounds.min_y(),
            bounds.width(),
            bounds.height(),
        ],
    )
    .map_err(SvgExportError::Document)?;
    text.push_str("\">\n");
    if !writer.defs.is_empty() {
        text.push_str("<defs>\n");
        text.push_str(&writer.defs);
        text.push_str("</defs>\n");
    }
    text.push_str(&writer.body);
    text.push_str("</svg>\n");
    if text.len() > writer.limit {
        return Err(SvgExportError::OutputTooLarge {
            limit: writer.limit,
        });
    }
    Ok(SvgDocument {
        text,
        groups: writer.groups,
        clip_definitions: writer.clip_definitions,
    })
}

/// 부모 그룹까지 적용된 clip(바깥 → 안쪽).
type Applied<'a> = Rc<Vec<&'a ClipShape>>;

enum Step<'a> {
    Enter(&'a SceneNode, Applied<'a>),
    Exit,
}

struct Writer<'a> {
    export: &'a ConfirmedExport,
    capture: &'a ExportCapture,
    assets: &'a dyn ExportAssetSource,
    text: &'a dyn ExportTextSource,
    /// glyph outline 조회 cache(조회만 하며 출력 순서에 영향 없음).
    outlines: HashMap<GlyphOutlineKey, Result<GlyphOutline, GlyphOutlineError>>,
    limit: usize,
    ids: SvgIdAllocator,
    /// (chain 안 바로 바깥 정의, clip 값 canonical bytes) → 정의 id.
    clip_defs: HashMap<(Option<SvgId>, Vec<u8>), SvgId>,
    defs: String,
    body: String,
    groups: u64,
    clip_definitions: usize,
}

fn value_error(node: StableId) -> impl Fn(SvgValueError) -> SvgExportError {
    move |error| SvgExportError::Value { node, error }
}

impl<'a> Writer<'a> {
    fn check_size(&self, extra: usize) -> Result<(), SvgExportError> {
        let total = self
            .defs
            .len()
            .saturating_add(self.body.len())
            .saturating_add(extra);
        if total > self.limit {
            Err(SvgExportError::OutputTooLarge { limit: self.limit })
        } else {
            Ok(())
        }
    }

    fn write_root(&mut self, root: &'a ExportRoot) -> Result<(), SvgExportError> {
        let base: Vec<&'a ClipShape> = root
            .node()
            .clip()
            .to_outer_first()
            .into_iter()
            .take(root.base_clip_depth())
            .collect();
        let mut stack = vec![Step::Enter(root.node(), Rc::new(base))];
        while let Some(step) = stack.pop() {
            match step {
                Step::Exit => write_group_close(&mut self.body),
                Step::Enter(node, parent) => {
                    if node.hit().visibility == Visibility::Hidden {
                        continue;
                    }
                    let state = Rc::new(self.write_node_open(node, &parent)?);
                    self.check_size(0)?;
                    stack.push(Step::Exit);
                    let start = stack.len();
                    stack.extend(
                        node.children()
                            .map(|child| Step::Enter(child, Rc::clone(&state))),
                    );
                    stack[start..].reverse();
                }
            }
        }
        Ok(())
    }

    /// node 그룹을 열고 내용을 씁니다. 자식에게 넘길 적용 clip을 돌려줍니다.
    fn write_node_open(
        &mut self,
        node: &'a SceneNode,
        parent: &[&'a ClipShape],
    ) -> Result<Vec<&'a ClipShape>, SvgExportError> {
        let source = node.source();
        if let Some(feature) = first_unsupported(&node_features(node)) {
            return Err(SvgExportError::Unsupported {
                node: source,
                feature,
                support: feature.support(),
            });
        }
        let id = self.ids.node(source)?;
        let stack = node.clip().to_outer_first();
        let applied = parent.len();
        if stack.len() < applied || stack[..applied] != *parent {
            return Err(SvgExportError::ClipNotNested { node: source });
        }
        // 새로 늘어난 clip만 chain으로 만듭니다(조상 그룹 clip은 상속으로 적용됨).
        let mut chain: Option<SvgId> = None;
        for (offset, shape) in stack[applied..].iter().enumerate() {
            let key = (chain.clone(), clip_key(shape));
            let definition = match self.clip_defs.get(&key) {
                Some(existing) => existing.clone(),
                None => {
                    let definition = self.ids.clip(&id, applied + offset + 1)?;
                    write_clip_def(
                        &mut self.defs,
                        definition.as_str(),
                        shape,
                        chain.as_ref().map(SvgId::as_str),
                    )
                    .map_err(value_error(source))?;
                    self.clip_definitions += 1;
                    self.clip_defs.insert(key, definition.clone());
                    definition
                }
            };
            chain = Some(definition);
        }
        write_group_open(
            &mut self.body,
            id.as_str(),
            chain.as_ref().map(SvgId::as_str),
            node.paint().get().opacity,
        )
        .map_err(value_error(source))?;
        self.groups += 1;
        self.write_content(node, &id)?;
        Ok(stack)
    }

    /// text glyph outline을 만들고 run 고유 색 layer를 씁니다. paint layer 기하(모든 glyph)를 돌려줍니다.
    ///
    /// glyph pen 원점은 layout 상자(`primitive.bounds`, node local) 좌상단 기준이라 상자 원점만큼 옮깁니다.
    fn write_text(
        &mut self,
        node: &'a SceneNode,
        primitive: &TextPrimitive,
    ) -> Result<PathGeometry, SvgExportError> {
        let source = node.source();
        let layout = primitive.layout;
        let Some(glyphs) = self.capture.glyph_layout(self.text, layout) else {
            if self.export.omits_text_layout(layout) {
                // 사용자가 확인한 missing layout: glyph 없이 진행합니다.
                return Ok(PathGeometry::empty());
            }
            return Err(SvgExportError::MissingTextLayout {
                node: source,
                layout,
            });
        };
        // command 하나는 출력에 최소 2 byte("Z" + 구분자)라 남은 출력 예산의 절반을 넘으면 출력이 상한을 넘습니다.
        // 메모리 상한은 text_outlines가 따로 겁니다(MAX_TEXT_OUTLINE_COMMANDS).
        let output_budget = self
            .limit
            .saturating_sub(self.defs.len().saturating_add(self.body.len()))
            / 2;
        let (export, capture, text, cache) =
            (self.export, self.capture, self.text, &mut self.outlines);
        let mut lookup = |key: &GlyphOutlineKey| {
            let outline = cache
                .entry(key.clone())
                .or_insert_with(|| capture.glyph_outline(text, key))
                .clone();
            match outline {
                // 사용자가 확인한 missing font: 그 폰트 glyph를 생략합니다.
                Err(GlyphOutlineError::FontUnavailable) if export.omits_font(key.font) => {
                    Ok(GlyphOutline::Empty)
                }
                other => other,
            }
        };
        let origin = primitive.bounds.origin();
        let outlines = match text_outlines(&glyphs, origin, &mut lookup, output_budget) {
            Ok(outlines) => outlines,
            // 출력 예산이 더 작은 상한이었으면 출력 초과, 아니면 메모리 상한 초과(Text 오류)입니다.
            Err(TextOutlineError::TooManyCommands { .. })
                if output_budget < MAX_TEXT_OUTLINE_COMMANDS =>
            {
                return Err(SvgExportError::OutputTooLarge { limit: self.limit })
            }
            Err(error) => {
                return Err(SvgExportError::Text {
                    node: source,
                    error,
                })
            }
        };
        let transform = node.world_transform();
        for run in &outlines.runs {
            if let Some(color) = run.fill {
                // run 범위의 복사본은 이 run을 쓰는 동안만 둡니다.
                let geometry =
                    outlines
                        .run_geometry(run)
                        .map_err(|error| SvgExportError::Text {
                            node: source,
                            error,
                        })?;
                write_fill(
                    &mut self.body,
                    &SvgGeometry::Path(&geometry, FillRule::NonZero),
                    &transform,
                    Paint::Solid(color),
                )
                .map_err(value_error(source))?;
                self.check_size(0)?;
            }
        }
        Ok(outlines.combined)
    }

    /// content 고유 모습(image 픽셀, text run 색)과 칠하기 layer를 씁니다.
    fn write_content(&mut self, node: &'a SceneNode, id: &SvgId) -> Result<(), SvgExportError> {
        let source = node.source();
        let transform = node.world_transform();
        if let SceneContent::Image(image) = node.content() {
            match self.capture.encoded_image(self.assets, image.asset) {
                Some(encoded) => {
                    self.check_size(base64_len(encoded.bytes().len()).unwrap_or(usize::MAX))?;
                    write_image(&mut self.body, image, &transform, &encoded)
                        .map_err(value_error(source))?;
                }
                // 사용자가 확인한 missing image: pixel 없이 칠하기 layer만 씁니다.
                None if self.export.omits_image(image.asset) => {}
                None => {
                    return Err(SvgExportError::MissingImage {
                        node: source,
                        asset: image.asset,
                    })
                }
            }
        }
        let text_geometry = match node.content() {
            SceneContent::Text(text) => Some(self.write_text(node, text)?),
            SceneContent::Rect(_) | SceneContent::Path(_) | SceneContent::Image(_) => None,
        };
        let geometry = match &text_geometry {
            Some(glyphs) => SvgGeometry::Path(glyphs, FillRule::NonZero),
            None => match SvgGeometry::of_content(node.content()) {
                Some(geometry) => geometry,
                None => return Ok(()),
            },
        };
        if geometry.is_empty_path() {
            return Ok(());
        }
        let mut own_clip: Option<SvgId> = None;
        for layer in &node.paint().get().layers {
            match layer {
                PaintLayer::Fill(paint) => {
                    write_fill(&mut self.body, &geometry, &transform, *paint)
                        .map_err(value_error(source))?;
                }
                PaintLayer::Stroke(stroke) => {
                    let clip = match stroke.align() {
                        StrokeAlign::Inside => {
                            if own_clip.is_none() {
                                let clip = self.ids.geometry_clip(id)?;
                                write_geometry_clip_def(&mut self.defs, clip.as_str(), &geometry)
                                    .map_err(value_error(source))?;
                                self.clip_definitions += 1;
                                own_clip = Some(clip);
                            }
                            own_clip.as_ref().map(SvgId::as_str)
                        }
                        StrokeAlign::Center | StrokeAlign::Outside => None,
                    };
                    write_stroke(&mut self.body, &geometry, &transform, stroke, clip)
                        .map_err(value_error(source))?;
                }
            }
        }
        Ok(())
    }
}

/// clip 값의 canonical bytes(정의 공유 key). 같은 값이면 같은 bytes입니다.
fn clip_key(shape: &ClipShape) -> Vec<u8> {
    let mut key = Vec::new();
    let floats = |values: &[f32], key: &mut Vec<u8>| {
        for value in values {
            key.extend_from_slice(&value.to_le_bytes());
        }
    };
    match shape {
        ClipShape::Rect {
            rect,
            radii,
            transform,
        } => {
            key.push(1);
            let (tl, tr, br, bl) = radii.components();
            floats(
                &[
                    rect.origin().x(),
                    rect.origin().y(),
                    rect.size().width(),
                    rect.size().height(),
                    tl,
                    tr,
                    br,
                    bl,
                ],
                &mut key,
            );
            let (a, b, c, d, tx, ty) = transform.components();
            floats(&[a, b, c, d, tx, ty], &mut key);
        }
        ClipShape::Path {
            geometry,
            fill_rule,
            transform,
        } => {
            key.push(2);
            key.push(fill_rule.tag());
            key.extend_from_slice(&geometry.canonical_bytes());
            let (a, b, c, d, tx, ty) = transform.components();
            floats(&[a, b, c, d, tx, ty], &mut key);
        }
    }
    key
}

#[cfg(test)]
mod tests;
