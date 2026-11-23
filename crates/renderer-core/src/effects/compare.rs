//! image 비교: 채널·structural 허용 오차 config와 diff artifact (XI.11.2, ADR-0102 결정 6).
//!
//! ## 판정
//!
//! pixel마다 네 채널 byte 차이의 최대값 `delta`를 봅니다.
//!
//! 1. `delta == 0`: 같음.
//! 2. `delta ≤ channel`: 허용 안(within).
//! 3. structural: 반경 `radius`(Chebyshev) 안에서 실제 값과 `channel` 안인 기대 image 이웃이 있고, **동시에** 기대 값과
//!    `channel` 안인 실제 image 이웃이 있으면 위치 어긋남(shifted)입니다. 경계가 밀린 경우만 인정하고, 고립된 점이나
//!    빠진 가는 선은 한쪽 조건을 못 채워 failure입니다.
//! 4. 나머지는 초과(failure)입니다.
//!
//! failure 수 ≤ `max_failures`이고 shifted 수 ≤ `max_shifted`이면 통과입니다.
//!
//! ## config
//!
//! [`ToleranceConfig::parse`]는 줄 단위 `class key=value …`를 읽습니다(`#` 주석, 빈 줄 허용). key 네 개(`channel`,
//! `radius`, `max_failures`, `max_shifted`)가 모두 있어야 하고, 모르는 key·중복 class·`default` 누락은 오류입니다.
//! 기본 config는 [`BUILTIN_TOLERANCES`](`tolerances.conf`)입니다.
//!
//! ## diff artifact
//!
//! [`Comparison::write_artifact`]는 `dir/<name>/`에 `expected.png`·`actual.png`·`diff.png`·`failures.txt`와 두 raw를
//! 씁니다. `diff.png`는 같음·허용 안 pixel을 어둡게 한 기대 image 위에 shifted를 노랑, failure를 빨강으로 칠합니다.
//! `failures.txt`는 판정 요약과 failure·shifted pixel 위치·기대·실제 값입니다.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::io;
use std::path::{Path, PathBuf};

use super::artifact::RgbaImage;

/// 기본 허용 오차 config(`effects/tolerances.conf`).
pub const BUILTIN_TOLERANCES: &str = include_str!("tolerances.conf");

/// class 하나의 허용 오차.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tolerance {
    /// 채널 byte 차이 허용값.
    pub channel: u8,
    /// structural 반경(pixel).
    pub radius: u32,
    /// failure pixel 수 상한.
    pub max_failures: usize,
    /// shifted pixel 수 상한.
    pub max_shifted: usize,
}

impl Tolerance {
    /// 완전 일치.
    pub const EXACT: Tolerance = Tolerance {
        channel: 0,
        radius: 0,
        max_failures: 0,
        max_shifted: 0,
    };
}

/// config 해석 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// 줄 형식 오류(줄 번호, 이유).
    Syntax(usize, &'static str),
    /// 같은 class가 두 번.
    DuplicateClass(String),
    /// `default` class 없음.
    MissingDefault,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Syntax(line, reason) => write!(f, "{line}번째 줄: {reason}"),
            ConfigError::DuplicateClass(class) => write!(f, "class가 중복됩니다: {class}"),
            ConfigError::MissingDefault => f.write_str("default class가 없습니다"),
        }
    }
}

/// class별 허용 오차 표.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToleranceConfig {
    classes: BTreeMap<String, Tolerance>,
}

impl ToleranceConfig {
    /// config text를 읽습니다(모듈 문서 형식).
    pub fn parse(text: &str) -> Result<ToleranceConfig, ConfigError> {
        let mut classes = BTreeMap::new();
        for (index, raw) in text.lines().enumerate() {
            let line_no = index + 1;
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let mut words = line.split_whitespace();
            let class = words
                .next()
                .ok_or(ConfigError::Syntax(line_no, "class 이름이 없습니다"))?;
            let (mut channel, mut radius, mut failures, mut shifted) = (None, None, None, None);
            for word in words {
                let (key, value) = word
                    .split_once('=')
                    .ok_or(ConfigError::Syntax(line_no, "key=value 형식이 아닙니다"))?;
                let bad = ConfigError::Syntax(line_no, "값이 음이 아닌 정수가 아닙니다");
                let slot = match key {
                    "channel" => {
                        if channel.is_some() {
                            return Err(ConfigError::Syntax(line_no, "key가 중복됩니다"));
                        }
                        channel = Some(value.parse::<u8>().map_err(|_| bad.clone())?);
                        continue;
                    }
                    "radius" => &mut radius,
                    "max_failures" => &mut failures,
                    "max_shifted" => &mut shifted,
                    _ => return Err(ConfigError::Syntax(line_no, "모르는 key")),
                };
                if slot.is_some() {
                    return Err(ConfigError::Syntax(line_no, "key가 중복됩니다"));
                }
                *slot = Some(value.parse::<usize>().map_err(|_| bad)?);
            }
            let missing = ConfigError::Syntax(
                line_no,
                "channel·radius·max_failures·max_shifted가 모두 필요합니다",
            );
            let tolerance = Tolerance {
                channel: channel.ok_or(missing.clone())?,
                radius: u32::try_from(radius.ok_or(missing.clone())?)
                    .map_err(|_| ConfigError::Syntax(line_no, "radius가 너무 큽니다"))?,
                max_failures: failures.ok_or(missing.clone())?,
                max_shifted: shifted.ok_or(missing)?,
            };
            if classes.insert(class.to_owned(), tolerance).is_some() {
                return Err(ConfigError::DuplicateClass(class.to_owned()));
            }
        }
        if !classes.contains_key("default") {
            return Err(ConfigError::MissingDefault);
        }
        Ok(ToleranceConfig { classes })
    }

    /// 기본 config.
    pub fn builtin() -> ToleranceConfig {
        ToleranceConfig::parse(BUILTIN_TOLERANCES).expect("내장 허용 오차 config")
    }

    /// class의 허용 오차.
    pub fn get(&self, class: &str) -> Option<Tolerance> {
        self.classes.get(class).copied()
    }

    /// class 이름 목록.
    pub fn classes(&self) -> impl Iterator<Item = &str> + '_ {
        self.classes.keys().map(String::as_str)
    }
}

/// 비교 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareError {
    /// 두 image 크기가 다르거나 byte 수가 맞지 않음.
    SizeMismatch,
}

/// pixel 하나의 차이.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelDiff {
    /// x.
    pub x: u32,
    /// y.
    pub y: u32,
    /// 기대 값.
    pub expected: [u8; 4],
    /// 실제 값.
    pub actual: [u8; 4],
    /// 채널 최대 차이.
    pub delta: u8,
}

/// 비교 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    /// 적용한 허용 오차.
    pub tolerance: Tolerance,
    /// 너비.
    pub width: u32,
    /// 높이.
    pub height: u32,
    /// 완전히 같은 pixel 수.
    pub equal: usize,
    /// 허용 안 pixel 수.
    pub within: usize,
    /// 위치 어긋남 pixel.
    pub shifted: Vec<PixelDiff>,
    /// 초과 pixel.
    pub failures: Vec<PixelDiff>,
    /// 전체 pixel 최대 채널 차이.
    pub max_delta: u8,
}

fn delta(a: [u8; 4], b: [u8; 4]) -> u8 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

/// 두 image를 `tolerance`로 비교합니다(모듈 문서 판정).
pub fn compare(
    expected: &RgbaImage,
    actual: &RgbaImage,
    tolerance: Tolerance,
) -> Result<Comparison, CompareError> {
    if expected.width != actual.width
        || expected.height != actual.height
        || !expected.is_well_formed()
        || !actual.is_well_formed()
    {
        return Err(CompareError::SizeMismatch);
    }
    let (w, h) = (expected.width, expected.height);
    let mut result = Comparison {
        tolerance,
        width: w,
        height: h,
        equal: 0,
        within: 0,
        shifted: Vec::new(),
        failures: Vec::new(),
        max_delta: 0,
    };
    let r = i64::from(tolerance.radius);
    for y in 0..h {
        for x in 0..w {
            let e = expected.pixel(x, y);
            let a = actual.pixel(x, y);
            let d = delta(e, a);
            result.max_delta = result.max_delta.max(d);
            if d == 0 {
                result.equal += 1;
                continue;
            }
            if d <= tolerance.channel {
                result.within += 1;
                continue;
            }
            // 양방향: 실제 값이 기대 image 이웃에 있고, 기대 값도 실제 image 이웃에 있어야 위치 어긋남입니다.
            let mut actual_near = false;
            let mut expected_near = false;
            for dy in -r..=r {
                for dx in -r..=r {
                    let (nx, ny) = (i64::from(x) + dx, i64::from(y) + dy);
                    if (dx == 0 && dy == 0)
                        || nx < 0
                        || ny < 0
                        || nx >= i64::from(w)
                        || ny >= i64::from(h)
                    {
                        continue;
                    }
                    let (nx, ny) = (nx as u32, ny as u32);
                    actual_near |= delta(expected.pixel(nx, ny), a) <= tolerance.channel;
                    expected_near |= delta(actual.pixel(nx, ny), e) <= tolerance.channel;
                }
            }
            let near = actual_near && expected_near;
            let diff = PixelDiff {
                x,
                y,
                expected: e,
                actual: a,
                delta: d,
            };
            if near {
                result.shifted.push(diff);
            } else {
                result.failures.push(diff);
            }
        }
    }
    Ok(result)
}

impl Comparison {
    /// 허용 오차 안이면 참.
    pub fn passed(&self) -> bool {
        self.failures.len() <= self.tolerance.max_failures
            && self.shifted.len() <= self.tolerance.max_shifted
    }

    /// 한 줄 요약.
    pub fn summary(&self) -> String {
        format!(
            "{} equal={} within={} shifted={}/{} failures={}/{} max_delta={} (channel={} radius={})",
            if self.passed() { "pass" } else { "fail" },
            self.equal,
            self.within,
            self.shifted.len(),
            self.tolerance.max_shifted,
            self.failures.len(),
            self.tolerance.max_failures,
            self.max_delta,
            self.tolerance.channel,
            self.tolerance.radius,
        )
    }

    /// diff image(모듈 문서 색 규칙).
    pub fn diff_image(&self, expected: &RgbaImage) -> RgbaImage {
        let mut pixels = Vec::with_capacity(expected.pixels.len());
        for chunk in expected.pixels.chunks_exact(4) {
            let luma =
                (u16::from(chunk[0]) * 3 + u16::from(chunk[1]) * 6 + u16::from(chunk[2])) / 10;
            let dim = (luma / 4) as u8;
            pixels.extend_from_slice(&[dim, dim, dim, 255]);
        }
        let mut paint = |diff: &PixelDiff, color: [u8; 4]| {
            let o = ((diff.y * self.width + diff.x) * 4) as usize;
            pixels[o..o + 4].copy_from_slice(&color);
        };
        for diff in &self.shifted {
            paint(diff, [255, 200, 0, 255]);
        }
        for diff in &self.failures {
            paint(diff, [255, 0, 0, 255]);
        }
        RgbaImage {
            width: self.width,
            height: self.height,
            pixels,
        }
    }

    /// failure·shifted 목록 text.
    pub fn report_text(&self, name: &str) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "fixture {name} {}x{}", self.width, self.height);
        let _ = writeln!(out, "{}", self.summary());
        for (kind, list) in [("failure", &self.failures), ("shifted", &self.shifted)] {
            for diff in list.iter() {
                let _ = writeln!(
                    out,
                    "{kind} x={} y={} expected={:?} actual={:?} delta={}",
                    diff.x, diff.y, diff.expected, diff.actual, diff.delta
                );
            }
        }
        out
    }

    /// `dir/<name>/`에 diff artifact를 씁니다. 쓴 directory를 돌려줍니다.
    pub fn write_artifact(
        &self,
        dir: &Path,
        name: &str,
        expected: &RgbaImage,
        actual: &RgbaImage,
    ) -> io::Result<PathBuf> {
        let out = dir.join(name);
        expected.write_reference(&out, "expected")?;
        actual.write_reference(&out, "actual")?;
        std::fs::write(out.join("diff.png"), self.diff_image(expected).to_png())?;
        std::fs::write(out.join("failures.txt"), self.report_text(name))?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> RgbaImage {
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                pixels.extend_from_slice(&f(x, y));
            }
        }
        RgbaImage {
            width: w,
            height: h,
            pixels,
        }
    }

    fn edge(split: u32) -> RgbaImage {
        image(8, 8, move |x, _| {
            if x < split {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        })
    }

    const DEFAULT: Tolerance = Tolerance {
        channel: 1,
        radius: 1,
        max_failures: 0,
        max_shifted: 8,
    };

    #[test]
    fn known_equal_fixtures_pass() {
        // 검증 기준(XI.11.2): known equal fixture 판정 정확.
        let a = edge(4);
        let same = compare(&a, &a.clone(), Tolerance::EXACT).unwrap();
        assert!(same.passed());
        assert_eq!((same.equal, same.within, same.max_delta), (64, 0, 0));
        // 1 LSB 반올림 차이는 within.
        let noisy = image(8, 8, |x, y| {
            let v = if x < 4 { 0 } else { 254 };
            [v + ((x + y) % 2) as u8, v, v, 255]
        });
        let within = compare(&a, &noisy, DEFAULT).unwrap();
        assert!(within.passed(), "{}", within.summary());
        assert!(within.failures.is_empty() && within.shifted.is_empty());
        assert!(!compare(&a, &noisy, Tolerance::EXACT).unwrap().passed());
        // 경계가 한 pixel 밀린 image는 structural 반경 1로 통과(shifted 8개).
        let moved = compare(&a, &edge(5), DEFAULT).unwrap();
        assert!(moved.passed(), "{}", moved.summary());
        assert_eq!(moved.shifted.len(), 8);
        assert!(moved.shifted.iter().all(|d| d.x == 4));
    }

    #[test]
    fn known_different_fixtures_fail_with_exact_locations() {
        // 검증 기준(XI.11.2): known different fixture 판정 정확, 초과 pixel 위치 기록.
        let a = edge(4);
        let mut b = a.clone();
        let o = ((6 * 8 + 1) * 4) as usize;
        b.pixels[o] = 40; // (1, 6): 검정 영역에 붉은 점.
        let result = compare(&a, &b, DEFAULT).unwrap();
        assert!(!result.passed());
        assert_eq!(result.failures.len(), 1);
        let failure = result.failures[0];
        assert_eq!((failure.x, failure.y, failure.delta), (1, 6, 40));
        assert_eq!(failure.expected, [0, 0, 0, 255]);
        assert_eq!(failure.actual, [40, 0, 0, 255]);
        // 반경 0이면 밀린 경계도 failure.
        let strict = Tolerance {
            radius: 0,
            ..DEFAULT
        };
        let moved = compare(&a, &edge(5), strict).unwrap();
        assert!(!moved.passed());
        assert_eq!(moved.failures.len(), 8);
        // shifted 상한 초과도 실패.
        let tight = Tolerance {
            max_shifted: 7,
            ..DEFAULT
        };
        assert!(!compare(&a, &edge(5), tight).unwrap().passed());
        // 크기가 다르면 비교하지 않습니다.
        assert_eq!(
            compare(&a, &edge(4).clone_resized(), DEFAULT),
            Err(CompareError::SizeMismatch)
        );
    }

    impl RgbaImage {
        fn clone_resized(&self) -> RgbaImage {
            RgbaImage {
                width: self.width,
                height: self.height - 1,
                pixels: self.pixels[..(self.width * (self.height - 1) * 4) as usize].to_vec(),
            }
        }
    }

    #[test]
    fn diff_artifact_marks_failures_and_lists_positions() {
        let a = edge(4);
        let mut b = edge(5);
        let o = ((7 * 8) * 4) as usize;
        b.pixels[o + 1] = 99;
        let result = compare(&a, &b, DEFAULT).unwrap();
        let diff = result.diff_image(&a);
        assert_eq!(diff.pixel(0, 7), [255, 0, 0, 255]);
        assert_eq!(diff.pixel(4, 0), [255, 200, 0, 255]);
        assert_eq!(diff.pixel(7, 0)[0], 63);
        let text = result.report_text("edge");
        assert!(text
            .contains("failure x=0 y=7 expected=[0, 0, 0, 255] actual=[0, 99, 0, 255] delta=99"));
        assert!(text.lines().nth(1).unwrap().starts_with("fail "));
        let dir = std::env::temp_dir().join(format!("stellar-compare-{}", std::process::id()));
        let written = result.write_artifact(&dir, "edge", &a, &b).unwrap();
        for file in [
            "expected.rgba",
            "actual.rgba",
            "diff.png",
            "failures.txt",
            "expected.png",
        ] {
            assert!(written.join(file).exists(), "{file}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn config_parses_builtin_and_rejects_bad_lines() {
        let builtin = ToleranceConfig::builtin();
        assert_eq!(
            builtin.get("default"),
            Some(Tolerance {
                channel: 1,
                radius: 1,
                max_failures: 0,
                max_shifted: 24
            })
        );
        for class in ["curve", "blend", "filtered", "gradient", "blur"] {
            assert!(builtin.get(class).is_some(), "{class}");
        }
        let ok = ToleranceConfig::parse(
            "# c\n\ndefault channel=0 radius=0 max_failures=0 max_shifted=0 # tail\n",
        )
        .unwrap();
        assert_eq!(ok.get("default"), Some(Tolerance::EXACT));
        assert_eq!(ok.classes().collect::<Vec<_>>(), vec!["default"]);
        let cases = [
            ("default channel=1 radius=1 max_failures=0", ConfigError::Syntax(1, "channel·radius·max_failures·max_shifted가 모두 필요합니다")),
            ("default channel=1 radius=1 max_failures=0 max_shifted=0 extra=1", ConfigError::Syntax(1, "모르는 key")),
            ("default channel=300 radius=1 max_failures=0 max_shifted=0", ConfigError::Syntax(1, "값이 음이 아닌 정수가 아닙니다")),
            ("default channel=1 radius=-1 max_failures=0 max_shifted=0", ConfigError::Syntax(1, "값이 음이 아닌 정수가 아닙니다")),
            ("default channel radius=1", ConfigError::Syntax(1, "key=value 형식이 아닙니다")),
            ("other channel=1 radius=1 max_failures=0 max_shifted=0", ConfigError::MissingDefault),
            (
                "default channel=1 radius=1 max_failures=0 max_shifted=0\ndefault channel=1 radius=1 max_failures=0 max_shifted=0",
                ConfigError::DuplicateClass("default".into()),
            ),
            ("default radius=1 radius=2 channel=1 max_failures=0 max_shifted=0", ConfigError::Syntax(1, "key가 중복됩니다")),
        ];
        for (text, error) in cases {
            assert_eq!(ToleranceConfig::parse(text), Err(error), "{text}");
        }
    }
}
