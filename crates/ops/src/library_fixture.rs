//! 외부 library 제품 명령 시험 fixture (ADR-0137). 프로덕션 경로에서 쓰지 않습니다.
//!
//! 실제 Git 저장소에 텍스트 원본(ADR-0088) library를 release하고, 텍스트 원본 사용 프로젝트를 만듭니다. Git은 전역·시스템
//! config 없이 실행합니다. library 컴포넌트는 Property 없는 정의만 씁니다(텍스트 원본은 Property를 아직 담지 못함,
//! ADR-0088 결정 2).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use stellar_document::component::definition::ComponentLibrary;
use stellar_document::component::instance::ComponentInstancePayload;
use stellar_document::context::fixtures::{ComponentBuilder, SceneBuilder};
use stellar_document::model::DocumentModel;
use stellar_project::document_io::source_v4::project::{encode, write_new, TextProject};

use crate::core_types::ids::StableId;
use crate::core_types::reference::ComponentRef;

/// 시스템 Git을 전역 config 없이 실행하고 성공을 강제합니다. stdout을 돌려줍니다.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Library Fixture")
        .env("GIT_AUTHOR_EMAIL", "library@stellar.invalid")
        .env("GIT_COMMITTER_NAME", "Library Fixture")
        .env("GIT_COMMITTER_EMAIL", "library@stellar.invalid")
        .output()
        .expect("git 실행");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// 시스템 Git이 있는지.
pub fn has_git() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// fixture StableId(공간 `space`).
pub fn sid(space: u64, n: u64) -> StableId {
    StableId::new_v7(6_000_000 + space * 1_000 + n, 0x0137, n)
}

/// library id 텍스트(번호).
pub fn library_id_text(n: u64) -> String {
    format!("0195c530-3acd-7bc1-8414-{n:012}")
}

/// `stellar.yaml` 텍스트(schema 4, 의존성 없음).
pub fn manifest_text(id: &str, name: &str) -> String {
    format!(
        "schema_version: 4\nproject:\n  id: \"{id}\"\n  name: \"{name}\"\ntarget_profiles: []\ndependencies: {{}}\n"
    )
}

/// Property 없는 컴포넌트 정의 묶음: (component StableId, root StableId, 이름).
pub fn components(defs: &[(StableId, StableId, &str)]) -> ComponentLibrary {
    let mut library = ComponentLibrary::new();
    for (id, root, name) in defs {
        ComponentBuilder::new(*id, name, *root).register(&mut library);
    }
    library
}

/// 문서 하나(View 하나, root Frame)에 instance들을 넣습니다.
pub fn document(space: u64, instances: &[(StableId, ComponentInstancePayload)]) -> DocumentModel {
    let root = sid(space, 903);
    let mut builder = SceneBuilder::new(sid(space, 900), sid(space, 901), sid(space, 902), root);
    for (id, payload) in instances {
        builder = builder.instance(*id, root, payload.clone());
    }
    builder.build()
}

/// override 없는 instance payload.
pub fn instance_of(component: StableId) -> ComponentInstancePayload {
    ComponentInstancePayload::new(ComponentRef::new(component))
}

/// 텍스트 원본 `design/`을 씁니다(기존 `design/`은 지움).
pub fn write_design(root: &Path, document: DocumentModel, components: ComponentLibrary) {
    let design = root.join("design");
    let _ = std::fs::remove_dir_all(&design);
    let encoded = encode(&TextProject {
        document,
        components,
        token_sets: vec![],
        images: BTreeMap::new(),
    })
    .expect("텍스트 원본 encode");
    write_new(&design, &encoded).expect("텍스트 원본 쓰기");
}

/// library 저장소 하나.
pub struct LibraryRepo {
    /// 작업 폴더.
    pub dir: PathBuf,
    /// `file://` URL.
    pub url: String,
    /// library id 텍스트.
    pub id: String,
}

impl LibraryRepo {
    /// 빈 library 저장소를 만듭니다.
    pub fn init(parent: &Path, name: &str, id: u64) -> LibraryRepo {
        let dir = parent.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        LibraryRepo {
            url: format!("file://{}", dir.display()),
            dir,
            id: library_id_text(id),
        }
    }

    /// `components`를 담은 release를 commit하고 `tag`를 붙입니다. commit id를 돌려줍니다.
    pub fn release(&self, space: u64, library: ComponentLibrary, tag: &str) -> String {
        std::fs::write(
            self.dir.join("stellar.yaml"),
            manifest_text(&self.id, "Library"),
        )
        .unwrap();
        write_design(&self.dir, document(space, &[]), library);
        std::fs::write(self.dir.join("CHANGELOG.md"), format!("{tag}\n")).unwrap();
        git(&self.dir, &["add", "-A"]);
        git(&self.dir, &["commit", "-q", "-m", tag]);
        git(&self.dir, &["tag", tag]);
        git(&self.dir, &["rev-parse", "HEAD"]).trim().to_string()
    }
}

/// 사용 프로젝트(일반 폴더 + `stellar.yaml` + 텍스트 원본 문서)를 만듭니다.
pub fn using_project(parent: &Path, document: DocumentModel) -> PathBuf {
    let dir = parent.join("app-project");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::write(
        dir.join("stellar.yaml"),
        manifest_text(&library_id_text(93), "사용 프로젝트"),
    )
    .unwrap();
    write_design(&dir, document, ComponentLibrary::new());
    dir
}
