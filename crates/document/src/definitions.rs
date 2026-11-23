//! 문서·페이지·View 정의 (VI.1.1).
//!
//! [`ProjectDocumentIndex`] → [`DocumentDefinition`] → [`PageDefinition`] →
//! [`ViewDefinition`]의 소유 계층입니다. 각 타입은 고유 [`StableId`]와 표시
//! 이름을 갖습니다. 표시 이름은 식별에 사용하지 않습니다(VI.1.3).
//!
//! binary 인코딩은 canonical 규칙(V.1: LE 고정 폭, `u32` 길이 프리픽스 UTF-8)을
//! 따릅니다. crate 그래프상 storage에 의존하지 않으므로 이 모듈이 직접
//! 구현합니다 — chunk 직렬화(VI.10.2)가 이 인코딩을 소비합니다.

use std::fmt;

use stellar_core_types::ids::StableId;

/// 정의 계층 조작·디코딩 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefinitionError {
    /// 같은 StableId가 이미 존재합니다.
    DuplicateId {
        /// 중복 id.
        id: StableId,
    },
    /// 대상 id를 찾을 수 없습니다.
    NotFound {
        /// 요청 id.
        id: StableId,
    },
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
    /// View에 root Frame이 없습니다(VI.1.2).
    MissingRootFrame {
        /// 해당 View id.
        view: StableId,
    },
    /// View에 root Frame이 두 개 이상입니다(VI.1.2).
    MultipleRootFrames {
        /// 해당 View id.
        view: StableId,
        /// root 후보 수.
        count: usize,
    },
    /// 두 View가 같은 root Frame을 공유합니다(VI.1.2).
    SharedRootFrame {
        /// 공유된 root Frame id.
        root_frame: StableId,
    },
    /// 삽입 위치가 목록 길이보다 큽니다(ADR-0079).
    IndexOutOfRange {
        /// 요청 위치.
        index: usize,
        /// 현재 목록 길이.
        len: usize,
    },
}

impl fmt::Display for DefinitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DefinitionError::DuplicateId { id } => write!(f, "중복 StableId: {id}"),
            DefinitionError::NotFound { id } => write!(f, "찾을 수 없는 StableId: {id}"),
            DefinitionError::Malformed { reason } => write!(f, "binary 형식 위반: {reason}"),
            DefinitionError::MissingRootFrame { view } => {
                write!(f, "View {view}에 root Frame이 없습니다")
            }
            DefinitionError::MultipleRootFrames { view, count } => {
                write!(f, "View {view}에 root Frame이 {count}개입니다(정확히 1개)")
            }
            DefinitionError::SharedRootFrame { root_frame } => {
                write!(f, "root Frame {root_frame}을 두 View가 공유합니다")
            }
            DefinitionError::IndexOutOfRange { index, len } => {
                write!(f, "위치 {index}가 목록 길이 {len}보다 큽니다")
            }
        }
    }
}

impl std::error::Error for DefinitionError {}

/// View 정의: 편집 화면 하나.
///
/// 소유 관계(VI.1.2): View는 정확히 하나의 [`PageDefinition`]에 속하고,
/// **정확히 하나의 root Frame**을 가집니다 — 필드 타입이 단일 `StableId`이므로
/// root 없음/복수 root는 표현 자체가 불가능하고, 목록 입력 경로는
/// [`ViewDefinition::with_roots`]가 거부합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewDefinition {
    /// 고유 StableId.
    pub id: StableId,
    /// 표시 이름(비어 있을 수 있음). 식별·저장 경로에 사용 금지 — 접근은
    /// [`ViewDefinition::display_name`]/[`ViewDefinition::rename`]으로만.
    display_name: String,
    /// 이 View의 root Frame node StableId(정확히 하나).
    pub root_frame: StableId,
}

impl ViewDefinition {
    /// View를 만듭니다.
    pub fn new(id: StableId, display_name: String, root_frame: StableId) -> ViewDefinition {
        ViewDefinition {
            id,
            display_name,
            root_frame,
        }
    }

    /// 표시 이름(식별에 사용 금지).
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// 표시 이름을 바꿉니다. id·root frame·참조에는 영향이 없습니다(VI.1.3).
    pub fn rename(&mut self, display_name: String) {
        self.display_name = display_name;
    }

    /// root 후보 목록으로부터 View를 만듭니다(VI.1.2).
    ///
    /// 파일·병합 등 외부 입력이 주는 root 목록이 정확히 1개가 아니면
    /// 거부합니다.
    pub fn with_roots(
        id: StableId,
        display_name: String,
        roots: &[StableId],
    ) -> Result<ViewDefinition, DefinitionError> {
        match roots {
            [root] => Ok(ViewDefinition {
                id,
                display_name,
                root_frame: *root,
            }),
            [] => Err(DefinitionError::MissingRootFrame { view: id }),
            _ => Err(DefinitionError::MultipleRootFrames {
                view: id,
                count: roots.len(),
            }),
        }
    }
}

/// 페이지 정의: View들을 담습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageDefinition {
    /// 고유 StableId.
    pub id: StableId,
    /// 표시 이름(식별·저장 경로에 사용 금지).
    display_name: String,
    /// 소유한 View들(표시 순).
    pub views: Vec<ViewDefinition>,
}

/// 문서 정의: 페이지들을 담습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentDefinition {
    /// 문서 공통 Text style 표. 구형 정의에는 비어 있습니다.
    pub text_styles: crate::text_styles::DocumentTextStyles,
    /// 고유 StableId.
    pub id: StableId,
    /// 표시 이름(식별·저장 경로에 사용 금지).
    display_name: String,
    /// 소유한 페이지들(표시 순).
    pub pages: Vec<PageDefinition>,
}

/// 프로젝트의 문서 목록 index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectDocumentIndex {
    /// 문서들(표시 순).
    pub documents: Vec<DocumentDefinition>,
}

impl PageDefinition {
    /// 페이지를 만듭니다.
    pub fn new(id: StableId, display_name: String) -> PageDefinition {
        PageDefinition {
            id,
            display_name,
            views: Vec::new(),
        }
    }

    /// 표시 이름(식별에 사용 금지).
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// 표시 이름을 바꿉니다. id·소유 View·참조에는 영향이 없습니다(VI.1.3).
    pub fn rename(&mut self, display_name: String) {
        self.display_name = display_name;
    }

    /// View를 추가합니다. 같은 id가 있으면 거부합니다.
    pub fn add_view(&mut self, view: ViewDefinition) -> Result<(), DefinitionError> {
        if self.views.iter().any(|v| v.id == view.id) {
            return Err(DefinitionError::DuplicateId { id: view.id });
        }
        self.views.push(view);
        Ok(())
    }

    /// View를 삭제합니다. 없으면 오류입니다.
    pub fn remove_view(&mut self, id: StableId) -> Result<ViewDefinition, DefinitionError> {
        let index = self
            .views
            .iter()
            .position(|v| v.id == id)
            .ok_or(DefinitionError::NotFound { id })?;
        Ok(self.views.remove(index))
    }

    /// View를 `index` 위치(표시 순)에 넣습니다. 같은 id가 있거나 위치가 길이보다 크면 거부합니다
    /// (ADR-0079).
    pub fn insert_view(
        &mut self,
        index: usize,
        view: ViewDefinition,
    ) -> Result<(), DefinitionError> {
        if self.views.iter().any(|v| v.id == view.id) {
            return Err(DefinitionError::DuplicateId { id: view.id });
        }
        if index > self.views.len() {
            return Err(DefinitionError::IndexOutOfRange {
                index,
                len: self.views.len(),
            });
        }
        self.views.insert(index, view);
        Ok(())
    }

    /// id로 View를 찾습니다.
    pub fn view(&self, id: StableId) -> Option<&ViewDefinition> {
        self.views.iter().find(|v| v.id == id)
    }
}

impl DocumentDefinition {
    /// 문서를 만듭니다.
    pub fn new(id: StableId, display_name: String) -> DocumentDefinition {
        DocumentDefinition {
            text_styles: Default::default(),
            id,
            display_name,
            pages: Vec::new(),
        }
    }

    /// 표시 이름(식별에 사용 금지).
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// 표시 이름을 바꿉니다. id·소유 페이지·참조에는 영향이 없습니다(VI.1.3).
    pub fn rename(&mut self, display_name: String) {
        self.display_name = display_name;
    }

    /// 페이지를 추가합니다. 같은 id가 있으면 거부합니다.
    pub fn add_page(&mut self, page: PageDefinition) -> Result<(), DefinitionError> {
        if self.pages.iter().any(|p| p.id == page.id) {
            return Err(DefinitionError::DuplicateId { id: page.id });
        }
        self.pages.push(page);
        Ok(())
    }

    /// 페이지를 삭제합니다. 없으면 오류입니다.
    pub fn remove_page(&mut self, id: StableId) -> Result<PageDefinition, DefinitionError> {
        let index = self
            .pages
            .iter()
            .position(|p| p.id == id)
            .ok_or(DefinitionError::NotFound { id })?;
        Ok(self.pages.remove(index))
    }

    /// 페이지를 `index` 위치(표시 순)에 넣습니다. 같은 id가 있거나 위치가 길이보다 크면 거부합니다
    /// (ADR-0079).
    pub fn insert_page(
        &mut self,
        index: usize,
        page: PageDefinition,
    ) -> Result<(), DefinitionError> {
        if self.pages.iter().any(|p| p.id == page.id) {
            return Err(DefinitionError::DuplicateId { id: page.id });
        }
        if index > self.pages.len() {
            return Err(DefinitionError::IndexOutOfRange {
                index,
                len: self.pages.len(),
            });
        }
        self.pages.insert(index, page);
        Ok(())
    }

    /// id로 페이지를 찾습니다.
    pub fn page(&self, id: StableId) -> Option<&PageDefinition> {
        self.pages.iter().find(|p| p.id == id)
    }

    /// id로 페이지를 찾습니다(가변).
    pub fn page_mut(&mut self, id: StableId) -> Option<&mut PageDefinition> {
        self.pages.iter_mut().find(|p| p.id == id)
    }

    /// View의 (페이지 위치, 페이지 안 View 위치).
    pub fn view_position(&self, id: StableId) -> Option<(usize, usize)> {
        self.pages
            .iter()
            .enumerate()
            .find_map(|(page_index, page)| {
                page.views
                    .iter()
                    .position(|v| v.id == id)
                    .map(|view_index| (page_index, view_index))
            })
    }

    /// id로 View를 찾습니다(가변).
    pub fn view_mut(&mut self, id: StableId) -> Option<&mut ViewDefinition> {
        self.pages
            .iter_mut()
            .flat_map(|p| p.views.iter_mut())
            .find(|v| v.id == id)
    }
}

impl ProjectDocumentIndex {
    /// 빈 index.
    pub fn new() -> ProjectDocumentIndex {
        ProjectDocumentIndex::default()
    }

    /// 문서를 추가합니다. 같은 id가 있으면 거부합니다.
    pub fn add_document(&mut self, document: DocumentDefinition) -> Result<(), DefinitionError> {
        if self.documents.iter().any(|d| d.id == document.id) {
            return Err(DefinitionError::DuplicateId { id: document.id });
        }
        self.documents.push(document);
        Ok(())
    }

    /// 문서를 삭제합니다. 없으면 오류입니다.
    pub fn remove_document(&mut self, id: StableId) -> Result<DocumentDefinition, DefinitionError> {
        let index = self
            .documents
            .iter()
            .position(|d| d.id == id)
            .ok_or(DefinitionError::NotFound { id })?;
        Ok(self.documents.remove(index))
    }

    /// id로 문서를 찾습니다.
    pub fn document(&self, id: StableId) -> Option<&DocumentDefinition> {
        self.documents.iter().find(|d| d.id == id)
    }

    /// 소유 관계 불변식을 검증합니다(VI.1.2).
    ///
    /// - 문서·페이지·View id가 전체에서 유일해야 합니다(각 항목은 정확히 하나의
    ///   부모에 속함).
    /// - 서로 다른 View가 같은 root Frame을 공유할 수 없습니다.
    pub fn validate_ownership(&self) -> Result<(), DefinitionError> {
        let mut ids = std::collections::BTreeSet::new();
        let mut roots = std::collections::BTreeSet::new();
        for document in &self.documents {
            if !ids.insert(document.id) {
                return Err(DefinitionError::DuplicateId { id: document.id });
            }
            for page in &document.pages {
                if !ids.insert(page.id) {
                    return Err(DefinitionError::DuplicateId { id: page.id });
                }
                for view in &page.views {
                    if !ids.insert(view.id) {
                        return Err(DefinitionError::DuplicateId { id: view.id });
                    }
                    if !roots.insert(view.root_frame) {
                        return Err(DefinitionError::SharedRootFrame {
                            root_frame: view.root_frame,
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

// --- binary 인코딩 (canonical 규칙 R1~R3) ---

fn put_string(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(bytes);
}

struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DefinitionError> {
        if self.input.len() - self.pos < n {
            return Err(DefinitionError::Malformed {
                reason: "입력 부족",
            });
        }
        let slice = &self.input[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn take_u32(&mut self) -> Result<u32, DefinitionError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4바이트"),
        ))
    }

    fn take_id(&mut self) -> Result<StableId, DefinitionError> {
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(self.take(16)?);
        Ok(StableId::from_bytes(bytes))
    }

    fn take_string(&mut self) -> Result<String, DefinitionError> {
        let len = self.take_u32()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| DefinitionError::Malformed {
            reason: "UTF-8 아님",
        })
    }

    fn expect_end(&self) -> Result<(), DefinitionError> {
        if self.pos == self.input.len() {
            Ok(())
        } else {
            Err(DefinitionError::Malformed {
                reason: "trailing bytes",
            })
        }
    }
}

impl ViewDefinition {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.id.to_bytes());
        put_string(buf, &self.display_name);
        buf.extend_from_slice(&self.root_frame.to_bytes());
    }

    fn decode_from(reader: &mut Reader<'_>) -> Result<ViewDefinition, DefinitionError> {
        Ok(ViewDefinition {
            id: reader.take_id()?,
            display_name: reader.take_string()?,
            root_frame: reader.take_id()?,
        })
    }
}

impl PageDefinition {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.id.to_bytes());
        put_string(buf, &self.display_name);
        buf.extend_from_slice(&(self.views.len() as u32).to_le_bytes());
        for view in &self.views {
            view.encode_into(buf);
        }
    }

    fn decode_from(reader: &mut Reader<'_>) -> Result<PageDefinition, DefinitionError> {
        let id = reader.take_id()?;
        let display_name = reader.take_string()?;
        let count = reader.take_u32()? as usize;
        let mut views = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            views.push(ViewDefinition::decode_from(reader)?);
        }
        Ok(PageDefinition {
            id,
            display_name,
            views,
        })
    }
}

impl DocumentDefinition {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.id.to_bytes());
        put_string(buf, &self.display_name);
        buf.extend_from_slice(&(self.pages.len() as u32).to_le_bytes());
        for page in &self.pages {
            page.encode_into(buf);
        }
    }

    fn decode_from(reader: &mut Reader<'_>) -> Result<DocumentDefinition, DefinitionError> {
        let id = reader.take_id()?;
        let display_name = reader.take_string()?;
        let count = reader.take_u32()? as usize;
        let mut pages = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            pages.push(PageDefinition::decode_from(reader)?);
        }
        Ok(DocumentDefinition {
            text_styles: Default::default(),
            id,
            display_name,
            pages,
        })
    }
}

impl DocumentDefinition {
    /// 문서 정의 하나를 canonical binary bytes로 인코딩합니다(DocumentManifest chunk 용,
    /// VI.10.2). 형식은 index 안의 문서 항목과 같습니다.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode_into(&mut buf);
        if !self.text_styles.is_empty() {
            buf.extend(self.text_styles.encode());
        }
        buf
    }

    /// [`DocumentDefinition::encode`] bytes에서 디코딩합니다. malformed 입력은 panic 없이
    /// 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<DocumentDefinition, DefinitionError> {
        let mut reader = Reader {
            input: bytes,
            pos: 0,
        };
        let mut definition = DocumentDefinition::decode_from(&mut reader)?;
        if reader.pos != bytes.len() {
            definition.text_styles = crate::text_styles::DocumentTextStyles::decode(
                &bytes[reader.pos..],
            )
            .map_err(|_| DefinitionError::Malformed {
                reason: "Text style 표 형식",
            })?;
            if definition.text_styles.is_empty() {
                return Err(DefinitionError::Malformed {
                    reason: "빈 Text style 표 중복 표현",
                });
            }
        }
        Ok(definition)
    }
}

impl ProjectDocumentIndex {
    /// canonical binary bytes로 인코딩합니다. index는 문서 이름·페이지·View만 담습니다. Text style 표는
    /// 각 문서 정의([`DocumentDefinition::encode`])가 저장합니다.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(self.documents.len() as u32).to_le_bytes());
        for document in &self.documents {
            document.encode_into(&mut buf);
        }
        buf
    }

    /// binary bytes에서 디코딩합니다. malformed 입력은 panic 없이 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<ProjectDocumentIndex, DefinitionError> {
        let mut reader = Reader {
            input: bytes,
            pos: 0,
        };
        let count = reader.take_u32()? as usize;
        let mut documents = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            documents.push(DocumentDefinition::decode_from(&mut reader)?);
        }
        reader.expect_end()?;
        Ok(ProjectDocumentIndex { documents })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn sample_index() -> ProjectDocumentIndex {
        let mut index = ProjectDocumentIndex::new();
        let mut doc = DocumentDefinition {
            text_styles: Default::default(),
            id: stable(1),
            display_name: "메인 문서".to_string(),
            pages: vec![],
        };
        let mut page = PageDefinition {
            id: stable(10),
            display_name: "페이지 1".to_string(),
            views: vec![],
        };
        page.add_view(ViewDefinition {
            id: stable(100),
            display_name: "홈 화면".to_string(),
            root_frame: stable(5_100),
        })
        .unwrap();
        page.add_view(ViewDefinition {
            id: stable(101),
            display_name: String::new(), // 빈 표시 이름 허용.
            root_frame: stable(5_101),
        })
        .unwrap();
        doc.add_page(page).unwrap();
        index.add_document(doc).unwrap();
        index
    }

    #[test]
    fn create_and_remove_at_each_level() {
        // 검증 기준(VI.1.1): 생성·삭제.
        let mut index = sample_index();
        assert!(index.document(stable(1)).is_some());

        // 중복 생성 거부.
        assert_eq!(
            index
                .add_document(DocumentDefinition {
                    text_styles: Default::default(),
                    id: stable(1),
                    display_name: String::new(),
                    pages: vec![],
                })
                .unwrap_err(),
            DefinitionError::DuplicateId { id: stable(1) }
        );
        let page = &mut index.documents[0].pages[0];
        assert!(matches!(
            page.add_view(ViewDefinition {
                id: stable(100),
                display_name: String::new(),
                root_frame: stable(5_999),
            }),
            Err(DefinitionError::DuplicateId { .. })
        ));

        // 삭제와 미존재 삭제.
        assert_eq!(page.remove_view(stable(100)).unwrap().id, stable(100));
        assert!(matches!(
            page.remove_view(stable(100)),
            Err(DefinitionError::NotFound { .. })
        ));
        let removed = index.remove_document(stable(1)).unwrap();
        assert_eq!(removed.id, stable(1));
        assert!(index.documents.is_empty());
    }

    #[test]
    fn binary_round_trip() {
        // 검증 기준(VI.1.1): binary round-trip.
        let index = sample_index();
        let bytes = index.encode();
        let decoded = ProjectDocumentIndex::decode(&bytes).unwrap();
        assert_eq!(decoded, index);
        // 재인코딩 byte-identical.
        assert_eq!(decoded.encode(), bytes);
    }

    #[test]
    fn empty_index_round_trips() {
        let index = ProjectDocumentIndex::new();
        let bytes = index.encode();
        assert_eq!(bytes, vec![0, 0, 0, 0]);
        assert_eq!(ProjectDocumentIndex::decode(&bytes).unwrap(), index);
    }

    #[test]
    fn korean_and_emoji_names_round_trip() {
        let mut index = ProjectDocumentIndex::new();
        index
            .add_document(DocumentDefinition {
                text_styles: Default::default(),
                id: stable(1),
                display_name: "한글 이름 🎨".to_string(),
                pages: vec![],
            })
            .unwrap();
        let decoded = ProjectDocumentIndex::decode(&index.encode()).unwrap();
        assert_eq!(decoded.documents[0].display_name, "한글 이름 🎨");
    }

    #[test]
    fn view_requires_exactly_one_root_frame() {
        // 검증 기준(VI.1.2): root 없음·root 두 개 fixture 거부.
        assert_eq!(
            ViewDefinition::with_roots(stable(100), String::new(), &[]).unwrap_err(),
            DefinitionError::MissingRootFrame { view: stable(100) }
        );
        assert_eq!(
            ViewDefinition::with_roots(stable(100), String::new(), &[stable(5_100), stable(5_101)])
                .unwrap_err(),
            DefinitionError::MultipleRootFrames {
                view: stable(100),
                count: 2
            }
        );
        let view =
            ViewDefinition::with_roots(stable(100), String::new(), &[stable(5_100)]).unwrap();
        assert_eq!(view.root_frame, stable(5_100));
    }

    #[test]
    fn ownership_validation_rejects_shared_roots_and_duplicate_ids() {
        let mut index = sample_index();
        assert!(index.validate_ownership().is_ok());

        // 두 View가 같은 root Frame 공유 → 거부.
        index.documents[0].pages[0].views[1].root_frame = stable(5_100);
        assert_eq!(
            index.validate_ownership().unwrap_err(),
            DefinitionError::SharedRootFrame {
                root_frame: stable(5_100)
            }
        );

        // 페이지 id가 문서 id와 충돌 → 거부(전역 유일성).
        let mut index = sample_index();
        index.documents[0].pages[0].id = stable(1);
        assert!(matches!(
            index.validate_ownership().unwrap_err(),
            DefinitionError::DuplicateId { .. }
        ));
    }

    #[test]
    fn rename_does_not_change_ids_or_references() {
        // 검증 기준(VI.1.3): 이름 변경 후 참조·chunk 경로가 변하지 않습니다.
        // chunk 경로는 StableId/content hash로만 유도되므로(V.2.3) 여기서는
        // 식별자·참조 불변을 고정합니다.
        let mut index = sample_index();
        let doc_id = index.documents[0].id;
        let page_id = index.documents[0].pages[0].id;
        let view_id = index.documents[0].pages[0].views[0].id;
        let root_before = index.documents[0].pages[0].views[0].root_frame;

        index.documents[0].rename("문서 새 이름".to_string());
        index.documents[0].pages[0].rename("페이지 새 이름".to_string());
        index.documents[0].pages[0].views[0].rename("뷰 새 이름".to_string());
        // 빈 이름도 허용.
        index.documents[0].pages[0].views[0].rename(String::new());

        assert_eq!(index.documents[0].id, doc_id);
        assert_eq!(index.documents[0].pages[0].id, page_id);
        assert_eq!(index.documents[0].pages[0].views[0].id, view_id);
        assert_eq!(index.documents[0].pages[0].views[0].root_frame, root_before);
        // id 기반 조회(참조)가 그대로 동작.
        assert!(index.document(doc_id).is_some());
        assert!(index.validate_ownership().is_ok());

        // 인코딩에서 id 위치의 bytes도 불변(이름 길이만 다름).
        let bytes = index.encode();
        let decoded = ProjectDocumentIndex::decode(&bytes).unwrap();
        assert_eq!(decoded.documents[0].id, doc_id);
        assert_eq!(decoded.documents[0].display_name(), "문서 새 이름");
    }

    #[test]
    fn constructors_build_hierarchy() {
        let mut doc = DocumentDefinition::new(stable(1), "문서".to_string());
        let mut page = PageDefinition::new(stable(10), String::new());
        page.add_view(ViewDefinition::new(
            stable(100),
            "뷰".to_string(),
            stable(5_100),
        ))
        .unwrap();
        doc.add_page(page).unwrap();
        assert_eq!(doc.display_name(), "문서");
        assert_eq!(doc.pages[0].display_name(), "");
        assert_eq!(doc.pages[0].views[0].display_name(), "뷰");
    }

    #[test]
    fn malformed_bytes_are_rejected() {
        let bytes = sample_index().encode();
        for cut in [0, 3, 4, 19, bytes.len() - 1] {
            assert!(
                ProjectDocumentIndex::decode(&bytes[..cut]).is_err(),
                "절단 {cut}"
            );
        }
        let mut extended = bytes.clone();
        extended.push(0xff);
        assert!(ProjectDocumentIndex::decode(&extended).is_err());
        // 비UTF-8 이름.
        let mut index = ProjectDocumentIndex::new();
        index
            .add_document(DocumentDefinition {
                text_styles: Default::default(),
                id: stable(1),
                display_name: "ab".to_string(),
                pages: vec![],
            })
            .unwrap();
        let mut corrupt = index.encode();
        let len = corrupt.len();
        corrupt[len - 2] = 0xff;
        corrupt[len - 1] = 0xfe;
        assert!(ProjectDocumentIndex::decode(&corrupt).is_err());
    }

    #[test]
    fn single_document_definition_round_trips() {
        // VI.10.2: DocumentManifest chunk가 쓰는 단일 문서 인코딩.
        let index = sample_index();
        let definition = &index.documents[0];
        let bytes = definition.encode();
        let decoded = DocumentDefinition::decode(&bytes).unwrap();
        assert_eq!(&decoded, definition);
        assert_eq!(decoded.encode(), bytes);
        // index 안의 항목 형식과 동일(문서 수 u32 프리픽스 뒤).
        assert_eq!(&index.encode()[4..], &bytes[..]);
        assert!(DocumentDefinition::decode(&bytes[..bytes.len() - 1]).is_err());
        let mut extended = bytes;
        extended.push(0);
        assert!(DocumentDefinition::decode(&extended).is_err());
    }
}
