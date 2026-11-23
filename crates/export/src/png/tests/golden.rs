//! decoded pixel golden 비교 (XX.2.4, ADR-0103 결정 6).
//!
//! 같은 snapshot·옵션의 PNG 출력을 해석한 pixel이 저장한 golden PNG의 pixel과 채널마다 [`GOLDEN_TOLERANCE`] 안에서
//! 같아야 합니다. bytes가 아니라 해석한 pixel을 비교하므로 encoder 압축 선택이 바뀌어도 그림이 같으면 통과하고,
//! transfer 함수(`powf`)의 플랫폼 libm 차이(±1 code value)를 허용합니다. 색 정보 chunk(형식·순서)는 정확히 같아야
//! 합니다.
//!
//! golden 갱신: `STELLAR_PNG_BLESS=1 cargo test -p stellar-export --lib png::tests::golden`. 갱신한 파일은
//! `crates/export/tests/fixtures/png/`에 있고 diff로 검토합니다.

use super::*;

/// 채널별 허용 오차(8-bit code value).
const GOLDEN_TOLERANCE: u8 = 1;

/// golden 하나.
struct Case {
    name: &'static str,
    export: ConfirmedExport,
    target: SoftwareTarget,
    options: PngOptions,
}

fn cases() -> Vec<Case> {
    let p3 = PngOptions::new(OutputProfile::DisplayP3, PngBitDepth::Eight).unwrap();
    let basic_target = SoftwareTarget::new(Arc::new(assets()), Arc::new(Glyphs::default()));
    vec![
        Case {
            name: "showcase_srgb_1x",
            export: ConfirmedExport::assume_confirmed(capture_of(
                showcase_roots(),
                request(1.0, ExportBackground::Transparent),
            )),
            target: showcase_target(),
            options: PngOptions::default(),
        },
        Case {
            name: "showcase_p3_2x",
            export: ConfirmedExport::assume_confirmed(capture_of(
                showcase_roots(),
                request(2.0, ExportBackground::Transparent),
            )),
            target: showcase_target(),
            options: p3,
        },
        Case {
            name: "basic_srgb_1_5x_background",
            export: ConfirmedExport::assume_confirmed(capture_of(
                fx::basic_roots(&fx::Basic::default()),
                request(1.5, ExportBackground::Solid(srgb(0.85, 0.9, 1.0, 1.0))),
            )),
            target: basic_target,
            options: PngOptions::default(),
        },
    ]
}

fn golden_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/png")
        .join(format!("{name}.png"))
}

/// 두 해석 결과의 채널 차이가 허용 오차를 넘는 pixel 위치(최대 `limit`개)와 전체 수.
fn exceeding(a: &DecodedPng, b: &DecodedPng, limit: usize) -> (Vec<(u32, u32)>, usize) {
    let mut positions = Vec::new();
    let mut count = 0;
    for (index, (pa, pb)) in a.rgba.chunks(4).zip(b.rgba.chunks(4)).enumerate() {
        if pa
            .iter()
            .zip(pb)
            .any(|(x, y)| x.abs_diff(*y) > GOLDEN_TOLERANCE)
        {
            count += 1;
            if positions.len() < limit {
                let i = index as u32;
                positions.push((i % a.width, i / a.width));
            }
        }
    }
    (positions, count)
}

#[test]
fn decoded_pixels_match_golden_within_tolerance() {
    let bless = std::env::var_os("STELLAR_PNG_BLESS").is_some();
    for case in cases() {
        let file = render_png(
            &case.export,
            &case.target,
            case.options,
            &CancelToken::new(),
        )
        .unwrap_or_else(|error| panic!("{}: {error}", case.name));
        // 같은 snapshot·옵션이면 bytes도 같습니다(결정론).
        let again = render_png(
            &case.export,
            &case.target,
            case.options,
            &CancelToken::new(),
        )
        .unwrap();
        assert_eq!(file.bytes(), again.bytes(), "{}", case.name);
        let path = golden_path(case.name);
        if bless {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, file.bytes()).unwrap();
            continue;
        }
        let golden = std::fs::read(&path).unwrap_or_else(|_| {
            panic!(
                "{} golden이 없습니다. STELLAR_PNG_BLESS=1로 만든 뒤 검토하세요",
                path.display()
            )
        });
        let expected = decode_png(&golden, DecodeLimits::default()).unwrap();
        let actual = decode_png(file.bytes(), DecodeLimits::default()).unwrap();
        assert_eq!(
            (actual.width, actual.height),
            (expected.width, expected.height),
            "{} 크기",
            case.name
        );
        assert_eq!(actual.chunks, expected.chunks, "{} chunk 구성", case.name);
        assert_eq!(actual.color, expected.color, "{} 색 정보", case.name);
        let (positions, count) = exceeding(&actual, &expected, 16);
        assert_eq!(
            count, 0,
            "{}: 허용 오차 {GOLDEN_TOLERANCE} 초과 pixel {count}개, 처음 위치 {positions:?}",
            case.name
        );
    }
}

#[test]
fn golden_cases_cover_both_profiles_and_scales() {
    let cases = cases();
    let profiles: std::collections::BTreeSet<&str> =
        cases.iter().map(|c| c.options.profile().name()).collect();
    assert_eq!(profiles.len(), 2);
    let scales: std::collections::BTreeSet<u32> = cases
        .iter()
        .map(|c| (c.export.capture().request().scale().get() * 10.0) as u32)
        .collect();
    assert!(scales.len() >= 3);
}

#[test]
fn tolerance_comparison_finds_changed_pixels() {
    let image = sample_image(OutputProfile::Srgb);
    let base = decode_png(
        &encode_png(&image, 1.0, &CancelToken::new()).unwrap(),
        DecodeLimits::default(),
    )
    .unwrap();
    let mut near = base.clone();
    near.rgba[0] = near.rgba[0].saturating_add(1);
    assert_eq!(exceeding(&base, &near, 4).1, 0);
    let mut far = base.clone();
    far.rgba[4 * 5 + 2] = far.rgba[4 * 5 + 2].wrapping_add(9);
    assert_eq!(exceeding(&base, &far, 4), (vec![(1, 1)], 1));
}
