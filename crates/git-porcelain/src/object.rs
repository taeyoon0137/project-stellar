//! Git object ID·file mode 값 타입 (XIV.2.1~).
//!
//! Git 출력의 object ID와 file mode를 **검증된 값**으로만 다룹니다. object ID는 SHA-1(40자) 또는
//! SHA-256(64자) 소문자 16진수 문자열이고, file mode는 Git이 index·tree에 기록하는 8진수 값입니다.
//! 형식이 다르면 값 생성 자체가 실패하므로, 파싱된 결과에 잘못된 ID·mode가 섞이지 않습니다.

/// 검증된 Git object ID(소문자 16진수 40자 또는 64자).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(String);

impl ObjectId {
    /// 16진수 바이트에서 object ID를 만듭니다. 길이 40·64가 아니거나 소문자 16진수가 아니면 `None`.
    pub fn from_hex_bytes(raw: &[u8]) -> Option<ObjectId> {
        if !matches!(raw.len(), 40 | 64) {
            return None;
        }
        if !raw
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        {
            return None;
        }
        // 위에서 ASCII만 허용했으므로 UTF-8 변환은 실패하지 않습니다.
        String::from_utf8(raw.to_vec()).ok().map(ObjectId)
    }

    /// 16진수 문자열에서 object ID를 만듭니다.
    pub fn parse(hex: &str) -> Option<ObjectId> {
        ObjectId::from_hex_bytes(hex.as_bytes())
    }

    /// 16진수 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 모든 자리가 0인 ID(Git이 "없음"을 나타낼 때 씀)인지.
    pub fn is_null(&self) -> bool {
        self.0.bytes().all(|b| b == b'0')
    }

    /// SHA-256 저장소의 ID(64자)인지.
    pub fn is_sha256(&self) -> bool {
        self.0.len() == 64
    }
}

impl std::fmt::Display for ObjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Git file mode(8진수).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileMode(u32);

/// file mode가 나타내는 항목 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileModeKind {
    /// 항목 없음(`000000`).
    Absent,
    /// 일반 파일(`100644`, 구버전 group-writable `100664` 포함).
    Regular,
    /// 실행 파일(`100755`).
    Executable,
    /// symbolic link(`120000`).
    Symlink,
    /// submodule commit(gitlink, `160000`).
    Gitlink,
    /// tree(`040000`).
    Tree,
}

impl FileMode {
    /// 항목 없음.
    pub const ABSENT: FileMode = FileMode(0);
    /// 일반 파일.
    pub const REGULAR: FileMode = FileMode(0o100644);
    /// 실행 파일.
    pub const EXECUTABLE: FileMode = FileMode(0o100755);
    /// symbolic link.
    pub const SYMLINK: FileMode = FileMode(0o120000);
    /// gitlink(submodule).
    pub const GITLINK: FileMode = FileMode(0o160000);
    /// tree.
    pub const TREE: FileMode = FileMode(0o040000);

    /// 8진수 숫자 바이트에서 mode를 만듭니다. Git이 쓰는 mode가 아니면 `None`.
    pub fn from_octal_bytes(raw: &[u8]) -> Option<FileMode> {
        if raw.is_empty() || raw.len() > 6 || !raw.iter().all(|b| (b'0'..=b'7').contains(b)) {
            return None;
        }
        let value = raw
            .iter()
            .fold(0u32, |acc, b| acc * 8 + u32::from(*b - b'0'));
        let mode = FileMode(value);
        mode.kind().map(|_| mode)
    }

    /// 8진수 문자열에서 mode를 만듭니다.
    pub fn parse(octal: &str) -> Option<FileMode> {
        FileMode::from_octal_bytes(octal.as_bytes())
    }

    /// 숫자 값.
    pub fn bits(self) -> u32 {
        self.0
    }

    /// 항목 종류(Git이 쓰지 않는 값이면 `None`).
    pub fn kind(self) -> Option<FileModeKind> {
        match self.0 {
            0 => Some(FileModeKind::Absent),
            0o100644 | 0o100664 => Some(FileModeKind::Regular),
            0o100755 => Some(FileModeKind::Executable),
            0o120000 => Some(FileModeKind::Symlink),
            0o160000 => Some(FileModeKind::Gitlink),
            0o040000 => Some(FileModeKind::Tree),
            _ => None,
        }
    }

    /// Git 출력 형식의 6자리 8진수 문자열.
    pub fn to_octal_string(self) -> String {
        format!("{:06o}", self.0)
    }
}

impl std::fmt::Display for FileMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:06o}", self.0)
    }
}

/// Git object 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    /// blob.
    Blob,
    /// tree.
    Tree,
    /// commit.
    Commit,
    /// annotated tag.
    Tag,
}

impl ObjectKind {
    /// Git 출력의 종류 이름에서 만듭니다.
    pub fn from_name(raw: &[u8]) -> Option<ObjectKind> {
        match raw {
            b"blob" => Some(ObjectKind::Blob),
            b"tree" => Some(ObjectKind::Tree),
            b"commit" => Some(ObjectKind::Commit),
            b"tag" => Some(ObjectKind::Tag),
            _ => None,
        }
    }

    /// Git 종류 이름.
    pub fn as_str(self) -> &'static str {
        match self {
            ObjectKind::Blob => "blob",
            ObjectKind::Tree => "tree",
            ObjectKind::Commit => "commit",
            ObjectKind::Tag => "tag",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_ids_accept_sha1_and_sha256_lowercase_hex_only() {
        let sha1 = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
        let id = ObjectId::parse(sha1).unwrap();
        assert_eq!(id.as_str(), sha1);
        assert!(!id.is_sha256());
        assert!(!id.is_null());
        let sha256 = "a".repeat(64);
        assert!(ObjectId::parse(&sha256).unwrap().is_sha256());
        assert!(ObjectId::parse(&"0".repeat(40)).unwrap().is_null());
        for bad in [
            "",
            "abc",
            &"g".repeat(40),
            &"A".repeat(40),
            &"a".repeat(41),
            &"a".repeat(63),
        ] {
            assert_eq!(ObjectId::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn file_modes_parse_known_git_modes() {
        assert_eq!(
            FileMode::parse("100644").unwrap().kind(),
            Some(FileModeKind::Regular)
        );
        assert_eq!(
            FileMode::parse("100755").unwrap().kind(),
            Some(FileModeKind::Executable)
        );
        assert_eq!(
            FileMode::parse("120000").unwrap().kind(),
            Some(FileModeKind::Symlink)
        );
        assert_eq!(
            FileMode::parse("160000").unwrap().kind(),
            Some(FileModeKind::Gitlink)
        );
        assert_eq!(FileMode::parse("040000"), Some(FileMode::TREE));
        assert_eq!(FileMode::parse("000000"), Some(FileMode::ABSENT));
        assert_eq!(FileMode::REGULAR.to_octal_string(), "100644");
        assert_eq!(FileMode::ABSENT.to_string(), "000000");
        for bad in ["", "100645", "1006440", "10064x", "8", "777777"] {
            assert_eq!(FileMode::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn object_kinds_round_trip_names() {
        for kind in [
            ObjectKind::Blob,
            ObjectKind::Tree,
            ObjectKind::Commit,
            ObjectKind::Tag,
        ] {
            assert_eq!(ObjectKind::from_name(kind.as_str().as_bytes()), Some(kind));
        }
        assert_eq!(ObjectKind::from_name(b"missing"), None);
    }
}
