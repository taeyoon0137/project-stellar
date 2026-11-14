//! glyph atlas page 할당·eviction (XI.6.5).
//!
//! [`ShelfGlyphAtlas`]는 [`GlyphAtlas`] 구현입니다.
//!
//! - **page**: 한 변 `page_size`인 정사각 page이며 page마다 bitmap format 하나(Coverage8 또는
//!   Rgba8Premultiplied)를 담습니다. page 수는 예산 `max_pages` 이하입니다.
//! - **할당**: shelf packing입니다. glyph 하나는 사방 `padding` px gutter를 더한 cell을 차지합니다. 높이가
//!   맞는 shelf 중 남는 높이가 가장 작은 shelf(같으면 앞 shelf)에 넣고, 없으면 새 shelf를 엽니다. 같은 format
//!   page를 id 순서로 시도한 뒤, 예산이 남으면 새 page를, 아니면 page 하나를 비워 씁니다. 결과는 입력 순서만으로
//!   정해집니다(결정론).
//! - **eviction**: page 단위입니다(아키텍처 §10.4 "사용 빈도가 낮은 glyph atlas 페이지"). 마지막 사용 frame이
//!   가장 오래된 page부터 비우며, **현재 frame에서 쓴 page**와 **GPU가 아직 끝내지 않은 frame에서 쓴 page**는
//!   비우지 않습니다. 그래서 현재 frame이 조회·배치한 glyph는 frame이 끝날 때까지 유효하고, GPU가 읽는 중인
//!   texture 영역을 덮어쓰지 않습니다. 비울 page가 없으면 [`AtlasError::Full`]입니다.
//! - **세대**: page를 비우거나 없애면 그 slot의 세대가 올라가 이전 [`AtlasPlacement`]는 무효가 됩니다.
//! - **page texture 수명**: 비운 slot은 **다른 format의 page로 다시 쓰일 수 있습니다**. 그래서 renderer는 page
//!   texture를 page id만이 아니라 `(page id, 세대)`마다 하나 두어야 합니다. [`AtlasPlacement`]의 `(page,
//!   generation)`에 맞는 texture가 없으면(새 page, 또는 slot이 비워진 뒤 다시 쓰임) 그 placement의 `format`으로
//!   [`ShelfGlyphAtlas::page_texture_desc`]를 불러 새 texture를 만들고, 이전 세대 texture는 GPU가 그것을 쓴
//!   frame을 끝낸 뒤 파괴합니다. slot 목록은 줄지 않고 세대는 비울 때마다 올라가므로 `(page id, 세대)`는 page
//!   하나를 가리킵니다(같은 slot을 `u32` 범위만큼 비우기 전까지). 별도 page 변경 이벤트는 두지 않습니다.
//! - **color page pixel**: `Rgba8Premultiplied` page는 `Rgba8UnormSrgb` texture이며 pixel은 선형광에서
//!   premultiply한 뒤 sRGB로 인코딩한 값입니다([`GlyphBitmapFormat::Rgba8Premultiplied`] 문서).
//! - **memory pressure**: [`ShelfGlyphAtlas::apply_memory_pressure`]가 예산을 줄이고 비울 수 있는 page를
//!   없앱니다. 보호된 page는 남기고 그 수를 보고합니다(예산은 다음 frame들에서 수렴).
//! - **upload**: 새 glyph는 gutter까지 0으로 채운 cell 전체를 올려([`ShelfGlyphAtlas::upload_request`]) 재사용한
//!   page에 남은 이전 pixel이 bilinear sampling으로 번지지 않게 합니다.

use std::collections::HashMap;
use std::sync::Arc;

use crate::backend::{GpuTextureDesc, TextureHandle, TextureRegion, TextureUsage};
use crate::frame_resources::{UploadRequest, UploadTarget};
use crate::render_graph::{SampleCount, TextureFormat};

use super::{
    AtlasError, AtlasPageId, AtlasPlacement, GlyphAtlas, GlyphBitmap, GlyphBitmapFormat, GlyphKey,
};

/// page 한 변 최소·최대(px).
const MIN_PAGE_SIZE: u32 = 64;
const MAX_PAGE_SIZE: u32 = 8192;

/// page 예산 최대 수.
pub const MAX_ATLAS_PAGES: u32 = 256;

/// gutter 최대(px).
const MAX_PADDING: u32 = 8;

/// atlas 설정.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AtlasConfig {
    /// page 한 변(px, 64..=8192).
    pub page_size: u32,
    /// page 예산(1..=[`MAX_ATLAS_PAGES`]).
    pub max_pages: u32,
    /// glyph 사방 gutter(px, 0..=8).
    pub padding: u32,
}

impl AtlasConfig {
    /// 기본: 1024 px page 8장, gutter 1 px.
    pub const DEFAULT: AtlasConfig = AtlasConfig {
        page_size: 1024,
        max_pages: 8,
        padding: 1,
    };

    fn validate(&self) -> Result<(), AtlasError> {
        if !(MIN_PAGE_SIZE..=MAX_PAGE_SIZE).contains(&self.page_size) {
            return Err(AtlasError::InvalidConfig {
                reason: "page 크기는 64..=8192 px이어야 합니다",
            });
        }
        if self.max_pages == 0 || self.max_pages > MAX_ATLAS_PAGES {
            return Err(AtlasError::InvalidConfig {
                reason: "page 예산은 1..=256이어야 합니다",
            });
        }
        if self.padding > MAX_PADDING {
            return Err(AtlasError::InvalidConfig {
                reason: "gutter는 0..=8 px이어야 합니다",
            });
        }
        Ok(())
    }
}

impl Default for AtlasConfig {
    fn default() -> AtlasConfig {
        AtlasConfig::DEFAULT
    }
}

/// atlas 통계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AtlasStats {
    /// 현재 page 수.
    pub pages: u32,
    /// 현재 glyph 수.
    pub glyphs: usize,
    /// 조회 hit.
    pub hits: u64,
    /// 조회 miss.
    pub misses: u64,
    /// 새로 배치한 glyph 수.
    pub inserted: u64,
    /// 비운(또는 없앤) page 수.
    pub evicted_pages: u64,
    /// eviction으로 사라진 glyph 수.
    pub evicted_glyphs: u64,
    /// 공간 부족으로 거부한 배치 수.
    pub full: u64,
}

/// memory pressure 처리 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrimOutcome {
    /// 이번에 없앤 page 수.
    pub evicted_pages: u32,
    /// 남은 page 수.
    pub remaining_pages: u32,
    /// 예산을 넘지만 현재 frame·GPU 진행 중이라 남긴 page 수.
    pub protected_over_budget: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shelf {
    y: u32,
    height: u32,
    cursor: u32,
}

#[derive(Debug, Clone)]
struct PageState {
    format: GlyphBitmapFormat,
    shelves: Vec<Shelf>,
    next_y: u32,
    last_used: Option<u64>,
    glyphs: Vec<GlyphKey>,
}

impl PageState {
    fn new(format: GlyphBitmapFormat) -> PageState {
        PageState {
            format,
            shelves: Vec::new(),
            next_y: 0,
            last_used: None,
            glyphs: Vec::new(),
        }
    }

    /// `w × h` cell 위치(좌상단)를 찾습니다.
    fn allocate(&mut self, w: u32, h: u32, page_size: u32) -> Option<(u32, u32)> {
        let best = self
            .shelves
            .iter()
            .enumerate()
            .filter(|(_, shelf)| shelf.height >= h && page_size - shelf.cursor >= w)
            .min_by_key(|(index, shelf)| (shelf.height - h, *index))
            .map(|(index, _)| index);
        if let Some(index) = best {
            let shelf = &mut self.shelves[index];
            let x = shelf.cursor;
            shelf.cursor += w;
            return Some((x, shelf.y));
        }
        if page_size - self.next_y >= h {
            let y = self.next_y;
            self.next_y += h;
            self.shelves.push(Shelf {
                y,
                height: h,
                cursor: w,
            });
            return Some((0, y));
        }
        None
    }
}

#[derive(Debug, Clone)]
struct PageSlot {
    generation: u32,
    page: Option<PageState>,
}

/// shelf packing glyph atlas(모듈 문서).
#[derive(Debug, Clone)]
pub struct ShelfGlyphAtlas {
    config: AtlasConfig,
    slots: Vec<PageSlot>,
    glyphs: HashMap<GlyphKey, AtlasPlacement>,
    current_frame: u64,
    gpu_completed: Option<u64>,
    stats: AtlasStats,
}

fn texture_format(format: GlyphBitmapFormat) -> TextureFormat {
    match format {
        GlyphBitmapFormat::Coverage8 => TextureFormat::R8Unorm,
        GlyphBitmapFormat::Rgba8Premultiplied => TextureFormat::Rgba8UnormSrgb,
    }
}

impl ShelfGlyphAtlas {
    /// 설정을 검증해 빈 atlas를 만듭니다. 현재 frame은 0입니다.
    pub fn new(config: AtlasConfig) -> Result<ShelfGlyphAtlas, AtlasError> {
        config.validate()?;
        Ok(ShelfGlyphAtlas {
            config,
            slots: Vec::new(),
            glyphs: HashMap::new(),
            current_frame: 0,
            gpu_completed: None,
            stats: AtlasStats::default(),
        })
    }

    /// 설정.
    pub fn config(&self) -> AtlasConfig {
        self.config
    }

    /// 현재 frame.
    pub fn current_frame(&self) -> u64 {
        self.current_frame
    }

    /// 통계.
    pub fn stats(&self) -> AtlasStats {
        AtlasStats {
            pages: self.page_count(),
            glyphs: self.glyphs.len(),
            ..self.stats
        }
    }

    /// 현재 page 수.
    pub fn page_count(&self) -> u32 {
        self.slots.iter().filter(|slot| slot.page.is_some()).count() as u32
    }

    /// page texture가 차지하는 byte 합.
    pub fn memory_bytes(&self) -> u64 {
        let side = u64::from(self.config.page_size);
        self.slots
            .iter()
            .filter_map(|slot| slot.page.as_ref())
            .map(|page| side * side * page.format.bytes_per_pixel() as u64)
            .sum()
    }

    /// page의 bitmap format(page가 없으면 `None`).
    pub fn page_format(&self, page: AtlasPageId) -> Option<GlyphBitmapFormat> {
        self.slots
            .get(page.0 as usize)
            .and_then(|slot| slot.page.as_ref())
            .map(|page| page.format)
    }

    /// page texture descriptor(backend `create_texture` 입력).
    pub fn page_texture_desc(&self, format: GlyphBitmapFormat) -> GpuTextureDesc {
        GpuTextureDesc {
            width: self.config.page_size,
            height: self.config.page_size,
            format: texture_format(format),
            mip_levels: 1,
            samples: SampleCount::X1,
            usage: TextureUsage {
                sampled: true,
                render_target: false,
            },
        }
    }

    /// `placement`의 bitmap을 gutter까지 0으로 채운 cell 전체 upload 요청으로 만듭니다.
    pub fn upload_request(
        &self,
        texture: TextureHandle,
        placement: &AtlasPlacement,
        bitmap: &GlyphBitmap,
    ) -> Result<UploadRequest, AtlasError> {
        if bitmap.format() != placement.format
            || bitmap.width() != placement.width
            || bitmap.height() != placement.height
        {
            return Err(AtlasError::FormatMismatch);
        }
        let pad = self.config.padding;
        let bpp = bitmap.format().bytes_per_pixel();
        let cell_w = (placement.width + 2 * pad) as usize;
        let cell_h = (placement.height + 2 * pad) as usize;
        let mut data = vec![0u8; cell_w * cell_h * bpp];
        let row = placement.width as usize * bpp;
        for (y, src) in bitmap.pixels().chunks_exact(row).enumerate() {
            let start = ((y + pad as usize) * cell_w + pad as usize) * bpp;
            data[start..start + row].copy_from_slice(src);
        }
        Ok(UploadRequest {
            target: UploadTarget::Texture {
                texture,
                region: TextureRegion {
                    mip_level: 0,
                    x: placement.x - pad,
                    y: placement.y - pad,
                    width: cell_w as u32,
                    height: cell_h as u32,
                },
                format: texture_format(bitmap.format()),
            },
            data: Arc::from(data),
        })
    }

    /// page를 비울 수 있으면 참입니다: 현재 frame에서 쓰지 않았고, 마지막 사용 frame을 GPU가 끝냈습니다.
    fn evictable(&self, page: &PageState) -> bool {
        match page.last_used {
            None => true,
            Some(frame) => {
                frame < self.current_frame && self.gpu_completed.is_some_and(|done| frame <= done)
            }
        }
    }

    /// 비울 수 있는 page 중 가장 오래 쓰지 않은 page(같으면 id 작은 쪽).
    fn lru_evictable(&self) -> Option<usize> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.page.as_ref().map(|page| (index, page)))
            .filter(|(_, page)| self.evictable(page))
            .min_by_key(|(index, page)| (page.last_used, *index))
            .map(|(index, _)| index)
    }

    /// slot의 page를 없애고 세대를 올립니다(그 page의 glyph·placement는 모두 무효).
    fn evict(&mut self, index: usize) {
        let Some(slot) = self.slots.get_mut(index) else {
            return;
        };
        let Some(page) = slot.page.take() else {
            return;
        };
        slot.generation = slot.generation.wrapping_add(1);
        for key in &page.glyphs {
            self.glyphs.remove(key);
        }
        self.stats.evicted_pages += 1;
        self.stats.evicted_glyphs += page.glyphs.len() as u64;
    }

    fn mark_used(&mut self, page: AtlasPageId) {
        let frame = self.current_frame;
        if let Some(state) = self
            .slots
            .get_mut(page.0 as usize)
            .and_then(|slot| slot.page.as_mut())
        {
            state.last_used = Some(state.last_used.map_or(frame, |old| old.max(frame)));
        }
    }

    /// 새 page를 만들 slot(빈 slot 재사용, 없으면 추가).
    fn new_page(&mut self, format: GlyphBitmapFormat) -> usize {
        if let Some(index) = self.slots.iter().position(|slot| slot.page.is_none()) {
            if let Some(slot) = self.slots.get_mut(index) {
                slot.page = Some(PageState::new(format));
            }
            return index;
        }
        self.slots.push(PageSlot {
            generation: 0,
            page: Some(PageState::new(format)),
        });
        self.slots.len() - 1
    }

    /// memory pressure: 예산을 `max_pages`(1 이상)로 바꾸고, 넘는 page를 오래된 순으로 없앱니다.
    pub fn apply_memory_pressure(&mut self, max_pages: u32) -> TrimOutcome {
        self.config.max_pages = max_pages.clamp(1, MAX_ATLAS_PAGES);
        let mut outcome = TrimOutcome::default();
        while self.page_count() > self.config.max_pages {
            match self.lru_evictable() {
                Some(index) => {
                    self.evict(index);
                    outcome.evicted_pages += 1;
                }
                None => break,
            }
        }
        outcome.remaining_pages = self.page_count();
        outcome.protected_over_budget = outcome
            .remaining_pages
            .saturating_sub(self.config.max_pages);
        outcome
    }

    fn place(
        &mut self,
        index: usize,
        key: &GlyphKey,
        bitmap: &GlyphBitmap,
        cell: (u32, u32),
    ) -> Option<AtlasPlacement> {
        let page_size = self.config.page_size;
        let pad = self.config.padding;
        let slot = self.slots.get_mut(index)?;
        let generation = slot.generation;
        let page = slot.page.as_mut()?;
        if page.format != bitmap.format() {
            return None;
        }
        let (x, y) = page.allocate(cell.0, cell.1, page_size)?;
        page.glyphs.push(key.clone());
        Some(AtlasPlacement {
            page: AtlasPageId(index as u32),
            generation,
            x: x + pad,
            y: y + pad,
            width: bitmap.width(),
            height: bitmap.height(),
            left: bitmap.left(),
            top: bitmap.top(),
            format: bitmap.format(),
        })
    }
}

impl GlyphAtlas for ShelfGlyphAtlas {
    fn begin_frame(&mut self, frame: u64) -> Result<(), AtlasError> {
        if frame <= self.current_frame {
            return Err(AtlasError::FrameOrder);
        }
        self.current_frame = frame;
        Ok(())
    }

    fn gpu_completed(&mut self, frame: u64) {
        self.gpu_completed = Some(self.gpu_completed.map_or(frame, |old| old.max(frame)));
    }

    fn lookup(&mut self, key: &GlyphKey) -> Option<AtlasPlacement> {
        match self.glyphs.get(key).copied() {
            Some(placement) => {
                self.stats.hits += 1;
                self.mark_used(placement.page);
                Some(placement)
            }
            None => {
                self.stats.misses += 1;
                None
            }
        }
    }

    fn insert(
        &mut self,
        key: GlyphKey,
        bitmap: &GlyphBitmap,
    ) -> Result<AtlasPlacement, AtlasError> {
        if key.mode.bitmap_format() != Some(bitmap.format()) {
            return Err(AtlasError::FormatMismatch);
        }
        if let Some(placement) = self.glyphs.get(&key).copied() {
            self.mark_used(placement.page);
            return Ok(placement);
        }
        let pad = self.config.padding;
        let cell = (bitmap.width() + 2 * pad, bitmap.height() + 2 * pad);
        if cell.0 > self.config.page_size || cell.1 > self.config.page_size {
            return Err(AtlasError::TooLarge {
                width: bitmap.width(),
                height: bitmap.height(),
            });
        }
        // 1) 같은 format page를 id 순서로.
        let mut placed = None;
        for index in 0..self.slots.len() {
            placed = self.place(index, &key, bitmap, cell);
            if placed.is_some() {
                break;
            }
        }
        // 2) 예산 안이면 새 page.
        if placed.is_none() && self.page_count() < self.config.max_pages {
            let index = self.new_page(bitmap.format());
            placed = self.place(index, &key, bitmap, cell);
        }
        // 3) 가장 오래 쓰지 않은 비울 수 있는 page를 비우고 그 자리에.
        if placed.is_none() {
            if let Some(index) = self.lru_evictable() {
                self.evict(index);
                let index = self.new_page(bitmap.format());
                placed = self.place(index, &key, bitmap, cell);
            }
        }
        let Some(placement) = placed else {
            self.stats.full += 1;
            return Err(AtlasError::Full);
        };
        self.glyphs.insert(key, placement);
        self.mark_used(placement.page);
        self.stats.inserted += 1;
        Ok(placement)
    }

    fn is_valid(&self, placement: &AtlasPlacement) -> bool {
        self.slots
            .get(placement.page.0 as usize)
            .is_some_and(|slot| slot.page.is_some() && slot.generation == placement.generation)
    }
}

#[cfg(test)]
mod tests;
