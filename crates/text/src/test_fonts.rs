//! 테스트 전용 합성 OpenType 폰트 생성기.
//!
//! 저장소에 외부 폰트 파일을 넣지 않고(라이선스·그래픽 리소스 정책), shaping·fallback·레이아웃
//! fixture에 필요한 최소 SFNT를 코드로 결정론적으로 만듭니다. 외곽선(`glyf`)은 없고 advance·cmap·
//! line metric과, 필요할 때 `GSUB` ligature(`liga`)·`fvar`/`HVAR`(`wght` axis advance 변화)만
//! 담습니다. 같은 정의는 항상 같은 bytes를 냅니다(골든 비교의 "동일 font files").

use std::collections::BTreeMap;

/// `wght` axis 하나와 glyph별 최대값(`max`)에서의 advance delta.
#[derive(Debug, Clone)]
pub(crate) struct WeightAxis {
    /// 최소 user 값.
    pub min: i32,
    /// 기본 user 값.
    pub default: i32,
    /// 최대 user 값.
    pub max: i32,
    /// glyph id별 advance delta(정규화 좌표 1.0에서). 길이가 glyph 수보다 짧으면 나머지는 0.
    pub advance_deltas: Vec<i16>,
}

/// 합성 폰트 정의.
#[derive(Debug, Clone)]
pub(crate) struct TestFont {
    /// em당 font unit.
    pub upem: u16,
    /// hhea ascender.
    pub ascender: i16,
    /// hhea descender(음수).
    pub descender: i16,
    /// hhea line gap.
    pub line_gap: i16,
    /// glyph id별 advance. 0번은 `.notdef`.
    pub advances: Vec<u16>,
    /// (codepoint 시작, 끝(포함), 시작 glyph id) 연속 매핑 group.
    pub cmap_groups: Vec<(u32, u32, u32)>,
    /// (구성 glyph들, ligature glyph). 비어 있지 않으면 `liga` feature를 만듭니다.
    pub ligatures: Vec<(Vec<u16>, u16)>,
    /// 있으면 `fvar`·`HVAR`를 만듭니다.
    pub weight_axis: Option<WeightAxis>,
}

/// 단위시험용 TrueType. 공백은 빈 contour, 'A'는 quadratic과 직선으로 구성된 1개 contour입니다.
/// 폰트의 실제 bitstream을 거쳐 shaping 및 outline renderer 결과를 검사합니다.
pub fn outline_test_font() -> Vec<u8> {
    let spec = FontSpec::new(1000, 800, -200, 500)
        .range(32, 32, |_| 250)
        .range(65, 65, |_| 600);
    let mut glyf = Vec::new();
    // (0,0) → quadratic control (250,500) → (500,0), 그리고 시작점으로 닫기.
    for n in [1i16, 0, 0, 500, 500, 2, 0] {
        put_i16(&mut glyf, n);
    }
    glyf.extend_from_slice(&[1, 0, 1]);
    for delta in [0i16, 250, 250, 0, 500, -500] {
        put_i16(&mut glyf, delta);
    }
    glyf.push(0);
    let mut loca = vec![0; 6];
    put_u16(&mut loca, (glyf.len() / 2) as u16);
    spec.font()
        .build_with_tables(BTreeMap::from([(*b"glyf", glyf), (*b"loca", loca)]))
}

impl TestFont {
    /// SFNT bytes를 만듭니다.
    pub(crate) fn build(&self) -> Vec<u8> {
        self.build_with_tables(BTreeMap::new())
    }

    fn build_with_tables(&self, mut tables: BTreeMap<[u8; 4], Vec<u8>>) -> Vec<u8> {
        tables.insert(*b"head", self.head());
        tables.insert(*b"hhea", self.hhea());
        tables.insert(*b"maxp", self.maxp());
        tables.insert(*b"hmtx", self.hmtx());
        tables.insert(*b"cmap", self.cmap());
        if !self.ligatures.is_empty() {
            tables.insert(*b"GSUB", self.gsub());
        }
        if let Some(axis) = &self.weight_axis {
            tables.insert(*b"fvar", fvar(axis));
            tables.insert(*b"HVAR", self.hvar(axis));
        }
        sfnt(&tables)
    }

    fn glyph_count(&self) -> u16 {
        u16::try_from(self.advances.len()).expect("테스트 폰트 glyph 수는 u16 범위")
    }

    /// 뒤쪽 같은 advance를 압축한 hmtx metric 수.
    fn h_metric_count(&self) -> u16 {
        let mut count = self.advances.len();
        while count > 1 && self.advances[count - 1] == self.advances[count - 2] {
            count -= 1;
        }
        u16::try_from(count).expect("u16 범위")
    }

    fn head(&self) -> Vec<u8> {
        let mut b = Vec::new();
        put_u16(&mut b, 1);
        put_u16(&mut b, 0);
        put_u32(&mut b, 0x0001_0000); // fontRevision
        put_u32(&mut b, 0); // checkSumAdjustment
        put_u32(&mut b, 0x5F0F_3CF5); // magic
        put_u16(&mut b, 0x0003); // flags
        put_u16(&mut b, self.upem);
        b.extend_from_slice(&[0; 16]); // created, modified
        put_i16(&mut b, 0);
        put_i16(&mut b, self.descender);
        put_i16(&mut b, i16::try_from(self.upem).expect("upem"));
        put_i16(&mut b, self.ascender);
        put_u16(&mut b, 0); // macStyle
        put_u16(&mut b, 8); // lowestRecPPEM
        put_i16(&mut b, 2); // fontDirectionHint
        put_i16(&mut b, 0); // indexToLocFormat
        put_i16(&mut b, 0); // glyphDataFormat
        b
    }

    fn hhea(&self) -> Vec<u8> {
        let max_advance = self.advances.iter().copied().max().unwrap_or(0);
        let mut b = Vec::new();
        put_u16(&mut b, 1);
        put_u16(&mut b, 0);
        put_i16(&mut b, self.ascender);
        put_i16(&mut b, self.descender);
        put_i16(&mut b, self.line_gap);
        put_u16(&mut b, max_advance);
        put_i16(&mut b, 0); // minLeftSideBearing
        put_i16(&mut b, 0); // minRightSideBearing
        put_i16(&mut b, i16::try_from(max_advance).unwrap_or(i16::MAX)); // xMaxExtent
        put_i16(&mut b, 1); // caretSlopeRise
        put_i16(&mut b, 0); // caretSlopeRun
        put_i16(&mut b, 0); // caretOffset
        b.extend_from_slice(&[0; 8]); // reserved
        put_i16(&mut b, 0); // metricDataFormat
        put_u16(&mut b, self.h_metric_count());
        b
    }

    fn maxp(&self) -> Vec<u8> {
        let mut b = Vec::new();
        put_u32(&mut b, 0x0000_5000);
        put_u16(&mut b, self.glyph_count());
        b
    }

    fn hmtx(&self) -> Vec<u8> {
        let metrics = usize::from(self.h_metric_count());
        let mut b = Vec::new();
        for advance in &self.advances[..metrics] {
            put_u16(&mut b, *advance);
            put_i16(&mut b, 0);
        }
        for _ in metrics..self.advances.len() {
            put_i16(&mut b, 0);
        }
        b
    }

    fn cmap(&self) -> Vec<u8> {
        let groups = &self.cmap_groups;
        let mut b = Vec::new();
        put_u16(&mut b, 0); // version
        put_u16(&mut b, 1); // numTables
        put_u16(&mut b, 3); // platform Windows
        put_u16(&mut b, 10); // UCS-4
        put_u32(&mut b, 12); // subtable offset
        put_u16(&mut b, 12); // format 12
        put_u16(&mut b, 0);
        put_u32(&mut b, u32::try_from(16 + 12 * groups.len()).expect("길이"));
        put_u32(&mut b, 0); // language
        put_u32(&mut b, u32::try_from(groups.len()).expect("group 수"));
        for (start, end, glyph) in groups {
            put_u32(&mut b, *start);
            put_u32(&mut b, *end);
            put_u32(&mut b, *glyph);
        }
        b
    }

    fn gsub(&self) -> Vec<u8> {
        // ScriptList: DFLT 하나, 기본 LangSys가 feature 0을 씁니다.
        let mut script_list = Vec::new();
        put_u16(&mut script_list, 1);
        script_list.extend_from_slice(b"DFLT");
        put_u16(&mut script_list, 8); // Script offset
        put_u16(&mut script_list, 4); // defaultLangSys offset
        put_u16(&mut script_list, 0); // langSysCount
        put_u16(&mut script_list, 0); // lookupOrder
        put_u16(&mut script_list, 0xFFFF); // requiredFeatureIndex
        put_u16(&mut script_list, 1);
        put_u16(&mut script_list, 0);

        // FeatureList: liga → lookup 0.
        let mut feature_list = Vec::new();
        put_u16(&mut feature_list, 1);
        feature_list.extend_from_slice(b"liga");
        put_u16(&mut feature_list, 8);
        put_u16(&mut feature_list, 0); // featureParams
        put_u16(&mut feature_list, 1);
        put_u16(&mut feature_list, 0);

        // LigatureSubstFormat1: 첫 구성 glyph별 LigatureSet.
        let mut by_first: BTreeMap<u16, Vec<&(Vec<u16>, u16)>> = BTreeMap::new();
        for lig in &self.ligatures {
            by_first.entry(lig.0[0]).or_default().push(lig);
        }
        let set_count = by_first.len();
        let header_len = 6 + 2 * set_count;
        let mut coverage = Vec::new();
        put_u16(&mut coverage, 1);
        put_u16(&mut coverage, u16::try_from(set_count).expect("set 수"));
        for first in by_first.keys() {
            put_u16(&mut coverage, *first);
        }
        let mut sets: Vec<Vec<u8>> = Vec::new();
        for ligs in by_first.values() {
            let mut set = Vec::new();
            put_u16(&mut set, u16::try_from(ligs.len()).expect("lig 수"));
            let mut offset = 2 + 2 * ligs.len();
            let mut bodies = Vec::new();
            for (components, glyph) in ligs.iter().map(|l| (&l.0, l.1)) {
                put_u16(&mut set, u16::try_from(offset).expect("offset"));
                let mut body = Vec::new();
                put_u16(&mut body, glyph);
                put_u16(&mut body, u16::try_from(components.len()).expect("구성 수"));
                for component in &components[1..] {
                    put_u16(&mut body, *component);
                }
                offset += body.len();
                bodies.extend(body);
            }
            set.extend(bodies);
            sets.push(set);
        }
        let mut subtable = Vec::new();
        put_u16(&mut subtable, 1);
        put_u16(&mut subtable, u16::try_from(header_len).expect("offset"));
        put_u16(&mut subtable, u16::try_from(set_count).expect("set 수"));
        let mut offset = header_len + coverage.len();
        for set in &sets {
            put_u16(&mut subtable, u16::try_from(offset).expect("offset"));
            offset += set.len();
        }
        subtable.extend(coverage);
        for set in sets {
            subtable.extend(set);
        }

        let mut lookup_list = Vec::new();
        put_u16(&mut lookup_list, 1);
        put_u16(&mut lookup_list, 4); // Lookup offset
        put_u16(&mut lookup_list, 4); // LookupType 4 = ligature
        put_u16(&mut lookup_list, 0); // flag
        put_u16(&mut lookup_list, 1);
        put_u16(&mut lookup_list, 8); // subtable offset from Lookup
        lookup_list.extend(subtable);

        let mut b = Vec::new();
        put_u16(&mut b, 1);
        put_u16(&mut b, 0);
        let script_offset = 10;
        let feature_offset = script_offset + script_list.len();
        let lookup_offset = feature_offset + feature_list.len();
        put_u16(&mut b, u16::try_from(script_offset).expect("offset"));
        put_u16(&mut b, u16::try_from(feature_offset).expect("offset"));
        put_u16(&mut b, u16::try_from(lookup_offset).expect("offset"));
        b.extend(script_list);
        b.extend(feature_list);
        b.extend(lookup_list);
        b
    }

    fn hvar(&self, axis: &WeightAxis) -> Vec<u8> {
        let glyphs = usize::from(self.glyph_count());
        let mut b = Vec::new();
        put_u16(&mut b, 1);
        put_u16(&mut b, 0);
        put_u32(&mut b, 20); // ItemVariationStore offset
        put_u32(&mut b, 0); // advanceWidthMapping(없음 = glyph id 암시 매핑)
        put_u32(&mut b, 0);
        put_u32(&mut b, 0);
        // ItemVariationStore.
        put_u16(&mut b, 1); // format
        put_u32(&mut b, 12); // VariationRegionList offset(store 기준)
        put_u16(&mut b, 1); // data 수
        put_u32(&mut b, 22); // ItemVariationData offset(store 기준)
                             // VariationRegionList: axis 1개, region 1개(0 → 1.0 peak).
        put_u16(&mut b, 1);
        put_u16(&mut b, 1);
        put_u16(&mut b, 0);
        put_u16(&mut b, 0x4000);
        put_u16(&mut b, 0x4000);
        // ItemVariationData: glyph마다 i16 delta 하나.
        put_u16(&mut b, u16::try_from(glyphs).expect("glyph 수"));
        put_u16(&mut b, 1); // wordDeltaCount
        put_u16(&mut b, 1); // regionIndexCount
        put_u16(&mut b, 0);
        for gid in 0..glyphs {
            put_i16(&mut b, axis.advance_deltas.get(gid).copied().unwrap_or(0));
        }
        b
    }
}

fn fvar(axis: &WeightAxis) -> Vec<u8> {
    let mut b = Vec::new();
    put_u16(&mut b, 1);
    put_u16(&mut b, 0);
    put_u16(&mut b, 16); // axesArrayOffset
    put_u16(&mut b, 2); // reserved
    put_u16(&mut b, 1); // axisCount
    put_u16(&mut b, 20); // axisSize
    put_u16(&mut b, 0); // instanceCount
    put_u16(&mut b, 8); // instanceSize
    b.extend_from_slice(b"wght");
    for value in [axis.min, axis.default, axis.max] {
        put_u32(&mut b, (value as u32) << 16);
    }
    put_u16(&mut b, 0);
    put_u16(&mut b, 256);
    b
}

fn sfnt(tables: &BTreeMap<[u8; 4], Vec<u8>>) -> Vec<u8> {
    let count = tables.len();
    let mut entry_selector = 0u16;
    while (1usize << (entry_selector + 1)) <= count {
        entry_selector += 1;
    }
    let search_range = (1u16 << entry_selector) * 16;
    let mut out = Vec::new();
    put_u32(&mut out, 0x0001_0000);
    put_u16(&mut out, u16::try_from(count).expect("table 수"));
    put_u16(&mut out, search_range);
    put_u16(&mut out, entry_selector);
    put_u16(
        &mut out,
        u16::try_from(count * 16).expect("범위") - search_range,
    );
    let mut offset = 12 + 16 * count;
    let mut body = Vec::new();
    for (tag, data) in tables {
        out.extend_from_slice(tag);
        put_u32(&mut out, checksum(data));
        put_u32(&mut out, u32::try_from(offset).expect("offset"));
        put_u32(&mut out, u32::try_from(data.len()).expect("길이"));
        body.extend_from_slice(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        offset = 12 + 16 * count + body.len();
    }
    out.extend(body);
    out
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn put_u16(b: &mut Vec<u8>, v: u16) {
    b.extend_from_slice(&v.to_be_bytes());
}

fn put_i16(b: &mut Vec<u8>, v: i16) {
    b.extend_from_slice(&v.to_be_bytes());
}

fn put_u32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_be_bytes());
}

/// 연속 codepoint 범위를 차례로 glyph에 매핑하는 정의를 쌓는 helper.
#[derive(Debug, Clone)]
pub(crate) struct FontSpec {
    font: TestFont,
}

impl FontSpec {
    /// `.notdef`(advance `notdef_advance`)만 있는 정의.
    pub(crate) fn new(upem: u16, ascender: i16, descender: i16, notdef_advance: u16) -> FontSpec {
        FontSpec {
            font: TestFont {
                upem,
                ascender,
                descender,
                line_gap: 0,
                advances: vec![notdef_advance],
                cmap_groups: Vec::new(),
                ligatures: Vec::new(),
                weight_axis: None,
            },
        }
    }

    /// line gap을 정합니다.
    pub(crate) fn line_gap(mut self, gap: i16) -> FontSpec {
        self.font.line_gap = gap;
        self
    }

    /// `[start, end]` codepoint를 새 glyph들에 매핑하고 advance는 `advance(cp)`로 정합니다.
    pub(crate) fn range(mut self, start: u32, end: u32, advance: impl Fn(u32) -> u16) -> FontSpec {
        let first = u32::try_from(self.font.advances.len()).expect("glyph 수");
        for cp in start..=end {
            self.font.advances.push(advance(cp));
        }
        self.font.cmap_groups.push((start, end, first));
        self
    }

    /// cmap에 없는 추가 glyph(예: ligature)를 만들고 id를 돌려줍니다.
    pub(crate) fn extra_glyph(&mut self, advance: u16) -> u16 {
        self.font.advances.push(advance);
        u16::try_from(self.font.advances.len() - 1).expect("glyph id")
    }

    /// codepoint의 glyph id(정의된 범위에서).
    pub(crate) fn glyph_of(&self, cp: u32) -> u16 {
        self.font
            .cmap_groups
            .iter()
            .find(|(s, e, _)| (*s..=*e).contains(&cp))
            .map(|(s, _, g)| u16::try_from(g + (cp - s)).expect("glyph id"))
            .expect("정의된 codepoint")
    }

    /// ligature를 추가합니다.
    pub(crate) fn ligature(mut self, components: &[u32], advance: u16) -> FontSpec {
        let glyphs: Vec<u16> = components.iter().map(|cp| self.glyph_of(*cp)).collect();
        let lig = self.extra_glyph(advance);
        self.font.ligatures.push((glyphs, lig));
        self
    }

    /// `wght` axis를 추가합니다. `delta(gid)`는 최대값에서의 advance 변화입니다.
    pub(crate) fn weight_axis(
        mut self,
        min: i32,
        default: i32,
        max: i32,
        delta: impl Fn(u16) -> i16,
    ) -> FontSpec {
        let deltas = (0..self.font.advances.len())
            .map(|g| delta(u16::try_from(g).expect("glyph id")))
            .collect();
        self.font.weight_axis = Some(WeightAxis {
            min,
            default,
            max,
            advance_deltas: deltas,
        });
        self
    }

    /// 정의.
    pub(crate) fn font(&self) -> &TestFont {
        &self.font
    }

    /// SFNT bytes.
    pub(crate) fn build(&self) -> Vec<u8> {
        self.font.build()
    }
}

/// Latin 기본 advance: 공백 250, 좁은 글자(i, l, j, t, f, 구두점) 300, 넓은 글자(m, w, M, W) 800,
/// 나머지 500(upem 1000).
pub(crate) fn latin_advance(cp: u32) -> u16 {
    match char::from_u32(cp).unwrap_or(' ') {
        ' ' => 250,
        'i' | 'l' | 'j' | 't' | 'f' | '.' | ',' | '!' | '\'' | ':' | ';' => 300,
        'm' | 'w' | 'M' | 'W' => 800,
        _ => 500,
    }
}

/// 출력 가능한 ASCII(U+0020..U+007E)를 가진 Latin 폰트 정의(upem 1000, asc 800, desc -200).
pub(crate) fn latin_spec() -> FontSpec {
    FontSpec::new(1000, 800, -200, 500).range(0x20, 0x7E, latin_advance)
}

/// Latin 폰트 + `fi`·`ffi` ligature(`liga`).
pub(crate) fn latin_liga_spec() -> FontSpec {
    latin_spec()
        .ligature(&[u32::from('f'), u32::from('i')], 550)
        .ligature(&[u32::from('f'), u32::from('f'), u32::from('i')], 800)
}

/// Latin + 히브리어·아랍어 + 공백을 가진 폰트(upem 1000). 아랍어 400, 히브리어 450.
pub(crate) fn latin_rtl_spec() -> FontSpec {
    latin_spec()
        .range(0x0590, 0x05FF, |_| 450)
        .range(0x0600, 0x06FF, |_| 400)
}

/// 한글 음절·한자·가나·CJK 구두점·공백을 가진 CJK 폰트(upem 1000, 모두 1000, 공백 500).
pub(crate) fn cjk_spec() -> FontSpec {
    FontSpec::new(1000, 880, -120, 1000)
        .line_gap(100)
        .range(0x20, 0x20, |_| 500)
        .range(0x3000, 0x30FF, |_| 1000)
        .range(0x4E00, 0x9FFF, |_| 1000)
        .range(0xAC00, 0xD7A3, |_| 1000)
        .range(0xFF01, 0xFF5E, |_| 1000)
}

/// emoji 폰트(upem 1000, advance 1200): U+1F300..U+1F6FF, U+1F900..U+1F9FF, ZWJ, VS16.
pub(crate) fn emoji_spec() -> FontSpec {
    FontSpec::new(1000, 950, -250, 1200)
        .range(0x200D, 0x200D, |_| 0)
        .range(0xFE0F, 0xFE0F, |_| 0)
        .range(0x1F300, 0x1F6FF, |_| 1200)
        .range(0x1F900, 0x1F9FF, |_| 1200)
}

/// `wght` 100..400..900 variable Latin 폰트. 최대값에서 모든 cmap glyph가 +100 넓어집니다.
pub(crate) fn variable_latin_spec() -> FontSpec {
    latin_spec().weight_axis(100, 400, 900, |gid| if gid == 0 { 0 } else { 100 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_are_deterministic_and_well_formed() {
        let a = latin_liga_spec().build();
        let b = latin_liga_spec().build();
        assert_eq!(a, b);
        // sfnt version과 table 수(head, hhea, maxp, hmtx, cmap, GSUB = 6).
        assert_eq!(&a[0..4], &[0, 1, 0, 0]);
        assert_eq!(u16::from_be_bytes([a[4], a[5]]), 6);
        assert_eq!(latin_spec().glyph_of(u32::from('A')), 34);
        assert_eq!(latin_spec().font().advances.len(), 96);
        let variable = variable_latin_spec().build();
        assert_eq!(u16::from_be_bytes([variable[4], variable[5]]), 7);
    }

    #[test]
    fn every_fixture_font_loads_with_expected_coverage() {
        use crate::shaping::{FontFace, LineMetrics};
        let cases: [(FontSpec, char, char); 6] = [
            (latin_spec(), 'A', '한'),
            (latin_liga_spec(), 'f', '😀'),
            (latin_rtl_spec(), 'م', '한'),
            (cjk_spec(), '한', 'A'),
            (emoji_spec(), '😀', 'A'),
            (variable_latin_spec(), 'z', 'م'),
        ];
        for (spec, covered, uncovered) in cases {
            let face = FontFace::from_bytes(spec.build(), 0).expect("합성 폰트 로드");
            assert_eq!(
                face.glyph_count() as usize,
                spec.font().advances.len(),
                "{covered}"
            );
            assert_eq!(
                face.nominal_glyph(covered).map(|g| g.raw()),
                Some(u32::from(spec.glyph_of(u32::from(covered))))
            );
            assert_eq!(face.nominal_glyph(uncovered), None);
        }
        let cjk = FontFace::from_bytes(cjk_spec().build(), 0).expect("cjk");
        assert_eq!(
            cjk.line_metrics(),
            LineMetrics {
                ascender: 880,
                descender: -120,
                line_gap: 100
            }
        );
    }
}
