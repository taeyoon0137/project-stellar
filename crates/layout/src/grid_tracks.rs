//! Grid 명시 track sizing (X.4.2).
//!
//! 한 축(열 또는 행)의 track 크기를 정합니다. item 기여 크기는 호출자가 넘깁니다(배치 X.4.3,
//! 텍스트·이미지 intrinsic hook X.4.5).
//!
//! # 규칙
//!
//! 각 track은 (최솟값, 최댓값) 쌍으로 봅니다: `Fixed(v)` = (v, v), `Fraction(f)` = (0, f fr),
//! `MinContent`·`MaxContent` = (콘텐츠, 콘텐츠), `MinMax(a, b)` = (a, b).
//!
//! item은 축마다 **min-content 기여**와 **max-content 기여** 두 값을 줍니다(X.4.5). 텍스트는 가장 넓은
//! 단어와 한 줄 폭, 이미지·고정 콘텐츠는 두 값이 같습니다([`crate::intrinsic`]). `MinContent` 경계는
//! min-content 기여, `MaxContent` 경계는 max-content 기여를 씁니다.
//!
//! 1. **초기화**: base = 최솟값(고정 값 또는 span 1 item의 해당 기여 최댓값), limit = 최댓값(고정 값,
//!    해당 콘텐츠 기여, fr은 무한). limit < base면 limit = base. 따라서 `minmax(min-content,
//!    max-content)`는 min-content에서 시작해 공간이 남으면 max-content까지 늘어납니다.
//! 2. **span 여러 칸 item**: fr track을 걸치지 않는 item의 기여가 걸친 track base 합 + gap보다 크면
//!    부족분을 걸친 track 중 콘텐츠 최솟값 track(없으면 콘텐츠 최댓값 track)에 균등 분배합니다
//!    (span 오름차순, 같은 span은 item 순서). 대상 track 중 `MaxContent` 경계가 있으면 max-content
//!    기여, 없으면 min-content 기여를 씁니다. 잔여 subpixel은 track 순서.
//! 3. **maximize**: 가용 크기가 definite이고 남으면 base < limit인 비fr track을 limit까지 균등하게
//!    늘립니다.
//! 4. **fr**: 가용 크기가 definite면 남은 공간(가용 − gap − 비fr track 합)을 fr 비례로 나눕니다.
//!    몫이 base보다 작은 fr track은 base로 고정하고 나머지로 다시 나눕니다. 잔여 subpixel은 track
//!    순서. 가용 크기가 indefinite면 `fr 단위 = max(span 1 item max-content 기여 / fr, base / fr)`로 정하고 각 fr
//!    track을 `max(base, 단위 × fr)`로 둡니다.
//!
//! # 작업량 상한
//!
//! track sizing은 반복(maximize·fr freeze) 한 바퀴마다 track 수만큼, span 여러 칸 item은 걸친 track
//! 수만큼 [`WorkBudget`]을 소모합니다. 상한을 넘으면 [`TrackError::WorkLimit`]입니다. span 1 기여는
//! item 목록을 한 번만 훑어 미리 모으므로 track 수 × item 수 비용이 들지 않습니다.

use crate::distribute::{distribute_equal, distribute_weighted};
use crate::rounding::{LayoutUnit, LayoutUnitError};

/// grid 계산 한 번의 결정적 작업량 상한 (crate 내부).
///
/// 단위는 cell 검사·track 갱신 같은 기본 연산 하나입니다. 같은 입력은 플랫폼·빌드와 무관하게 같은
/// 양을 소모하므로 상한 초과 여부도 결정적입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkBudget {
    remaining: u64,
}

/// 작업량 상한을 넘었습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkExceeded;

impl WorkBudget {
    /// `limit` 단위의 예산.
    pub(crate) const fn new(limit: u64) -> WorkBudget {
        WorkBudget { remaining: limit }
    }

    /// `units`만큼 소모합니다. 남은 양보다 많으면 오류이며 남은 양은 0이 됩니다.
    pub(crate) fn charge(&mut self, units: usize) -> Result<(), WorkExceeded> {
        // usize → u64는 지원 플랫폼(64비트)에서 손실이 없습니다.
        let units = units as u64;
        match self.remaining.checked_sub(units) {
            Some(rest) => {
                self.remaining = rest;
                Ok(())
            }
            None => {
                self.remaining = 0;
                Err(WorkExceeded)
            }
        }
    }
}

/// track sizing 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackError {
    /// 수치 범위 오류.
    Arithmetic(LayoutUnitError),
    /// 작업량 상한 초과.
    WorkLimit,
}

impl From<LayoutUnitError> for TrackError {
    fn from(error: LayoutUnitError) -> TrackError {
        TrackError::Arithmetic(error)
    }
}

impl From<WorkExceeded> for TrackError {
    fn from(_: WorkExceeded) -> TrackError {
        TrackError::WorkLimit
    }
}

/// track 경계(변환됨).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Breadth {
    /// 고정 크기.
    Fixed(LayoutUnit),
    /// fr(1/64 단위 가중치).
    Fraction(u64),
    /// 최소 콘텐츠.
    MinContent,
    /// 최대 콘텐츠.
    MaxContent,
}

impl Breadth {
    pub(crate) const fn is_content(self) -> bool {
        matches!(self, Breadth::MinContent | Breadth::MaxContent)
    }
}

/// 변환된 track(최솟값, 최댓값).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Track {
    pub(crate) min: Breadth,
    pub(crate) max: Breadth,
}

impl Track {
    /// fr 가중치(0이면 비fr track).
    const fn flex(self) -> u64 {
        match self.max {
            Breadth::Fraction(weight) => weight,
            _ => 0,
        }
    }

    pub(crate) const fn is_flexible(self) -> bool {
        matches!(self.max, Breadth::Fraction(_))
    }
}

/// track 축에 놓인 item 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TrackItem {
    /// 시작 track index(0부터).
    pub(crate) start: usize,
    /// 걸친 track 수(1 이상).
    pub(crate) span: usize,
    /// 이 축의 min-content 기여 크기.
    pub(crate) min_contribution: LayoutUnit,
    /// 이 축의 max-content 기여 크기.
    pub(crate) max_contribution: LayoutUnit,
}

impl TrackItem {
    /// 두 기여가 같은 item(고정 콘텐츠 등).
    #[cfg(test)]
    pub(crate) const fn fixed(start: usize, span: usize, contribution: LayoutUnit) -> TrackItem {
        TrackItem {
            start,
            span,
            min_contribution: contribution,
            max_contribution: contribution,
        }
    }
}

/// track마다 그 track에 놓인 span 1 item의 (min-content, max-content) 기여 최댓값(item 목록 한 번 순회).
fn single_span_contents(items: &[TrackItem], count: usize) -> Vec<(LayoutUnit, LayoutUnit)> {
    let mut contents = vec![(LayoutUnit::ZERO, LayoutUnit::ZERO); count];
    for item in items {
        if item.span == 1 && item.start < count {
            let slot = &mut contents[item.start];
            slot.0 = slot.0.max(item.min_contribution);
            slot.1 = slot.1.max(item.max_contribution);
        }
    }
    contents
}

/// 경계가 쓰는 span 1 콘텐츠 기여.
fn breadth_content(breadth: Breadth, single: (LayoutUnit, LayoutUnit)) -> LayoutUnit {
    match breadth {
        Breadth::MinContent => single.0,
        _ => single.1,
    }
}

/// track 크기(track 순서)를 계산합니다. `available`은 content 크기(indefinite면 `None`).
pub(crate) fn size_tracks(
    tracks: &[Track],
    gap: LayoutUnit,
    available: Option<LayoutUnit>,
    items: &[TrackItem],
    budget: &mut WorkBudget,
) -> Result<Vec<LayoutUnit>, TrackError> {
    let count = tracks.len();
    budget.charge(count.saturating_add(items.len()))?;
    let single = single_span_contents(items, count);
    // 1) 초기화.
    let mut base = Vec::with_capacity(count);
    let mut limit: Vec<Option<LayoutUnit>> = Vec::with_capacity(count);
    for (i, track) in tracks.iter().enumerate() {
        let lo = match track.min {
            Breadth::Fixed(value) => value,
            Breadth::MinContent | Breadth::MaxContent => breadth_content(track.min, single[i]),
            Breadth::Fraction(_) => LayoutUnit::ZERO,
        };
        let hi = match track.max {
            Breadth::Fixed(value) => Some(value.max(lo)),
            Breadth::MinContent | Breadth::MaxContent => {
                Some(breadth_content(track.max, single[i]).max(lo))
            }
            Breadth::Fraction(_) => None,
        };
        base.push(lo);
        limit.push(hi);
    }
    let mut gaps_total = LayoutUnit::ZERO;
    for _ in 1..count {
        gaps_total = gaps_total.checked_add(gap)?;
    }

    // 2) span 여러 칸 item(fr track을 걸치지 않는 것만).
    let mut spanning: Vec<&TrackItem> = items
        .iter()
        .filter(|item| item.span > 1 && item.start + item.span <= count)
        .filter(|item| {
            !tracks[item.start..item.start + item.span]
                .iter()
                .any(|t| t.is_flexible())
        })
        .collect();
    spanning.sort_by_key(|item| item.span);
    for item in spanning {
        // 걸친 track 합·대상 선택(위 fr 검사 포함)은 span에 비례합니다.
        budget.charge(item.span.saturating_mul(2))?;
        let range = item.start..item.start + item.span;
        let mut covered = LayoutUnit::ZERO;
        for i in range.clone() {
            covered = covered.checked_add(base[i])?;
        }
        for _ in 1..item.span {
            covered = covered.checked_add(gap)?;
        }
        let mut targets: Vec<usize> = range
            .clone()
            .filter(|&i| tracks[i].min.is_content())
            .collect();
        if targets.is_empty() {
            targets = range.filter(|&i| tracks[i].max.is_content()).collect();
        }
        if targets.is_empty() {
            continue;
        }
        let wants_max = targets.iter().any(|&i| {
            matches!(tracks[i].min, Breadth::MaxContent)
                || matches!(tracks[i].max, Breadth::MaxContent)
        });
        let contribution = if wants_max {
            item.max_contribution
        } else {
            item.min_contribution
        };
        let deficit = contribution.checked_sub(covered)?;
        if deficit <= LayoutUnit::ZERO {
            continue;
        }
        for (i, add) in targets.iter().zip(distribute_equal(deficit, targets.len())) {
            base[*i] = base[*i].checked_add(add)?;
            if let Some(hi) = limit[*i] {
                limit[*i] = Some(hi.max(base[*i]));
            }
        }
    }

    // 3) maximize: 남는 공간으로 비fr track을 limit까지.
    if let Some(extent) = available {
        for _ in 0..=count {
            budget.charge(count)?;
            let mut used = gaps_total;
            for value in &base {
                used = used.checked_add(*value)?;
            }
            let free = extent.checked_sub(used)?;
            let growable: Vec<usize> = (0..count)
                .filter(|&i| !tracks[i].is_flexible() && limit[i].is_some_and(|hi| base[i] < hi))
                .collect();
            if free <= LayoutUnit::ZERO || growable.is_empty() {
                break;
            }
            // 가장 작은 남은 여유와 균등 몫 중 작은 값까지 늘리고 반복합니다.
            let shares = distribute_equal(free, growable.len());
            let mut capped = false;
            for (i, share) in growable.iter().zip(shares) {
                let room = limit[*i].unwrap_or(base[*i]).checked_sub(base[*i])?;
                let add = share.min(room);
                capped |= share > room;
                base[*i] = base[*i].checked_add(add)?;
            }
            if !capped {
                break;
            }
        }
    }

    // 4) fr track.
    let flexible: Vec<usize> = (0..count).filter(|&i| tracks[i].flex() > 0).collect();
    if flexible.is_empty() {
        return Ok(base);
    }
    let mut is_flexible = vec![false; count];
    for &i in &flexible {
        is_flexible[i] = true;
    }
    match available {
        Some(extent) => {
            let mut inflexible = vec![false; count];
            for _ in 0..=count {
                budget.charge(count)?;
                let mut leftover = extent.checked_sub(gaps_total)?;
                for i in 0..count {
                    if !is_flexible[i] || inflexible[i] {
                        leftover = leftover.checked_sub(base[i])?;
                    }
                }
                let weights: Vec<u64> = (0..count)
                    .map(|i| {
                        if is_flexible[i] && !inflexible[i] {
                            tracks[i].flex()
                        } else {
                            0
                        }
                    })
                    .collect();
                let shares = distribute_weighted(leftover.max(LayoutUnit::ZERO), &weights)?;
                let mut changed = false;
                for &i in &flexible {
                    if !inflexible[i] && shares[i] < base[i] {
                        inflexible[i] = true;
                        changed = true;
                    }
                }
                if !changed {
                    for &i in &flexible {
                        if !inflexible[i] {
                            base[i] = shares[i];
                        }
                    }
                    break;
                }
            }
            Ok(base)
        }
        None => {
            // fr 단위 = max(기여 / fr, base / fr)를 유리수 (분자, 분모)로 비교합니다.
            let mut best: Option<(i128, i128)> = None;
            for &i in &flexible {
                let weight = i128::from(tracks[i].flex());
                let content = single[i].1.max(base[i]);
                let candidate = (i128::from(content.raw()), weight);
                best = match best {
                    Some((num, den)) if num * candidate.1 >= candidate.0 * den => Some((num, den)),
                    _ => Some(candidate),
                };
            }
            if let Some((num, den)) = best {
                let out_of_range = |_| TrackError::Arithmetic(LayoutUnitError::OutOfRange);
                let unit = LayoutUnit::from_raw(i32::try_from(num).map_err(out_of_range)?);
                let den = i64::try_from(den).map_err(out_of_range)?;
                for &i in &flexible {
                    let weight = i64::try_from(tracks[i].flex()).map_err(out_of_range)?;
                    base[i] = base[i].max(unit.checked_mul_ratio(weight, den)?);
                }
            }
            Ok(base)
        }
    }
}

/// track 시작 offset(track 순서, content 원점 기준)과 전체 extent.
pub(crate) fn track_offsets(
    sizes: &[LayoutUnit],
    gap: LayoutUnit,
) -> Result<(Vec<LayoutUnit>, LayoutUnit), LayoutUnitError> {
    let mut offsets = Vec::with_capacity(sizes.len());
    let mut cursor = LayoutUnit::ZERO;
    for (i, size) in sizes.iter().enumerate() {
        if i > 0 {
            cursor = cursor.checked_add(gap)?;
        }
        offsets.push(cursor);
        cursor = cursor.checked_add(*size)?;
    }
    Ok((offsets, cursor))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(value: i32) -> LayoutUnit {
        LayoutUnit::from_px_int(value).unwrap()
    }

    /// 넉넉한 예산으로 track sizing.
    fn size(
        tracks: &[Track],
        gap: LayoutUnit,
        available: Option<LayoutUnit>,
        items: &[TrackItem],
    ) -> Result<Vec<LayoutUnit>, TrackError> {
        size_tracks(
            tracks,
            gap,
            available,
            items,
            &mut WorkBudget::new(u64::MAX),
        )
    }

    fn fixed(value: i32) -> Track {
        Track {
            min: Breadth::Fixed(px(value)),
            max: Breadth::Fixed(px(value)),
        }
    }

    fn fr(weight: u64) -> Track {
        Track {
            min: Breadth::Fixed(LayoutUnit::ZERO),
            max: Breadth::Fraction(weight * 64),
        }
    }

    fn content() -> Track {
        Track {
            min: Breadth::MaxContent,
            max: Breadth::MaxContent,
        }
    }

    fn item(start: usize, span: usize, contribution: i32) -> TrackItem {
        TrackItem::fixed(start, span, px(contribution))
    }

    fn raw(values: &[LayoutUnit]) -> Vec<i32> {
        values.iter().map(LayoutUnit::raw).collect()
    }

    fn as_px(values: &[LayoutUnit]) -> Vec<f64> {
        values.iter().map(LayoutUnit::to_px_f64).collect()
    }

    #[test]
    fn fixed_and_fraction_fixture() {
        // [100, 1fr, 2fr], 가용 400, gap 10 → 남은 280 px = 17920 subpixel을 1:2.
        // 5973.33 → 5973(+잔여 1 → 5974), 11946.67 → 11946.
        let sizes = size(&[fixed(100), fr(1), fr(2)], px(10), Some(px(400)), &[]).unwrap();
        assert_eq!(raw(&sizes), vec![6400, 5974, 11946]);
        let (offsets, extent) = track_offsets(&sizes, px(10)).unwrap();
        assert_eq!(raw(&offsets), vec![0, 6400 + 640, 6400 + 640 + 5974 + 640]);
        assert_eq!(extent, px(400));
        // 고정 track만: 가용보다 커도 줄이지 않음(넘침).
        let sizes = size(&[fixed(300), fixed(200)], px(0), Some(px(400)), &[]).unwrap();
        assert_eq!(as_px(&sizes), vec![300.0, 200.0]);
        // fr만, 가용 300, gap 0 → 1:1:1 = 100.
        let sizes = size(&[fr(1), fr(1), fr(1)], px(0), Some(px(300)), &[]).unwrap();
        assert_eq!(as_px(&sizes), vec![100.0, 100.0, 100.0]);
        // 고정이 가용을 다 쓰면 fr은 0.
        let sizes = size(&[fixed(400), fr(1)], px(0), Some(px(300)), &[]).unwrap();
        assert_eq!(as_px(&sizes), vec![400.0, 0.0]);
    }

    #[test]
    fn minmax_fraction_freezes_at_base() {
        // minmax(150, 1fr) + 1fr in 200: 몫 100 < 150 → 150 고정, 나머지 50.
        let track = Track {
            min: Breadth::Fixed(px(150)),
            max: Breadth::Fraction(64),
        };
        let sizes = size(&[track, fr(1)], px(0), Some(px(200)), &[]).unwrap();
        assert_eq!(as_px(&sizes), vec![150.0, 50.0]);
        // maximize: minmax(20, 50) + 고정 10 in 100 → 50까지 늘어나고 나머지는 빈 공간.
        let growable = Track {
            min: Breadth::Fixed(px(20)),
            max: Breadth::Fixed(px(50)),
        };
        let sizes = size(&[growable, fixed(10)], px(0), Some(px(100)), &[]).unwrap();
        assert_eq!(as_px(&sizes), vec![50.0, 10.0]);
    }

    #[test]
    fn content_tracks_and_spanning_items() {
        // content track: span 1 기여 최댓값. span 2 item(70)은 부족분 10을 두 content track에 5씩.
        let sizes = size(
            &[content(), content(), fixed(20)],
            px(0),
            None,
            &[
                item(0, 1, 30),
                item(0, 1, 25),
                item(1, 1, 30),
                item(0, 2, 70),
            ],
        )
        .unwrap();
        assert_eq!(as_px(&sizes), vec![35.0, 35.0, 20.0]);
        // fr를 걸친 span item은 intrinsic 단계에서 무시.
        let sizes = size(
            &[content(), fr(1)],
            px(0),
            Some(px(100)),
            &[item(0, 2, 500)],
        )
        .unwrap();
        assert_eq!(as_px(&sizes), vec![0.0, 100.0]);
    }

    #[test]
    fn minmax_content_minimum_grows_to_limit() {
        // minmax(min-content, 100) with item 30 in 가용 80 → base 30에서 maximize로 80.
        let track = Track {
            min: Breadth::MinContent,
            max: Breadth::Fixed(px(100)),
        };
        let sizes = size(&[track], px(0), Some(px(80)), &[item(0, 1, 30)]).unwrap();
        assert_eq!(as_px(&sizes), vec![80.0]);
        // indefinite면 base(콘텐츠 30) 그대로.
        let sizes = size(&[track], px(0), None, &[item(0, 1, 30)]).unwrap();
        assert_eq!(as_px(&sizes), vec![30.0]);
    }

    #[test]
    fn indefinite_fraction_uses_largest_fr_unit() {
        // 가용 indefinite: 1fr(기여 30), 2fr(기여 40) → 단위 max(30/1, 40/2) = 30 → 30, 60.
        let sizes = size(
            &[fr(1), fr(2)],
            px(5),
            None,
            &[item(0, 1, 30), item(1, 1, 40)],
        )
        .unwrap();
        assert_eq!(as_px(&sizes), vec![30.0, 60.0]);
        // item이 없으면 0.
        let sizes = size(&[fr(1), fixed(7)], px(0), None, &[]).unwrap();
        assert_eq!(as_px(&sizes), vec![0.0, 7.0]);
    }

    #[test]
    fn work_budget_is_deterministic_and_bounded() {
        let mut budget = WorkBudget::new(5);
        assert_eq!(budget.charge(3), Ok(()));
        assert_eq!(budget.charge(3), Err(WorkExceeded));
        // 초과 뒤에는 남은 양이 0이라 계속 실패합니다.
        assert_eq!(budget.charge(1), Err(WorkExceeded));
        assert_eq!(budget.charge(0), Ok(()));
        // track sizing은 track·item 수에 비례해 소모하고 상한을 넘으면 WorkLimit입니다.
        let tracks = vec![fr(1); 100];
        let items: Vec<TrackItem> = (0..100).map(|i| item(i, 1, 1)).collect();
        assert_eq!(
            size_tracks(
                &tracks,
                px(0),
                Some(px(1000)),
                &items,
                &mut WorkBudget::new(150)
            ),
            Err(TrackError::WorkLimit)
        );
        let mut enough = WorkBudget::new(10_000);
        let sizes = size_tracks(&tracks, px(0), Some(px(1000)), &items, &mut enough).unwrap();
        assert!(sizes.iter().all(|s| *s == px(10)));
    }

    #[test]
    fn min_and_max_content_contributions_feed_matching_breadths() {
        // item 기여 (min 20, max 70): MinContent track 20, MaxContent track 70,
        // minmax(min, max)는 20에서 가용 50까지 늘어남.
        let both = |start: usize, span: usize| TrackItem {
            start,
            span,
            min_contribution: px(20),
            max_contribution: px(70),
        };
        let min_track = Track {
            min: Breadth::MinContent,
            max: Breadth::MinContent,
        };
        let range = Track {
            min: Breadth::MinContent,
            max: Breadth::MaxContent,
        };
        let sizes = size(
            &[min_track, content(), range],
            px(0),
            None,
            &[both(0, 1), both(1, 1), both(2, 1)],
        )
        .unwrap();
        assert_eq!(as_px(&sizes), vec![20.0, 70.0, 20.0]);
        let sizes = size(&[range], px(0), Some(px(50)), &[both(0, 1)]).unwrap();
        assert_eq!(as_px(&sizes), vec![50.0]);
        // span 2 item: 대상이 모두 MinContent면 min 기여(20 − base 10 → 5씩), MaxContent면 max 기여(70 − 10 → 30씩).
        let sizes = size(
            &[min_track, min_track],
            px(0),
            None,
            &[both(0, 2), item(0, 1, 5), item(1, 1, 5)],
        )
        .unwrap();
        assert_eq!(as_px(&sizes), vec![10.0, 10.0]);
        let sizes = size(
            &[content(), content()],
            px(0),
            None,
            &[both(0, 2), item(0, 1, 5), item(1, 1, 5)],
        )
        .unwrap();
        assert_eq!(as_px(&sizes), vec![35.0, 35.0]);
        // indefinite fr 단위는 max-content 기여.
        let sizes = size(&[fr(1)], px(0), None, &[both(0, 1)]).unwrap();
        assert_eq!(as_px(&sizes), vec![70.0]);
    }
}
