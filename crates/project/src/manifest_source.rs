//! exact commit의 library `stellar.yaml` 읽기 (XV.2.2 resolver 연결).
//!
//! resolver([`resolve_dependency_graph`](stellar_dependencies::resolver::resolve_dependency_graph))는
//! 각 resolved commit의 library 매니페스트를 [`LibraryManifestSource`]로 읽습니다. 이 모듈은 그
//! 공급자를 실제 `stellar.yaml` 파서(제한 loader → [`Manifest::from_yaml`])로 구현합니다.
//!
//! [`GitManifestSource`]는 이미 fetch된 repository mirror(XV.3.2)에서 시스템 Git 명령 출력만
//! 해석합니다(Git 객체를 직접 파싱하지 않음).
//!
//! 1. `git --git-dir <mirror> ls-tree -l -z <commit> -- stellar.yaml`: commit이 없으면 `Unavailable`,
//!    항목이 없으면 `NotAStellarProject`(일반 Git 저장소)입니다. 일반 파일 blob이 아니거나(symlink 등)
//!    크기가 제한 loader 입력 상한을 넘으면 `InvalidManifest`입니다.
//! 2. `git --git-dir <mirror> cat-file blob <oid>`: 원시 bytes(filter·textconv 없음)를 실제 파서로
//!    검증합니다. `project.id`가 [`LibraryId`]가 아니면 `InvalidManifest`입니다.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use stellar_dependencies::cache_root::LibraryCacheRoot;
use stellar_dependencies::declaration::{CommitId, LibraryId, RepositoryUrl};
use stellar_dependencies::git::{git_args, GitCommandRunner};
use stellar_dependencies::mirror::{mirror_path, RepositoryKeyer};
use stellar_dependencies::resolver::{LibraryManifest, LibraryManifestSource, ManifestSourceError};

use crate::manifest::Manifest;
use crate::root::MANIFEST_FILE;
use crate::yaml::load_restricted;
use crate::yaml::loader::MAX_INPUT_BYTES;

/// `stellar.yaml` 텍스트를 실제 파서로 읽어 resolver용 library 매니페스트로 바꿉니다.
///
/// 문법·스키마 오류, `project.id`가 UUID가 아닌 경우는 [`ManifestSourceError::InvalidManifest`]입니다.
/// library 매니페스트의 `local_override`는 resolver가 무시합니다(사용 프로젝트 선언에만 적용).
pub fn library_manifest_from_text(text: &str) -> Result<LibraryManifest, ManifestSourceError> {
    let value = load_restricted(text).map_err(|_| ManifestSourceError::InvalidManifest)?;
    let manifest = Manifest::from_yaml(&value).map_err(|_| ManifestSourceError::InvalidManifest)?;
    let library_id = manifest
        .project
        .id
        .parse::<LibraryId>()
        .map_err(|_| ManifestSourceError::InvalidManifest)?;
    Ok(LibraryManifest {
        library_id,
        schema_version: manifest.schema_version,
        dependencies: manifest.dependencies,
    })
}

/// fetch된 mirror에서 commit의 `stellar.yaml`을 읽는 공급자.
pub struct GitManifestSource<R, K> {
    runner: R,
    root: LibraryCacheRoot,
    keyer: K,
}

impl<R: GitCommandRunner, K: RepositoryKeyer> GitManifestSource<R, K> {
    /// 공급자를 만듭니다. `root`·`keyer`는 mirror cache(XV.3.2)와 같아야 같은 mirror를 읽습니다.
    pub fn new(runner: R, root: LibraryCacheRoot, keyer: K) -> GitManifestSource<R, K> {
        GitManifestSource {
            runner,
            root,
            keyer,
        }
    }

    /// 실행기.
    pub fn runner(&self) -> &R {
        &self.runner
    }

    fn mirror(&self, repository: &RepositoryUrl) -> Result<PathBuf, ManifestSourceError> {
        let dir = mirror_path(&self.root, &self.keyer, repository);
        if dir.join("HEAD").is_file() {
            Ok(dir)
        } else {
            Err(ManifestSourceError::Unavailable)
        }
    }

    fn git(&mut self, git_dir: &Path, args: &[&str]) -> Result<Vec<u8>, ManifestSourceError> {
        let mut full = vec![OsString::from("--git-dir"), git_dir.as_os_str().to_owned()];
        full.extend(git_args(args.iter().copied()));
        let out = self
            .runner
            .run_git(&full, git_dir)
            .map_err(|_| ManifestSourceError::Unavailable)?;
        if out.success() {
            Ok(out.stdout)
        } else {
            Err(ManifestSourceError::Unavailable)
        }
    }
}

/// `ls-tree -l -z` 한 항목(`<mode> SP <type> SP <oid> SP+ <size> TAB <path> NUL`)을 해석합니다.
fn parse_manifest_entry(stdout: &[u8]) -> Result<Option<(String, u64)>, ManifestSourceError> {
    let mut records = stdout.split(|b| *b == 0).filter(|r| !r.is_empty());
    let Some(record) = records.next() else {
        return Ok(None);
    };
    if records.next().is_some() {
        return Err(ManifestSourceError::InvalidManifest);
    }
    let text = std::str::from_utf8(record).map_err(|_| ManifestSourceError::InvalidManifest)?;
    let (meta, path) = text
        .split_once('\t')
        .ok_or(ManifestSourceError::InvalidManifest)?;
    if path != MANIFEST_FILE {
        return Err(ManifestSourceError::InvalidManifest);
    }
    let mut fields = meta.split(' ').filter(|f| !f.is_empty());
    let (Some(mode), Some(kind), Some(oid), Some(size), None) = (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    ) else {
        return Err(ManifestSourceError::InvalidManifest);
    };
    // 일반 파일 blob만(symlink `120000`·submodule `160000`·tree 거부).
    if !matches!(mode, "100644" | "100755") || kind != "blob" {
        return Err(ManifestSourceError::InvalidManifest);
    }
    if oid.parse::<CommitId>().is_err() {
        // object id 형식(40/64 hex)은 commit id와 같습니다.
        return Err(ManifestSourceError::InvalidManifest);
    }
    let size = size
        .parse::<u64>()
        .map_err(|_| ManifestSourceError::InvalidManifest)?;
    Ok(Some((oid.to_string(), size)))
}

impl<R: GitCommandRunner, K: RepositoryKeyer> LibraryManifestSource for GitManifestSource<R, K> {
    fn manifest_at(
        &mut self,
        repository: &RepositoryUrl,
        commit: &CommitId,
    ) -> Result<LibraryManifest, ManifestSourceError> {
        let dir = self.mirror(repository)?;
        let listing = self.git(
            &dir,
            &["ls-tree", "-l", "-z", commit.as_str(), "--", MANIFEST_FILE],
        )?;
        let Some((oid, size)) = parse_manifest_entry(&listing)? else {
            return Err(ManifestSourceError::NotAStellarProject);
        };
        if size > MAX_INPUT_BYTES as u64 {
            return Err(ManifestSourceError::InvalidManifest);
        }
        let bytes = self.git(&dir, &["cat-file", "blob", &oid])?;
        let text = String::from_utf8(bytes).map_err(|_| ManifestSourceError::InvalidManifest)?;
        library_manifest_from_text(&text)
    }
}

#[cfg(test)]
pub(crate) mod test_git {
    //! 테스트용 hermetic 시스템 Git 실행기와 fixture 도구.

    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use stellar_dependencies::declaration::RepositoryUrl;
    use stellar_dependencies::git::{
        git_args, GitCommandOutput, GitCommandRunner, GitInvocationError,
    };
    use stellar_dependencies::mirror::{RepositoryKey, RepositoryKeyer};

    /// drop 시 삭제되는 임시 디렉터리(canonical 경로).
    pub(crate) struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        pub(crate) fn new(tag: &str) -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-project-deps-{tag}-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("임시 디렉터리");
            let path = std::fs::canonicalize(&path).expect("canonicalize");
            TempDir { path }
        }

        pub(crate) fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// PATH의 `git`. 없으면 테스트 실패(조용히 통과하지 않음).
    fn git_binary() -> PathBuf {
        let path = std::env::var_os("PATH").unwrap_or_default();
        std::env::split_paths(&path)
            .map(|dir| dir.join("git"))
            .find(|candidate| candidate.is_file())
            .expect("실제 Git fixture 테스트에는 시스템 git이 필요합니다")
    }

    /// 전역 config·HOME과 무관한 시스템 Git 실행기. 하위 명령별 호출 횟수를 기록합니다.
    pub(crate) struct TestGit {
        git: PathBuf,
        home: PathBuf,
        pub(crate) calls: BTreeMap<String, usize>,
    }

    impl TestGit {
        pub(crate) fn new(home: &Path) -> TestGit {
            TestGit {
                git: git_binary(),
                home: home.to_path_buf(),
                calls: BTreeMap::new(),
            }
        }

        pub(crate) fn count(&self, subcommand: &str) -> usize {
            self.calls.get(subcommand).copied().unwrap_or(0)
        }

        fn command(&self, cwd: &Path) -> Command {
            let mut command = Command::new(&self.git);
            command
                .current_dir(cwd)
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("HOME", &self.home)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_AUTHOR_NAME", "Fixture")
                .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
                .env("GIT_COMMITTER_NAME", "Fixture")
                .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
                .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
                .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
                .env("LC_ALL", "C");
            command
        }

        /// fixture 구성용 실행(성공 강제, 호출 기록 제외). stdout을 반환합니다.
        pub(crate) fn ok<I, S>(&self, cwd: &Path, args: I) -> String
        where
            I: IntoIterator<Item = S>,
            S: Into<OsString>,
        {
            let args = git_args(args);
            let out = self.command(cwd).args(&args).output().expect("git 실행");
            assert!(
                out.status.success(),
                "fixture git 실패 {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8(out.stdout).expect("utf8")
        }

        /// `main` branch로 빈 저장소를 만듭니다(서명 끔).
        pub(crate) fn init_repo(&self, dir: &Path) {
            std::fs::create_dir_all(dir).expect("repo dir");
            self.ok(dir, ["init", "-q", "-b", "main"]);
            self.ok(dir, ["config", "commit.gpgsign", "false"]);
            self.ok(dir, ["config", "tag.gpgsign", "false"]);
        }

        /// `stellar.yaml`을 쓰고 commit합니다. 새 HEAD commit을 반환합니다.
        pub(crate) fn commit_manifest(&self, dir: &Path, text: &str, message: &str) -> String {
            std::fs::write(dir.join("stellar.yaml"), text).expect("write");
            self.ok(dir, ["add", "stellar.yaml"]);
            self.ok(dir, ["commit", "-q", "-m", message]);
            self.ok(dir, ["rev-parse", "HEAD"]).trim().to_string()
        }
    }

    impl GitCommandRunner for TestGit {
        fn run_git(
            &mut self,
            args: &[OsString],
            cwd: &Path,
        ) -> Result<GitCommandOutput, GitInvocationError> {
            let mut iter = args.iter().map(|a| a.to_string_lossy().into_owned());
            let mut subcommand = String::new();
            while let Some(arg) = iter.next() {
                if arg == "--git-dir" || arg == "-C" {
                    iter.next();
                    continue;
                }
                if !arg.starts_with('-') {
                    subcommand = arg;
                    break;
                }
            }
            *self.calls.entry(subcommand).or_default() += 1;
            let out = self
                .command(cwd)
                .args(args)
                .output()
                .map_err(|_| GitInvocationError::Spawn)?;
            Ok(GitCommandOutput {
                exit_code: out.status.code(),
                stdout: out.stdout,
                stderr: out.stderr,
            })
        }
    }

    /// 테스트 keyed identifier(FNV-1a 64, 암호학적 아님).
    pub(crate) struct FnvKeyer;

    impl RepositoryKeyer for FnvKeyer {
        fn repository_key(&self, repository: &RepositoryUrl) -> RepositoryKey {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for b in repository.comparison_key().as_bytes() {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
            RepositoryKey::from_hex(&format!("{h:016x}")).expect("16자리 hex")
        }
    }
}

#[cfg(test)]
pub(crate) mod text_resolver {
    //! 테스트용 결정적 resolver 입력: 표 기반 선택자 해석기와, library `stellar.yaml` **텍스트**를
    //! 실제 파서([`library_manifest_from_text`])로 읽는 매니페스트 공급자.

    use std::collections::BTreeMap;

    use stellar_dependencies::declaration::{CommitId, RefName, RepositoryUrl, VersionSelector};
    use stellar_dependencies::graph::DependencyGraph;
    use stellar_dependencies::resolver::{
        resolve_dependency_graph, LibraryManifest, LibraryManifestSource, ManifestSourceError,
        ResolveError,
    };
    use stellar_dependencies::selection::{SelectorError, SelectorResolver};

    use super::library_manifest_from_text;
    use crate::manifest::fixtures::library;
    use crate::manifest::Manifest;
    use crate::yaml::load_restricted;

    /// repository 비교 키.
    pub(crate) fn repo_key(url: &str) -> String {
        RepositoryUrl::new(url).expect("url").comparison_key()
    }

    /// tag 선택자.
    pub(crate) fn tag(name: &str) -> VersionSelector {
        VersionSelector::Tag(RefName::new(name).expect("tag"))
    }

    /// branch 선택자.
    pub(crate) fn branch(name: &str) -> VersionSelector {
        VersionSelector::Branch(RefName::new(name).expect("branch"))
    }

    /// `(repository 비교 키, 선택자)` → commit 표.
    #[derive(Default)]
    pub(crate) struct TableSelectors(pub(crate) BTreeMap<(String, VersionSelector), CommitId>);

    impl TableSelectors {
        pub(crate) fn with(
            mut self,
            url: &str,
            selector: VersionSelector,
            commit: CommitId,
        ) -> Self {
            self.0.insert((repo_key(url), selector), commit);
            self
        }
    }

    impl SelectorResolver for TableSelectors {
        fn resolve(
            &mut self,
            repository: &RepositoryUrl,
            selector: &VersionSelector,
        ) -> Result<CommitId, SelectorError> {
            self.0
                .get(&(repository.comparison_key(), selector.clone()))
                .cloned()
                .ok_or(SelectorError::CommitNotFound)
        }
    }

    /// `(repository 비교 키, commit)` → library `stellar.yaml` 텍스트.
    #[derive(Default)]
    pub(crate) struct TextManifests(pub(crate) BTreeMap<(String, CommitId), String>);

    impl TextManifests {
        pub(crate) fn with(mut self, url: &str, commit: CommitId, text: String) -> Self {
            self.0.insert((repo_key(url), commit), text);
            self
        }
    }

    impl LibraryManifestSource for TextManifests {
        fn manifest_at(
            &mut self,
            repository: &RepositoryUrl,
            commit: &CommitId,
        ) -> Result<LibraryManifest, ManifestSourceError> {
            let text = self
                .0
                .get(&(repository.comparison_key(), commit.clone()))
                .ok_or(ManifestSourceError::Unavailable)?;
            library_manifest_from_text(text)
        }
    }

    /// `dependencies:` 아래 entry 하나(앞 줄바꿈 포함). `version`은 `" v1"` 또는
    /// `"\n      branch: main"` 같은 `version:` 뒤 텍스트입니다.
    pub(crate) fn entry_text(name: &str, lib: u8, url: &str, version: &str) -> String {
        format!(
            "\n  {name}:\n    source: git\n    library_id: {}\n    repository: \"{url}\"\n    \
             version:{version}",
            library(lib)
        )
    }

    /// library 프로젝트 `stellar.yaml`. `entries`는 [`entry_text`] 연결(비면 `{}`).
    pub(crate) fn library_text(lib: u8, schema_version: u32, entries: &str) -> String {
        let dependencies = if entries.is_empty() {
            " {}".to_string()
        } else {
            entries.to_string()
        };
        format!(
            "schema_version: {schema_version}\nproject:\n  id: {}\n  name: lib\n\
             target_profiles: []\ndependencies:{dependencies}\n",
            library(lib)
        )
    }

    /// 사용 프로젝트 `stellar.yaml`(project id `...0ff`).
    pub(crate) fn root_text(entries: &str) -> String {
        library_text(0xff, 1, entries)
    }

    /// 사용 프로젝트 텍스트를 실제 파서로 읽어 resolve합니다.
    pub(crate) fn resolve_root(
        root: &str,
        selectors: &mut TableSelectors,
        manifests: &mut TextManifests,
    ) -> Result<DependencyGraph, ResolveError> {
        let manifest = Manifest::from_yaml(&load_restricted(root).expect("YAML")).expect("schema");
        resolve_dependency_graph(&manifest.dependencies, selectors, manifests)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use stellar_dependencies::content_hash::ContentHashText;
    use stellar_dependencies::git_refs::GitSelectorResolver;
    use stellar_dependencies::graph::DependencyNodeKey;
    use stellar_dependencies::lock::LockDocument;
    use stellar_dependencies::mirror::MirrorCache;
    use stellar_dependencies::resolver::{resolve_dependency_graph, ResolveError};

    use super::test_git::{FnvKeyer, TempDir, TestGit};
    use super::*;
    use crate::lock::Lockfile;

    /// library 매니페스트 텍스트. `deps`는 `dependencies:` 아래 entry 블록(들여쓰기 포함).
    fn library_yaml(id: &str, schema_version: u32, deps: &str) -> String {
        let deps = if deps.is_empty() {
            " {}\n".to_string()
        } else {
            format!("\n{deps}")
        };
        format!(
            "schema_version: {schema_version}\nproject:\n  id: {id}\n  name: lib\n\
             target_profiles: []\ndependencies:{deps}"
        )
    }

    fn dep_block(name: &str, id: &str, url: &str, version: &str) -> String {
        format!(
            "  {name}:\n    source: git\n    library_id: {id}\n    repository: \"{url}\"\n    \
             version:{version}\n"
        )
    }

    const LIB_A: &str = "0195c530-3acd-7bc1-8414-00000000000a";
    const LIB_B: &str = "0195c530-3acd-7bc1-8414-00000000000b";
    const LIB_C: &str = "0195c530-3acd-7bc1-8414-00000000000c";

    struct Fixture {
        _tmp: TempDir,
        root_text: String,
        reordered_root_text: String,
        cache: LibraryCacheRoot,
        neutral: PathBuf,
        home: PathBuf,
        c_v1: String,
        c_v2: String,
    }

    /// a(tag v1) → c(commit = v1), b(tag v1) → c(branch main = v2). 사용 프로젝트는 a, b를
    /// tag로 선언합니다(tag·branch·commit 선택자 모두 실제 Git으로 해석, XV.2.4).
    fn fixture() -> Fixture {
        let tmp = TempDir::new("resolve");
        let git = TestGit::new(tmp.path());
        let c = tmp.path().join("c");
        git.init_repo(&c);
        let c_v1 = git.commit_manifest(&c, &library_yaml(LIB_C, 1, ""), "c v1");
        git.ok(&c, ["tag", "v1"]);
        let c_v2 = git.commit_manifest(&c, &library_yaml(LIB_C, 2, ""), "c v2");
        git.ok(&c, ["tag", "-a", "v2", "-m", "v2"]);
        let c_url = c.to_str().unwrap().to_string();

        let a = tmp.path().join("a");
        git.init_repo(&a);
        // a는 c를 exact commit 선택자로 선언(tag는 사용 프로젝트 선언, branch는 b가 씀).
        git.commit_manifest(
            &a,
            &library_yaml(
                LIB_A,
                1,
                &dep_block("c", LIB_C, &c_url, &format!("\n      commit: {c_v1}")),
            ),
            "a",
        );
        git.ok(&a, ["tag", "v1"]);
        let b = tmp.path().join("b");
        git.init_repo(&b);
        git.commit_manifest(
            &b,
            &library_yaml(
                LIB_B,
                1,
                &dep_block("c", LIB_C, &c_url, "\n      branch: main"),
            ),
            "b",
        );
        git.ok(&b, ["tag", "v1"]);

        let block_a = dep_block("a", LIB_A, a.to_str().unwrap(), " v1");
        let block_b = dep_block("b", LIB_B, b.to_str().unwrap(), " v1");
        let root = |first: &str, second: &str| {
            format!(
                "schema_version: 1\nproject:\n  id: 0195c530-3acd-7bc1-8414-0000000000ff\n  \
                 name: app\ntarget_profiles: []\ndependencies:\n{first}{second}"
            )
        };
        let neutral = tmp.path().join("neutral");
        std::fs::create_dir_all(&neutral).unwrap();
        Fixture {
            root_text: root(&block_a, &block_b),
            reordered_root_text: root(&block_b, &block_a),
            cache: LibraryCacheRoot::in_app_data(&tmp.path().join("app-data")).unwrap(),
            neutral,
            home: tmp.path().to_path_buf(),
            c_v1,
            c_v2,
            _tmp: tmp,
        }
    }

    /// 실제 파서로 사용 프로젝트 선언을 읽고, 실제 Git·mirror·library 매니페스트 파서로 해석합니다.
    fn resolve_text(
        f: &Fixture,
        root_text: &str,
    ) -> Result<stellar_dependencies::graph::DependencyGraph, ResolveError> {
        resolve_counting(f, root_text).0
    }

    /// [`resolve_text`]와 같고, mirror fetch 횟수와 매니페스트 공급자의 하위 명령 기록을 함께
    /// 반환합니다.
    fn resolve_counting(
        f: &Fixture,
        root_text: &str,
    ) -> (
        Result<stellar_dependencies::graph::DependencyGraph, ResolveError>,
        usize,
        BTreeMap<String, usize>,
    ) {
        let manifest = Manifest::from_yaml(&load_restricted(root_text).unwrap()).unwrap();
        let mirrors = MirrorCache::new(f.cache.clone(), TestGit::new(&f.home), FnvKeyer);
        let mut selectors = GitSelectorResolver::new(TestGit::new(&f.home), mirrors, &f.neutral);
        let mut manifests =
            GitManifestSource::new(TestGit::new(&f.home), f.cache.clone(), FnvKeyer);
        let graph =
            resolve_dependency_graph(&manifest.dependencies, &mut selectors, &mut manifests);
        let fetches = selectors.fetched().runner().count("fetch");
        (graph, fetches, manifests.runner().calls.clone())
    }

    /// node마다 결정적 content hash(실제 cache entry hash는 XV.3.3·XV.3.4 범위).
    fn hashes(
        graph: &stellar_dependencies::graph::DependencyGraph,
    ) -> BTreeMap<DependencyNodeKey, ContentHashText> {
        graph
            .nodes()
            .map(|n| {
                let mut digest = [0u8; 32];
                for (i, b) in n.key.commit.as_str().bytes().take(32).enumerate() {
                    digest[i] = b;
                }
                (n.key.clone(), ContentHashText::from_sha256_digest(digest))
            })
            .collect()
    }

    fn lock_text(graph: &stellar_dependencies::graph::DependencyGraph) -> String {
        let document = LockDocument::from_graph(graph, &hashes(graph)).unwrap();
        Lockfile::from_document(document).write_checked().unwrap()
    }

    #[test]
    fn real_git_resolution_through_real_parsers_pins_branch_to_exact_commit() {
        let f = fixture();
        let (graph, fetches, manifest_calls) = resolve_counting(&f, &f.root_text);
        let graph = graph.expect("resolve");
        // 저장소 3개(a, b, c)를 각각 한 번만 fetch(c는 두 경로에서 선언, XV.3.2). 매니페스트
        // 읽기는 fetch된 mirror에서 ls-tree·cat-file만 실행(네트워크 없음).
        assert_eq!(fetches, 3);
        assert_eq!(
            manifest_calls
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["cat-file", "ls-tree"]
        );
        // a, b, c@v1, c@v2: 같은 library 두 버전은 별도 node(XV.2.1), 경고 1건(XV.2.5).
        assert_eq!(graph.node_count(), 4);
        let warnings = graph.duplicate_library_versions();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].library_id, LIB_C.parse().unwrap());
        assert_eq!(warnings[0].versions.len(), 2);

        let text = lock_text(&graph);
        let lock = Lockfile::from_yaml(&load_restricted(&text).unwrap()).unwrap();
        // XV.1.2: branch(main) 선언은 lock에 exact commit(v2)으로, requested는 선언 그대로.
        let via_b = lock
            .resolved
            .iter()
            .find(|e| e.dependency_path.to_string() == "b > c")
            .expect("b > c entry");
        assert_eq!(via_b.exact_commit.as_str(), f.c_v2);
        assert_eq!(
            via_b.requested,
            stellar_dependencies::declaration::VersionSelector::Branch(
                stellar_dependencies::declaration::RefName::new("main").unwrap()
            )
        );
        let via_a = lock
            .resolved
            .iter()
            .find(|e| e.dependency_path.to_string() == "a > c")
            .expect("a > c entry");
        assert_eq!(via_a.exact_commit.as_str(), f.c_v1);
        // library 매니페스트 schema version(실제 파서가 읽은 값)이 lock에 기록됨.
        assert_eq!(via_a.schema_version, 1);
        assert_eq!(via_b.schema_version, 2);
        assert!(lock
            .resolved
            .iter()
            .all(|e| e.exact_commit.as_str().len() == 40));
        assert!(text.contains("branch: main"));

        // XV.2.6: 선언 순서를 바꿔도 lock 텍스트 byte 동일.
        let reordered = resolve_text(&f, &f.reordered_root_text).expect("resolve");
        assert_eq!(lock_text(&reordered), text);
    }

    #[test]
    fn plain_repository_and_broken_library_manifest_are_rejected() {
        let f = fixture();
        let git = TestGit::new(&f.home);
        let plain = f.home.join("plain");
        git.init_repo(&plain);
        std::fs::write(plain.join("README.md"), "plain\n").unwrap();
        git.ok(&plain, ["add", "README.md"]);
        git.ok(&plain, ["commit", "-q", "-m", "plain"]);
        git.ok(&plain, ["tag", "v1"]);
        let broken = f.home.join("broken");
        git.init_repo(&broken);
        git.commit_manifest(&broken, "schema_version: 1\nproject: [oops\n", "broken");
        git.ok(&broken, ["tag", "v1"]);

        for (dir, expected) in [
            (&plain, ManifestSourceError::NotAStellarProject),
            (&broken, ManifestSourceError::InvalidManifest),
        ] {
            let text = format!(
                "schema_version: 1\nproject:\n  id: 0195c530-3acd-7bc1-8414-0000000000ff\n  \
                 name: app\ntarget_profiles: []\ndependencies:\n{}",
                dep_block("x", LIB_A, dir.to_str().unwrap(), " v1")
            );
            match resolve_text(&f, &text) {
                Err(ResolveError::Manifest { error, path }) => {
                    assert_eq!(error, expected);
                    assert_eq!(path.to_string(), "x");
                }
                other => panic!("{expected:?} 기대: {other:?}"),
            }
        }
    }

    #[test]
    fn manifest_text_and_listing_rules() {
        let manifest = library_manifest_from_text(&library_yaml(LIB_A, 3, "")).unwrap();
        assert_eq!(manifest.library_id, LIB_A.parse().unwrap());
        assert_eq!(manifest.schema_version, 3);
        // project.id가 UUID가 아니면 library 매니페스트로 쓸 수 없음.
        assert_eq!(
            library_manifest_from_text(&library_yaml("not-a-uuid", 1, "")),
            Err(ManifestSourceError::InvalidManifest)
        );
        let oid = "a".repeat(40);
        let entry = |mode: &str, kind: &str, path: &str| {
            format!("{mode} {kind} {oid}     120\t{path}\0").into_bytes()
        };
        assert_eq!(
            parse_manifest_entry(&entry("100644", "blob", "stellar.yaml")),
            Ok(Some((oid.clone(), 120)))
        );
        assert_eq!(parse_manifest_entry(b""), Ok(None));
        for bad in [
            entry("120000", "blob", "stellar.yaml"),
            entry("160000", "commit", "stellar.yaml"),
            entry("100644", "blob", "other.yaml"),
            [
                entry("100644", "blob", "stellar.yaml"),
                entry("100644", "blob", "stellar.yaml"),
            ]
            .concat(),
            b"garbage\0".to_vec(),
        ] {
            assert_eq!(
                parse_manifest_entry(&bad),
                Err(ManifestSourceError::InvalidManifest)
            );
        }
    }

    #[test]
    fn diamond_and_cycle_through_real_stellar_yaml_parser() {
        use super::text_resolver::{
            branch, entry_text, library_text, resolve_root, root_text, tag, TableSelectors,
            TextManifests,
        };
        use crate::lock::fixtures::commit;

        const A: &str = "https://e.com/a.git";
        const B: &str = "https://e.com/b.git";
        const C: &str = "https://e.com/c.git";

        // XV.2.2 diamond: root → a, b; a → c(tag v1); b → c(branch main). 두 선택자가 같은
        // commit으로 해석되므로 c는 node 하나(LibraryId+commit), edge(경로)는 둘.
        let mut selectors = TableSelectors::default()
            .with(A, tag("v1"), commit(0xa1))
            .with(B, tag("v1"), commit(0xb1))
            .with(C, tag("v1"), commit(0xc1))
            .with(C, branch("main"), commit(0xc1));
        let mut manifests = TextManifests::default()
            .with(
                A,
                commit(0xa1),
                library_text(0x0a, 1, &entry_text("c", 0x0c, C, " v1")),
            )
            .with(
                B,
                commit(0xb1),
                library_text(0x0b, 1, &entry_text("c", 0x0c, C, "\n      branch: main")),
            )
            .with(C, commit(0xc1), library_text(0x0c, 1, ""));
        let root = root_text(&(entry_text("a", 0x0a, A, " v1") + &entry_text("b", 0x0b, B, " v1")));
        let graph = resolve_root(&root, &mut selectors, &mut manifests).expect("diamond");
        assert_eq!(graph.node_count(), 3);
        let c = DependencyNodeKey::new(crate::manifest::fixtures::library(0x0c), commit(0xc1));
        let mut paths: Vec<String> = graph.edges_to(&c).map(|e| e.path.to_string()).collect();
        paths.sort();
        assert_eq!(paths, ["a > c", "b > c"]);
        assert!(graph.duplicate_library_versions().is_empty());

        // XV.2.3 cycle: root → a; a → b; b → a. 전체 경로와 함께 실패.
        let mut selectors = TableSelectors::default()
            .with(A, tag("v1"), commit(0xa1))
            .with(B, tag("v1"), commit(0xb1));
        let mut manifests = TextManifests::default()
            .with(
                A,
                commit(0xa1),
                library_text(0x0a, 1, &entry_text("b", 0x0b, B, " v1")),
            )
            .with(
                B,
                commit(0xb1),
                library_text(0x0b, 1, &entry_text("a", 0x0a, A, " v1")),
            );
        let root = root_text(&entry_text("a", 0x0a, A, " v1"));
        match resolve_root(&root, &mut selectors, &mut manifests) {
            Err(ResolveError::Cycle { path, node }) => {
                assert_eq!(path.to_string(), "a > b > a");
                assert_eq!(
                    node,
                    DependencyNodeKey::new(crate::manifest::fixtures::library(0x0a), commit(0xa1))
                );
            }
            other => panic!("순환 오류 기대: {other:?}"),
        }
    }
}
