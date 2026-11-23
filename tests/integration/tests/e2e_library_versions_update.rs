//! XVI.11.4 종단 시나리오(부분): 외부 library 두 버전 동시 사용 → update → override orphan 입력까지.
//!
//! 실제 Git 저장소 세 개(library C의 tag v1·v2, library A는 C v1에 의존, library B는 C v2에 의존)와 그 둘을 쓰는
//! 사용 프로젝트를 만들고, 제품 의존성 엔진으로 끝까지 해석합니다.
//!
//! | 단계 | 경로 | 판정 |
//! |---|---|---|
//! | 해석 | `MirrorCache`(bare mirror fetch) + `GitSelectorResolver`(ls-remote·cat-file) + `GitManifestSource`(commit의 `stellar.yaml`) + `resolve_dependency_graph` | C가 두 commit으로 그래프에 있음 |
//! | 경고 | `duplicate_version_reports` | C 하나에 버전 두 개, 버전마다 dependency path, 크기는 cache 없음이라 하한, telemetry에 URL 없음 |
//! | cache entry | `LibraryEntryStore::materialize`(commit의 Stellar project files를 content-addressed entry로) | node마다 실제 content hash·크기, 다중 버전 추가 용량이 정확한 값 |
//! | update | `prepare_library_update`(선택자를 시스템 Git으로 다시 해석) + 제품 `CacheUpdateSource`(후보 commit materialize → entry 검증 → cache snapshot, lock content hash 대조) + `diff_library_snapshots` | A v1 → v2 후보 commit, 삭제된 component property 목록 |
//! | update 뒤 그래프 | 사용 프로젝트 선언을 바꿔 다시 해석 | 다중 버전 경고가 사라짐 |
//!
//! 제품에 아직 없는 것(ADR-0111 결정 7, 계획 XV.7.3·XV.7.4·XV.8.4):
//!
//! - **override orphan 진단 생산자**: 후보 library와 사용 프로젝트 instance override를 대조해 진단을 내는 함수가
//!   없습니다(XV.7.3 미착수). 이 시나리오는 그 입력인 `removed_properties`까지 확인하고, 진단을 테스트 안에서 따로
//!   만들지 않습니다(이중 판정 금지).
//! - **시스템 Git 실행기·repository keyer·SHA-256·entry 검증기·export 공급자의 제품 구현**: 의존성 crate는 trait만
//!   두고 제품 구현이 없습니다. 아래 `PorcelainGit`·`Sha256Keyer`·`StorageSha256`은 git-porcelain `GitRunner`와 저장
//!   hash를 trait에 잇는 얇은 연결입니다. entry 검증기(`ManifestProbe`)는 materialize한 entry의 `stellar.yaml`을 제품
//!   매니페스트 parser로 읽기만 하고 storage 검증은 통과로 둡니다(entry에는 `.git`이 없어 `probe_project`를 쓸 수 없음).
//!   export는 library 저장소 파일에서 읽는 제품 공급자가 없어, 각 release가 commit한 `design/e2e-exports.txt`를 materialize한
//!   entry에서 읽는 공급자(`EntryExports`)로 둡니다. 그래서 후보 snapshot은 실제로 그 commit에서 꺼낸 파일로 만들어집니다.

mod e2e_common;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use e2e_common::{git, test_home, trusted_git, TempDir};
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_dependencies::cache_entry::{LibraryEntryStore, Sha256Digest};
use stellar_dependencies::cache_privacy::PrivateCacheIndex;
use stellar_dependencies::cache_root::LibraryCacheRoot;
use stellar_dependencies::content_hash::ContentHashText;
use stellar_dependencies::declaration::{
    CommitId, DependencyDeclaration, LibraryId, LogicalName, RefName, RepositoryUrl,
    VersionSelector,
};
use stellar_dependencies::duplicate_report::{duplicate_version_reports, AdditionalSize};
use stellar_dependencies::entry_validation::{
    LibraryProbeFailure, LibraryProjectValidator, ProbedLibrary, StorageValidationFailure,
};
use stellar_dependencies::git::{GitCommandOutput, GitCommandRunner, GitInvocationError};
use stellar_dependencies::git_refs::GitSelectorResolver;
use stellar_dependencies::graph::{DependencyGraph, DependencyNodeKey};
use stellar_dependencies::library_snapshot::{
    ExportFingerprint, ExportReadFailure, ExportVisibility, ExportedComponent, ExportedProperty,
    LibraryExportSource, LibraryExports, SnapshotLimits,
};
use stellar_dependencies::library_update::{
    prepare_library_update, CacheUpdateSource, LibraryUpdateRequest,
};
use stellar_dependencies::lock::LockDocument;
use stellar_dependencies::mirror::{MirrorCache, RepositoryKey, RepositoryKeyer};
use stellar_dependencies::resolver::resolve_dependency_graph;
use stellar_dependencies::resource_diff::diff_library_snapshots;
use stellar_git_porcelain::runner::GitRunner;
use stellar_project::manifest_source::{library_manifest_from_text, GitManifestSource};
use stellar_storage::hash::ContentHash;

/// git-porcelain `GitRunner`(테스트 HOME 환경·timeout)를 의존성 crate의 Git 실행 trait에 잇습니다.
struct PorcelainGit(GitRunner);

impl GitCommandRunner for PorcelainGit {
    fn run_git(
        &mut self,
        args: &[OsString],
        cwd: &Path,
    ) -> Result<GitCommandOutput, GitInvocationError> {
        let out = self
            .0
            .run(args, cwd)
            .map_err(|_| GitInvocationError::Spawn)?;
        Ok(GitCommandOutput {
            exit_code: out.exit_code,
            stdout: out.stdout,
            stderr: out.stderr,
        })
    }

    fn run_git_with_input(
        &mut self,
        args: &[OsString],
        cwd: &Path,
        input: &[u8],
    ) -> Result<GitCommandOutput, GitInvocationError> {
        let out = self
            .0
            .run_with_input(args, cwd, input)
            .map_err(|_| GitInvocationError::Spawn)?;
        Ok(GitCommandOutput {
            exit_code: out.exit_code,
            stdout: out.stdout,
            stderr: out.stderr,
        })
    }
}

/// repository 비교 키의 SHA-256(저장 hash 함수) 앞 32 hex.
#[derive(Clone, Copy)]
struct Sha256Keyer;

impl RepositoryKeyer for Sha256Keyer {
    fn repository_key(&self, repository: &RepositoryUrl) -> RepositoryKey {
        let digest = ContentHash::compute(repository.comparison_key().as_bytes());
        let hex: String = digest.digest().iter().map(|b| format!("{b:02x}")).collect();
        RepositoryKey::from_hex(&hex[..32]).expect("hex")
    }
}

/// 저장 hash 함수(SHA-256)를 cache entry content hash 계산에 잇습니다.
#[derive(Clone, Copy)]
struct StorageSha256;

impl Sha256Digest for StorageSha256 {
    fn sha256(&self, bytes: &[u8]) -> [u8; 32] {
        *ContentHash::compute(bytes).digest()
    }
}

/// entry 검증기: materialize한 entry의 `stellar.yaml`을 제품 매니페스트 parser로 읽습니다.
struct ManifestProbe;

impl LibraryProjectValidator for ManifestProbe {
    fn probe(&mut self, root: &Path) -> Result<ProbedLibrary, LibraryProbeFailure> {
        let text = match std::fs::read_to_string(root.join("stellar.yaml")) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(LibraryProbeFailure::NotAStellarProject)
            }
            Err(e) => return Err(LibraryProbeFailure::Io { kind: e.kind() }),
        };
        let manifest =
            library_manifest_from_text(&text).map_err(|_| LibraryProbeFailure::CorruptManifest)?;
        Ok(ProbedLibrary {
            project_id: manifest.library_id,
            schema_version: manifest.schema_version,
        })
    }

    fn validate_storage(&mut self, _: &Path, _: LibraryId) -> Result<(), StorageValidationFailure> {
        Ok(())
    }
}

/// release가 commit한 export 목록 파일(entry 기준 경로). materialize 대상(`design/`)이라 entry에 들어갑니다.
const EXPORTS_FILE: &str = "design/e2e-exports.txt";

/// export 공급자: entry의 [`EXPORTS_FILE`]에서 Card component의 property(`<번호> <이름>` 줄)를 읽습니다. `#` 줄은
/// 버전 표시입니다. property 줄이 없으면 export가 없는 library입니다.
#[derive(Default)]
struct EntryExports {
    /// 읽은 entry 디렉터리(후보가 실제로 materialize한 entry에서 읽었는지 확인용).
    reads: Vec<PathBuf>,
}

impl LibraryExportSource for EntryExports {
    fn read_exports(
        &mut self,
        root: &Path,
        _: LibraryId,
        _: &SnapshotLimits,
    ) -> Result<LibraryExports, ExportReadFailure> {
        self.reads.push(root.to_path_buf());
        let text = std::fs::read_to_string(root.join(EXPORTS_FILE))
            .map_err(|e| ExportReadFailure::Io { kind: e.kind() })?;
        let properties: Vec<(u64, String)> = text
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                let (n, name) = l.split_once(' ').ok_or(ExportReadFailure::Corrupt)?;
                Ok((
                    n.parse().map_err(|_| ExportReadFailure::Corrupt)?,
                    name.to_string(),
                ))
            })
            .collect::<Result<_, _>>()?;
        if properties.is_empty() {
            return Ok(LibraryExports {
                components: Vec::new(),
                token_sets: Vec::new(),
            });
        }
        let refs: Vec<(u64, &str)> = properties.iter().map(|(n, s)| (*n, s.as_str())).collect();
        Ok(card(&refs))
    }
}

fn runner() -> GitRunner {
    test_home();
    stellar_ops::git::system_git_runner().expect("시스템 Git")
}

fn library_id(n: u64) -> LibraryId {
    format!("0195c530-3acd-7bc1-8414-{n:012}").parse().unwrap()
}

fn sid(n: u64) -> StableId {
    StableId::new_v7(4_000_000 + n, 0x0111, n)
}

/// library 저장소 하나(일반 작업 저장소, tag로 버전 표시).
struct Library {
    dir: PathBuf,
    url: String,
}

fn manifest_text(
    id: LibraryId,
    name: &str,
    dependencies: &[(&str, LibraryId, &str, &str)],
) -> String {
    let mut text = format!(
        "schema_version: 4\nproject:\n  id: \"{id}\"\n  name: \"{name}\"\ntarget_profiles: []\n"
    );
    if dependencies.is_empty() {
        text.push_str("dependencies: {}\n");
    } else {
        text.push_str("dependencies:\n");
        for (dep, dep_id, url, tag) in dependencies {
            text.push_str(&format!(
                "  {dep}:\n    source: git\n    library_id: \"{dep_id}\"\n    repository: \"{url}\"\n    version: \"{tag}\"\n"
            ));
        }
    }
    text
}

fn library(
    gate: &stellar_git_porcelain::trust::GatedGitRunner,
    parent: &Path,
    name: &str,
) -> Library {
    let dir = parent.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    git(gate, &dir, &["init", "-q", "-b", "main"]);
    Library {
        url: format!("file://{}", dir.display()),
        dir,
    }
}

/// release 하나: 매니페스트와 export 목록(`properties`: Card component의 property 번호·이름)을 commit하고 tag를 붙입니다.
fn release(
    gate: &stellar_git_porcelain::trust::GatedGitRunner,
    lib: &Library,
    manifest: &str,
    tag: &str,
    properties: &[(u64, &str)],
) -> CommitId {
    // 매니페스트가 실제 파서를 통과하는지 먼저 확인합니다(형식 오류를 fixture 실수로 숨기지 않음).
    library_manifest_from_text(manifest).expect("library stellar.yaml");
    std::fs::write(lib.dir.join("stellar.yaml"), manifest).unwrap();
    let mut exports = format!("# {tag}\n");
    for (n, name) in properties {
        exports.push_str(&format!("{n} {name}\n"));
    }
    std::fs::create_dir_all(lib.dir.join("design")).unwrap();
    std::fs::write(lib.dir.join(EXPORTS_FILE), exports).unwrap();
    // materialize 대상이 아닌 파일(entry에 들어가지 않음).
    std::fs::write(lib.dir.join("CHANGELOG.md"), format!("{tag}\n")).unwrap();
    git(gate, &lib.dir, &["add", "-A"]);
    git(gate, &lib.dir, &["commit", "-q", "-m", tag]);
    git(gate, &lib.dir, &["tag", tag]);
    git(gate, &lib.dir, &["rev-parse", "HEAD"])
        .trim()
        .parse()
        .unwrap()
}

/// 사용 프로젝트 선언(실제 `stellar.yaml` 파서로 읽음).
fn declarations(text: &str) -> Vec<DependencyDeclaration> {
    library_manifest_from_text(text)
        .expect("사용 프로젝트 stellar.yaml")
        .dependencies
}

fn resolve(
    cache: &LibraryCacheRoot,
    work: &Path,
    root: &[DependencyDeclaration],
) -> DependencyGraph {
    let mut selectors = GitSelectorResolver::new(
        PorcelainGit(runner()),
        MirrorCache::new(cache.clone(), PorcelainGit(runner()), Sha256Keyer),
        work,
    );
    let mut manifests = GitManifestSource::new(PorcelainGit(runner()), cache.clone(), Sha256Keyer);
    resolve_dependency_graph(root, &mut selectors, &mut manifests).expect("의존성 해석")
}

fn card(properties: &[(u64, &str)]) -> LibraryExports {
    LibraryExports {
        components: vec![ExportedComponent {
            id: ComponentRef::new(sid(1)),
            name: "Card".into(),
            properties: properties
                .iter()
                .map(|(n, name)| ExportedProperty {
                    id: sid(*n),
                    name: (*name).into(),
                    type_signature: "string".into(),
                    visibility: ExportVisibility::Public,
                    default: ExportFingerprint::new(name.as_bytes().to_vec()),
                })
                .collect(),
            slots: Vec::new(),
            references: Vec::new(),
        }],
        token_sets: Vec::new(),
    }
}

#[test]
fn two_versions_warning_update_and_removed_override_property() {
    let Some(gate) = trusted_git() else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };
    let dir = TempDir::new("library-versions");
    let libraries = dir.path.join("libraries");
    let (c_id, a_id, b_id) = (library_id(3), library_id(1), library_id(2));

    // library C: v1, v2.
    let c = library(&gate, &libraries, "c");
    let c_v1 = release(&gate, &c, &manifest_text(c_id, "C", &[]), "v1", &[]);
    let c_v2 = release(&gate, &c, &manifest_text(c_id, "C", &[]), "v2", &[]);
    // A v1의 Card는 title·badge, v2는 badge를 지웁니다(override orphan 입력).
    let badge = 11;
    // library A: v1은 C v1, v2는 C v2에 의존. B는 C v2에 의존.
    let a = library(&gate, &libraries, "a");
    let a_v1 = release(
        &gate,
        &a,
        &manifest_text(a_id, "A", &[("c", c_id, &c.url, "v1")]),
        "v1",
        &[(10, "title"), (badge, "badge")],
    );
    let a_v2 = release(
        &gate,
        &a,
        &manifest_text(a_id, "A", &[("c", c_id, &c.url, "v2")]),
        "v2",
        &[(10, "title")],
    );
    let b = library(&gate, &libraries, "b");
    release(
        &gate,
        &b,
        &manifest_text(b_id, "B", &[("c", c_id, &c.url, "v2")]),
        "v1",
        &[],
    );

    let project = manifest_text(
        library_id(99),
        "사용 프로젝트",
        &[("a", a_id, &a.url, "v1"), ("b", b_id, &b.url, "v1")],
    );
    let root = declarations(&project);
    let cache = LibraryCacheRoot::in_app_data(&test_home().join("library-cache")).unwrap();
    let work = dir.path.join("neutral");
    std::fs::create_dir_all(&work).unwrap();

    // ── 1. 두 버전 동시 사용: C가 v1(a > c)과 v2(b > c) 두 node로 그래프에 있습니다.
    let graph = resolve(&cache, &work, &root);
    let c_nodes: Vec<_> = graph
        .nodes()
        .map(|n| &n.key)
        .filter(|k| k.library_id == c_id)
        .map(|k| k.commit.clone())
        .collect();
    let mut expected = vec![c_v1.clone(), c_v2.clone()];
    expected.sort();
    assert_eq!(c_nodes, expected, "C 두 버전 격리");
    let warnings = graph.duplicate_library_versions();
    assert_eq!(warnings.len(), 1);
    let text = warnings[0].to_string();
    assert!(text.contains("a > c") && text.contains("b > c"), "{text}");

    // ── 2. 경고 상세: 버전마다 경로, cache entry가 없으므로 추가 용량은 하한, telemetry에는 URL·경로 이름이 없음.
    let reports = duplicate_version_reports(&graph, &PrivateCacheIndex::default(), &Sha256Keyer);
    assert_eq!(reports.len(), 1);
    let report = &reports[0];
    assert_eq!(report.library_id, c_id);
    let paths: Vec<Vec<String>> = report
        .versions
        .iter()
        .map(|v| v.paths.iter().map(ToString::to_string).collect())
        .collect();
    assert!(
        paths.contains(&vec!["a > c".to_string()]) && paths.contains(&vec!["b > c".to_string()]),
        "{paths:?}"
    );
    assert!(matches!(
        report.expected_additional,
        AdditionalSize::LowerBound {
            unknown_versions: 2,
            ..
        }
    ));
    let telemetry = format!("{:?}", report.telemetry());
    assert!(
        !telemetry.contains(c.url.as_str()) && !telemetry.contains("file://"),
        "{telemetry}"
    );
    assert_eq!(report.telemetry().version_count, 2);

    // ── 3. cache entry: graph의 node마다 commit의 Stellar project files를 materialize합니다. lock content hash는 그 실제
    //    entry hash이고, 다중 버전 추가 용량은 이제 cache index의 크기로 정확히 계산됩니다.
    let mut store = LibraryEntryStore::new(cache.clone(), StorageSha256, Sha256Keyer);
    let mut mirrors = MirrorCache::new(cache.clone(), PorcelainGit(runner()), Sha256Keyer);
    let mut entry_git = PorcelainGit(runner());
    let mut hashes: BTreeMap<DependencyNodeKey, ContentHashText> = BTreeMap::new();
    let mut sizes: BTreeMap<DependencyNodeKey, u64> = BTreeMap::new();
    for entry in graph.sorted_entries() {
        let materialized = store
            .materialize(
                &mut entry_git,
                &mut mirrors,
                &entry.repository,
                &entry.key.commit,
                1,
            )
            .expect("entry materialize");
        assert!(materialized.dir.join("stellar.yaml").is_file());
        assert!(
            !materialized.dir.join("CHANGELOG.md").exists(),
            "project file만 entry에 들어감"
        );
        hashes.insert(entry.key.clone(), materialized.content_hash);
        sizes.insert(entry.key.clone(), materialized.size_bytes);
    }
    let c_v1_key = DependencyNodeKey::new(c_id, c_v1.clone());
    let c_v2_key = DependencyNodeKey::new(c_id, c_v2.clone());
    assert_ne!(
        hashes[&c_v1_key], hashes[&c_v2_key],
        "C 두 버전은 내용이 다름"
    );
    assert_eq!(sizes[&c_v1_key], sizes[&c_v2_key]);
    let index = PrivateCacheIndex::read_from(&cache.index_path()).expect("cache index");
    let reports = duplicate_version_reports(&graph, &index, &Sha256Keyer);
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0].expected_additional,
        AdditionalSize::Known(sizes[&c_v1_key]),
        "크기가 같은 고유 content 두 개: 추가 용량 = 한 버전 크기"
    );

    // ── 4. update: A를 v2로. 선택자를 시스템 Git으로 다시 해석하고, 제품 cache 공급자가 현재 entry(lock hash 대조)와
    //    후보 commit(새로 materialize → entry 검증)을 snapshot으로 만듭니다. lock은 바뀌지 않습니다.
    let lock = LockDocument::from_graph(&graph, &hashes).expect("lock");
    let mut selectors = GitSelectorResolver::new(
        PorcelainGit(runner()),
        MirrorCache::new(cache.clone(), PorcelainGit(runner()), Sha256Keyer),
        &work,
    );
    let mut validator = ManifestProbe;
    let mut exports = EntryExports::default();
    let mut source = CacheUpdateSource {
        store: &mut store,
        runner: &mut entry_git,
        mirrors: &mut mirrors,
        validator: &mut validator,
        exports: &mut exports,
        limits: SnapshotLimits::default(),
        now_unix_ms: 2,
    };
    let candidate = prepare_library_update(
        &root,
        &lock,
        &LibraryUpdateRequest {
            dependency: LogicalName::new("a").unwrap(),
            selector: Some(VersionSelector::Tag(RefName::new("v2").unwrap())),
        },
        &mut selectors,
        &mut source,
    )
    .expect("update 후보");
    assert_eq!(candidate.current_commit(), &a_v1);
    assert_eq!(candidate.candidate_commit(), &a_v2);
    assert!(candidate.selector_changed() && !candidate.is_same_commit());
    let a_v1_key = DependencyNodeKey::new(a_id, a_v1.clone());
    assert_eq!(
        candidate.current.snapshot.content_hash(),
        &hashes[&a_v1_key],
        "현재 snapshot = lock에 고정된 entry"
    );
    let a_v2_entry = store
        .lookup(&candidate.candidate.repository, &a_v2)
        .expect("cache index")
        .expect("후보 commit이 materialize됨");
    assert_eq!(
        candidate.candidate.snapshot.content_hash(),
        &a_v2_entry.content_hash
    );
    assert_ne!(a_v2_entry.content_hash, hashes[&a_v1_key]);
    assert_eq!(
        exports.reads,
        vec![
            store.entry_dir(&hashes[&a_v1_key]),
            store.entry_dir(&a_v2_entry.content_hash)
        ],
        "현재·후보 export를 각 commit의 entry에서 읽음"
    );
    assert_eq!(
        lock,
        LockDocument::from_graph(&graph, &hashes).unwrap(),
        "후보 준비는 lock을 바꾸지 않음"
    );

    // ── 5. override orphan 입력: v2에서 사라진 property(badge)를 override한 instance는 orphan이 됩니다.
    let diff =
        diff_library_snapshots(&candidate.current.snapshot, &candidate.candidate.snapshot).unwrap();
    let removed: Vec<_> = diff.removed_properties().collect();
    assert_eq!(
        removed,
        vec![(sid(1), sid(badge))],
        "삭제된 property = orphan override 대상"
    );

    // ── 6. update 뒤 사용 프로젝트 선언(A v2)으로 다시 해석하면 C는 v2 하나이고 경고가 없습니다.
    let updated = declarations(&manifest_text(
        library_id(99),
        "사용 프로젝트",
        &[("a", a_id, &a.url, "v2"), ("b", b_id, &b.url, "v1")],
    ));
    let graph = resolve(&cache, &work, &updated);
    let c_nodes: Vec<_> = graph
        .nodes()
        .map(|n| &n.key)
        .filter(|k| k.library_id == c_id)
        .map(|k| k.commit.clone())
        .collect();
    assert_eq!(c_nodes, vec![c_v2], "C v2 하나로 합쳐짐");
    assert!(graph.duplicate_library_versions().is_empty());
    assert!(
        duplicate_version_reports(&graph, &PrivateCacheIndex::default(), &Sha256Keyer).is_empty()
    );
}
