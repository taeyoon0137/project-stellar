//! 원시 YAML blob 하나만 선택하여 stage/unstage합니다. 문서 generation은 직렬화하지 않습니다.
use super::git_names::live;
use super::{GitPanelHead, ProjectSummary, SelectionOperation};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use stellar_core_types::ids::StableId;
use stellar_git_porcelain::{
    blob_write::{write_blob, WrittenCandidate, WrittenFileDelta},
    cancel::GitCancelToken,
    decode::GitPath,
    index_publish::{publish_isolated, restore_published},
    object::FileMode,
    runner::argv,
    snapshot::{
        read_index_snapshot, resolve_tree,
        working_tree::{list_working_tree_pathspec, WorkingTreeFileKind},
        IndexSnapshot, ResolvedTree, SnapshotEntry, SnapshotSource, TreeSnapshot,
    },
    trust::GatedGitRunner,
};
use stellar_project::{
    document_io::component_source::{read_project_library, ComponentFiles},
    saved_project::{head::git_component_library, SavedProject},
    yaml::loader::MAX_INPUT_BYTES,
};
use stellar_storage::{hash::ContentHash, source_file::read_resolved};

/// 한 컴포넌트 원본의 표시 정보와 정확히 관측한 비교 기준.
#[derive(Clone, Debug)]
pub struct GitComponentRow {
    /// 정의 ID.
    pub id: StableId,
    /// git root 기준 정규 YAML 파일명.
    pub path: String,
    /// HEAD 원본 SHA-256. 없으면 추가된 파일입니다.
    pub head: Option<ContentHash>,
    /// Index 원본 SHA-256.
    pub index: Option<ContentHash>,
    /// Working 원본 SHA-256.
    pub working: Option<ContentHash>,
    head_ref: Arc<HeadReview>,
    head_entry: Option<SnapshotEntry>,
    index_entry: Option<SnapshotEntry>,
    working_mode: Option<FileMode>,
}

#[derive(Debug)]
pub(super) struct HeadReview {
    tree: ResolvedTree,
    reference: Option<Vec<u8>>,
}

impl HeadReview {
    pub(super) fn current(
        git: &GatedGitRunner,
        root: &Path,
        expected: &ResolvedTree,
    ) -> Result<Self, String> {
        let head = git
            .run(&argv(["symbolic-ref", "-q", "HEAD"]), root)
            .map_err(|e| e.to_string())?;
        let reference = match head.exit_code {
            Some(0) if head.stdout.starts_with(b"refs/") => Some(head.stdout),
            Some(1) if head.stdout.is_empty() => None, // Detached HEAD.
            _ => return Err("현재 HEAD 참조를 읽을 수 없습니다".into()),
        };
        let tree = resolve_tree(git, root, &SnapshotSource::Head).map_err(|e| e.to_string())?;
        if tree != *expected {
            return Err("컴포넌트 표시 이후 HEAD가 바뀌었습니다. 새로고침하세요".into());
        }
        Ok(Self { tree, reference })
    }

    pub(super) fn panel_head(&self, index_files: usize) -> Result<GitPanelHead, String> {
        if let Some(reference) = &self.reference {
            let branch = reference.strip_suffix(b"\n").unwrap_or(reference);
            let branch = branch
                .strip_prefix(b"refs/heads/")
                .ok_or("HEAD가 로컬 브랜치를 가리키지 않습니다")?;
            let branch = String::from_utf8_lossy(branch).into_owned();
            return Ok(if self.tree == ResolvedTree::Unborn {
                GitPanelHead::Unborn {
                    branch,
                    staged_files: u32::try_from(index_files).map_err(|_| "index 파일 수 초과")?,
                }
            } else {
                GitPanelHead::Branch(branch)
            });
        }
        match &self.tree {
            ResolvedTree::Tree {
                commit: Some(id), ..
            } => Ok(GitPanelHead::Detached(id.to_string())),
            _ => Err("분리된 HEAD의 commit을 확인할 수 없습니다".into()),
        }
    }

    fn verify(&self, git: &GatedGitRunner, root: &Path) -> Result<(), String> {
        let current = Self::current(git, root, &self.tree)?;
        if current.reference != self.reference {
            return Err("컴포넌트 표시 이후 HEAD 브랜치가 바뀌었습니다. 새로고침하세요".into());
        }
        Ok(())
    }
}

impl GitComponentRow {
    /// HEAD와 Index가 다르며 추가 또는 수정된 원본인지.
    pub fn staged(&self) -> bool {
        self.index.is_some() && self.head_entry != self.index_entry
    }
    /// Index와 Working이 다르며 추가 또는 수정된 원본인지.
    pub fn unstaged(&self) -> bool {
        self.working.is_some()
            && (self.index != self.working
                || self.index_entry.as_ref().map(|e| e.mode) != self.working_mode)
    }
}

pub(super) fn ensure_text_library(saved: &SavedProject, has_files: bool) -> Result<(), String> {
    if has_files
        && saved
            .generation
            .manifest()
            .chunk_pointers
            .iter()
            .any(|p| p.kind == stellar_storage::chunk::kinds::KIND_COMPONENT_LIBRARY)
    {
        Err("바이너리와 YAML 컴포넌트 라이브러리가 함께 있습니다".into())
    } else {
        Ok(())
    }
}

fn relative_files(
    files: ComponentFiles,
    root: Option<&Path>,
) -> Result<BTreeMap<String, (StableId, ContentHash)>, String> {
    files
        .into_iter()
        .map(|(id, (path, hash))| {
            let path = match root {
                Some(root) => path
                    .strip_prefix(root)
                    .map_err(|_| "프로젝트 밖의 컴포넌트 경로")?,
                None => &path,
            };
            if path.parent() != Some(Path::new("design/components")) {
                return Err("Git의 실제 design/components 경로가 필요합니다".into());
            }
            Ok((
                path.to_str()
                    .ok_or("UTF-8 컴포넌트 경로가 필요합니다")?
                    .to_owned(),
                (id, hash),
            ))
        })
        .collect()
}

fn index_files(
    git: &GatedGitRunner,
    root: &Path,
    entries: &[SnapshotEntry],
) -> Result<BTreeMap<String, (StableId, ContentHash)>, String> {
    let (_, files) = git_component_library(git, root, entries)
        .map_err(|_| "Git 컴포넌트 원본이 잘못되었거나 읽기 상한을 초과했습니다")?;
    relative_files(files, None)
}

fn lookup(entries: &[SnapshotEntry]) -> BTreeMap<&[u8], &SnapshotEntry> {
    entries.iter().map(|e| (e.path.as_bytes(), e)).collect()
}

pub(super) fn rows(
    git: &GatedGitRunner,
    root: &Path,
    head: &TreeSnapshot,
    index: &IndexSnapshot,
    working: &SavedProject,
    head_ref: &Arc<HeadReview>,
) -> Result<(Vec<GitComponentRow>, bool), String> {
    let indexed = index_files(git, root, &index.entries)?;
    let committed = if head.entries == index.entries {
        indexed.clone()
    } else {
        index_files(git, root, &head.entries)?
    };
    let (_, files) = read_project_library(root).map_err(|e| e.to_string())?;
    ensure_text_library(working, !files.is_empty())?;
    let saved = relative_files(files, Some(root))?;
    if committed.is_empty() && indexed.is_empty() && saved.is_empty() {
        return Ok((vec![], false));
    }
    // Respect ignored/untracked files, track executable bits, and reject Git symlinks.
    let listing = list_working_tree_pathspec(git, root, ":(top,literal)design/components")
        .map_err(|e| e.to_string())?;
    let modes: BTreeMap<_, _> = listing
        .files
        .iter()
        .map(|f| (f.path.as_bytes(), f.kind))
        .collect();
    let before = lookup(&head.entries);
    let staged = lookup(&index.entries);
    let paths: BTreeSet<_> = committed
        .keys()
        .chain(indexed.keys())
        .chain(saved.keys())
        .collect();
    // A rename changes two paths together. A file checkbox cannot represent both halves.
    let mut names = BTreeMap::new();
    let mut renamed = BTreeSet::new();
    for (name, (id, _)) in committed.iter().chain(&indexed).chain(&saved) {
        if names
            .insert(id, name)
            .is_some_and(|previous| previous != name)
        {
            renamed.insert(*id);
        }
    }
    let mut other = !renamed.is_empty();
    let mut rows = Vec::new();
    for path in paths {
        let old = committed.get(path);
        let middle = indexed.get(path);
        let file = saved
            .get(path)
            .filter(|_| modes.contains_key(path.as_bytes()));
        let Some((id, _)) = file.or(middle).or(old) else {
            continue;
        };
        if renamed.contains(id) {
            continue;
        }
        let disk_mode = match modes.get(path.as_bytes()) {
            Some(WorkingTreeFileKind::Regular) => Some(FileMode::REGULAR),
            Some(WorkingTreeFileKind::Executable) => Some(FileMode::EXECUTABLE),
            Some(WorkingTreeFileKind::Symlink) => {
                return Err("컴포넌트 Git Stage에는 일반 파일이 필요합니다".into())
            }
            None => None,
        };
        // Assignment to another ID at a colliding file name requires a separate reconciliation.
        if [old, middle, file]
            .iter()
            .flatten()
            .any(|(other, _)| other != id)
        {
            return Err("같은 경로의 컴포넌트 ID가 바뀌었습니다".into());
        }
        let row = GitComponentRow {
            id: *id,
            path: path.clone(),
            head: old.map(|e| e.1),
            index: middle.map(|e| e.1),
            working: file.map(|e| e.1),
            head_ref: Arc::clone(head_ref),
            head_entry: before.get(path.as_bytes()).map(|e| (*e).clone()),
            index_entry: staged.get(path.as_bytes()).map(|e| (*e).clone()),
            working_mode: disk_mode,
        };
        other |= old.is_some() && middle.is_none() || middle.is_some() && file.is_none();
        if row.staged() || row.unstaged() {
            if rows.len() == 64 {
                return Err("표시할 컴포넌트 변경이 64개를 넘습니다".into());
            }
            rows.push(row);
        }
    }
    Ok((rows, other))
}

pub(super) fn apply(
    git: &GatedGitRunner,
    project: &ProjectSummary,
    row: &GitComponentRow,
    operation: SelectionOperation,
    parent: &Path,
    cancel: &GitCancelToken,
) -> Result<(), String> {
    let root = &project.canonical_root;
    let stale = || "표시한 컴포넌트 원본이나 index가 바뀌었습니다. 새로고침하세요".to_owned();
    row.head_ref.verify(git, root)?;
    let index = read_index_snapshot(git, root).map_err(|e| e.to_string())?;
    let path = GitPath::from_bytes(row.path.as_bytes());
    if index.has_conflicts()
        || index.entries.iter().find(|e| e.path == path) != row.index_entry.as_ref()
    {
        return Err(stale());
    }
    let working_bytes = || -> Result<Vec<u8>, String> {
        // Canonical filename from the decoded component, no symlink ancestors or final links.
        let local: PathBuf = root.join(&row.path);
        if local.canonicalize().map_err(|_| stale())? != local {
            return Err(stale());
        }
        let bytes = read_resolved(&local, MAX_INPUT_BYTES as u64).map_err(|_| stale())?;
        if Some(ContentHash::compute(&bytes)) != row.working {
            return Err(stale());
        }
        Ok(bytes)
    };
    live(cancel)?;
    project.verify_trust()?;
    let target = match operation {
        SelectionOperation::Stage if row.unstaged() => {
            // A formerly untracked file may have become ignored; working mode may also have changed.
            let selected_path = format!(":(top,literal){}", row.path);
            let listing =
                list_working_tree_pathspec(git, root, &selected_path).map_err(|e| e.to_string())?;
            let mode = match listing
                .files
                .iter()
                .find(|f| f.path == path)
                .map(|f| f.kind)
            {
                Some(WorkingTreeFileKind::Regular) => FileMode::REGULAR,
                Some(WorkingTreeFileKind::Executable) => FileMode::EXECUTABLE,
                _ => return Err(stale()),
            };
            if Some(mode) != row.working_mode {
                return Err(stale());
            }
            let bytes = working_bytes()?;
            let oid = write_blob(git, root, &bytes).map_err(|e| e.to_string())?;
            Some(SnapshotEntry {
                path: path.clone(),
                mode,
                oid,
            })
        }
        SelectionOperation::Unstage if row.staged() => row.head_entry.clone(),
        _ => return Err(stale()),
    };
    let mut candidate_entries = index.entries.clone();
    candidate_entries.retain(|e| e.path != path);
    candidate_entries.extend(target.iter().cloned());
    // Validate the complete resulting library, including ID/filename collisions across sibling files.
    index_files(git, root, &candidate_entries)?;
    let (_files, _saved) =
        super::git_names::stored_project(git, project, &candidate_entries, parent)?;
    let change = match target {
        Some(entry) => WrittenFileDelta::Upsert {
            path,
            mode: entry.mode,
            oid: entry.oid,
        },
        None => WrittenFileDelta::Remove { path },
    };
    let candidate = WrittenCandidate::single_file(index, change);
    row.head_ref.verify(git, root)?;
    if operation == SelectionOperation::Stage {
        working_bytes()?;
    }
    live(cancel)?;
    project.verify_trust()?;
    let receipt =
        publish_isolated(git, root, parent, &candidate, cancel).map_err(|e| e.to_string())?;
    if let Err(error) = row.head_ref.verify(git, root) {
        restore_published(git, receipt, cancel)
            .map_err(|e| format!("{error}. index 조건부 복원 실패: {e}"))?;
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ProjectSessionId;
    use stellar_core_types::value::Size2;
    use stellar_document::{
        frame::CornerRadius,
        model::DocumentModel,
        records::NodePayload,
        shape::{RectangleParams, ShapeGeometry},
    };
    use stellar_git_porcelain::{fixture::GitFixture, snapshot::read_tree_snapshot};
    use stellar_ops::test_support::{project_id, save_documents, scaffold_project, text_document};
    use stellar_project::{
        document_io::component_source::{self, ComponentSource},
        saved_project::SavedProjectLoader,
        trust::TrustStatus,
    };

    fn mixed_document() -> GitFixture {
        let fx = GitFixture::with_system_git("git-panel-source")
            .unwrap()
            .expect("system Git");
        let root = fx.work().canonicalize().unwrap();
        scaffold_project(&root);
        let mut records = text_document().to_records().unwrap();
        let parents: BTreeSet<_> = records.nodes.iter().filter_map(|n| n.parent).collect();
        let leaf = records
            .nodes
            .iter_mut()
            .find(|n| !parents.contains(&n.id) && matches!(n.payload, NodePayload::Fragment(_)))
            .unwrap();
        leaf.payload = NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
            size: Size2::new(15.0, 10.0).unwrap(),
            corner_radius: CornerRadius::ZERO,
        }));
        let (model, _) = DocumentModel::from_records(records).unwrap();
        save_documents(&root, &[&model]);
        fx.commit_all("document with text and rectangle").unwrap();
        fx
    }

    #[test]
    fn shape_and_text_document_exposes_component_rows_and_rejects_checkout_or_replaced_index() {
        let fx = mixed_document();
        let root = fx.work().canonicalize().unwrap();
        fx.git(["switch", "-c", "external"]).unwrap();
        fx.git([
            "commit",
            "--allow-empty",
            "-qm",
            "same tree, different commit",
        ])
        .unwrap();
        fx.git(["switch", "-"]).unwrap();
        let sources = root.join("design/components");
        std::fs::create_dir_all(&sources).unwrap();
        let component = ComponentSource::new(
            StableId::new_v7(900, 0, 1),
            "Button".into(),
            StableId::new_v7(900, 0, 2),
        )
        .unwrap();
        let (file, hash) = component_source::create(&sources, &component).unwrap();
        let git = fx.gated();
        let saved = SavedProjectLoader::new().open(&root).unwrap();
        let read = || {
            let tree = read_tree_snapshot(&git, &root, &SnapshotSource::Head).unwrap();
            let index = read_index_snapshot(&git, &root).unwrap();
            let review = Arc::new(HeadReview::current(&git, &root, &tree.resolved).unwrap());
            let (mut items, other) = rows(&git, &root, &tree, &index, &saved, &review).unwrap();
            assert!(!other);
            assert_eq!(items.len(), 1);
            items.pop().unwrap()
        };
        let project = ProjectSummary {
            project: ProjectSessionId(1),
            project_id: project_id(),
            name: "Fixture".into(),
            root: root.clone(),
            canonical_root: root.clone(),
            trust: TrustStatus::Trusted,
        };
        let shown = read();
        assert_eq!(shown.working, Some(hash));
        assert_eq!(shown.head, None);
        let bytes = std::fs::read(&file).unwrap();
        let parent = fx.root();
        fx.git(["switch", "-c", "same-commit"]).unwrap();
        let before = std::fs::read(root.join(".git/index")).unwrap();
        let err = apply(
            &git,
            &project,
            &shown,
            SelectionOperation::Stage,
            parent,
            &GitCancelToken::new(),
        )
        .unwrap_err();
        assert!(err.contains("브랜치"), "{err}");
        assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), before);
        let shown = read();
        // Checkout can refresh stat bytes by itself. Record the exact index after checkout.
        fx.git(["switch", "external"]).unwrap();
        let before = std::fs::read(root.join(".git/index")).unwrap();
        let err = apply(
            &git,
            &project,
            &shown,
            SelectionOperation::Stage,
            parent,
            &GitCancelToken::new(),
        )
        .unwrap_err();
        assert!(err.contains("HEAD"), "{err}");
        assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), before);
        let fresh = read();
        fx.git(["add", "--", &fresh.path]).unwrap();
        let replaced = std::fs::read(root.join(".git/index")).unwrap();
        let err = apply(
            &git,
            &project,
            &fresh,
            SelectionOperation::Stage,
            parent,
            &GitCancelToken::new(),
        )
        .unwrap_err();
        assert!(err.contains("index"), "{err}");
        assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), replaced);
        assert_eq!(std::fs::read(&file).unwrap(), bytes);

        fx.commit_all("tracked component").unwrap();
        std::fs::remove_file(file).unwrap();
        let moved = ComponentSource::new(
            component.definition().id,
            "Renamed".into(),
            component.root().id,
        )
        .unwrap();
        component_source::create(&sources, &moved).unwrap();
        let tree = read_tree_snapshot(&git, &root, &SnapshotSource::Head).unwrap();
        let index = read_index_snapshot(&git, &root).unwrap();
        let review = Arc::new(HeadReview::current(&git, &root, &tree.resolved).unwrap());
        let (items, other) = rows(&git, &root, &tree, &index, &saved, &review).unwrap();
        assert!(
            other && items.is_empty(),
            "rename requires a multi-path selection"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires a built Stellar Debug executable in STELLAR_APP_PATH"]
    fn native_checkbox_stages_component_in_document_with_shape_and_text() {
        use std::process::Command;
        use stellar_project::{identity::ProjectIdentity, trust_store::FileTrustStore};

        let app = std::env::var_os("STELLAR_APP_PATH").expect("STELLAR_APP_PATH");
        let fx = mixed_document();
        let root = fx.work().canonicalize().unwrap();
        let sources = root.join("design/components");
        std::fs::create_dir_all(&sources).unwrap();
        let component = ComponentSource::new(
            StableId::new_v7(902, 0, 1),
            "Card".into(),
            StableId::new_v7(902, 0, 2),
        )
        .unwrap();
        let (file, _) = component_source::create(&sources, &component).unwrap();
        let home = fx.root().join("app-home");
        let data = home
            .join("Library/Application Support")
            .join(stellar_ops::lock::APP_DATA_DIR);
        let identity = ProjectIdentity::new(project_id(), &root).unwrap();
        FileTrustStore::open(&data)
            .unwrap()
            .grant(&identity)
            .unwrap();
        let saved = fx.root().join("staged.index");
        let before = read_index_snapshot(&fx.gated(), &root).unwrap();
        let working = std::fs::read(&file).unwrap();
        let output = Command::new(app)
            .arg("--git-stage-check")
            .arg(&root)
            .arg(&saved)
            .arg(file.file_name().unwrap())
            .env("HOME", &home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_ALLOW_PROTOCOL", "file")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let installed = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args([
                "show",
                &format!(":{}", file.strip_prefix(&root).unwrap().display()),
            ])
            .env("GIT_INDEX_FILE", saved)
            .output()
            .unwrap();
        assert!(installed.status.success());
        assert_eq!(installed.stdout, working);
        assert_eq!(std::fs::read(file).unwrap(), working);
        assert_eq!(read_index_snapshot(&fx.gated(), &root).unwrap(), before);
    }
}
