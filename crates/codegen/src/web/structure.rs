//! IR node → HTML semantic 구조 (XXII.2.1, ADR-0106 결정 4).
//!
//! Fragment가 아닌 IR node마다 element 하나를 만들고, Fragment 자식은 가장 가까운 element 조상에 바로 붙입니다.
//! tag는 element 종류와 Control contract로 정합니다.
//!
//! | 입력 | element |
//! |---|---|
//! | Frame | `div` |
//! | Fragment | 없음 |
//! | Text | `span`(텍스트 내용) |
//! | Image | 자식이 없으면 `img alt=""`, 있으면 `div` |
//! | Shape·Path·ComponentInstance | `div` |
//! | Button contract | `button type="button"` |
//! | TextInput contract | 자식·텍스트가 없으면 `input type="text"`, 아니면 `div` |
//! | Modal contract | `dialog open` |
//! | List contract | element 자식이 모두 contract 없는 element면 `ul`(자식은 `li`), 아니면 `div` |
//! | SingleSelectionGroup contract | `div` |
//!
//! contract가 둘 이상이면 번호 오름차순 첫 contract가 tag를 정합니다. 접근성 mapper([`super::aria`])는 이 구조
//! 위에서 접근성 entry가 있는 element만 role에 맞게 고칩니다.

use stellar_target_profile::capability::{ContractCode, ElementKind};

use crate::ir::IrTree;
use crate::web::html::{HtmlChild, HtmlDocument};

/// 구조 매핑 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebStructure {
    /// HTML 조각.
    pub document: HtmlDocument,
    /// IR node index → element index(Fragment는 `None`).
    pub element_of: Vec<Option<u32>>,
}

/// tag를 정하는 contract(번호 오름차순 첫 contract).
fn primary_contract(contracts: &[ContractCode]) -> Option<ContractCode> {
    contracts.iter().copied().min()
}

/// IR tree의 HTML semantic 구조를 만듭니다.
pub fn map_structure(ir: &IrTree) -> WebStructure {
    let mut doc = HtmlDocument::new();
    let mut element_of: Vec<Option<u32>> = vec![None; ir.nodes.len()];
    let parents = ir.element_parents();
    // 1단계: element tree(preorder라 부모 element가 먼저 생김).
    for (index, node) in ir.nodes.iter().enumerate() {
        if node.element == ElementKind::FRAGMENT {
            continue;
        }
        let element = doc.add("div", Some(node.id));
        element_of[index] = Some(element);
        match parents[index].and_then(|p| element_of[p as usize]) {
            Some(parent) => doc.append(parent, element),
            None => doc.push_root(element),
        }
    }
    // 2단계: tag와 기본 속성.
    for (index, node) in ir.nodes.iter().enumerate() {
        let Some(element) = element_of[index] else {
            continue;
        };
        let has_children = doc
            .element(element)
            .children
            .iter()
            .any(|c| matches!(c, HtmlChild::Element(_)));
        match primary_contract(&node.contracts) {
            Some(ContractCode::BUTTON) => {
                doc.set_tag(element, "button");
                doc.set_attr(element, "type", "button");
            }
            Some(ContractCode::TEXT_INPUT) if !has_children && node.text.is_none() => {
                doc.set_tag(element, "input");
                doc.set_attr(element, "type", "text");
            }
            Some(ContractCode::MODAL) => {
                doc.set_tag(element, "dialog");
                doc.set_attr(element, "open", "");
            }
            Some(ContractCode::LIST) => {
                let items: Vec<u32> = element_children(&doc, element);
                let plain = !items.is_empty()
                    && items.iter().all(|child| {
                        let source = doc.element(*child).source;
                        source
                            .and_then(|id| ir.node(id))
                            .is_some_and(|n| n.contracts.is_empty())
                    });
                if plain {
                    doc.set_tag(element, "ul");
                }
            }
            Some(_) => {}
            None => match node.element {
                ElementKind::TEXT => doc.set_tag(element, "span"),
                ElementKind::IMAGE if !has_children => {
                    doc.set_tag(element, "img");
                    doc.set_attr(element, "alt", "");
                }
                _ => {}
            },
        }
    }
    // 3단계: `ul` 자식은 `li`, 텍스트 내용.
    for (index, node) in ir.nodes.iter().enumerate() {
        let Some(element) = element_of[index] else {
            continue;
        };
        let in_list = doc
            .element(element)
            .parent
            .is_some_and(|p| doc.element(p).tag == "ul");
        if in_list {
            doc.set_tag(element, "li");
        }
        if let Some(text) = &node.text {
            let leaf = element_children(&doc, element).is_empty();
            if leaf && !doc.element(element).is_void() {
                doc.append_text(element, text.clone());
            }
        }
    }
    WebStructure {
        document: doc,
        element_of,
    }
}

fn element_children(doc: &HtmlDocument, element: u32) -> Vec<u32> {
    doc.element(element)
        .children
        .iter()
        .filter_map(|child| match child {
            HtmlChild::Element(child) => Some(*child),
            HtmlChild::Text(_) => None,
        })
        .collect()
}

/// golden 비교용 DOM tree 텍스트: element마다 한 줄, 두 칸 들여쓰기, `<tag 속성> #<source StableId>`.
pub fn dom_tree_text(doc: &HtmlDocument) -> String {
    let mut out = String::new();
    let mut stack: Vec<(u32, usize)> = doc.roots().iter().rev().map(|r| (*r, 0)).collect();
    while let Some((index, depth)) = stack.pop() {
        let element = doc.element(index);
        let mut attributes: Vec<String> = element
            .attributes
            .iter()
            .map(|(name, value)| {
                if value.is_empty() {
                    name.clone()
                } else {
                    format!("{name}={value:?}")
                }
            })
            .collect();
        attributes.sort();
        let source = element
            .source
            .map(|id| format!(" #{id}"))
            .unwrap_or_default();
        let attrs = if attributes.is_empty() {
            String::new()
        } else {
            format!(" {}", attributes.join(" "))
        };
        let texts: Vec<&str> = element
            .children
            .iter()
            .filter_map(|c| match c {
                HtmlChild::Text(t) => Some(t.as_str()),
                HtmlChild::Element(_) => None,
            })
            .collect();
        let text = if texts.is_empty() {
            String::new()
        } else {
            format!(" text={:?}", texts.join(""))
        };
        out.push_str(&format!(
            "{}<{}{attrs}>{source}{text}\n",
            "  ".repeat(depth),
            element.tag
        ));
        for child in element.children.iter().rev() {
            if let HtmlChild::Element(child) = child {
                stack.push((*child, depth + 1));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{self as fx, check_golden, sid, Fixture};

    #[test]
    fn golden_dom_tree_maps_frame_fragment_text_image_and_controls() {
        // 검증 기준(XXII.2.1): golden DOM tree.
        let fixture = Fixture::new();
        let ir = fixture.ir(&fx::web_profile());
        let structure = map_structure(&ir);
        check_golden("structure_site.tree", &dom_tree_text(&structure.document));
        let tag = |node: u8| {
            let index = ir.node_index(sid(node)).expect("node") as usize;
            structure.element_of[index].map(|e| structure.document.element(e).tag)
        };
        assert_eq!(tag(1), Some("div")); // Frame
        assert_eq!(tag(2), Some("span")); // Text
        assert_eq!(tag(3), None); // Fragment
        assert_eq!(tag(4), Some("img")); // Image(leaf)
        assert_eq!(tag(8), Some("button")); // Button contract
        assert_eq!(tag(11), Some("input")); // TextInput contract
        assert_eq!(tag(16), Some("dialog")); // Modal contract
        assert_eq!(tag(6), Some("div")); // SingleSelectionGroup
                                         // Fragment 자식은 가장 가까운 element 조상(1)에 붙습니다.
        let image = structure.element_of[ir.node_index(sid(4)).unwrap() as usize].unwrap();
        let root = structure.element_of[0].unwrap();
        assert_eq!(structure.document.element(image).parent, Some(root));
    }

    #[test]
    fn list_contract_with_plain_items_is_ul_li() {
        let mut fixture = fx::list_fixture(3);
        fixture.view.nodes[0].contracts = vec![ContractCode::LIST];
        let ir = fixture.ir(&fx::web_profile());
        let structure = map_structure(&ir);
        let text = dom_tree_text(&structure.document);
        check_golden("structure_list.tree", &text);
    }

    #[test]
    fn structure_is_deterministic() {
        let fixture = Fixture::new();
        let a = map_structure(&fixture.ir(&fx::web_profile()));
        let b = map_structure(&fixture.ir(&fx::web_profile()));
        assert_eq!(a, b);
        assert_eq!(a.document.to_html(), b.document.to_html());
    }
}
