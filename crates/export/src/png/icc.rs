//! ICC profile 생성과 primaries 판정 (XX.2.2).
//!
//! - [`display_p3_profile`]: Display P3(D65 white, sRGB transfer 곡선) ICC v4.3 display profile입니다. PNG `iCCP`
//!   chunk에 넣습니다. 값은 CIE 1931 xy primaries에서 계산하고 Bradford로 PCS 백색점(D50)에 맞춥니다. 날짜·생성기
//!   field는 고정값이라 bytes가 결정론적입니다.
//! - [`primaries_space`]: ICC profile의 `rXYZ`·`gXYZ`·`bXYZ`가 sRGB 또는 Display P3와 같고(허용 오차
//!   [`PRIMARY_TOLERANCE`]) `rTRC`·`gTRC`·`bTRC`가 모두 sRGB transfer 곡선인지([`is_srgb_curve`], 허용 오차
//!   [`TRC_TOLERANCE`]) 판정합니다. 이미지 asset의 내장 profile 해석에 씁니다. primaries가 같아도 곡선이 다르면(선형
//!   P3, gamma 1.8 등) 어느 색공간도 아닙니다. 다른 곡선의 pixel을 sRGB 곡선으로 읽으면 색이 조용히 틀어지기
//!   때문입니다.
//!
//! transfer 곡선은 두 색공간 모두 sRGB 곡선입니다(core-types `ColorSpace::encode_channel`, ADR-0002). parametric
//! curve type 3의 계수는 그 곡선의 정의 값입니다.

use stellar_core_types::color::ColorSpace;

/// primaries 판정 허용 오차(XYZ 성분).
pub const PRIMARY_TOLERANCE: f64 = 0.002;

/// transfer 곡선 판정 허용 오차(0~1 출력 값, 8-bit 반 code value).
pub const TRC_TOLERANCE: f64 = 0.5 / 255.0;

/// 곡선 판정에 쓰는 표본 수(입력 0~1 균등).
const TRC_SAMPLES: usize = 64;

/// ICC PCS 백색점(D50, ICC.1 7.2.16).
const D50: [f64; 3] = [0.9642, 1.0, 0.8249];

/// Bradford cone 응답 행렬.
const BRADFORD: [[f64; 3]; 3] = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
];

/// 색공간의 xy primaries(R, G, B)와 백색점(D65).
fn chromaticities(space: ColorSpace) -> ([[f64; 2]; 3], [f64; 2]) {
    let white = [0.3127, 0.3290];
    let primaries = match space {
        ColorSpace::Srgb => [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]],
        ColorSpace::DisplayP3 => [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060]],
    };
    (primaries, white)
}

fn xy_to_xyz([x, y]: [f64; 2]) -> [f64; 3] {
    [x / y, 1.0, (1.0 - x - y) / y]
}

fn mul(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn mat_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn inverse(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let c = |a: usize, b: usize, c: usize, d: usize| m[a][b] * m[c][d];
    [
        [
            (c(1, 1, 2, 2) - c(1, 2, 2, 1)) / det,
            (c(0, 2, 2, 1) - c(0, 1, 2, 2)) / det,
            (c(0, 1, 1, 2) - c(0, 2, 1, 1)) / det,
        ],
        [
            (c(1, 2, 2, 0) - c(1, 0, 2, 2)) / det,
            (c(0, 0, 2, 2) - c(0, 2, 2, 0)) / det,
            (c(0, 2, 1, 0) - c(0, 0, 1, 2)) / det,
        ],
        [
            (c(1, 0, 2, 1) - c(1, 1, 2, 0)) / det,
            (c(0, 1, 2, 0) - c(0, 0, 2, 1)) / det,
            (c(0, 0, 1, 1) - c(0, 1, 1, 0)) / det,
        ],
    ]
}

/// D65 → D50 Bradford 적응 행렬.
fn adaptation(source_white: [f64; 3]) -> [[f64; 3]; 3] {
    let src = mul(&BRADFORD, source_white);
    let dst = mul(&BRADFORD, D50);
    let scale = [
        [dst[0] / src[0], 0.0, 0.0],
        [0.0, dst[1] / src[1], 0.0],
        [0.0, 0.0, dst[2] / src[2]],
    ];
    mat_mul(&inverse(&BRADFORD), &mat_mul(&scale, &BRADFORD))
}

/// 색공간의 D50 적응 colorant(rXYZ, gXYZ, bXYZ)와 적응 행렬.
fn colorants(space: ColorSpace) -> ([[f64; 3]; 3], [[f64; 3]; 3]) {
    let (primaries, white) = chromaticities(space);
    let white = xy_to_xyz(white);
    // RGB → XYZ: 열이 primaries XYZ이고, 백색 (1,1,1)이 백색점이 되도록 열마다 배율 S를 곱합니다.
    let columns = primaries.map(xy_to_xyz);
    let m = [
        [columns[0][0], columns[1][0], columns[2][0]],
        [columns[0][1], columns[1][1], columns[2][1]],
        [columns[0][2], columns[1][2], columns[2][2]],
    ];
    let s = mul(&inverse(&m), white);
    let adapt = adaptation(white);
    let mut out = [[0.0; 3]; 3];
    for (channel, column) in out.iter_mut().enumerate() {
        let xyz = columns[channel].map(|v| v * s[channel]);
        *column = mul(&adapt, xyz);
    }
    (out, adapt)
}

fn s15f16(value: f64) -> [u8; 4] {
    ((value * 65_536.0).round() as i32).to_be_bytes()
}

fn read_s15f16(bytes: &[u8]) -> Option<f64> {
    let raw: [u8; 4] = bytes.get(..4)?.try_into().ok()?;
    Some(f64::from(i32::from_be_bytes(raw)) / 65_536.0)
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(raw))
}

/// `mluc` tag(en-US 한 항목).
fn mluc(text: &str) -> Vec<u8> {
    let utf16: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let mut out = Vec::new();
    out.extend_from_slice(b"mluc\0\0\0\0");
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&12u32.to_be_bytes());
    out.extend_from_slice(b"enUS");
    out.extend_from_slice(&(utf16.len() as u32).to_be_bytes());
    out.extend_from_slice(&28u32.to_be_bytes());
    out.extend_from_slice(&utf16);
    out
}

fn xyz_tag(xyz: [f64; 3]) -> Vec<u8> {
    let mut out = b"XYZ \0\0\0\0".to_vec();
    for v in xyz {
        out.extend_from_slice(&s15f16(v));
    }
    out
}

/// sRGB transfer 곡선(parametric type 3): `Y = (aX + b)^g` (X ≥ d), `Y = cX` (X < d).
fn srgb_curve() -> Vec<u8> {
    let mut out = b"para\0\0\0\0".to_vec();
    out.extend_from_slice(&3u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    for v in [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045] {
        out.extend_from_slice(&s15f16(v));
    }
    out
}

fn sf32(matrix: &[[f64; 3]; 3]) -> Vec<u8> {
    let mut out = b"sf32\0\0\0\0".to_vec();
    for row in matrix {
        for v in row {
            out.extend_from_slice(&s15f16(*v));
        }
    }
    out
}

/// Display P3 ICC v4.3 display profile bytes(모듈 문서).
pub fn display_p3_profile() -> Vec<u8> {
    let (columns, adapt) = colorants(ColorSpace::DisplayP3);
    let curve = srgb_curve();
    // (signature, data). 같은 data를 쓰는 TRC 셋은 offset 하나를 공유합니다.
    let tags: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"desc", mluc("Display P3")),
        (b"cprt", mluc("No copyright, use freely")),
        (b"wtpt", xyz_tag(D50)),
        (b"rXYZ", xyz_tag(columns[0])),
        (b"gXYZ", xyz_tag(columns[1])),
        (b"bXYZ", xyz_tag(columns[2])),
        (b"rTRC", curve.clone()),
        (b"gTRC", curve.clone()),
        (b"bTRC", curve),
        (b"chad", sf32(&adapt)),
    ];
    let table_len = 4 + 12 * tags.len();
    let mut data = Vec::new();
    let mut entries = Vec::new();
    let mut shared_curve: Option<(u32, u32)> = None;
    for (signature, bytes) in &tags {
        let is_trc = matches!(*signature, b"rTRC" | b"gTRC" | b"bTRC");
        let (offset, size) = match shared_curve.filter(|_| is_trc) {
            Some(slot) => slot,
            None => {
                let offset = (128 + table_len + data.len()) as u32;
                let size = bytes.len() as u32;
                data.extend_from_slice(bytes);
                while data.len() % 4 != 0 {
                    data.push(0);
                }
                if is_trc {
                    shared_curve = Some((offset, size));
                }
                (offset, size)
            }
        };
        entries.push((**signature, offset, size));
    }
    let total = 128 + table_len + data.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&(total as u32).to_be_bytes());
    out.extend_from_slice(&[0; 4]); // CMM
    out.extend_from_slice(&0x0430_0000u32.to_be_bytes()); // version 4.3
    out.extend_from_slice(b"mntr");
    out.extend_from_slice(b"RGB ");
    out.extend_from_slice(b"XYZ ");
    // 생성 날짜(고정): 2026-01-01 00:00:00.
    for v in [2026u16, 1, 1, 0, 0, 0] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(b"acsp");
    out.extend_from_slice(&[0; 4]); // platform
    out.extend_from_slice(&[0; 4]); // flags
    out.extend_from_slice(&[0; 4]); // manufacturer
    out.extend_from_slice(&[0; 4]); // model
    out.extend_from_slice(&[0; 8]); // attributes
    out.extend_from_slice(&0u32.to_be_bytes()); // perceptual
    for v in D50 {
        out.extend_from_slice(&s15f16(v));
    }
    out.extend_from_slice(&[0; 4]); // creator
    out.extend_from_slice(&[0; 16]); // profile ID(선택)
    out.extend_from_slice(&[0; 28]); // reserved
    debug_assert_eq!(out.len(), 128);
    out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (signature, offset, size) in entries {
        out.extend_from_slice(&signature);
        out.extend_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(&size.to_be_bytes());
    }
    out.extend_from_slice(&data);
    out
}

/// ICC profile의 tag data(signature로 찾음). 범위 밖·잘린 table이면 `None`.
pub fn tag<'a>(profile: &'a [u8], signature: &[u8; 4]) -> Option<&'a [u8]> {
    let count = read_u32(profile, 128)? as usize;
    if count > 1024 {
        return None;
    }
    for i in 0..count {
        let at = 132 + i * 12;
        if profile.get(at..at + 4)? == signature {
            let offset = read_u32(profile, at + 4)? as usize;
            let size = read_u32(profile, at + 8)? as usize;
            return profile.get(offset..offset.checked_add(size)?);
        }
    }
    None
}

/// `XYZ ` tag의 값.
pub fn xyz_value(data: &[u8]) -> Option<[f64; 3]> {
    if data.get(..4)? != b"XYZ " {
        return None;
    }
    Some([
        read_s15f16(data.get(8..)?)?,
        read_s15f16(data.get(12..)?)?,
        read_s15f16(data.get(16..)?)?,
    ])
}

/// `curv`·`para` tag가 sRGB transfer 곡선(core-types [`ColorSpace::decode_channel`])과 표본마다
/// [`TRC_TOLERANCE`] 안에서 같으면 참입니다. 다른 tag 형식이거나 잘린 tag는 거짓입니다.
pub fn is_srgb_curve(data: &[u8]) -> bool {
    let Some(curve) = Curve::parse(data) else {
        return false;
    };
    (0..=TRC_SAMPLES).all(|i| {
        let x = i as f64 / TRC_SAMPLES as f64;
        let expected = f64::from(ColorSpace::Srgb.decode_channel(x as f32));
        curve
            .eval(x)
            .is_some_and(|y| y.is_finite() && (y - expected).abs() <= TRC_TOLERANCE)
    })
}

/// ICC transfer 곡선(ICC.1 10.6 `curv`, 10.18 `para`).
enum Curve<'a> {
    /// 표본 table(u16, 0~65535 → 0~1). 표본 0개는 항등, 1개는 u8Fixed8 gamma입니다.
    Table(&'a [u8]),
    /// parametric 함수 type 0~4와 계수.
    Parametric(u16, [f64; 7]),
}

impl<'a> Curve<'a> {
    fn parse(data: &'a [u8]) -> Option<Curve<'a>> {
        match data.get(..4)? {
            b"curv" => {
                let count = read_u32(data, 8)? as usize;
                let table = data.get(12..12usize.checked_add(count.checked_mul(2)?)?)?;
                Some(Curve::Table(table))
            }
            b"para" => {
                let kind = u16::from_be_bytes(data.get(8..10)?.try_into().ok()?);
                let params = match kind {
                    0 => 1,
                    1 => 3,
                    2 => 4,
                    3 => 5,
                    4 => 7,
                    _ => return None,
                };
                let mut values = [0.0; 7];
                for (i, value) in values.iter_mut().enumerate().take(params) {
                    *value = read_s15f16(data.get(12 + i * 4..)?)?;
                }
                Some(Curve::Parametric(kind, values))
            }
            _ => None,
        }
    }

    fn eval(&self, x: f64) -> Option<f64> {
        match self {
            Curve::Table(table) => {
                let entry = |i: usize| -> Option<f64> {
                    let at = i * 2;
                    Some(
                        f64::from(u16::from_be_bytes(table.get(at..at + 2)?.try_into().ok()?))
                            / 65535.0,
                    )
                };
                match table.len() / 2 {
                    0 => Some(x),
                    1 => {
                        let gamma =
                            f64::from(u16::from_be_bytes(table.get(..2)?.try_into().ok()?)) / 256.0;
                        Some(x.powf(gamma))
                    }
                    n => {
                        let position = x * (n - 1) as f64;
                        let low = (position.floor() as usize).min(n - 1);
                        let high = (low + 1).min(n - 1);
                        let t = position - low as f64;
                        Some(entry(low)? + (entry(high)? - entry(low)?) * t)
                    }
                }
            }
            Curve::Parametric(kind, p) => {
                let [g, a, b, c, d, e, f] = *p;
                let power = |base: f64| if base > 0.0 { base.powf(g) } else { 0.0 };
                Some(match kind {
                    0 => power(x),
                    1 => {
                        if x >= -b / a {
                            power(a * x + b)
                        } else {
                            0.0
                        }
                    }
                    2 => {
                        if x >= -b / a {
                            power(a * x + b) + c
                        } else {
                            c
                        }
                    }
                    3 => {
                        if x >= d {
                            power(a * x + b)
                        } else {
                            c * x
                        }
                    }
                    _ => {
                        if x >= d {
                            power(a * x + b) + e
                        } else {
                            c * x + f
                        }
                    }
                })
            }
        }
    }
}

/// ICC profile의 colorant가 sRGB·Display P3 중 하나와 같으면 그 색공간입니다(모듈 문서). RGB display profile이
/// 아니거나 colorant가 둘 다 아니면 `None`입니다.
pub fn primaries_space(profile: &[u8]) -> Option<ColorSpace> {
    if profile.get(36..40)? != b"acsp"
        || profile.get(16..20)? != b"RGB "
        || profile.get(20..24)? != b"XYZ "
    {
        return None;
    }
    for signature in [b"rTRC", b"gTRC", b"bTRC"] {
        if !is_srgb_curve(tag(profile, signature)?) {
            return None;
        }
    }
    let read = |sig: &[u8; 4]| tag(profile, sig).and_then(xyz_value);
    let found = [read(b"rXYZ")?, read(b"gXYZ")?, read(b"bXYZ")?];
    [ColorSpace::Srgb, ColorSpace::DisplayP3]
        .into_iter()
        .find(|space| {
            let (expected, _) = colorants(*space);
            expected.iter().zip(&found).all(|(e, f)| {
                e.iter()
                    .zip(f)
                    .all(|(a, b)| (a - b).abs() <= PRIMARY_TOLERANCE)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_p3_colorants_match_published_values() {
        // Apple Display P3 profile(ColorSync)의 D50 colorant.
        let (columns, _) = colorants(ColorSpace::DisplayP3);
        let published = [
            [0.5151, 0.2412, -0.0011],
            [0.2920, 0.6922, 0.0419],
            [0.1571, 0.0666, 0.7841],
        ];
        for (c, p) in columns.iter().zip(&published) {
            for (a, b) in c.iter().zip(p) {
                assert!((a - b).abs() < 0.001, "{c:?} vs {p:?}");
            }
        }
        // 세 colorant 합은 PCS 백색(D50)입니다.
        for axis in 0..3 {
            let sum: f64 = columns.iter().map(|c| c[axis]).sum();
            assert!((sum - D50[axis]).abs() < 1e-6);
        }
        // sRGB는 IEC 61966-2-1 D50 값(ICC sRGB v4 profile).
        let (srgb, _) = colorants(ColorSpace::Srgb);
        assert!((srgb[0][0] - 0.4361).abs() < 0.001);
        assert!((srgb[1][1] - 0.7169).abs() < 0.001);
        assert!((srgb[2][2] - 0.7141).abs() < 0.001);
    }

    #[test]
    fn display_p3_profile_is_well_formed_and_deterministic() {
        let profile = display_p3_profile();
        assert_eq!(profile, display_p3_profile());
        assert_eq!(read_u32(&profile, 0), Some(profile.len() as u32));
        assert_eq!(&profile[36..40], b"acsp");
        assert_eq!(&profile[12..16], b"mntr");
        assert_eq!(read_u32(&profile, 8), Some(0x0430_0000));
        assert_eq!(primaries_space(&profile), Some(ColorSpace::DisplayP3));
        let desc = tag(&profile, b"desc").unwrap();
        assert_eq!(&desc[..4], b"mluc");
        // TRC 셋은 같은 data를 가리킵니다.
        assert_eq!(tag(&profile, b"rTRC"), tag(&profile, b"bTRC"));
        assert_eq!(&tag(&profile, b"gTRC").unwrap()[..4], b"para");
        assert_eq!(profile.len() % 4, 0);
    }

    #[test]
    fn primaries_space_rejects_other_profiles() {
        let mut profile = display_p3_profile();
        assert_eq!(primaries_space(&profile[..100]), None);
        // rXYZ 값을 바꾸면 어느 색공간도 아닙니다.
        let offset = read_u32(&profile, 132 + 3 * 12 + 4).unwrap() as usize;
        profile[offset + 8..offset + 12].copy_from_slice(&s15f16(0.6));
        assert_eq!(primaries_space(&profile), None);
        let mut gray = display_p3_profile();
        gray[16..20].copy_from_slice(b"GRAY");
        assert_eq!(primaries_space(&gray), None);
    }

    /// colorant가 P3여도 transfer 곡선이 sRGB가 아니면 어느 색공간도 아닙니다.
    #[test]
    fn primaries_space_requires_srgb_transfer_curves() {
        let profile = display_p3_profile();
        let trc_offset = |profile: &[u8]| {
            let count = read_u32(profile, 128).unwrap() as usize;
            (0..count)
                .map(|i| 132 + i * 12)
                .find(|at| &profile[*at..*at + 4] == b"rTRC")
                .map(|at| read_u32(profile, at + 4).unwrap() as usize)
                .unwrap()
        };
        let offset = trc_offset(&profile);
        // 선형 곡선(para type 3, gamma 1, c = 1)은 거부합니다.
        let mut linear = profile.clone();
        for (i, v) in [1.0, 1.0, 0.0, 1.0, 0.0].into_iter().enumerate() {
            linear[offset + 12 + i * 4..offset + 16 + i * 4].copy_from_slice(&s15f16(v));
        }
        assert_eq!(primaries_space(&linear), None);
        // gamma 1.8(para type 3, a = 1, b = 0, c = 0, d = 0)도 거부합니다.
        let mut gamma = profile.clone();
        for (i, v) in [1.8, 1.0, 0.0, 0.0, 0.0].into_iter().enumerate() {
            gamma[offset + 12 + i * 4..offset + 16 + i * 4].copy_from_slice(&s15f16(v));
        }
        assert_eq!(primaries_space(&gamma), None);
        // 1024 표본 `curv` sRGB 곡선(ICC v2 sRGB profile 형식)은 sRGB 곡선입니다.
        let mut table = b"curv\0\0\0\0".to_vec();
        table.extend_from_slice(&1024u32.to_be_bytes());
        for i in 0..1024 {
            let y = ColorSpace::Srgb.decode_channel(i as f32 / 1023.0);
            table.extend_from_slice(&((f64::from(y) * 65535.0).round() as u16).to_be_bytes());
        }
        assert!(is_srgb_curve(&table));
        // 표본 하나(gamma 2.2)는 sRGB 곡선이 아닙니다.
        let mut gamma22 = b"curv\0\0\0\0".to_vec();
        gamma22.extend_from_slice(&1u32.to_be_bytes());
        gamma22.extend_from_slice(&((2.2f64 * 256.0).round() as u16).to_be_bytes());
        assert!(!is_srgb_curve(&gamma22));
        assert!(!is_srgb_curve(b"curv"));
        assert!(!is_srgb_curve(b"sf32\0\0\0\0"));
    }
}
