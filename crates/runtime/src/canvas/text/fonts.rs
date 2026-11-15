//! canvas Text가 쓰는 글꼴 face를 document thread 밖에서 읽습니다(ADR-0067 §5).
//!
//! - 기본 chain: 문서가 family를 정하지 않았거나(CSS initial) 정한 face를 하나도 쓸 수 없을 때 쓰는 플랫폼 글꼴
//!   목록입니다. 프로세스에서 한 번만 읽고 문서에는 쓰지 않습니다.
//! - project face: 문서 style 표에 등록된 project 글꼴입니다. 처음 필요할 때 loader thread에 요청하고, 읽는 동안에는
//!   기본 chain으로 대체해 그립니다. 다 읽으면 `notify`가 창 scene을 다시 게시합니다.
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex, OnceLock, PoisonError, Weak},
};
use stellar_core_types::font::{FontFaceRef, FontFileHash, FontSource};
use stellar_document::text_styles::DocumentTextStyles;
use stellar_text::{registry::FontFaceKey, shaping::FontFace};

/// macOS 기본 chain(PostScript 이름, 순서대로 시도). Latin, 한글, 일본어 가나·한자를 덮습니다.
#[cfg(target_os = "macos")]
const DEFAULT_CHAIN: [&str; 3] = ["Helvetica", "AppleGothic", "HiraginoSans-W3"];

/// 기본 chain 파일 하나의 상한. 이보다 큰 파일은 건너뜁니다.
const MAX_DEFAULT_FILE_BYTES: u64 = 64 * 1024 * 1024;

static DEFAULT_FACES: OnceLock<Vec<(FontFaceRef, FontFace)>> = OnceLock::new();

/// project 루트, face, 그 face를 등록한 글꼴·asset record. record가 바뀌면 다시 검증해 읽습니다.
type Key = (PathBuf, FontFaceKey, Vec<u8>, Vec<u8>);

struct Job {
    key: Key,
    styles: DocumentTextStyles,
    face: FontFaceRef,
}

#[derive(Default)]
struct State {
    loaded: HashMap<Key, Option<FontFace>>,
    pending: HashSet<Key>,
}

/// 창들이 함께 쓰는 face loader. `inline`은 호출한 thread에서 바로 읽습니다(테스트 전용).
pub(crate) struct FaceLoader {
    jobs: Option<Mutex<mpsc::Sender<Job>>>,
    state: Mutex<State>,
}

impl std::fmt::Debug for FaceLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FaceLoader")
            .field("background", &self.jobs.is_some())
            .finish_non_exhaustive()
    }
}

impl FaceLoader {
    /// loader thread를 시작합니다. 먼저 기본 chain을 읽고, 이후 project face 요청을 처리합니다. 새 face를 읽을
    /// 때마다 `notify`를 부릅니다. thread를 만들 수 없으면 face를 읽지 않는 loader를 돌려줍니다.
    pub(crate) fn spawn(notify: impl Fn() + Send + 'static) -> Arc<FaceLoader> {
        let (tx, rx) = mpsc::channel::<Job>();
        let loader = Arc::new(FaceLoader {
            jobs: Some(Mutex::new(tx)),
            state: Mutex::default(),
        });
        let weak: Weak<FaceLoader> = Arc::downgrade(&loader);
        let spawned = std::thread::Builder::new()
            .name("stellar-font-loader".into())
            .spawn(move || {
                default_faces();
                notify();
                while let Ok(job) = rx.recv() {
                    let face = load_project(&job);
                    let Some(loader) = weak.upgrade() else { return };
                    let mut state = loader.state.lock().unwrap_or_else(PoisonError::into_inner);
                    state.pending.remove(&job.key);
                    state.loaded.insert(job.key, face);
                    drop(state);
                    drop(loader);
                    notify();
                }
            });
        if spawned.is_err() {
            return Arc::new(FaceLoader {
                jobs: None,
                state: Mutex::default(),
            });
        }
        loader
    }

    /// 요청한 thread에서 바로 읽는 loader(테스트 전용).
    #[cfg(test)]
    pub(crate) fn inline() -> Arc<FaceLoader> {
        Arc::new(FaceLoader {
            jobs: None,
            state: Mutex::default(),
        })
    }

    fn background(&self) -> bool {
        self.jobs.is_some()
    }

    /// 읽은 기본 chain. background loader가 아직 읽는 중이면 `None`입니다.
    pub(crate) fn defaults(&self) -> Option<&'static [(FontFaceRef, FontFace)]> {
        if self.background() {
            DEFAULT_FACES.get().map(Vec::as_slice)
        } else {
            Some(default_faces())
        }
    }

    /// style 표에 등록된 project face. 처음이면 loader에 요청하고 `None`을 돌려줍니다. 읽기에 실패한 face도
    /// `None`입니다.
    pub(crate) fn project_face(
        &self,
        root: &Path,
        styles: &DocumentTextStyles,
        face: &FontFaceRef,
    ) -> Option<FontFace> {
        let entry = styles.fonts.get(&face.file_hash())?;
        let key = (
            root.to_owned(),
            FontFaceKey::from(face),
            entry.font_record.clone(),
            entry.asset_record.clone(),
        );
        let job = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(loaded) = state.loaded.get(&key) {
                return loaded.clone();
            }
            if !state.pending.insert(key.clone()) {
                return None;
            }
            // 필요한 글꼴 record 하나만 넘깁니다. 같은 hash면 bytes도 같습니다.
            let mut subset = DocumentTextStyles::default();
            subset.fonts.insert(face.file_hash(), entry.clone());
            Job {
                key,
                styles: subset,
                face: face.clone(),
            }
        };
        match &self.jobs {
            Some(jobs) => {
                let sent = jobs
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .send(job);
                if let Err(mpsc::SendError(job)) = sent {
                    let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                    state.pending.remove(&job.key);
                    state.loaded.insert(job.key, None);
                }
                None
            }
            None => {
                let face = load_project(&job);
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.pending.remove(&job.key);
                state.loaded.insert(job.key, face.clone());
                face
            }
        }
    }
}

fn load_project(job: &Job) -> Option<FontFace> {
    stellar_project::fonts::document::load_document_face(&job.key.0, &job.styles, &job.face).ok()
}

/// 기본 chain을 한 번만 읽습니다. 다른 thread가 읽는 중이면 끝날 때까지 기다립니다.
fn default_faces() -> &'static [(FontFaceRef, FontFace)] {
    DEFAULT_FACES.get_or_init(load_default_faces)
}

#[cfg(target_os = "macos")]
fn load_default_faces() -> Vec<(FontFaceRef, FontFace)> {
    let locator = stellar_project::fonts::system_macos::CoreTextFontLocator::new();
    DEFAULT_CHAIN
        .iter()
        .filter_map(|name| {
            let path = locator.locate_postscript_name(name)?;
            load_system_face(&path, name)
        })
        .collect()
}

#[cfg(not(target_os = "macos"))]
fn load_default_faces() -> Vec<(FontFaceRef, FontFace)> {
    Vec::new()
}

/// 파일을 한 번 읽어 hash·face index·shaping face를 같은 bytes에서 구합니다.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn load_system_face(path: &Path, postscript: &str) -> Option<(FontFaceRef, FontFace)> {
    if std::fs::metadata(path).ok()?.len() > MAX_DEFAULT_FILE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let info = stellar_project::fonts::sfnt::read_font_file(
        &mut std::io::Cursor::new(&bytes),
        bytes.len() as u64,
    )
    .ok()?;
    let index = info
        .faces
        .iter()
        .position(|f| f.names.postscript() == Some(postscript))?;
    let index = u32::try_from(index).ok()?;
    let hash = stellar_storage::hash::ContentHash::compute(&bytes);
    let reference = FontFaceRef::new(
        FontSource::System,
        FontFileHash::from_sha256_digest(*hash.digest()),
        index,
        Vec::new(),
    );
    let face = FontFace::from_bytes(bytes, index).ok()?;
    Some((reference, face))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn default_chain_loads_known_system_faces_once() {
        let loader = FaceLoader::inline();
        let faces = loader.defaults().unwrap();
        assert!(!faces.is_empty(), "macOS 기본 chain 글꼴이 하나도 없습니다");
        assert!(faces
            .iter()
            .all(|(r, f)| r.source() == FontSource::System && f.face_index() == r.face_index()));
        assert!(std::ptr::eq(faces, loader.defaults().unwrap()));
    }

    #[test]
    fn background_loader_reports_pending_then_loaded_project_faces() {
        use crate::window_session::test_fixtures::{sid, TempDir};
        use stellar_core_types::color::{Color, ColorSpace};
        let project = TempDir::new("font-loader");
        let black = Color::new(ColorSpace::Srgb, 0., 0., 0., 1.).unwrap();
        let (styles, face) =
            super::super::tests::project_font_styles(project.path(), [sid(1), sid(2)], 1024, black);
        let (tx, rx) = mpsc::channel();
        let loader = FaceLoader::spawn(move || {
            let _ = tx.send(());
        });
        // 기본 chain 완료 알림.
        rx.recv().unwrap();
        assert!(loader.defaults().is_some());
        assert!(loader
            .project_face(project.path(), &styles, &face)
            .is_none());
        assert!(loader
            .project_face(project.path(), &styles, &face)
            .is_none());
        rx.recv().unwrap();
        assert!(loader
            .project_face(project.path(), &styles, &face)
            .is_some());
        // 등록되지 않은 face는 요청하지 않습니다.
        let other = FontFaceRef::new(
            FontSource::Project,
            FontFileHash::from_sha256_digest([9; 32]),
            0,
            vec![],
        );
        assert!(loader
            .project_face(project.path(), &styles, &other)
            .is_none());
    }
}
