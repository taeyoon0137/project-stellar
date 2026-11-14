//! 생성 HTML의 구조 모델과 결정론 writer (XXII.2.4).
//!
//! adapter는 문자열을 이어 붙이지 않고 [`HtmlDocument`]를 만든 뒤 [`HtmlDocument::to_html`]로 씁니다. 정적 접근성
//! 검사([`crate::web::check`])도 같은 모델을 읽습니다.
//!
//! writer 규칙: element마다 한 줄, 깊이마다 두 칸 들여쓰기. 텍스트만 가진 element는 한 줄에 닫습니다. 속성은
//! `id`, `type`, `role` 다음 이름 오름차순이고, 빈 값 속성은 이름만 씁니다(boolean 속성). 속성 값은 `&`·`"`·`<`·`>`,
//! 텍스트는 `&`·`<`·`>`를 escape합니다.
//! void element(`input`, `img`)는 닫는 tag가 없습니다. 재귀 없이 stack으로 씁니다.

use std::fmt::Write as _;

use stellar_core_types::ids::StableId;

/// element 자식.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HtmlChild {
    /// element(문서 안 index).
    Element(u32),
    /// 텍스트.
    Text(String),
}

/// element 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlElement {
    /// 이 element를 만든 IR node(손으로 만든 문서는 `None`).
    pub source: Option<StableId>,
    /// tag 이름(소문자).
    pub tag: &'static str,
    /// 속성(이름, 값). 같은 이름은 한 번만 둡니다.
    pub attributes: Vec<(String, String)>,
    /// 자식(순서대로).
    pub children: Vec<HtmlChild>,
    /// 부모 element index(root면 `None`).
    pub parent: Option<u32>,
}

impl HtmlElement {
    /// 속성 값.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// void element인지.
    pub fn is_void(&self) -> bool {
        matches!(self.tag, "input" | "img")
    }
}

/// HTML 조각(root element 하나 이상).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HtmlDocument {
    elements: Vec<HtmlElement>,
    roots: Vec<u32>,
}

fn attribute_rank(name: &str) -> (u8, &str) {
    match name {
        "id" => (0, name),
        "type" => (1, name),
        "role" => (2, name),
        _ => (3, name),
    }
}

fn escape_attribute(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn escape_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

impl HtmlDocument {
    /// 빈 문서.
    pub fn new() -> HtmlDocument {
        HtmlDocument::default()
    }

    /// element를 만들고 index를 돌려줍니다(부모 없음).
    pub fn add(&mut self, tag: &'static str, source: Option<StableId>) -> u32 {
        let index = self.elements.len() as u32;
        self.elements.push(HtmlElement {
            source,
            tag,
            attributes: Vec::new(),
            children: Vec::new(),
            parent: None,
        });
        index
    }

    /// root element로 등록합니다.
    pub fn push_root(&mut self, element: u32) {
        self.roots.push(element);
    }

    /// 자식 element를 붙입니다.
    pub fn append(&mut self, parent: u32, child: u32) {
        self.elements[child as usize].parent = Some(parent);
        self.elements[parent as usize]
            .children
            .push(HtmlChild::Element(child));
    }

    /// 텍스트 자식을 붙입니다.
    pub fn append_text(&mut self, parent: u32, text: impl Into<String>) {
        self.elements[parent as usize]
            .children
            .push(HtmlChild::Text(text.into()));
    }

    /// 속성을 설정합니다(이미 있으면 값을 바꿈).
    pub fn set_attr(&mut self, element: u32, name: &str, value: impl Into<String>) {
        let value = value.into();
        let attributes = &mut self.elements[element as usize].attributes;
        match attributes.iter_mut().find(|(key, _)| key == name) {
            Some(entry) => entry.1 = value,
            None => attributes.push((name.to_string(), value)),
        }
    }

    /// 속성을 지웁니다(없으면 그대로).
    pub fn remove_attr(&mut self, element: u32, name: &str) {
        self.elements[element as usize]
            .attributes
            .retain(|(key, _)| key != name);
    }

    /// tag를 바꿉니다.
    pub fn set_tag(&mut self, element: u32, tag: &'static str) {
        self.elements[element as usize].tag = tag;
    }

    /// element.
    pub fn element(&self, index: u32) -> &HtmlElement {
        &self.elements[index as usize]
    }

    /// 모든 element(만든 순서).
    pub fn elements(&self) -> &[HtmlElement] {
        &self.elements
    }

    /// root element.
    pub fn roots(&self) -> &[u32] {
        &self.roots
    }

    /// `id` 속성으로 element를 찾습니다(첫 element).
    pub fn by_id(&self, id: &str) -> Option<u32> {
        self.elements
            .iter()
            .position(|element| element.attr("id") == Some(id))
            .map(|index| index as u32)
    }

    /// 결정론 HTML 텍스트(모듈 문서 규칙).
    pub fn to_html(&self) -> String {
        enum Step<'a> {
            Open(u32, usize),
            Text(&'a str, usize),
            Close(u32, usize),
        }
        let mut out = String::new();
        let mut stack: Vec<Step<'_>> = self.roots.iter().rev().map(|r| Step::Open(*r, 0)).collect();
        while let Some(step) = stack.pop() {
            match step {
                Step::Open(index, depth) => {
                    let element = &self.elements[index as usize];
                    let _ = write!(out, "{}<{}", "  ".repeat(depth), element.tag);
                    let mut attributes: Vec<&(String, String)> =
                        element.attributes.iter().collect();
                    attributes.sort_by(|a, b| attribute_rank(&a.0).cmp(&attribute_rank(&b.0)));
                    for (name, value) in attributes {
                        if value.is_empty() {
                            let _ = write!(out, " {name}");
                        } else {
                            let _ = write!(out, " {name}=\"{}\"", escape_attribute(value));
                        }
                    }
                    out.push('>');
                    if element.is_void() {
                        out.push('\n');
                        continue;
                    }
                    let text_only = element
                        .children
                        .iter()
                        .all(|child| matches!(child, HtmlChild::Text(_)));
                    if text_only {
                        for child in &element.children {
                            if let HtmlChild::Text(text) = child {
                                out.push_str(&escape_text(text));
                            }
                        }
                        let _ = writeln!(out, "</{}>", element.tag);
                        continue;
                    }
                    out.push('\n');
                    stack.push(Step::Close(index, depth));
                    for child in element.children.iter().rev() {
                        stack.push(match child {
                            HtmlChild::Element(child) => Step::Open(*child, depth + 1),
                            HtmlChild::Text(text) => Step::Text(text, depth + 1),
                        });
                    }
                }
                Step::Text(text, depth) => {
                    let _ = writeln!(out, "{}{}", "  ".repeat(depth), escape_text(text));
                }
                Step::Close(index, depth) => {
                    let _ = writeln!(
                        out,
                        "{}</{}>",
                        "  ".repeat(depth),
                        self.elements[index as usize].tag
                    );
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_orders_attributes_escapes_and_closes_deterministically() {
        let mut doc = HtmlDocument::new();
        let root = doc.add("div", None);
        doc.push_root(root);
        doc.set_attr(root, "role", "group");
        doc.set_attr(root, "aria-label", "a \"b\" & <c>");
        doc.set_attr(root, "id", "r");
        let span = doc.add("span", None);
        doc.append(root, span);
        doc.append_text(span, "x < y & z");
        let input = doc.add("input", None);
        doc.append(root, input);
        doc.set_attr(input, "type", "text");
        doc.set_attr(input, "type", "text");
        doc.set_attr(input, "readonly", "");
        assert_eq!(
            doc.to_html(),
            "<div id=\"r\" role=\"group\" aria-label=\"a &quot;b&quot; &amp; &lt;c&gt;\">\n  \
             <span>x &lt; y &amp; z</span>\n  <input type=\"text\" readonly>\n</div>\n"
        );
        assert_eq!(doc.by_id("r"), Some(root));
        assert_eq!(doc.element(input).parent, Some(root));
    }

    #[test]
    fn deep_documents_are_written_without_recursion() {
        let mut doc = HtmlDocument::new();
        let mut parent = doc.add("div", None);
        doc.push_root(parent);
        for _ in 0..3_000 {
            let child = doc.add("div", None);
            doc.append(parent, child);
            parent = child;
        }
        let html = doc.to_html();
        assert_eq!(html.matches("<div").count(), 3_001);
        assert_eq!(html.matches("</div>").count(), 3_001);
    }
}
