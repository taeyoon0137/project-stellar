//! 출력 디코더 fixture와 golden (XXIX.2.9, XXIX.2.11).
//!
//! - launch profile(decoder profile)마다 `tests/fixtures/decoder/<decoder id>/<case>.jsonl` 원문과
//!   `<case>.golden` 기대 결과가 있습니다. fixture 목록 자체도 `INDEX` golden으로 고정합니다.
//! - 원문을 한 번에, 1 byte씩, 여러 크기 조각으로 나눠 넣어도 결과가 golden과 같아야 합니다(입력 경계, 결정론).
//! - `fail-*` fixture는 실패 판정이 정확히 하나입니다.
//! - fixture에는 XVI.8 금지 정보(계정·메일·토큰·사용자 경로)가 없습니다.
//!
//! golden 갱신: `STELLAR_BLESS_DECODER_GOLDEN=1 cargo test -p stellar-agent-host --test decoder_fixtures`
//! (갱신 결과는 사람이 검토합니다).

use std::path::{Path, PathBuf};

use stellar_agent_host::decoder::{CliSignal, DecodeFailure, Decoder, DecoderKind};
use stellar_agent_host::profile::{official_profiles, LaunchProfile};

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/decoder")
}

fn bless() -> bool {
    std::env::var_os("STELLAR_BLESS_DECODER_GOLDEN").is_some()
}

fn cases(kind: DecoderKind) -> Vec<(String, Vec<u8>)> {
    let dir = fixtures_root().join(kind.id());
    let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .map(|p| {
            (
                p.file_stem().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&p).unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

fn render(
    items: &[stellar_agent_host::chat::AgentChatItem],
    signals: &[CliSignal],
    failure: Option<DecodeFailure>,
) -> String {
    let mut text = String::new();
    for item in items {
        text.push_str(&item.to_golden());
        text.push('\n');
    }
    for signal in signals {
        text.push_str(match signal {
            CliSignal::UsageLimit => "signal usage-limit\n",
            CliSignal::Authentication => "signal authentication\n",
        });
    }
    match failure {
        Some(f) => text.push_str(&format!("failure {} line {}\n", f.code, f.line)),
        None => text.push_str("ok\n"),
    }
    text
}

fn decode_chunked(kind: DecoderKind, raw: &[u8], chunk: usize) -> String {
    let mut decoder = Decoder::new(kind);
    let mut items = Vec::new();
    let mut signals = Vec::new();
    let mut failure = None;
    for piece in raw.chunks(chunk.max(1)) {
        let out = decoder.feed(piece);
        items.extend(out.items);
        signals.extend(out.signals);
        failure = failure.or(out.failure);
    }
    let out = decoder.finish();
    items.extend(out.items);
    signals.extend(out.signals);
    failure = failure.or(out.failure);
    render(&items, &signals, failure)
}

#[test]
fn every_launch_profile_has_a_decoder_fixture_set() {
    let mut profiles: Vec<LaunchProfile> = official_profiles().unwrap();
    for kind in ["claude", "codex"] {
        let path = fixtures_root().join(format!("../profiles/test-double-{kind}.json"));
        profiles.push(LaunchProfile::parse(&std::fs::read_to_string(path).unwrap()).unwrap());
    }
    for profile in profiles {
        let kind = DecoderKind::resolve(&profile.decoder).unwrap();
        let cases = cases(kind);
        assert!(cases.iter().any(|(n, _)| n == "normal"), "{}", profile.id);
        assert!(
            cases.iter().any(|(n, _)| n.starts_with("fail-")),
            "{}",
            profile.id
        );
    }
}

#[test]
fn fixture_index_is_golden() {
    let mut index = String::new();
    for kind in DecoderKind::ALL {
        for (name, raw) in cases(kind) {
            let status = match stellar_agent_host::decoder::decode_all(kind, &raw).2 {
                Some(f) => format!("failure:{}", f.code),
                None => "ok".to_string(),
            };
            index.push_str(&format!("{}/{name} {status}\n", kind.id()));
            assert!(
                bless()
                    || fixtures_root()
                        .join(kind.id())
                        .join(format!("{name}.golden"))
                        .exists(),
                "기대 결과 파일 없음: {}/{name}",
                kind.id()
            );
        }
    }
    let path = fixtures_root().join("INDEX");
    if bless() {
        std::fs::write(&path, &index).unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), index);
}

#[test]
fn decode_matches_golden_for_any_input_split() {
    for kind in DecoderKind::ALL {
        for (name, raw) in cases(kind) {
            let whole = decode_chunked(kind, &raw, usize::MAX);
            let golden_path = fixtures_root()
                .join(kind.id())
                .join(format!("{name}.golden"));
            if bless() {
                std::fs::write(&golden_path, &whole).unwrap();
            }
            let golden = std::fs::read_to_string(&golden_path)
                .unwrap_or_else(|_| panic!("golden 없음: {}", golden_path.display()));
            assert_eq!(whole, golden, "{}/{name}", kind.id());
            for chunk in [1, 2, 3, 5, 7, 13, 64, 1000] {
                assert_eq!(
                    decode_chunked(kind, &raw, chunk),
                    golden,
                    "{}/{name} chunk {chunk}",
                    kind.id()
                );
            }
            // 반복 실행 결과 동일.
            assert_eq!(decode_chunked(kind, &raw, 4), decode_chunked(kind, &raw, 4));
        }
    }
}

#[test]
fn unparseable_fixtures_fail_exactly_once() {
    for kind in DecoderKind::ALL {
        for (name, raw) in cases(kind) {
            let golden = decode_chunked(kind, &raw, 3);
            let failures = golden.lines().filter(|l| l.starts_with("failure ")).count();
            if name.starts_with("fail-") {
                assert_eq!(failures, 1, "{}/{name}", kind.id());
                // 실패 뒤에는 item을 내지 않습니다(닫힘).
                assert!(golden.trim_end().ends_with(golden.lines().last().unwrap()));
            } else {
                assert_eq!(failures, 0, "{}/{name}", kind.id());
            }
        }
    }
}

#[test]
fn fixture_privacy_scan_is_clean() {
    // XVI.8 금지 정보: 계정 식별(메일), 자격 증명·토큰, 사용자 홈 경로.
    let banned = [
        "@",
        "/Users/",
        "/home/",
        "sk-",
        "Bearer ",
        "api_key",
        "apiKey\":\"",
        "password",
    ];
    let mut hits = Vec::new();
    for entry in walk(&fixtures_root().join("..")) {
        let text = String::from_utf8_lossy(&std::fs::read(&entry).unwrap()).into_owned();
        for word in banned {
            if text.contains(word) {
                hits.push(format!("{} {word}", entry.display()));
            }
        }
    }
    assert!(hits.is_empty(), "{hits:?}");
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}
