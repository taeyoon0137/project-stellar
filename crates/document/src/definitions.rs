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
}

impl fmt::Display for DefinitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DefinitionError::DuplicateId { id } => write!(f, "중복 StableId: {id}"),
            DefinitionError::NotFound { id } => write!(f, "찾을 수 없는 StableId: {id}"),
            DefinitionError::Malformed { reason } => write!(f, "binary 형식 위반: {reason}"),
        }
    }
}

impl std::error::Error for DefinitionError {}

/// View 정의: 편집 화면 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewDefinition {
    /// 고유 StableId.
    pub id: StableId,
    /// 표시 이름(비어 있을 수 있음, 식별에 사용 금지).
    pub display_name: String,
}

/// 페이지 정의: View들을 담습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageDefinition {
    /// 고유 StableId.
    pub id: StableId,
    /// 표시 이름.
    pub display_name: String,
    /// 소유한 View들(표시 순).
    pub views: Vec<ViewDefinition>,
}

/// 문서 정의: 페이지들을 담습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentDefinition {
    /// 고유 StableId.
    pub id: StableId,
    /// 표시 이름.
    pub display_name: String,
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
}

impl DocumentDefinition {
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
    }

    fn decode_from(reader: &mut Reader<'_>) -> Result<ViewDefinition, DefinitionError> {
        Ok(ViewDefinition {
            id: reader.take_id()?,
            display_name: reader.take_string()?,
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
            id,
            display_name,
            pages,
        })
    }
}

impl ProjectDocumentIndex {
    /// canonical binary bytes로 인코딩합니다.
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
        })
        .unwrap();
        page.add_view(ViewDefinition {
            id: stable(101),
            display_name: String::new(), // 빈 표시 이름 허용.
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
                id: stable(1),
                display_name: "한글 이름 🎨".to_string(),
                pages: vec![],
            })
            .unwrap();
        let decoded = ProjectDocumentIndex::decode(&index.encode()).unwrap();
        assert_eq!(decoded.documents[0].display_name, "한글 이름 🎨");
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
}
