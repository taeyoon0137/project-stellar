//! 프로젝트 전체 의미의 다중 파일 텍스트 원본(V.10.4, ADR-0077 결정 1).
//!
//! `design/` 아래 배치:
//!
//! ```text
//! design/
//!   document.stellar.yaml                       문서·페이지·View 목록과 참조
//!   pages/<page>/<view>.stellar.yaml            View 하나와 node
//!   pages/<page>/<view>/<frame>.stellar.yaml    큰 Frame 하위 파일($ref)
//!   components/<이름>-<hash12>.stellar.yaml     컴포넌트 정의 하나
//!   tokens/<set>.stellar.yaml                   token set 하나
//!   assets/catalog.stellar.yaml                 이미지·글꼴·Text style 표
//! ```
//!
//! 파일 이름은 slug이고 정체성은 파일 안의 StableId입니다. 결과는 모델만의 함수라 같은 프로젝트는 같은 파일
//! 집합·bytes가 됩니다. reader는 참조하지 않은 파일, 두 번 참조한 파일, 모르는 경로를 거부합니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

use stellar_assets::record::AssetRecord;
use stellar_core_types::ids::StableId;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::ModifierStack;
use stellar_document::records::{DocumentRecords, NodePayload, NodeRecord};
use stellar_document::validate::validate_document;
use stellar_storage::source_file;

use super::super::component_source::ComponentSource;
use super::super::text_codec::{field, fields, mapping, scalar, text};
use super::super::text_source::TextSourceError;
use super::catalog::{self, AssetCatalog, CATALOG_PATH};
use super::css::{self, invalid, unsupported};
use super::slug::unique_slugs;
use super::span::{node_spans, SpanTable};
use super::token::{self, TokenSetSource};
use super::{node, VERSION};
use crate::yaml::loader::MAX_INPUT_BYTES;
use crate::yaml::{load_restricted, write_canonical, write_canonical_checked, YamlValue};

/// 문서 파일 경로(`design/` 기준).
pub const DOCUMENT_PATH: &str = "document.stellar.yaml";
/// 직계 자식이 이 수를 넘는 Frame은 하위 파일로 나눕니다.
pub const SPLIT_CHILD_THRESHOLD: usize = 2_000;
/// 파일 하나의 상한(bytes). 넘으면 큰 Frame부터 하위 파일로 나눕니다.
pub const SPLIT_FILE_BYTES: usize = MAX_INPUT_BYTES;
/// 1 MiB 상한 때문에 나누는 Frame의 하위 tree 최소 크기. 작은 Frame을 하나씩 떼어 내 파일이 폭증하지 않게 합니다.
pub const MIN_SPLIT_SUBTREE_BYTES: usize = 64 * 1024;
/// 다중 파일 원본 전체 읽기 상한.
pub const MAX_PROJECT_BYTES: u64 = 64 * 1024 * 1024;
/// 다중 파일 원본 파일 수 상한.
pub const MAX_PROJECT_FILES: usize = 4_096;

const EXT: &str = ".stellar.yaml";
const REF: &str = "$ref";

/// 텍스트 원본이 담는 프로젝트 전체 의미.
#[derive(Debug)]
pub struct TextProject {
    /// root 문서 하나.
    pub document: DocumentModel,
    /// 컴포넌트 정의.
    pub components: ComponentLibrary,
    /// token set.
    pub token_sets: Vec<TokenSetSource>,
    /// asset StableId → 이미지 record.
    pub images: BTreeMap<StableId, AssetRecord>,
}

impl TextProject {
    /// 의미 비교용 정규 표현: 문서 레코드(정렬), 컴포넌트 library bytes, token set, catalog.
    pub fn semantic_key(&self) -> Result<SemanticKey, TextSourceError> {
        let mut records = self.document.to_records().map_err(invalid)?;
        records.sort_canonical();
        let mut tokens = self.token_sets.clone();
        tokens.sort_by_key(|set| set.id);
        Ok(SemanticKey {
            records,
            components: self.components.encode(),
            tokens,
            images: self.images.clone(),
        })
    }
}

/// [`TextProject::semantic_key`]의 결과.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticKey {
    /// 정렬한 문서 레코드.
    pub records: DocumentRecords,
    /// 컴포넌트 library canonical bytes.
    pub components: Vec<u8>,
    /// token set(id 순).
    pub tokens: Vec<TokenSetSource>,
    /// 이미지 record.
    pub images: BTreeMap<StableId, AssetRecord>,
}

/// 쓴 결과: `design/` 기준 경로 → 텍스트, node 위치 표.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedProject {
    /// 파일 내용.
    pub files: BTreeMap<String, String>,
    /// node → 파일·줄 범위(TS-MAP).
    pub spans: SpanTable,
}

fn line(value: impl Into<String>) -> YamlValue {
    YamlValue::scalar(value.into())
}

fn get<'a>(value: &'a YamlValue, key: &str) -> Result<&'a str, TextSourceError> {
    text(field(value, key)?)
}

fn sequence<'a>(value: &'a YamlValue, key: &str) -> Result<&'a [YamlValue], TextSourceError> {
    field(value, key)?
        .as_sequence()
        .ok_or_else(|| invalid(format!("{key}는 sequence여야 합니다")))
}

fn header(format: &str) -> Vec<(YamlValue, YamlValue)> {
    vec![
        (scalar("format"), line(format)),
        (scalar("schema_version"), line(VERSION)),
    ]
}

fn check_header(value: &YamlValue, format: &str) -> Result<(), TextSourceError> {
    if get(value, "format")? != format {
        return Err(unsupported(format!("{format} 파일이 아닙니다")));
    }
    if get(value, "schema_version")? != VERSION {
        return Err(unsupported("텍스트 원본 버전"));
    }
    Ok(())
}

/// `dir/name` 형식으로 이어 붙입니다(빈 dir은 name 그대로).
fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// 참조 파일 기준 상대 `$ref`를 `design/` 기준 경로로 바꿉니다. 아래쪽 상대 경로만 받습니다.
fn resolve_ref(from: &str, target: &str) -> Result<String, TextSourceError> {
    if target.is_empty()
        || target.starts_with('/')
        || target.contains('\\')
        || !target.ends_with(EXT)
        || target
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(invalid(format!(
            "$ref는 아래쪽 상대 경로여야 합니다: {target}"
        )));
    }
    Ok(join(parent_dir(from), target))
}

// --- 쓰기 ---

struct ViewLayout<'a> {
    records: &'a BTreeMap<StableId, &'a NodeRecord>,
    children: BTreeMap<StableId, Vec<StableId>>,
    stacks: &'a BTreeMap<StableId, &'a ModifierStack>,
    sizes: BTreeMap<StableId, usize>,
}

impl ViewLayout<'_> {
    fn node_value(&self, id: StableId) -> Result<YamlValue, TextSourceError> {
        node::node(self.records[&id], Some(self.stacks.get(&id).copied()))
    }

    /// file root `root`가 담는 node(나뉜 하위 Frame과 그 아래는 제외), StableId 순.
    fn members(&self, root: StableId, splits: &BTreeSet<StableId>) -> Vec<StableId> {
        let mut out = Vec::new();
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            out.push(id);
            for child in self.children.get(&id).into_iter().flatten() {
                if !splits.contains(child) {
                    pending.push(*child);
                }
            }
        }
        out.sort();
        out
    }

    /// file root 아래에서 바로 나뉜 하위 Frame.
    fn nested_splits(&self, root: StableId, splits: &BTreeSet<StableId>) -> Vec<StableId> {
        let mut out = Vec::new();
        for member in self.members(root, splits) {
            for child in self.children.get(&member).into_iter().flatten() {
                if splits.contains(child) {
                    out.push(*child);
                }
            }
        }
        out.sort();
        out
    }

    fn file_bytes(&self, root: StableId, splits: &BTreeSet<StableId>) -> usize {
        self.members(root, splits)
            .iter()
            .map(|id| self.sizes[id])
            .sum::<usize>()
            + self.nested_splits(root, splits).len() * 128
            + 512
    }

    /// 파일 상한을 지키는 분할 Frame 집합(모델만의 함수).
    fn splits(&self, view_root: StableId) -> Result<BTreeSet<StableId>, TextSourceError> {
        let mut splits: BTreeSet<StableId> = self
            .records
            .values()
            .filter(|record| {
                record.id != view_root
                    && matches!(record.payload, NodePayload::Frame(_))
                    && self
                        .children
                        .get(&record.id)
                        .is_some_and(|c| c.len() > SPLIT_CHILD_THRESHOLD)
            })
            .map(|record| record.id)
            .collect();
        loop {
            let roots: Vec<StableId> = std::iter::once(view_root)
                .chain(splits.iter().copied())
                .collect();
            let Some(&over) = roots
                .iter()
                .find(|root| self.file_bytes(**root, &splits) > SPLIT_FILE_BYTES)
            else {
                return Ok(splits);
            };
            // 그 파일 안에서 하위 tree가 가장 큰 Frame을 나눕니다(같으면 작은 StableId).
            let members = self.members(over, &splits);
            let candidate = members
                .iter()
                .filter(|id| **id != over)
                .filter(|id| matches!(self.records[id].payload, NodePayload::Frame(_)))
                .map(|id| {
                    let size: usize = self
                        .members(*id, &splits)
                        .iter()
                        .map(|m| self.sizes[m])
                        .sum();
                    (std::cmp::Reverse(size), *id)
                })
                .filter(|(std::cmp::Reverse(size), _)| *size >= MIN_SPLIT_SUBTREE_BYTES)
                .min();
            let Some((_, id)) = candidate else {
                return Err(unsupported(
                    "64 KiB 이상인 하위 Frame이 없어 파일 하나가 1 MiB 상한을 넘습니다",
                ));
            };
            splits.insert(id);
        }
    }
}

fn file_text(fields_: Vec<(YamlValue, YamlValue)>) -> Result<String, TextSourceError> {
    write_canonical_checked(&YamlValue::Mapping(fields_)).map_err(invalid)
}

/// 프로젝트 전체 의미를 다중 파일 v4로 씁니다. 정확히 복원할 수 없는 의미가 있으면 거부합니다.
pub fn encode(project: &TextProject) -> Result<EncodedProject, TextSourceError> {
    let records = project.document.to_records().map_err(invalid)?;
    validate_document(&records).map_err(invalid)?;
    let definition = &records.definition;
    let mut files = BTreeMap::new();

    let by_id: BTreeMap<StableId, &NodeRecord> = records
        .nodes
        .iter()
        .map(|record| (record.id, record))
        .collect();
    let stacks: BTreeMap<StableId, &ModifierStack> = records
        .modifier_stacks
        .iter()
        .map(|stack| (stack.node, stack))
        .collect();
    let mut children: BTreeMap<StableId, Vec<StableId>> = BTreeMap::new();
    for record in &records.nodes {
        if let Some(parent) = record.parent {
            children.entry(parent).or_default().push(record.id);
        }
    }
    let mut sizes = BTreeMap::new();
    for record in &records.nodes {
        let value = node::node(record, Some(stacks.get(&record.id).copied()))?;
        let text = write_canonical(&YamlValue::Mapping(vec![(
            scalar("nodes"),
            YamlValue::Sequence(vec![value]),
        )]));
        sizes.insert(record.id, text.len());
    }
    let layout = ViewLayout {
        records: &by_id,
        children,
        stacks: &stacks,
        sizes,
    };

    let page_slugs = unique_slugs(
        definition.pages.iter().map(|p| (p.id, p.display_name())),
        "page",
    );
    let mut pages = Vec::new();
    for page in &definition.pages {
        let page_dir = join("pages", &page_slugs[&page.id]);
        let view_slugs = unique_slugs(page.views.iter().map(|v| (v.id, v.display_name())), "view");
        let mut view_refs = Vec::new();
        for view in &page.views {
            let view_path = join(&page_dir, &format!("{}{EXT}", view_slugs[&view.id]));
            let view_dir = join(&page_dir, &view_slugs[&view.id]);
            let splits = layout.splits(view.root_frame)?;
            let frame_slugs = unique_slugs(
                splits
                    .iter()
                    .map(|id| (*id, by_id[id].display_name.as_str())),
                "frame",
            );
            let path_of = |root: StableId| -> String {
                if root == view.root_frame {
                    view_path.clone()
                } else {
                    join(&view_dir, &format!("{}{EXT}", frame_slugs[&root]))
                }
            };
            for root in std::iter::once(view.root_frame).chain(splits.iter().copied()) {
                let path = path_of(root);
                let mut items = layout
                    .members(root, &splits)
                    .into_iter()
                    .map(|id| layout.node_value(id))
                    .collect::<Result<Vec<_>, _>>()?;
                let mut refs: Vec<String> = layout
                    .nested_splits(root, &splits)
                    .into_iter()
                    .map(|child| {
                        path_of(child)
                            .strip_prefix(&format!("{}/", parent_dir(&path)))
                            .expect("하위 파일은 참조 파일 아래에 있습니다")
                            .to_owned()
                    })
                    .collect();
                refs.sort();
                items.extend(
                    refs.into_iter()
                        .map(|target| YamlValue::Mapping(vec![(scalar(REF), line(target))])),
                );
                let mut content = if root == view.root_frame {
                    let mut fields_ = header("stellar-view");
                    fields_.push((
                        scalar("view"),
                        mapping([
                            ("id", line(css::id(view.id))),
                            ("display_name", line(view.display_name())),
                            ("root_frame", line(css::id(view.root_frame))),
                        ]),
                    ));
                    fields_
                } else {
                    let mut fields_ = header("stellar-frame");
                    fields_.push((scalar("frame"), line(css::id(root))));
                    fields_
                };
                content.push((scalar("nodes"), YamlValue::Sequence(items)));
                files.insert(path, file_text(content)?);
            }
            view_refs.push(line(view_path));
        }
        pages.push(mapping([
            ("id", line(css::id(page.id))),
            ("display_name", line(page.display_name())),
            ("views", YamlValue::Sequence(view_refs)),
        ]));
    }
    let mut document = header("stellar-document");
    document.push((
        scalar("document"),
        mapping([
            ("id", line(css::id(definition.id))),
            ("display_name", line(definition.display_name())),
        ]),
    ));
    document.push((scalar("pages"), YamlValue::Sequence(pages)));
    files.insert(DOCUMENT_PATH.to_owned(), file_text(document)?);

    // 컴포넌트: 정의 하나 = 파일 하나. 파일 이름은 컴포넌트 원본 규칙(이름과 StableId 접미사)입니다.
    for entry in project.components.iter() {
        let source = ComponentSource::from_entry(entry)?;
        files.insert(
            format!("components/{}", source.filename()?),
            source.to_text()?,
        );
    }
    let token_slugs = unique_slugs(
        project
            .token_sets
            .iter()
            .map(|set| (set.id, set.name.as_str())),
        "tokens",
    );
    if token_slugs.len() != project.token_sets.len() {
        return Err(invalid("중복 token set id"));
    }
    for set in &project.token_sets {
        files.insert(
            format!("tokens/{}{EXT}", token_slugs[&set.id]),
            token::encode(set)?,
        );
    }
    let mut catalog_value = AssetCatalog {
        images: project.images.clone(),
        ..AssetCatalog::default()
    };
    if !definition.text_styles.is_empty() {
        catalog_value
            .text_styles
            .insert(definition.id, definition.text_styles.clone());
    }
    files.insert(CATALOG_PATH.to_owned(), catalog::encode(&catalog_value)?);

    let encoded = EncodedProject {
        spans: spans(&files),
        files,
    };
    // 다시 읽은 의미가 같아야 씁니다(손실 저장 없음).
    if decode(&encoded.files)?.semantic_key()? != project.semantic_key()? {
        return Err(unsupported(
            "다중 파일 원본으로 정확히 복원할 수 없는 의미가 있습니다",
        ));
    }
    Ok(encoded)
}

/// 파일 집합의 node 위치 표.
pub fn spans(files: &BTreeMap<String, String>) -> SpanTable {
    files
        .iter()
        .filter(|(path, _)| path.starts_with("pages/") || path.starts_with("components/"))
        .flat_map(|(path, text)| node_spans(path, text))
        .collect()
}

// --- 읽기 ---

struct Reader<'a> {
    files: &'a BTreeMap<String, String>,
    used: BTreeSet<String>,
    records: DocumentRecords,
}

impl Reader<'_> {
    fn load(&mut self, path: &str) -> Result<YamlValue, TextSourceError> {
        let source = self
            .files
            .get(path)
            .ok_or_else(|| invalid(format!("참조한 파일이 없습니다: {path}")))?;
        if !self.used.insert(path.to_owned()) {
            return Err(invalid(format!("같은 파일을 두 번 참조합니다: {path}")));
        }
        load_restricted(source).map_err(|e| invalid(format!("{path}: {e}")))
    }

    /// View 또는 Frame 파일의 `nodes`를 읽습니다. `root`의 부모는 참조한 파일에 있어야 합니다.
    fn nodes(
        &mut self,
        path: &str,
        value: &YamlValue,
        root: StableId,
        outer: &BTreeSet<StableId>,
    ) -> Result<(), TextSourceError> {
        let mut ids = BTreeSet::new();
        let mut refs = Vec::new();
        let mut parents = Vec::new();
        let mut last = None;
        for item in sequence(value, "nodes")? {
            if item.get(REF).is_some() {
                fields(item, &[REF])?;
                refs.push(resolve_ref(path, get(item, REF)?)?);
                continue;
            }
            if !refs.is_empty() {
                return Err(invalid("$ref 항목은 node 뒤에 씁니다"));
            }
            let (record, stack) = node::parse_node(item, true)?;
            if last.is_some_and(|prev| prev >= record.id) {
                return Err(invalid("node는 StableId 오름차순, 중복 없이 씁니다"));
            }
            last = Some(record.id);
            ids.insert(record.id);
            parents.push((record.id, record.parent));
            self.records.nodes.push(record);
            self.records.modifier_stacks.extend(stack);
        }
        if !ids.contains(&root) {
            return Err(invalid(format!("{path}: 파일 root node가 없습니다")));
        }
        for (id, parent) in parents {
            let ok = if id == root {
                parent.is_none_or(|p| outer.contains(&p))
            } else {
                parent.is_some_and(|p| ids.contains(&p))
            };
            if !ok {
                return Err(invalid(format!(
                    "{path}: node {id}의 부모가 이 파일(하위 파일 root는 참조한 파일)에 없습니다"
                )));
            }
        }
        if refs.windows(2).any(|w| w[0] >= w[1]) {
            return Err(invalid("$ref는 경로 오름차순, 중복 없이 씁니다"));
        }
        for target in refs {
            let value = self.load(&target)?;
            fields(&value, &["format", "schema_version", "frame", "nodes"])?;
            check_header(&value, "stellar-frame")?;
            let frame = css::parse_id(get(&value, "frame")?)?;
            self.nodes(&target, &value, frame, &ids)?;
        }
        Ok(())
    }
}

/// 다중 파일 v4를 읽어 프로젝트 의미를 재구축합니다.
pub fn decode(files: &BTreeMap<String, String>) -> Result<TextProject, TextSourceError> {
    for path in files.keys() {
        let known = path == DOCUMENT_PATH
            || path == CATALOG_PATH
            || (path.starts_with("pages/") && path.ends_with(EXT))
            || (path.starts_with("components/")
                && path.ends_with(EXT)
                && path.matches('/').count() == 1)
            || (path.starts_with("tokens/")
                && path.ends_with(EXT)
                && path.matches('/').count() == 1);
        if !known {
            return Err(invalid(format!(
                "텍스트 원본에 모르는 파일이 있습니다: {path}"
            )));
        }
    }
    let mut reader = Reader {
        files,
        used: BTreeSet::new(),
        records: DocumentRecords::new(DocumentDefinition::new(StableId::NIL, String::new())),
    };
    let document = reader.load(DOCUMENT_PATH)?;
    fields(
        &document,
        &["format", "schema_version", "document", "pages"],
    )?;
    check_header(&document, "stellar-document")?;
    let doc = field(&document, "document")?;
    fields(doc, &["id", "display_name"])?;
    let mut definition = DocumentDefinition::new(
        css::parse_id(get(doc, "id")?)?,
        get(doc, "display_name")?.to_owned(),
    );
    for page in sequence(&document, "pages")? {
        fields(page, &["id", "display_name", "views"])?;
        let mut page_definition = PageDefinition::new(
            css::parse_id(get(page, "id")?)?,
            get(page, "display_name")?.to_owned(),
        );
        for view_ref in sequence(page, "views")? {
            let path = text(view_ref)?.to_owned();
            if !path.starts_with("pages/") {
                return Err(invalid(format!(
                    "View 파일은 pages/ 아래에 있어야 합니다: {path}"
                )));
            }
            resolve_ref(DOCUMENT_PATH, &path)?;
            let value = reader.load(&path)?;
            fields(&value, &["format", "schema_version", "view", "nodes"])?;
            check_header(&value, "stellar-view")?;
            let view = field(&value, "view")?;
            fields(view, &["id", "display_name", "root_frame"])?;
            let root = css::parse_id(get(view, "root_frame")?)?;
            page_definition
                .add_view(ViewDefinition::new(
                    css::parse_id(get(view, "id")?)?,
                    get(view, "display_name")?.to_owned(),
                    root,
                ))
                .map_err(invalid)?;
            reader.nodes(&path, &value, root, &BTreeSet::new())?;
        }
        definition.add_page(page_definition).map_err(invalid)?;
    }
    let catalog = catalog::decode(
        files
            .get(CATALOG_PATH)
            .ok_or_else(|| invalid("asset catalog가 없습니다"))?,
    )?;
    reader.used.insert(CATALOG_PATH.to_owned());
    for document_id in catalog.text_styles.keys() {
        if *document_id != definition.id {
            return Err(invalid("catalog의 Text style 표가 다른 문서를 가리킵니다"));
        }
    }
    if let Some(styles) = catalog.text_styles.get(&definition.id) {
        if styles.is_empty() {
            return Err(invalid("빈 Text style 표는 catalog에 쓰지 않습니다"));
        }
        definition.text_styles = styles.clone();
    }
    let mut components = ComponentLibrary::new();
    let mut token_sets = Vec::new();
    for (path, source) in files {
        if path.starts_with("components/") {
            ComponentSource::from_text(source)?.add_to(&mut components)?;
            reader.used.insert(path.clone());
        } else if path.starts_with("tokens/") {
            token_sets.push(token::decode(source)?);
            reader.used.insert(path.clone());
        }
    }
    if let Some(unused) = files.keys().find(|path| !reader.used.contains(*path)) {
        return Err(invalid(format!("어디서도 참조하지 않은 파일: {unused}")));
    }
    let mut ids = BTreeSet::new();
    if token_sets.iter().any(|set| !ids.insert(set.id)) {
        return Err(invalid("중복 token set id"));
    }
    token_sets.sort_by_key(|set| set.id);
    let mut records = reader.records;
    records.definition = definition;
    let (document, _) = DocumentModel::from_records(records).map_err(invalid)?;
    Ok(TextProject {
        document,
        components,
        token_sets,
        images: catalog.images,
    })
}

// --- 디스크 ---

fn collect(
    root: &Path,
    dir: &Path,
    prefix: &str,
    out: &mut BTreeMap<String, String>,
    budget: &mut u64,
) -> Result<(), TextSourceError> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid("UTF-8이 아닌 파일 이름"))?;
        let path = entry.path();
        let kind = fs::symlink_metadata(&path)?.file_type();
        let relative = join(prefix, &name);
        if kind.is_symlink() {
            return Err(invalid(format!(
                "텍스트 원본 안의 symlink는 따르지 않습니다: {relative}"
            )));
        }
        if kind.is_dir() {
            collect(root, &path, &relative, out, budget)?;
        } else if name.ends_with(EXT) {
            if out.len() == MAX_PROJECT_FILES {
                return Err(invalid("텍스트 원본 파일 수 상한 초과"));
            }
            let bytes = source_file::read(&path, (*budget).min(MAX_INPUT_BYTES as u64))?;
            *budget -= bytes.len() as u64;
            out.insert(
                relative,
                String::from_utf8(bytes).map_err(|_| invalid("UTF-8이 아닌 텍스트 원본"))?,
            );
        }
    }
    let _ = root;
    Ok(())
}

/// `design` 폴더의 `.stellar.yaml` 파일을 모두 읽습니다. symlink는 거부합니다.
pub fn read_dir(design: &Path) -> Result<BTreeMap<String, String>, TextSourceError> {
    if fs::symlink_metadata(design)?.file_type().is_symlink() {
        return Err(invalid("design 폴더가 symlink입니다"));
    }
    let mut files = BTreeMap::new();
    let mut budget = MAX_PROJECT_BYTES;
    collect(design, design, "", &mut files, &mut budget)?;
    Ok(files)
}

/// 다중 파일 원본을 읽고 검증합니다.
pub fn open_dir(design: &Path) -> Result<(TextProject, SpanTable), TextSourceError> {
    let files = read_dir(design)?;
    let project = decode(&files)?;
    Ok((project, spans(&files)))
}

/// 아직 없는 `design` 폴더에 새 원본을 씁니다. 기존 파일은 덮어쓰지 않습니다.
///
/// 파일마다 temp/fsync/hard link로 게시합니다. 여러 파일의 원자성(저장 journal)은 V.10.5 범위라, 도중에 실패하면
/// 일부 파일만 있는 새 폴더가 남을 수 있습니다. 이 폴더는 원본이 아니며 다시 export하기 전 지웁니다.
pub fn write_new(design: &Path, encoded: &EncodedProject) -> Result<(), TextSourceError> {
    if fs::symlink_metadata(design).is_ok() {
        return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists).into());
    }
    fs::create_dir_all(design)?;
    for (relative, text) in &encoded.files {
        let relative_path = Path::new(relative);
        if relative_path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(invalid(format!("잘못된 원본 경로: {relative}")));
        }
        let path = design.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        source_file::write_checked(&path, text.as_bytes(), None, MAX_INPUT_BYTES as u64)?;
    }
    Ok(())
}

// --- 저장된 세대에서 옮기기(V.10.7 준비) ---

/// 저장된 generation 하나를 텍스트 원본 의미로 옮깁니다.
///
/// generation의 chunk pointer 전체를 소비해야 합니다: 문서 하나의 DocumentManifest·ViewTree·subtree(다시 직렬화한
/// 집합과 ChunkId·hash·kind·schema가 정확히 같아야 함), component library chunk 0~1개, asset metadata chunk.
/// 알 수 없거나 남는 chunk가 하나라도 있으면 거부합니다. `yaml_components`는 프로젝트 `design/components/`의
/// 기존 YAML 원본에서 조립한 정의입니다(generation에 library chunk가 있으면 비어 있어야 합니다).
pub fn from_saved(
    saved: &stellar_storage::generation::lazy::LazyGeneration,
    document: DocumentModel,
    yaml_components: ComponentLibrary,
) -> Result<TextProject, TextSourceError> {
    use stellar_storage::chunk::kinds::{KIND_ASSET_METADATA, KIND_COMPONENT_LIBRARY};
    use stellar_storage::chunk::pointer::ChunkPointer;
    use stellar_storage::generation::lazy::NoopRecorder;
    use stellar_storage::hash::ContentHash;

    let manifest = saved.manifest();
    if manifest.root_document_ids.as_slice() != [document.document_id()] {
        return Err(unsupported("텍스트 원본은 root 문서 하나만 담습니다"));
    }
    let serialized = super::super::serialize::serialize_document(&document).map_err(invalid)?;
    let mut expected: Vec<ChunkPointer> = serialized
        .chunks()
        .into_iter()
        .map(|chunk| ChunkPointer {
            chunk_id: chunk.chunk_id,
            content_hash: ContentHash::compute(&chunk.payload),
            kind: chunk.kind,
            schema_version: chunk.schema_version,
        })
        .collect();
    let mut components = yaml_components;
    let mut images = BTreeMap::new();
    let read = |pointer: &ChunkPointer| {
        saved
            .read_chunk_payload(pointer.chunk_id, &mut NoopRecorder)
            .map_err(invalid)
    };
    for pointer in &manifest.chunk_pointers {
        if pointer.kind == KIND_COMPONENT_LIBRARY && pointer.schema_version == 1 {
            if !components.is_empty() {
                return Err(unsupported(
                    "generation component library와 YAML 컴포넌트 원본이 함께 있습니다",
                ));
            }
            components = ComponentLibrary::decode(&read(pointer)?).map_err(invalid)?;
            expected.push(*pointer);
        } else if pointer.kind == KIND_ASSET_METADATA && pointer.schema_version == 1 {
            let record = AssetRecord::decode(&read(pointer)?).map_err(invalid)?;
            images.insert(pointer.chunk_id.stable_id(), record);
            expected.push(*pointer);
        }
    }
    expected.sort();
    let mut stored = manifest.chunk_pointers.clone();
    stored.sort();
    if stored != expected {
        return Err(unsupported(
            "텍스트 원본이 소비하지 않는 chunk 또는 다시 직렬화로 같아지지 않는 chunk가 있습니다",
        ));
    }
    Ok(TextProject {
        document,
        components,
        token_sets: Vec::new(),
        images,
    })
}
