//! 실사용 QA용 샘플 프로젝트 생성기(`scripts/macos/qa.sh`가 부릅니다).
//!
//! ```text
//! cargo run -q -p stellar-project --example qa_sample_project -- <부모 폴더> <폴더 이름> [--image <이미지 파일>]
//! ```
//!
//! 코어 생성 API만 씁니다. `stellar` CLI에 프로젝트 생성 명령이 아직 없어서 example로 둡니다.
//!
//! 1. [`create_project`]: 폴더·기본 scaffold·`git init`·최초 커밋(Git identity가 없으면 커밋 없이 계속).
//! 2. `--image`가 있으면 [`AssetStore`]로 `assets/<prefix>/<hash>.<ext>`에 복사합니다(`config/asset-import.yaml` 정책).
//! 3. 문서 하나(페이지 2개, View 3개, Frame·Fragment·사각형·타원·선·다각형·path·텍스트·이미지, modifier)를
//!    [`DocumentRecords`] → [`DocumentModel::from_records`](전체 validator) → 저장 파이프라인(V.5)으로 씁니다.
//! 4. [`SavedProjectLoader`]로 다시 열어 같은 문서가 읽히는지 확인합니다.
//! 5. 최초 커밋이 있었으면 문서와 에셋을 두 번째 커밋으로 남겨 작업 트리를 깨끗하게 둡니다.
//!
//! 1~5는 같은 부모 아래 임시 이름(`.<폴더 이름>.creating-<pid>`)으로 만들고, 모두 성공한 뒤에만
//! `<폴더 이름>`으로 바꿉니다. 중간에 실패하면 임시 폴더를 지웁니다. 그래서 `<폴더 이름>` 폴더는
//! 끝까지 만든 샘플일 때만 생깁니다. 프로세스가 강제로 끝나면 임시 폴더가 남을 수 있지만
//! `<폴더 이름>`으로는 보이지 않습니다.
//!
//! 한계(QA 문서에 같은 내용이 있습니다):
//! - Component 정의·인스턴스는 문서 레코드에 아직 표현할 수 없어(`record_codec`의 `UnsupportedNodeKind`) 넣지
//!   않습니다. "Components" 페이지에는 컴포넌트로 만들 Frame 묶음만 둡니다.
//! - 에셋 catalog(`AssetId → AssetRecord`)는 아직 프로젝트에 저장되지 않으므로, 이미지 노드의 asset 참조는
//!   복사한 파일과 이어져 있지 않습니다.
//!
//! id는 폴더 이름에서 유도한 결정론적 UUIDv7입니다. 같은 이름으로 다시 만들면 같은 id가 나옵니다.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use stellar_assets::media_type::MediaType;
use stellar_assets::policy::AssetImportPolicy;
use stellar_assets::store::AssetStore;
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::text::UserText;
use stellar_core_types::value::{Angle, FiniteF32, Rect, Size2, Transform2D, Vec2};
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::{Border, Clipping, CornerRadius, FramePayload};
use stellar_document::image::{FitMode, ImagePayload};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierPayload, ModifierStack};
use stellar_document::path::{IdentifiedPath, PathCommand, PathData, WindingRule};
use stellar_document::records::{DocumentRecords, NodePayload, NodeRecord};
use stellar_document::shape::{
    EllipseParams, LineCap, LineParams, PolygonParams, RectangleParams, ShapeGeometry,
};
use stellar_document::text::{Paragraph, TextPayload};
use stellar_git_porcelain::discovery::{discover_git, GitDiscoveryConfig};
use stellar_git_porcelain::repo;
use stellar_git_porcelain::runner::GitRunner;
use stellar_git_porcelain::version::probe_git_version;
use stellar_project::create::{create_project, ProjectCreateRequest, ProjectCreated};
use stellar_project::document_io::serialize::serialize_document;
use stellar_project::folder_name::validate_folder_name;
use stellar_project::saved_project::SavedProjectLoader;
use stellar_storage::generation::loader::load_current_generation;
use stellar_storage::save::cancel::{run_save, CancelToken};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::build_save_plan;

/// 사람이 읽는 프로젝트 이름.
const PROJECT_NAME: &str = "Stellar QA Sample";
/// 첫 저장 schema version.
const SCHEMA_VERSION: u32 = 1;
/// 두 번째 커밋 메시지(샘플 프로젝트 저장소 안에서만 쓰임).
const SAMPLE_COMMIT_MESSAGE: &str = "QA 샘플 문서와 에셋 추가";
/// 에셋 import 정책 파일(저장소 기준).
const ASSET_POLICY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../config/asset-import.yaml"
);
/// id timestamp 기준값(ms).
const ID_BASE_MS: u64 = 1_760_000_000_000;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{}", usage());
        return ExitCode::SUCCESS;
    }
    let options = match Options::parse(&args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("qa_sample_project: {message}\n\n{}", usage());
            return ExitCode::from(64);
        }
    };
    match run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("qa_sample_project: {message}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> &'static str {
    "사용법: qa_sample_project <부모 폴더> <폴더 이름> [--image <이미지 파일>]"
}

/// 명령 인자.
struct Options {
    parent: PathBuf,
    folder: String,
    image: Option<PathBuf>,
}

impl Options {
    fn parse(args: &[String]) -> Result<Options, String> {
        let mut positional = Vec::new();
        let mut image = None;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--image" => {
                    let value = iter.next().ok_or("--image에 파일 경로가 필요합니다")?;
                    if image.replace(PathBuf::from(value)).is_some() {
                        return Err("--image를 두 번 지정했습니다".to_string());
                    }
                }
                other if other.starts_with('-') => {
                    return Err(format!("알 수 없는 옵션: {other}"));
                }
                other => positional.push(other.to_string()),
            }
        }
        let [parent, folder] = <[String; 2]>::try_from(positional)
            .map_err(|_| "<부모 폴더>와 <폴더 이름>이 필요합니다".to_string())?;
        Ok(Options {
            parent: PathBuf::from(parent),
            folder,
            image,
        })
    }
}

fn run(options: &Options) -> Result<(), String> {
    validate_folder_name(&options.folder)
        .map_err(|e| format!("폴더 이름 오류({}): {e}", options.folder))?;
    let target = options.parent.join(&options.folder);
    if target.exists() {
        return Err(format!("이미 있습니다: {}", target.display()));
    }
    let mut ids = Ids::for_folder(&options.folder);
    let project_id = ids.next();

    // 1. 프로젝트 생성(scaffold + git init + 최초 커밋)을 임시 이름으로 합니다.
    let git = discover_git(&GitDiscoveryConfig::default())
        .ok_or("시스템 Git을 찾지 못했습니다(xcode-select --install)")?;
    let version = probe_git_version(&git).map_err(|e| format!("Git 버전 확인 실패: {e:?}"))?;
    // 사용자 Git 설정(identity)을 읽도록 HOME을 넘깁니다.
    let runner = GitRunner::new(
        git,
        vec!["PATH".into(), "HOME".into()],
        Duration::from_secs(30),
    );
    let request = ProjectCreateRequest {
        parent_dir: options.parent.clone(),
        folder_name: format!(".{}.creating-{}", options.folder, std::process::id()),
        project_name: PROJECT_NAME.to_string(),
    };
    // 실패하면 create_project가 만든 폴더를 스스로 되돌리므로(git init 전) 여기서 지울 것이 없습니다.
    let created = create_project(&request, &project_id.to_string(), &runner, &version)
        .map_err(|e| format!("프로젝트 생성 실패: {e}"))?;
    let (staging, committed) = match created {
        ProjectCreated::Created { path } => (path, true),
        ProjectCreated::InitializationIncomplete { path, reason } => {
            eprintln!(
                "qa_sample_project: 최초 커밋 없이 계속합니다({reason:?}). Git user.name·user.email을 설정하면 커밋까지 만듭니다."
            );
            (path, false)
        }
    };

    // 2~5. 여기부터 임시 폴더는 이 실행이 만든 것이므로, 실패하면 지웁니다.
    let outcome = fill_sample(&staging, &mut ids, project_id, options, &runner, committed)
        .and_then(|summary| {
            std::fs::rename(&staging, &target)
                .map_err(|e| format!("최종 이름으로 바꾸기 실패({}): {e}", target.display()))?;
            Ok(summary)
        });
    let summary = match outcome {
        Ok(summary) => summary,
        Err(message) => {
            if let Err(e) = std::fs::remove_dir_all(&staging) {
                eprintln!(
                    "qa_sample_project: 임시 폴더를 지우지 못했습니다({}): {e}",
                    staging.display()
                );
            }
            return Err(message);
        }
    };

    println!("샘플 프로젝트: {}", target.display());
    println!("{summary}");
    if !committed {
        println!("Git 커밋: 없음(identity 미설정)");
    }
    Ok(())
}

/// 만든 프로젝트 폴더(`root`)에 에셋·문서를 넣고 확인·커밋한 뒤 요약을 돌려줍니다.
fn fill_sample(
    root: &Path,
    ids: &mut Ids,
    project_id: StableId,
    options: &Options,
    runner: &GitRunner,
    committed: bool,
) -> Result<String, String> {
    // 2. 이미지 에셋 복사(선택).
    let asset = AssetRef::new(ids.next());
    if let Some(image) = &options.image {
        import_image(root, image)?;
    }

    // 3. 문서 저장.
    let records = sample_document(ids, asset)?;
    let (model, _diagnostics) =
        DocumentModel::from_records(records).map_err(|e| format!("샘플 문서 검증 실패: {e:?}"))?;
    save_document(root, project_id, &model)?;

    // 4. 다시 열어 확인.
    let summary = verify_saved(root, &model)?;

    // 5. 문서·에셋 커밋.
    if committed {
        repo::git_add_all(runner, root).map_err(|e| format!("git add 실패: {e:?}"))?;
        repo::git_commit(runner, root, SAMPLE_COMMIT_MESSAGE)
            .map_err(|e| format!("git commit 실패: {e:?}"))?;
    }
    Ok(summary)
}

/// `config/asset-import.yaml` 정책으로 이미지를 `assets/`에 복사합니다.
fn import_image(root: &Path, image: &Path) -> Result<(), String> {
    let policy = AssetImportPolicy::from_config_file(Path::new(ASSET_POLICY))
        .map_err(|e| format!("에셋 정책 읽기 실패: {e:?}"))?;
    let media = match image
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        _ => return Err(format!("PNG·JPEG만 받습니다: {}", image.display())),
    };
    let media = MediaType::new(media).map_err(|e| format!("media type 오류: {e:?}"))?;
    let request = policy
        .request(image, &media)
        .map_err(|e| format!("에셋 정책 거부: {e:?}"))?
        .without_source_metadata();
    let imported = AssetStore::new(root)
        .import_file(&request)
        .map_err(|e| format!("에셋 복사 실패: {e:?}"))?;
    println!(
        "이미지 에셋: {}",
        imported.object_path.to_relative_path().display()
    );
    Ok(())
}

/// 저장 파이프라인(V.5)으로 current generation을 씁니다.
fn save_document(root: &Path, project_id: StableId, model: &DocumentModel) -> Result<(), String> {
    let base = load_current_generation(root, project_id).ok();
    let serialized = serialize_document(model).map_err(|e| format!("직렬화 실패: {e:?}"))?;
    let plan = build_save_plan(
        base.as_ref(),
        project_id,
        SCHEMA_VERSION,
        &serialized.to_dirty_states(),
        vec![serialized.document_id],
    )
    .map_err(|e| format!("저장 계획 실패: {e:?}"))?;
    let mut memory = SavedGenerationState::new(
        base.as_ref()
            .map(|b| (b.manifest.generation, b.manifest_hash)),
    );
    run_save(root, &plan, &mut memory, &CancelToken::new()).map_err(|e| format!("저장 실패: {e:?}"))
}

/// 저장한 프로젝트를 다시 열어 문서 정의와 node 수가 같은지 확인하고 요약을 돌려줍니다.
fn verify_saved(root: &Path, model: &DocumentModel) -> Result<String, String> {
    let project = SavedProjectLoader::new()
        .open(root)
        .map_err(|e| format!("저장한 프로젝트 열기 실패: {e:?}"))?;
    let loaded = project
        .load_document(model.document_id())
        .map_err(|e| format!("저장한 문서 읽기 실패: {e:?}"))?;
    let expected = model
        .to_records()
        .map_err(|e| format!("원본 레코드 실패: {e:?}"))?;
    let actual = loaded
        .model
        .to_records()
        .map_err(|e| format!("읽은 레코드 실패: {e:?}"))?;
    if expected != actual {
        return Err("다시 읽은 문서가 저장한 문서와 다릅니다".to_string());
    }
    let pages = &actual.definition.pages;
    let views: usize = pages.iter().map(|p| p.views.len()).sum();
    let page_names: Vec<&str> = pages.iter().map(|p| p.display_name()).collect();
    Ok(format!(
        "문서 \"{}\": 페이지 {}개({}), View {}개, node {}개, modifier stack {}개, generation {}",
        actual.definition.display_name(),
        pages.len(),
        page_names.join(", "),
        views,
        actual.nodes.len(),
        actual.modifier_stacks.len(),
        project.generation_number()
    ))
}

// --- 샘플 문서 ---

/// 결정론적 UUIDv7 발급기.
struct Ids {
    base_ms: u64,
    seed: u64,
    issued: u64,
}

impl Ids {
    /// 폴더 이름(FNV-1a)에서 기준값을 유도합니다.
    fn for_folder(folder: &str) -> Ids {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in folder.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        Ids {
            base_ms: ID_BASE_MS + (hash % (1 << 20)) * (1 << 12),
            seed: hash,
            issued: 0,
        }
    }

    fn next(&mut self) -> StableId {
        self.issued += 1;
        let mix = self.seed ^ self.issued.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        StableId::new_v7(self.base_ms + self.issued, (mix & 0x0FFF) as u16, mix >> 2)
    }
}

/// sRGB 색(0~255).
fn rgb(r: u8, g: u8, b: u8) -> Result<Color, String> {
    let c = |v: u8| f32::from(v) / 255.0;
    Color::new(ColorSpace::Srgb, c(r), c(g), c(b), 1.0).map_err(|e| format!("색 오류: {e:?}"))
}

fn size(w: f32, h: f32) -> Result<Size2, String> {
    Size2::new(w, h).map_err(|e| format!("크기 오류: {e:?}"))
}

fn point(x: f32, y: f32) -> Result<Vec2, String> {
    Vec2::new(x, y).map_err(|e| format!("좌표 오류: {e:?}"))
}

fn translation(x: f32, y: f32) -> Result<Transform2D, String> {
    Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y).map_err(|e| format!("변환 오류: {e:?}"))
}

fn radius(r: f32) -> Result<CornerRadius, String> {
    CornerRadius::new(r, r, r, r).map_err(|e| format!("radius 오류: {e:?}"))
}

fn finite(v: f32) -> Result<FiniteF32, String> {
    FiniteF32::new(v).map_err(|e| format!("수치 오류: {e:?}"))
}

/// 여러 줄 텍스트(줄마다 paragraph 하나, style run 없음).
fn text(content: &str) -> Result<NodePayload, String> {
    let paragraphs = (0..=content.matches('\n').count())
        .map(|_| Paragraph { style: 0 })
        .collect();
    TextPayload::new(UserText::new(content), paragraphs, Vec::new())
        .map(NodePayload::Text)
        .map_err(|e| format!("텍스트 오류: {e:?}"))
}

/// View 하나의 node 묶음을 쌓는 도우미. 형제 순서는 추가 순서입니다.
struct Builder<'a> {
    ids: &'a mut Ids,
    nodes: Vec<NodeRecord>,
    stacks: Vec<ModifierStack>,
    last_child: std::collections::BTreeMap<Option<StableId>, OrderKey>,
}

impl<'a> Builder<'a> {
    fn new(ids: &'a mut Ids) -> Builder<'a> {
        Builder {
            ids,
            nodes: Vec::new(),
            stacks: Vec::new(),
            last_child: std::collections::BTreeMap::new(),
        }
    }

    /// node를 추가하고 id를 돌려줍니다.
    fn add(&mut self, parent: Option<StableId>, name: &str, payload: NodePayload) -> StableId {
        let id = self.ids.next();
        let order_key = OrderKey::between(self.last_child.get(&parent), None);
        self.last_child.insert(parent, order_key.clone());
        self.nodes.push(NodeRecord {
            id,
            parent,
            order_key,
            visible: true,
            locked: false,
            display_name: name.to_string(),
            payload,
        });
        id
    }

    /// node에 modifier stack을 붙입니다(순서는 목록 순서).
    fn modifiers(&mut self, node: StableId, payloads: Vec<ModifierPayload>) -> Result<(), String> {
        let mut stack = ModifierStack::new(node);
        let keys = OrderKey::evenly_spaced(payloads.len());
        for (payload, order) in payloads.into_iter().zip(keys) {
            stack
                .insert(ModifierEntry {
                    id: ModifierId::new(self.ids.next()),
                    order,
                    kind: payload.kind(),
                    enabled: true,
                    payload,
                })
                .map_err(|e| format!("modifier 오류: {e:?}"))?;
        }
        self.stacks.push(stack);
        Ok(())
    }

    /// 위치(Transform)와 채우기(Fill)를 가진 도형·텍스트·이미지를 추가합니다.
    fn placed(
        &mut self,
        parent: StableId,
        name: &str,
        payload: NodePayload,
        at: (f32, f32),
        fill: Option<Color>,
    ) -> Result<StableId, String> {
        let id = self.add(Some(parent), name, payload);
        let mut payloads = vec![ModifierPayload::Transform(translation(at.0, at.1)?)];
        if let Some(color) = fill {
            payloads.push(ModifierPayload::Fill { color });
        }
        self.modifiers(id, payloads)?;
        Ok(id)
    }

    fn set_flags(&mut self, id: StableId, visible: bool, locked: bool) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.id == id) {
            node.visible = visible;
            node.locked = locked;
        }
    }
}

/// Frame payload.
fn frame(
    w: f32,
    h: f32,
    at: (f32, f32),
    background: Option<Color>,
    corner: f32,
) -> Result<NodePayload, String> {
    Ok(NodePayload::Frame(FramePayload {
        size: size(w, h)?,
        transform: translation(at.0, at.1)?,
        clipping: Clipping::FrameBounds,
        background,
        border: None,
        corner_radius: radius(corner)?,
        layout_mode: None,
    }))
}

fn rectangle(w: f32, h: f32, corner: f32) -> Result<NodePayload, String> {
    Ok(NodePayload::Shape(ShapeGeometry::Rectangle(
        RectangleParams {
            size: size(w, h)?,
            corner_radius: radius(corner)?,
        },
    )))
}

fn ellipse(w: f32, h: f32) -> Result<NodePayload, String> {
    Ok(NodePayload::Shape(ShapeGeometry::Ellipse(EllipseParams {
        bounds: Rect::new(Vec2::ZERO, size(w, h)?),
    })))
}

/// 샘플 문서 레코드: 페이지 2개(Home: Desktop·Mobile, Components: Buttons).
fn sample_document(ids: &mut Ids, asset: AssetRef) -> Result<DocumentRecords, String> {
    let ink = rgb(0x1d, 0x1d, 0x1f)?;
    let paper = rgb(0xff, 0xff, 0xff)?;
    let mist = rgb(0xf2, 0xf3, 0xf5)?;
    let accent = rgb(0x2f, 0x6f, 0xeb)?;
    let warm = rgb(0xf5, 0x9e, 0x0b)?;
    let green = rgb(0x16, 0xa3, 0x4a)?;

    let mut definition = DocumentDefinition::new(ids.next(), "QA Sample".to_string());
    let mut home = PageDefinition::new(ids.next(), "Home".to_string());
    let mut components = PageDefinition::new(ids.next(), "Components".to_string());

    let mut b = Builder::new(ids);

    // Home ▸ Desktop.
    let desktop = b.add(
        None,
        "Desktop",
        frame(1440.0, 1024.0, (0.0, 0.0), Some(paper), 0.0)?,
    );
    let header = b.add(
        Some(desktop),
        "Header",
        frame(1440.0, 80.0, (0.0, 0.0), Some(mist), 0.0)?,
    );
    b.placed(
        header,
        "Logo",
        ellipse(40.0, 40.0)?,
        (32.0, 20.0),
        Some(accent),
    )?;
    b.placed(
        header,
        "Title",
        text("Stellar QA Sample")?,
        (88.0, 28.0),
        Some(ink),
    )?;
    let hero = b.add(
        Some(desktop),
        "Hero",
        frame(1200.0, 480.0, (120.0, 120.0), Some(mist), 24.0)?,
    );
    let mut hero_image = ImagePayload::new(asset, size(560.0, 400.0)?);
    hero_image.fit = FitMode::Cover;
    b.placed(
        hero,
        "Hero Image",
        NodePayload::Image(hero_image),
        (600.0, 40.0),
        None,
    )?;
    b.placed(
        hero,
        "Headline",
        text("폴더와 Git이 원본인 디자인 도구\nFolders and Git are the source of truth")?,
        (48.0, 160.0),
        Some(ink),
    )?;
    b.placed(
        hero,
        "Get Started",
        rectangle(180.0, 48.0, 12.0)?,
        (48.0, 300.0),
        Some(accent),
    )?;
    let cards = b.add(
        Some(desktop),
        "Cards",
        frame(1200.0, 280.0, (120.0, 660.0), None, 0.0)?,
    );
    for (index, (name, tint)) in [("Card 1", warm), ("Card 2", green), ("Card 3", accent)]
        .into_iter()
        .enumerate()
    {
        let x = index as f32 * 410.0;
        let card = b.add(
            Some(cards),
            name,
            frame(380.0, 280.0, (x, 0.0), Some(mist), 16.0)?,
        );
        b.modifiers(
            card,
            vec![ModifierPayload::Shadow {
                color: Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 0.25)
                    .map_err(|e| format!("색 오류: {e:?}"))?,
                offset: point(0.0, 4.0)?,
                blur: finite(12.0)?,
            }],
        )?;
        b.placed(
            card,
            "Thumbnail",
            rectangle(348.0, 160.0, 8.0)?,
            (16.0, 16.0),
            Some(tint),
        )?;
        b.placed(
            card,
            "Caption",
            text(&format!("{name}\nSecond line"))?,
            (16.0, 196.0),
            Some(ink),
        )?;
    }
    let hidden = b.placed(
        desktop,
        "Accent (Hidden)",
        ellipse(200.0, 200.0)?,
        (1200.0, 40.0),
        Some(warm),
    )?;
    b.set_flags(hidden, false, false);
    let locked = b.placed(
        desktop,
        "Footer (Locked)",
        rectangle(1440.0, 64.0, 0.0)?,
        (0.0, 960.0),
        Some(mist),
    )?;
    b.set_flags(locked, true, true);
    home.add_view(ViewDefinition::new(
        b.ids.next(),
        "Desktop".to_string(),
        desktop,
    ))
    .map_err(|e| format!("View 오류: {e:?}"))?;

    // Home ▸ Mobile.
    let mobile = b.add(
        None,
        "Mobile",
        frame(390.0, 844.0, (0.0, 0.0), Some(paper), 0.0)?,
    );
    let bar = b.add(
        Some(mobile),
        "Header",
        frame(390.0, 64.0, (0.0, 0.0), Some(mist), 0.0)?,
    );
    b.placed(bar, "Title", text("Stellar")?, (20.0, 22.0), Some(ink))?;
    let mut mobile_image = ImagePayload::new(asset, size(350.0, 220.0)?);
    mobile_image.fit = FitMode::Cover;
    b.placed(
        mobile,
        "Hero Image",
        NodePayload::Image(mobile_image),
        (20.0, 84.0),
        None,
    )?;
    b.placed(
        mobile,
        "Headline",
        text("안녕하세요, Stellar")?,
        (20.0, 324.0),
        Some(ink),
    )?;
    b.placed(
        mobile,
        "Button",
        rectangle(350.0, 48.0, 12.0)?,
        (20.0, 760.0),
        Some(accent),
    )?;
    home.add_view(ViewDefinition::new(
        b.ids.next(),
        "Mobile".to_string(),
        mobile,
    ))
    .map_err(|e| format!("View 오류: {e:?}"))?;

    // Components ▸ Buttons: 컴포넌트로 만들 Frame 묶음(Component 레코드는 아직 저장 포맷에 없음).
    let buttons = b.add(
        None,
        "Buttons",
        frame(800.0, 600.0, (0.0, 0.0), Some(paper), 0.0)?,
    );
    for (index, (name, fill)) in [("Button / Primary", accent), ("Button / Secondary", mist)]
        .into_iter()
        .enumerate()
    {
        let y = 40.0 + index as f32 * 88.0;
        let mut payload = frame(200.0, 48.0, (40.0, y), Some(fill), 12.0)?;
        if let NodePayload::Frame(frame) = &mut payload {
            frame.border =
                Some(Border::new(accent, 1.0).map_err(|e| format!("테두리 오류: {e:?}"))?);
        }
        let button = b.add(Some(buttons), name, payload);
        b.placed(button, "Label", text("Button")?, (60.0, 14.0), Some(ink))?;
    }
    let icons = b.add(
        Some(buttons),
        "Icon Group",
        NodePayload::Fragment(FragmentPayload),
    );
    b.placed(
        icons,
        "Dot",
        ellipse(24.0, 24.0)?,
        (320.0, 52.0),
        Some(green),
    )?;
    b.placed(
        icons,
        "Square",
        rectangle(24.0, 24.0, 4.0)?,
        (360.0, 52.0),
        Some(warm),
    )?;
    b.placed(
        buttons,
        "Divider",
        NodePayload::Shape(ShapeGeometry::Line(LineParams {
            start: point(0.0, 0.0)?,
            end: point(720.0, 0.0)?,
            cap: LineCap::Round,
        })),
        (40.0, 240.0),
        None,
    )?;
    let polygon = PolygonParams::new(
        Rect::new(Vec2::ZERO, size(96.0, 96.0)?),
        6,
        Angle::new(0.0).map_err(|e| format!("각도 오류: {e:?}"))?,
        0.0,
    )
    .map_err(|e| format!("다각형 오류: {e:?}"))?;
    b.placed(
        buttons,
        "Hexagon",
        NodePayload::Shape(ShapeGeometry::Polygon(polygon)),
        (40.0, 280.0),
        Some(warm),
    )?;
    let star = PathData::new(
        WindingRule::NonZero,
        vec![
            PathCommand::MoveTo(point(48.0, 0.0)?),
            PathCommand::LineTo(point(60.0, 34.0)?),
            PathCommand::LineTo(point(96.0, 36.0)?),
            PathCommand::LineTo(point(68.0, 58.0)?),
            PathCommand::LineTo(point(78.0, 92.0)?),
            PathCommand::LineTo(point(48.0, 72.0)?),
            PathCommand::LineTo(point(18.0, 92.0)?),
            PathCommand::LineTo(point(28.0, 58.0)?),
            PathCommand::LineTo(point(0.0, 36.0)?),
            PathCommand::LineTo(point(36.0, 34.0)?),
            PathCommand::Close,
        ],
    )
    .map_err(|e| format!("path 오류: {e:?}"))?;
    let star = IdentifiedPath::assign(star, &mut || b.ids.next());
    b.placed(
        buttons,
        "Star",
        NodePayload::Path(star),
        (176.0, 280.0),
        Some(accent),
    )?;
    components
        .add_view(ViewDefinition::new(
            b.ids.next(),
            "Buttons".to_string(),
            buttons,
        ))
        .map_err(|e| format!("View 오류: {e:?}"))?;

    let Builder { nodes, stacks, .. } = b;
    definition
        .add_page(home)
        .map_err(|e| format!("페이지 오류: {e:?}"))?;
    definition
        .add_page(components)
        .map_err(|e| format!("페이지 오류: {e:?}"))?;
    let mut records = DocumentRecords::new(definition);
    records.nodes = nodes;
    records.modifier_stacks = stacks;
    Ok(records)
}
