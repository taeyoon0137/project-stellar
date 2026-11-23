//! 캔버스 Text 편집 buffer (XII.10.33, ADR-0118 결정 2).
//!
//! [`TextEditBuffer`]는 Text 하나를 편집하는 동안의 본문, 선택(anchor·focus), 입력기 조합(marked text)을 갖는
//! 순수 상태입니다. 문서·history를 모릅니다. 확정 입력(타이핑·삭제·조합 확정)은 [`TextEdit`] 하나를 돌려주고,
//! 호출자(runtime 캔버스 편집기)가 그 값을 문서 범위 교체 command로 바꿉니다. 조합 중 본문은 [`TextEditBuffer::
//! display_text`]로만 보이고 [`TextEdit`]를 만들지 않습니다.
//!
//! - 위치는 문서와 같은 Unicode scalar offset(`u32`)입니다. byte offset을 밖으로 내보내지 않습니다.
//! - 삭제와 caret 이동의 단위는 확장 grapheme cluster(UAX #29)입니다. 조합 문자·emoji ZWJ 열·국기를 한 번에
//!   지웁니다. 단어 이동은 UAX #29 단어 경계에서 공백·문장 부호만인 조각을 건너뜁니다.
//! - 조합(AppKit `setMarkedText`·`unmarkText`·`insertText`의 의미): 조합을 시작하면 그때의 선택 범위가 조합으로
//!   바뀔 범위입니다. 조합 중 `insert`는 조합 범위를 넣은 문자열로 확정합니다. 빈 marked text는 조합 취소입니다.
//! - 조합 중 삭제·caret 이동은 입력기 몫이라 buffer는 아무것도 하지 않습니다(`None`/`false`).

use unicode_segmentation::UnicodeSegmentation;

/// scalar offset 범위. `start <= end`입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ScalarRange {
    /// 시작(포함).
    pub start: u32,
    /// 끝(제외).
    pub end: u32,
}

impl ScalarRange {
    /// `start <= end`일 때만 만듭니다.
    pub const fn new(start: u32, end: u32) -> Option<ScalarRange> {
        if start <= end {
            Some(ScalarRange { start, end })
        } else {
            None
        }
    }

    /// 빈 범위(caret).
    pub const fn caret(at: u32) -> ScalarRange {
        ScalarRange { start: at, end: at }
    }

    /// 길이.
    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    /// 비었는지.
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// 편집 선택. `anchor`는 고정 끝, `focus`는 움직이는 끝(caret)입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EditSelection {
    /// 고정 끝.
    pub anchor: u32,
    /// 움직이는 끝.
    pub focus: u32,
}

impl EditSelection {
    /// caret 하나.
    pub const fn caret(at: u32) -> EditSelection {
        EditSelection {
            anchor: at,
            focus: at,
        }
    }

    /// 정렬한 범위.
    pub fn range(self) -> ScalarRange {
        ScalarRange {
            start: self.anchor.min(self.focus),
            end: self.anchor.max(self.focus),
        }
    }
}

/// 확정 입력의 종류. history 연속 입력 묶기(VII.4.3)의 입력 종류와 1:1입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditKind {
    /// 빈 범위에 문자열을 넣음(타이핑, 빈 선택의 조합 확정).
    Insert,
    /// 뒤로 지우기(Backspace, 선택 지우기 포함).
    DeleteBackward,
    /// 앞으로 지우기(Delete).
    DeleteForward,
    /// 비어 있지 않은 범위를 다른 문자열로 바꿈(선택 위 타이핑·붙여넣기, 선택 위 조합 확정).
    Replace,
}

/// 확정 입력 하나: 편집 전 본문의 `range`를 `replacement`로 바꿉니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    /// 편집 전 본문 기준 scalar 범위.
    pub range: ScalarRange,
    /// 넣는 문자열(삭제는 빈 문자열).
    pub replacement: String,
    /// 입력 종류.
    pub kind: EditKind,
}

/// 진행 중 입력기 조합.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composition {
    /// 확정 본문에서 조합이 바꿀 범위(조합 시작 때 선택).
    pub replaced: ScalarRange,
    /// 조합 중 문자열.
    pub marked: String,
    /// 조합 문자열 안에서 입력기가 고른 범위(변환 후보 밑줄 등, marked 기준 scalar offset).
    pub selected: ScalarRange,
}

/// 편집 중 Text 하나의 본문·선택·조합.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEditBuffer {
    text: String,
    selection: EditSelection,
    composition: Option<Composition>,
}

fn scalar_len(text: &str) -> u32 {
    u32::try_from(text.chars().count()).unwrap_or(u32::MAX)
}

fn byte_at(text: &str, scalar: u32) -> usize {
    text.char_indices()
        .nth(scalar as usize)
        .map_or(text.len(), |(b, _)| b)
}

fn scalar_at(text: &str, byte: usize) -> u32 {
    u32::try_from(text[..byte].chars().count()).unwrap_or(u32::MAX)
}

/// grapheme 경계(scalar offset, 끝 포함). 앞 경계부터 이어 세어 본문 길이에 선형입니다.
fn grapheme_bounds(text: &str) -> Vec<u32> {
    let mut bounds = Vec::new();
    let mut at = 0u32;
    for grapheme in text.graphemes(true) {
        bounds.push(at);
        at = at.saturating_add(scalar_len(grapheme));
    }
    bounds.push(at);
    bounds
}

impl TextEditBuffer {
    /// 본문 끝에 caret을 둔 buffer.
    pub fn new(text: impl Into<String>) -> TextEditBuffer {
        let text = text.into();
        let end = scalar_len(&text);
        TextEditBuffer {
            text,
            selection: EditSelection::caret(end),
            composition: None,
        }
    }

    /// 확정 본문.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 확정 본문 scalar 길이.
    pub fn len(&self) -> u32 {
        scalar_len(&self.text)
    }

    /// 확정 본문이 비었는지.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// 선택.
    pub fn selection(&self) -> EditSelection {
        self.selection
    }

    /// 진행 중 조합.
    pub fn composition(&self) -> Option<&Composition> {
        self.composition.as_ref()
    }

    /// 조합을 반영한 표시 본문. 조합이 없으면 확정 본문과 같습니다.
    pub fn display_text(&self) -> String {
        match &self.composition {
            None => self.text.clone(),
            Some(c) => {
                let start = byte_at(&self.text, c.replaced.start);
                let end = byte_at(&self.text, c.replaced.end);
                format!("{}{}{}", &self.text[..start], c.marked, &self.text[end..])
            }
        }
    }

    /// 표시 본문 기준 caret(조합 중이면 조합 안의 입력기 선택 끝).
    pub fn display_caret(&self) -> u32 {
        match &self.composition {
            None => self.selection.focus,
            Some(c) => c.replaced.start + c.selected.end,
        }
    }

    /// 선택을 바꿉니다(pointer로 caret 놓기 등). 범위 밖은 끝으로 맞추고 grapheme 경계로 내립니다. 조합 중이면
    /// 바꾸지 않고 `false`입니다.
    pub fn set_selection(&mut self, selection: EditSelection) -> bool {
        if self.composition.is_some() {
            return false;
        }
        let bounds = grapheme_bounds(&self.text);
        let snap = |offset: u32| {
            let index = bounds.partition_point(|&b| b <= offset);
            bounds[index.saturating_sub(1)]
        };
        self.selection = EditSelection {
            anchor: snap(selection.anchor),
            focus: snap(selection.focus),
        };
        true
    }

    /// 전체 선택.
    pub fn select_all(&mut self) -> bool {
        self.set_selection(EditSelection {
            anchor: 0,
            focus: self.len(),
        })
    }

    fn replace(&mut self, range: ScalarRange, replacement: &str, kind: EditKind) -> TextEdit {
        let start = byte_at(&self.text, range.start);
        let end = byte_at(&self.text, range.end);
        self.text.replace_range(start..end, replacement);
        self.selection = EditSelection::caret(range.start + scalar_len(replacement));
        TextEdit {
            range,
            replacement: replacement.to_owned(),
            kind,
        }
    }

    /// 확정 입력(타이핑·붙여넣기·조합 확정). 조합 중이면 조합 범위를, 아니면 선택을 `text`로 바꿉니다. 바뀌는
    /// 것이 없으면 `None`입니다(빈 선택에 빈 문자열, 같은 문자열로 같은 범위 교체).
    pub fn insert(&mut self, text: &str) -> Option<TextEdit> {
        let range = match self.composition.take() {
            Some(c) => c.replaced,
            None => self.selection.range(),
        };
        let start = byte_at(&self.text, range.start);
        let end = byte_at(&self.text, range.end);
        if self.text[start..end] == *text {
            self.selection = EditSelection::caret(range.start + scalar_len(text));
            return None;
        }
        let kind = if range.is_empty() {
            EditKind::Insert
        } else if text.is_empty() {
            EditKind::DeleteBackward
        } else {
            EditKind::Replace
        };
        Some(self.replace(range, text, kind))
    }

    /// 조합 문자열을 바꿉니다(문서 편집 없음). 처음 부르면 지금 선택이 조합으로 바뀔 범위입니다. `marked`가
    /// 비면 조합을 취소합니다. `selected`는 marked 기준이며 길이를 넘으면 끝으로 맞춥니다.
    pub fn set_marked(&mut self, marked: &str, selected: ScalarRange) {
        if marked.is_empty() {
            self.composition = None;
            return;
        }
        let replaced = match &self.composition {
            Some(c) => c.replaced,
            None => self.selection.range(),
        };
        let len = scalar_len(marked);
        self.composition = Some(Composition {
            replaced,
            marked: marked.to_owned(),
            selected: ScalarRange {
                start: selected.start.min(len),
                end: selected.end.min(len),
            },
        });
    }

    /// 조합을 지금 문자열 그대로 확정합니다(`unmarkText`). 조합이 없으면 `None`입니다.
    pub fn commit_composition(&mut self) -> Option<TextEdit> {
        let marked = self.composition.as_ref()?.marked.clone();
        self.insert(&marked)
    }

    /// 조합을 버립니다(확정 본문·선택은 그대로).
    pub fn cancel_composition(&mut self) -> bool {
        self.composition.take().is_some()
    }

    /// 뒤로 지우기. 선택이 있으면 선택을, 없으면 caret 앞 grapheme 하나를 지웁니다.
    pub fn delete_backward(&mut self) -> Option<TextEdit> {
        if self.composition.is_some() {
            return None;
        }
        let range = self.selection.range();
        let range = if range.is_empty() {
            let bounds = grapheme_bounds(&self.text);
            let index = bounds.partition_point(|&b| b < range.start);
            let start = *bounds.get(index.checked_sub(1)?)?;
            ScalarRange::new(start, range.start)?
        } else {
            range
        };
        Some(self.replace(range, "", EditKind::DeleteBackward))
    }

    /// 앞으로 지우기. 선택이 있으면 선택을, 없으면 caret 뒤 grapheme 하나를 지웁니다.
    pub fn delete_forward(&mut self) -> Option<TextEdit> {
        if self.composition.is_some() {
            return None;
        }
        let range = self.selection.range();
        let (range, kind) = if range.is_empty() {
            let bounds = grapheme_bounds(&self.text);
            let end = *bounds.iter().find(|&&b| b > range.start)?;
            (ScalarRange::new(range.start, end)?, EditKind::DeleteForward)
        } else {
            (range, EditKind::DeleteBackward)
        };
        Some(self.replace(range, "", kind))
    }

    fn move_focus(&mut self, to: u32, extend: bool) -> bool {
        if self.composition.is_some() {
            return false;
        }
        let before = self.selection;
        self.selection = if extend {
            EditSelection {
                anchor: before.anchor,
                focus: to,
            }
        } else {
            EditSelection::caret(to)
        };
        self.selection != before
    }

    /// 왼쪽(논리 앞) grapheme 하나. 선택이 있고 `extend`가 아니면 선택 앞 끝으로 접습니다.
    pub fn move_backward(&mut self, extend: bool) -> bool {
        let range = self.selection.range();
        if !extend && !range.is_empty() {
            return self.move_focus(range.start, false);
        }
        let focus = self.selection.focus;
        let bounds = grapheme_bounds(&self.text);
        let index = bounds.partition_point(|&b| b < focus);
        let to = index.checked_sub(1).map_or(0, |i| bounds[i]);
        self.move_focus(to, extend)
    }

    /// 오른쪽(논리 뒤) grapheme 하나. 선택이 있고 `extend`가 아니면 선택 뒤 끝으로 접습니다.
    pub fn move_forward(&mut self, extend: bool) -> bool {
        let range = self.selection.range();
        if !extend && !range.is_empty() {
            return self.move_focus(range.end, false);
        }
        let focus = self.selection.focus;
        let to = grapheme_bounds(&self.text)
            .into_iter()
            .find(|&b| b > focus)
            .unwrap_or(focus);
        self.move_focus(to, extend)
    }

    /// 단어 조각의 (시작, 끝). 앞 조각부터 이어 세어 본문 길이에 선형입니다.
    fn word_starts(&self) -> Vec<(u32, u32)> {
        let mut at = 0u32;
        let mut words = Vec::new();
        for word in self.text.split_word_bounds() {
            let start = at;
            at = at.saturating_add(scalar_len(word));
            if word.chars().any(char::is_alphanumeric) {
                words.push((start, at));
            }
        }
        words
    }

    /// 앞 단어 시작으로(⌥←).
    pub fn move_word_backward(&mut self, extend: bool) -> bool {
        let focus = self.selection.focus;
        let to = self
            .word_starts()
            .into_iter()
            .rev()
            .find(|&(start, _)| start < focus)
            .map_or(0, |(start, _)| start);
        self.move_focus(to, extend)
    }

    /// 뒤 단어 끝으로(⌥→).
    pub fn move_word_forward(&mut self, extend: bool) -> bool {
        let focus = self.selection.focus;
        let to = self
            .word_starts()
            .into_iter()
            .find(|&(_, end)| end > focus)
            .map_or(self.len(), |(_, end)| end);
        self.move_focus(to, extend)
    }

    /// 지금 문단(`'\n'` 사이) 시작으로.
    pub fn move_paragraph_start(&mut self, extend: bool) -> bool {
        let byte = byte_at(&self.text, self.selection.focus);
        let to = self.text[..byte]
            .rfind('\n')
            .map_or(0, |b| scalar_at(&self.text, b) + 1);
        self.move_focus(to, extend)
    }

    /// 지금 문단 끝으로.
    pub fn move_paragraph_end(&mut self, extend: bool) -> bool {
        let byte = byte_at(&self.text, self.selection.focus);
        let to = self.text[byte..]
            .find('\n')
            .map_or(self.len(), |b| scalar_at(&self.text, byte + b));
        self.move_focus(to, extend)
    }

    /// 외부에서 확정 본문이 바뀌었을 때(undo 등) 본문을 바꿉니다. 본문이 다르면 caret을 바뀐 구간 끝(공통 앞·뒤
    /// 부분을 뺀 구간)에 두고, 같으면 선택을 범위 안으로 맞춥니다. 조합은 버립니다.
    pub fn reset_text(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.composition = None;
        if text == self.text {
            return;
        }
        let old: Vec<char> = self.text.chars().collect();
        let new: Vec<char> = text.chars().collect();
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let caret = u32::try_from(new.len() - suffix).unwrap_or(u32::MAX);
        self.text = text;
        self.set_selection(EditSelection::caret(caret));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(start: u32, end: u32, text: &str, kind: EditKind) -> Option<TextEdit> {
        Some(TextEdit {
            range: ScalarRange::new(start, end).unwrap(),
            replacement: text.into(),
            kind,
        })
    }

    #[test]
    fn hangul_composition_stays_transient_until_commit() {
        let mut b = TextEditBuffer::new("가");
        // ㅎ → 하 → 한: 조합 중에는 확정 본문이 그대로이고 TextEdit가 없습니다.
        for step in ["ㅎ", "하", "한"] {
            b.set_marked(step, ScalarRange::caret(1));
            assert_eq!(b.text(), "가");
            assert_eq!(b.display_text(), format!("가{step}"));
        }
        assert_eq!(b.display_caret(), 2);
        // 다음 자모가 오면 입력기가 "한"을 확정하고 새 조합을 시작합니다.
        assert_eq!(b.insert("한"), edit(1, 1, "한", EditKind::Insert));
        b.set_marked("ㄱ", ScalarRange::caret(1));
        b.set_marked("그", ScalarRange::caret(1));
        b.set_marked("글", ScalarRange::caret(1));
        assert_eq!(b.display_text(), "가한글");
        assert_eq!(b.commit_composition(), edit(2, 2, "글", EditKind::Insert));
        assert_eq!(b.text(), "가한글");
        assert_eq!(b.selection(), EditSelection::caret(3));
        assert_eq!(b.composition(), None);
    }

    #[test]
    fn japanese_conversion_replaces_the_selection_once() {
        let mut b = TextEditBuffer::new("abc");
        b.set_selection(EditSelection {
            anchor: 1,
            focus: 2,
        });
        // かな 입력 → 漢字 변환(후보 선택은 marked 안 범위) → 확정.
        b.set_marked("か", ScalarRange::caret(1));
        b.set_marked("かん", ScalarRange::caret(2));
        b.set_marked("かんじ", ScalarRange::caret(3));
        b.set_marked("漢字", ScalarRange::new(0, 2).unwrap());
        assert_eq!(b.text(), "abc");
        assert_eq!(b.display_text(), "a漢字c");
        assert_eq!(
            b.composition().unwrap().selected,
            ScalarRange::new(0, 2).unwrap()
        );
        // 조합 중에는 삭제·caret 이동을 입력기에 맡깁니다.
        assert_eq!(b.delete_backward(), None);
        assert!(!b.move_forward(false));
        assert_eq!(b.insert("漢字"), edit(1, 2, "漢字", EditKind::Replace));
        assert_eq!(b.text(), "a漢字c");
        assert_eq!(b.selection(), EditSelection::caret(3));
    }

    #[test]
    fn empty_marked_text_cancels_and_same_text_is_no_edit() {
        let mut b = TextEditBuffer::new("x");
        b.set_marked("ㅎ", ScalarRange::caret(1));
        b.set_marked("", ScalarRange::caret(0));
        assert_eq!(b.composition(), None);
        assert_eq!(b.display_text(), "x");
        assert_eq!(b.insert(""), None);
        b.select_all();
        assert_eq!(b.insert("x"), None);
        assert_eq!(b.selection(), EditSelection::caret(1));
    }

    #[test]
    fn deletion_and_motion_use_grapheme_clusters() {
        // 👩‍👩‍👧 (ZWJ 5 scalar) + 결합 문자 e + U+0301.
        let mut b = TextEditBuffer::new("a👩\u{200d}👩\u{200d}👧e\u{301}");
        assert_eq!(b.len(), 8);
        assert_eq!(
            b.delete_backward(),
            edit(6, 8, "", EditKind::DeleteBackward)
        );
        assert_eq!(
            b.delete_backward(),
            edit(1, 6, "", EditKind::DeleteBackward)
        );
        assert_eq!(b.text(), "a");
        let mut b = TextEditBuffer::new("🇰🇷x");
        b.set_selection(EditSelection::caret(0));
        assert!(b.move_forward(false));
        assert_eq!(b.selection(), EditSelection::caret(2));
        assert_eq!(b.delete_forward(), edit(2, 3, "", EditKind::DeleteForward));
        assert!(b.move_backward(true));
        assert_eq!(
            b.selection(),
            EditSelection {
                anchor: 2,
                focus: 0
            }
        );
        // 선택이 있으면 앞으로 지우기도 선택 지우기입니다.
        assert_eq!(b.delete_forward(), edit(0, 2, "", EditKind::DeleteBackward));
        assert_eq!(b.delete_backward(), None);
        // 중간 offset은 grapheme 경계로 내립니다.
        let mut b = TextEditBuffer::new("🇰🇷");
        b.set_selection(EditSelection::caret(1));
        assert_eq!(b.selection(), EditSelection::caret(0));
    }

    #[test]
    fn word_and_paragraph_motion() {
        let mut b = TextEditBuffer::new("hello, 세계 ok\nnext line");
        b.set_selection(EditSelection::caret(0));
        assert!(b.move_word_forward(false));
        assert_eq!(b.selection().focus, 5);
        assert!(b.move_word_forward(false));
        assert_eq!(b.selection().focus, 9);
        assert!(b.move_paragraph_end(true));
        assert_eq!(b.selection().range(), ScalarRange::new(9, 12).unwrap());
        b.set_selection(EditSelection::caret(17));
        assert!(b.move_paragraph_start(false));
        assert_eq!(b.selection().focus, 13);
        assert!(b.move_word_backward(false));
        assert_eq!(b.selection().focus, 10);
        assert_eq!(
            b.insert("!"),
            edit(10, 10, "!", EditKind::Insert),
            "단어 시작에 넣기"
        );
    }

    #[test]
    fn reset_text_clamps_selection_and_drops_composition() {
        let mut b = TextEditBuffer::new("abcdef");
        b.set_marked("ㅎ", ScalarRange::caret(1));
        b.reset_text("ab");
        assert_eq!(b.composition(), None);
        assert_eq!(b.selection(), EditSelection::caret(2));
        // undo가 지운 "g"를 되살리면 caret은 되살린 구간 끝입니다.
        let mut b = TextEditBuffer::new("abcdef");
        b.set_selection(EditSelection::caret(2));
        b.reset_text("abcdefg");
        assert_eq!(b.selection(), EditSelection::caret(7));
        b.reset_text("aXcdefg");
        assert_eq!(b.selection(), EditSelection::caret(2));
    }
}
