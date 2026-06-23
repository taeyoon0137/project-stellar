//! WBS ID parser (I.2.2).
//!
//! 형식: `<Roman Phase>.<group>.<item>` (예: `I.1.1`, `XXVIII.3.3`).
//! Phase는 정규 로마 숫자, group·item은 선행 0이 없는 1 이상의 십진수입니다.

use std::cmp::Ordering;
use std::fmt;

/// 파싱·정렬 가능한 WBS 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WbsId {
    phase: u32,
    group: u32,
    item: u32,
}

impl WbsId {
    /// 문자열을 WBS ID로 파싱합니다. 형식 위반은 `None`.
    pub fn parse(text: &str) -> Option<WbsId> {
        let parts: Vec<&str> = text.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        let phase = parse_roman(parts[0])?;
        let group = parse_subnumber(parts[1])?;
        let item = parse_subnumber(parts[2])?;
        Some(WbsId { phase, group, item })
    }

    pub fn phase(&self) -> u32 {
        self.phase
    }
    pub fn group(&self) -> u32 {
        self.group
    }
    pub fn item(&self) -> u32 {
        self.item
    }
}

impl fmt::Display for WbsId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // phase는 파싱 시 정규 로마 숫자임이 보장됩니다.
        let roman = u32_to_roman(self.phase).unwrap_or_default();
        write!(f, "{roman}.{}.{}", self.group, self.item)
    }
}

impl Ord for WbsId {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.phase, self.group, self.item).cmp(&(other.phase, other.group, other.item))
    }
}

impl PartialOrd for WbsId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 선행 0이 없는 1 이상의 십진수만 허용합니다.
fn parse_subnumber(text: &str) -> Option<u32> {
    if text.is_empty() {
        return None;
    }
    if text.len() > 1 && text.starts_with('0') {
        return None; // 선행 0
    }
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return None; // suffix·비숫자
    }
    let value: u32 = text.parse().ok()?;
    if value == 0 {
        None
    } else {
        Some(value)
    }
}

/// 정규(canonical) 로마 숫자만 허용합니다. 비정규 표기는 `None`.
fn parse_roman(text: &str) -> Option<u32> {
    let values: Vec<u32> = text
        .chars()
        .map(roman_char_value)
        .collect::<Option<Vec<u32>>>()?;
    if values.is_empty() {
        return None;
    }
    let mut total: i64 = 0;
    let mut iter = values.iter().peekable();
    while let Some(&value) = iter.next() {
        match iter.peek() {
            Some(&&next) if value < next => total -= i64::from(value),
            _ => total += i64::from(value),
        }
    }
    let number = u32::try_from(total).ok()?;
    // 정규성 검사: 다시 인코딩한 결과가 입력과 같아야 합니다.
    if u32_to_roman(number)?.as_str() == text {
        Some(number)
    } else {
        None
    }
}

fn roman_char_value(ch: char) -> Option<u32> {
    match ch {
        'I' => Some(1),
        'V' => Some(5),
        'X' => Some(10),
        'L' => Some(50),
        'C' => Some(100),
        'D' => Some(500),
        'M' => Some(1000),
        _ => None,
    }
}

fn u32_to_roman(mut number: u32) -> Option<String> {
    if number == 0 || number > 3999 {
        return None;
    }
    const TABLE: [(u32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (value, symbol) in TABLE {
        while number >= value {
            out.push_str(symbol);
            number -= value;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_ids() {
        let a = WbsId::parse("I.1.1").unwrap();
        assert_eq!(a.phase(), 1);
        assert_eq!(a.to_string(), "I.1.1");

        let b = WbsId::parse("XXVIII.3.3").unwrap();
        assert_eq!(b.phase(), 28);
        assert_eq!(b.to_string(), "XXVIII.3.3");
    }

    #[test]
    fn rejects_malformed_ids() {
        for bad in [
            "1.1.1", "I.0.1", "I.1", "I.1.1.1", "I.01.1", "I.1.1a", "", "IIII.1.1",
        ] {
            assert!(WbsId::parse(bad).is_none(), "should reject {bad}");
        }
    }

    #[test]
    fn orders_by_phase_then_group_then_item() {
        let mut ids = [
            WbsId::parse("II.1.1").unwrap(),
            WbsId::parse("I.2.1").unwrap(),
            WbsId::parse("I.1.2").unwrap(),
            WbsId::parse("I.1.1").unwrap(),
        ];
        ids.sort();
        let ordered: Vec<String> = ids.iter().map(WbsId::to_string).collect();
        assert_eq!(ordered, ["I.1.1", "I.1.2", "I.2.1", "II.1.1"]);
    }

    #[test]
    fn display_round_trips_through_parse() {
        for text in ["I.1.1", "X.10.20", "XXVIII.3.3"] {
            let id = WbsId::parse(text).unwrap();
            assert_eq!(WbsId::parse(&id.to_string()), Some(id));
        }
    }
}
