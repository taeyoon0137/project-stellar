//! journal manifest 텍스트 형식(ADR-0078, Git 밖 로컬 형식).
//!
//! ```text
//! stellar-journal 1
//! tx <거래 ID>
//! write <기대 hash|none> <기대 길이> <새 hash> <새 길이> <상대 경로>
//! delete <기대 hash> <기대 길이> none 0 <상대 경로>
//! ```
//!
//! 줄 순서가 항목 번호입니다(`orig/<번호>`, `new/<번호>`). 경로는 마지막 필드라 공백을 담을 수 있고,
//! 줄바꿈·제어 문자는 경로 규칙([`super::path::components`])이 거부합니다. 복구는 이 파일을 신뢰하지 않고
//! 다시 검사합니다.

use std::collections::HashSet;

use crate::hash::ContentHash;

use super::path;

/// 형식 첫 줄.
const MAGIC: &str = "stellar-journal 1";

/// 항목 수 상한.
pub(crate) const ENTRY_LIMIT: usize = 65_536;

/// 거래 ID 길이 상한.
const TXID_LIMIT: usize = 64;

/// 대상 하나의 내용 요약(hash와 길이).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Content {
    pub(crate) hash: ContentHash,
    pub(crate) len: u64,
}

impl Content {
    /// bytes의 요약.
    pub(crate) fn of(bytes: &[u8]) -> Content {
        Content {
            hash: ContentHash::compute(bytes),
            len: bytes.len() as u64,
        }
    }
}

/// 항목 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    /// 새 bytes로 만들거나 교체합니다.
    Write,
    /// 지웁니다.
    Delete,
}

/// 대상 하나. `Write`는 `new`가 항상 있고, `Delete`는 `expected`가 항상 있고 `new`가 없습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) op: Op,
    pub(crate) path: String,
    /// 앞 세대 내용. `None`은 앞 세대에 파일이 없음입니다.
    pub(crate) expected: Option<Content>,
    /// 새 세대 내용. `None`은 새 세대에 파일이 없음입니다.
    pub(crate) new: Option<Content>,
}

/// 거래 하나의 manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Manifest {
    pub(crate) txid: String,
    pub(crate) entries: Vec<Entry>,
}

/// 거래 ID 문자 규칙(소문자 16진수와 `-`).
pub(crate) fn valid_txid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= TXID_LIMIT
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
}

fn field(content: Option<Content>) -> (String, u64) {
    match content {
        Some(c) => (c.hash.to_string(), c.len),
        None => ("none".to_string(), 0),
    }
}

/// manifest bytes(결정론적).
pub(crate) fn encode(manifest: &Manifest) -> Vec<u8> {
    let mut out = format!("{MAGIC}\ntx {}\n", manifest.txid);
    for entry in &manifest.entries {
        let op = match entry.op {
            Op::Write => "write",
            Op::Delete => "delete",
        };
        let (expected, expected_len) = field(entry.expected);
        let (new, new_len) = field(entry.new);
        out.push_str(&format!(
            "{op} {expected} {expected_len} {new} {new_len} {}\n",
            entry.path
        ));
    }
    out.into_bytes()
}

fn parse_content(hash: &str, len: &str) -> Result<Option<Content>, &'static str> {
    let len: u64 = len.parse().map_err(|_| "길이가 숫자가 아닙니다")?;
    if hash == "none" {
        return if len == 0 {
            Ok(None)
        } else {
            Err("없는 내용의 길이가 0이 아닙니다")
        };
    }
    let hash: ContentHash = hash.parse().map_err(|_| "hash 형식이 다릅니다")?;
    Ok(Some(Content { hash, len }))
}

/// manifest bytes를 읽습니다. 형식·불변식이 하나라도 어긋나면 거부합니다.
pub(crate) fn decode(bytes: &[u8]) -> Result<Manifest, &'static str> {
    let text = std::str::from_utf8(bytes).map_err(|_| "UTF-8이 아닙니다")?;
    let body = text.strip_suffix('\n').ok_or("마지막 줄바꿈이 없습니다")?;
    let mut lines = body.split('\n');
    if lines.next() != Some(MAGIC) {
        return Err("형식 머리가 다릅니다");
    }
    let txid = lines
        .next()
        .and_then(|line| line.strip_prefix("tx "))
        .ok_or("거래 ID 줄이 없습니다")?;
    if !valid_txid(txid) {
        return Err("거래 ID 형식이 다릅니다");
    }
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for line in lines {
        if entries.len() == ENTRY_LIMIT {
            return Err("항목이 너무 많습니다");
        }
        let fields: Vec<&str> = line.splitn(6, ' ').collect();
        let [op, expected, expected_len, new, new_len, path] = fields[..] else {
            return Err("항목 필드 수가 다릅니다");
        };
        let op = match op {
            "write" => Op::Write,
            "delete" => Op::Delete,
            _ => return Err("항목 종류가 다릅니다"),
        };
        let expected = parse_content(expected, expected_len)?;
        let new = parse_content(new, new_len)?;
        match op {
            Op::Write if new.is_none() => return Err("write 항목에 새 내용이 없습니다"),
            Op::Delete if expected.is_none() || new.is_some() => {
                return Err("delete 항목 내용이 다릅니다")
            }
            _ => {}
        }
        path::components(path)?;
        if !seen.insert(path::identity_key(path)) {
            return Err("같은 대상이 두 번 있습니다");
        }
        entries.push(Entry {
            op,
            path: path.to_string(),
            expected,
            new,
        });
    }
    Ok(Manifest {
        txid: txid.to_string(),
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        Manifest {
            txid: "0123abcd-00000001-00000002".to_string(),
            entries: vec![
                Entry {
                    op: Op::Write,
                    path: "design/views/home page.yaml".to_string(),
                    expected: Some(Content::of(b"old")),
                    new: Some(Content::of(b"new bytes")),
                },
                Entry {
                    op: Op::Write,
                    path: "design/new.yaml".to_string(),
                    expected: None,
                    new: Some(Content::of(b"")),
                },
                Entry {
                    op: Op::Delete,
                    path: "design/old.yaml".to_string(),
                    expected: Some(Content::of(b"gone")),
                    new: None,
                },
            ],
        }
    }

    #[test]
    fn round_trips_deterministically() {
        let manifest = sample();
        let bytes = encode(&manifest);
        assert_eq!(decode(&bytes).unwrap(), manifest);
        assert_eq!(encode(&decode(&bytes).unwrap()), bytes);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("stellar-journal 1\ntx 0123abcd-00000001-00000002\nwrite sha256:"));
        assert!(text.ends_with(" none 0 design/old.yaml\n"));
    }

    #[test]
    fn refuses_tampered_manifests() {
        let good = String::from_utf8(encode(&sample())).unwrap();
        let cases = [
            good.trim_end().to_string(),
            good.replacen("stellar-journal 1", "stellar-journal 2", 1),
            good.replacen("tx 0123", "tx 0123/../", 1),
            good.replacen("design/old.yaml", "../outside", 1),
            good.replacen("design/old.yaml", ".git/hooks/post-checkout", 1),
            good.replacen("design/new.yaml", "design/views/home page.yaml", 1),
            good.replacen("delete sha256", "delete none 0 none 0 x\nwrite sha256", 1),
            good.replacen("write", "move", 1),
            good.replacen(" 3 sha256", " three sha256", 1),
            good.replacen("none 0 design/old.yaml", "none 4 design/old.yaml", 1),
            format!("{good}write none 0 none 0 x\n"),
            format!("{good}\n"),
        ];
        for case in cases {
            assert!(decode(case.as_bytes()).is_err(), "{case:?}");
        }
        assert!(decode(&[0xff, b'\n']).is_err());
    }
}
