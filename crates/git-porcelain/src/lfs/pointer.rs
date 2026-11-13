//! LFS pointer만 있고 object가 없는 asset 진단 (XIII.7.5).
//!
//! Git LFS가 smudge하지 못한 파일(object를 받지 않았거나 LFS가 없음)은 작업 트리에 실제 내용 대신 LFS
//! pointer 텍스트가 남습니다. Stellar는 이 상태를 일반 "파일 없음"과 다른 진단으로 구분해, 문서는 열되
//! 누락 asset으로 표시할 수 있게 합니다(아키텍처 29.2).
//!
//! pointer 형식(Git LFS spec v1): `version <url>` 줄이 먼저 오고, 그 뒤 key가 사전순인 `key SP value LF`
//! 줄들(`ext-*`, `oid sha256:<64 hex>`, `size <10진수>`)이 옵니다. 마지막은 LF로 끝나며 전체는
//! [`MAX_POINTER_SIZE`] byte 미만입니다. 형식이 하나라도 다르면 pointer가 아닙니다(일반 파일로 판정).
//!
//! 로컬 LFS 저장소에 object가 있는지는 확인하지 않습니다. Stellar가 LFS 저장 경로를 추측하지 않으며,
//! 작업 트리에 pointer만 있으면 어느 경우든 asset 내용을 쓸 수 없기 때문입니다(`git lfs pull`
//! 또는 `git lfs checkout`으로 해결).

use std::path::Path;

use crate::decode::GitPath;
use crate::snapshot::working_tree::{checked_local_path, read_regular_no_follow};

/// pointer 파일 최대 크기(이 값 이상이면 pointer가 아님).
pub const MAX_POINTER_SIZE: u64 = 1024;

/// 허용 version URL(현행·구 이름).
const VERSIONS: [&[u8]; 2] = [
    b"https://git-lfs.github.com/spec/v1",
    b"https://hawser.github.com/spec/v1",
];

/// 해석한 LFS pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfsPointer {
    /// object ID(`sha256:` 뒤 소문자 16진수 64자).
    pub oid: String,
    /// 실제 content 크기(byte).
    pub size: u64,
}

fn valid_key(key: &[u8]) -> bool {
    !key.is_empty()
        && key
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'.' || *b == b'-')
}

/// 바이트가 LFS pointer이면 해석합니다(엄격: 형식이 하나라도 다르면 `None`).
pub fn parse_lfs_pointer(bytes: &[u8]) -> Option<LfsPointer> {
    if bytes.len() as u64 >= MAX_POINTER_SIZE {
        return None;
    }
    let body = bytes.strip_suffix(b"\n")?;
    let mut lines = body.split(|b| *b == b'\n');
    let version = lines.next()?.strip_prefix(b"version ")?;
    if !VERSIONS.contains(&version) {
        return None;
    }
    let mut previous: Option<&[u8]> = None;
    let (mut oid, mut size) = (None, None);
    for line in lines {
        let space = line.iter().position(|b| *b == b' ')?;
        let (key, value) = (&line[..space], &line[space + 1..]);
        if !valid_key(key) || value.is_empty() || value.contains(&b'\r') {
            return None;
        }
        // version 뒤 key는 엄격한 사전순이며 중복 없음.
        if previous.is_some_and(|p| p >= key) {
            return None;
        }
        previous = Some(key);
        match key {
            b"oid" => {
                let hex = value.strip_prefix(b"sha256:")?;
                if hex.len() != 64
                    || !hex
                        .iter()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
                {
                    return None;
                }
                oid = Some(String::from_utf8(hex.to_vec()).ok()?);
            }
            b"size" => {
                if !value.iter().all(u8::is_ascii_digit) || (value.len() > 1 && value[0] == b'0') {
                    return None;
                }
                size = Some(std::str::from_utf8(value).ok()?.parse().ok()?);
            }
            other if other.starts_with(b"ext-") => {}
            _ => return None,
        }
    }
    Some(LfsPointer {
        oid: oid?,
        size: size?,
    })
}

/// asset 파일 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetFileState {
    /// 실제 내용이 있음.
    Present {
        /// 파일 크기.
        len: u64,
    },
    /// 파일이 없음(일반 missing file).
    Missing,
    /// LFS pointer만 있고 object 내용이 없음.
    LfsObjectMissing {
        /// 작업 트리에 남은 pointer.
        pointer: LfsPointer,
    },
    /// 읽을 수 없음(일반 파일이 아님·상위 symlink·권한 등).
    Unreadable {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
}

impl AssetFileState {
    /// 일반 missing file 진단 코드.
    pub const MISSING_CODE: &'static str = "asset-file-missing";
    /// missing LFS object 진단 코드(일반 missing과 다름).
    pub const LFS_OBJECT_MISSING_CODE: &'static str = "asset-lfs-object-missing";
    /// 읽기 실패 진단 코드.
    pub const UNREADABLE_CODE: &'static str = "asset-file-unreadable";

    /// 진단 코드(정상이면 `None`).
    pub fn diagnostic_code(&self) -> Option<&'static str> {
        match self {
            AssetFileState::Present { .. } => None,
            AssetFileState::Missing => Some(Self::MISSING_CODE),
            AssetFileState::LfsObjectMissing { .. } => Some(Self::LFS_OBJECT_MISSING_CODE),
            AssetFileState::Unreadable { .. } => Some(Self::UNREADABLE_CODE),
        }
    }
}

/// `root` 아래 asset 파일 상태를 확인합니다(symlink를 따르지 않음).
///
/// `expected_size`(AssetRecord의 content 크기)가 파일 크기와 같으면 내용이 pointer처럼 보여도 실제
/// 내용으로 봅니다. [`MAX_POINTER_SIZE`] 이상인 파일은 읽지 않고 실제 내용으로 봅니다.
pub fn inspect_asset_file(
    root: &Path,
    path: &GitPath,
    expected_size: Option<u64>,
) -> AssetFileState {
    let local = match checked_local_path(root, path) {
        Ok(local) => local,
        Err(std::io::ErrorKind::NotFound) => return AssetFileState::Missing,
        Err(kind) => return AssetFileState::Unreadable { kind },
    };
    let meta = match std::fs::symlink_metadata(&local) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return AssetFileState::Missing,
        Err(e) => return AssetFileState::Unreadable { kind: e.kind() },
    };
    if !meta.is_file() {
        return AssetFileState::Unreadable {
            kind: std::io::ErrorKind::InvalidInput,
        };
    }
    let len = meta.len();
    if len >= MAX_POINTER_SIZE || expected_size == Some(len) {
        return AssetFileState::Present { len };
    }
    match read_regular_no_follow(&local, MAX_POINTER_SIZE) {
        Ok(bytes) => match parse_lfs_pointer(&bytes) {
            Some(pointer) => AssetFileState::LfsObjectMissing { pointer },
            None => AssetFileState::Present {
                len: bytes.len() as u64,
            },
        },
        Err(std::io::ErrorKind::NotFound) => AssetFileState::Missing,
        Err(kind) => AssetFileState::Unreadable { kind },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OID: &str = "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393";

    fn pointer_text(size: u64) -> String {
        format!("version https://git-lfs.github.com/spec/v1\noid sha256:{OID}\nsize {size}\n")
    }

    #[test]
    fn parses_spec_pointers() {
        assert_eq!(
            parse_lfs_pointer(pointer_text(12345).as_bytes()),
            Some(LfsPointer {
                oid: OID.into(),
                size: 12345
            })
        );
        let legacy =
            format!("version https://hawser.github.com/spec/v1\noid sha256:{OID}\nsize 0\n");
        assert_eq!(parse_lfs_pointer(legacy.as_bytes()).unwrap().size, 0);
        let with_ext = format!(
            "version https://git-lfs.github.com/spec/v1\next-0-foo sha256:{OID}\noid sha256:{OID}\nsize 7\n"
        );
        assert_eq!(parse_lfs_pointer(with_ext.as_bytes()).unwrap().size, 7);
    }

    #[test]
    fn non_pointers_are_rejected() {
        let v = "version https://git-lfs.github.com/spec/v1";
        let bad = [
            String::new(),
            "hello\n".into(),
            pointer_text(1).trim_end().to_string(),
            format!("{v}\nsize 1\noid sha256:{OID}\n"),
            format!("{v}\noid sha256:{OID}\n"),
            format!("{v}\nsize 1\n"),
            format!("version https://example.com/spec/v1\noid sha256:{OID}\nsize 1\n"),
            format!("{v}\noid sha256:{}\nsize 1\n", OID.to_uppercase()),
            format!("{v}\noid sha1:{}\nsize 1\n", &OID[..40]),
            format!("{v}\noid sha256:{}\nsize 1\n", &OID[..63]),
            format!("{v}\noid sha256:{OID}\nsize 01\n"),
            format!("{v}\noid sha256:{OID}\nsize -1\n"),
            format!("{v}\noid sha256:{OID}\nsize 18446744073709551616\n"),
            format!("{v}\noid sha256:{OID}\nsize 1\nunknown x\n"),
            format!("{v}\noid sha256:{OID}\noid sha256:{OID}\nsize 1\n"),
            format!("{v}\r\noid sha256:{OID}\r\nsize 1\r\n"),
            format!("{v}\noid sha256:{OID}\nsize 1\n\n"),
            format!("{v}\noid  sha256:{OID}\nsize 1\n"),
            format!("{}\n{v}\noid sha256:{OID}\nsize 1\n", "x"),
        ];
        for text in bad {
            assert_eq!(parse_lfs_pointer(text.as_bytes()), None, "{text:?}");
        }
        // 크기 상한: 1024 byte 이상은 pointer가 아님.
        let mut big = pointer_text(1).into_bytes();
        big.splice(
            big.len() - 1..big.len() - 1,
            std::iter::repeat_n(b'1', 1024),
        );
        assert_eq!(parse_lfs_pointer(&big), None);
        assert_eq!(parse_lfs_pointer(&[0xff, 0xfe, b'\n']), None);
    }

    #[test]
    fn missing_lfs_object_has_a_different_code_than_missing_file() {
        let dir = crate::test_support::TempDir::new("lfs-pointer");
        let root = dir.path();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("assets/pointer.bin"), pointer_text(5000)).unwrap();
        std::fs::write(root.join("assets/real.bin"), b"real content").unwrap();
        let p = |s: &str| GitPath::from_bytes(s.as_bytes());

        let missing = inspect_asset_file(root, &p("assets/gone.bin"), None);
        let missing_dir = inspect_asset_file(root, &p("nodir/gone.bin"), None);
        let lfs = inspect_asset_file(root, &p("assets/pointer.bin"), Some(5000));
        let real = inspect_asset_file(root, &p("assets/real.bin"), Some(12));
        assert_eq!(missing, AssetFileState::Missing);
        assert_eq!(missing_dir, AssetFileState::Missing);
        assert_eq!(
            lfs,
            AssetFileState::LfsObjectMissing {
                pointer: LfsPointer {
                    oid: OID.into(),
                    size: 5000
                }
            }
        );
        assert_eq!(real, AssetFileState::Present { len: 12 });
        assert_eq!(missing.diagnostic_code(), Some("asset-file-missing"));
        assert_eq!(lfs.diagnostic_code(), Some("asset-lfs-object-missing"));
        assert_ne!(missing.diagnostic_code(), lfs.diagnostic_code());
        assert_eq!(real.diagnostic_code(), None);

        // 기대 크기가 파일 크기와 같으면 pointer처럼 보여도 실제 내용.
        let len = pointer_text(5000).len() as u64;
        assert_eq!(
            inspect_asset_file(root, &p("assets/pointer.bin"), Some(len)),
            AssetFileState::Present { len }
        );
        // 잘못된 경로는 읽지 않음.
        assert_eq!(
            inspect_asset_file(root, &p("../x"), None).diagnostic_code(),
            Some("asset-file-unreadable")
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_followed() {
        let dir = crate::test_support::TempDir::new("lfs-pointer-link");
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let outside = dir.path().join("outside.bin");
        std::fs::write(&outside, pointer_text(1)).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link.bin")).unwrap();
        std::os::unix::fs::symlink(dir.path(), root.join("up")).unwrap();
        let p = |s: &str| GitPath::from_bytes(s.as_bytes());
        assert_eq!(
            inspect_asset_file(&root, &p("link.bin"), None),
            AssetFileState::Unreadable {
                kind: std::io::ErrorKind::InvalidInput
            }
        );
        assert_eq!(
            inspect_asset_file(&root, &p("up/outside.bin"), None),
            AssetFileState::Unreadable {
                kind: std::io::ErrorKind::InvalidInput
            }
        );
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::lfs::probe_lfs;
        use crate::test_support::{
            git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use std::ffi::OsString;

        #[test]
        fn clone_without_lfs_objects_is_missing_lfs_object() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let runner = hermetic_runner(git);
            if !probe_lfs(&runner).is_ok_and(|s| s.is_installed()) {
                return; // git-lfs 미설치 환경 건너뜀.
            }
            let dir = TempDir::new("lfs-pointer-clone");
            let origin = dir.path().join("origin");
            std::fs::create_dir_all(origin.join("assets")).unwrap();
            init_repo_with_identity(&runner, &origin);
            // 저장소 local LFS filter 설치 후 .bin을 LFS로 커밋(clean filter가 pointer로 저장).
            git_ok(&runner, &origin, ["lfs", "install", "--local"]);
            git_ok(&runner, &origin, ["lfs", "track", "--", "*.bin"]);
            let content = vec![7u8; 3000];
            std::fs::write(origin.join("assets/a.bin"), &content).unwrap();
            git_ok(
                &runner,
                &origin,
                ["add", "--", ".gitattributes", "assets/a.bin"],
            );
            git_ok(&runner, &origin, ["commit", "-q", "-m", "lfs asset"]);
            let blob = git_ok(&runner, &origin, ["cat-file", "-p", "HEAD:assets/a.bin"]).stdout;
            let committed = parse_lfs_pointer(&blob).expect("커밋된 blob은 LFS pointer");
            assert_eq!(committed.size, 3000);

            // LFS filter·object 없이 clone(전역 config 없음, smudge 생략) → 작업 트리에 pointer만.
            let clone = dir.path().join("clone");
            let skip = runner.clone().with_added_env(vec![(
                OsString::from("GIT_LFS_SKIP_SMUDGE"),
                OsString::from("1"),
            )]);
            git_ok(
                &skip,
                dir.path(),
                [
                    "clone",
                    "-q",
                    "--",
                    origin.to_str().unwrap(),
                    clone.to_str().unwrap(),
                ],
            );
            let p = |s: &str| GitPath::from_bytes(s.as_bytes());
            let state = inspect_asset_file(&clone, &p("assets/a.bin"), Some(3000));
            assert_eq!(
                state,
                AssetFileState::LfsObjectMissing { pointer: committed }
            );
            assert_eq!(state.diagnostic_code(), Some("asset-lfs-object-missing"));
            // 원본 작업 트리에는 실제 내용이 있음.
            assert_eq!(
                inspect_asset_file(&origin, &p("assets/a.bin"), Some(3000)),
                AssetFileState::Present { len: 3000 }
            );
            // 지운 파일은 일반 missing.
            std::fs::remove_file(clone.join("assets/a.bin")).unwrap();
            assert_eq!(
                inspect_asset_file(&clone, &p("assets/a.bin"), Some(3000)).diagnostic_code(),
                Some("asset-file-missing")
            );
        }
    }
}
