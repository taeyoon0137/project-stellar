//! 파일 이름 slug(ADR-0077 결정 1).
//!
//! 파일 이름은 표시 이름에서 만든 slug이고 정체성은 파일 안의 StableId입니다. slug는 ASCII 소문자·숫자·`-`만
//! 씁니다. 파일 시스템과 Git 사이의 Unicode 정규화(NFC·NFD) 차이로 같은 이름이 두 경로가 되는 일을 막기 위해
//! 비ASCII 문자는 쓰지 않습니다. 이름에서 slug가 비면 종류 이름을 씁니다. 같은 폴더에서 slug가 겹치면 겹친
//! 항목 모두에 StableId SHA-256 앞 12자리를 붙입니다. 결과는 모델만의 함수라 같은 모델은 같은 경로가 됩니다.

use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;
use stellar_storage::hash::ContentHash;

/// slug 최대 길이(문자).
pub(crate) const MAX_SLUG_CHARS: usize = 64;

/// 표시 이름의 slug. 비면 `fallback`입니다.
pub(crate) fn slug(name: &str, fallback: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            let separator = dash && !out.is_empty();
            if out.len() + 1 + usize::from(separator) > MAX_SLUG_CHARS {
                break;
            }
            if separator {
                out.push('-');
            }
            dash = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            dash = true;
        }
    }
    if out.is_empty() {
        fallback.to_owned()
    } else {
        out
    }
}

/// StableId 접미사(SHA-256 앞 12자리 hex).
pub(crate) fn id_suffix(id: StableId) -> String {
    ContentHash::compute(&id.to_bytes()).digest_hex()[..12].to_owned()
}

/// 같은 폴더 항목들의 고유 slug. 겹친 slug는 모두 `-<id 접미사>`를 붙입니다.
pub(crate) fn unique_slugs<'a>(
    entries: impl IntoIterator<Item = (StableId, &'a str)>,
    fallback: &str,
) -> BTreeMap<StableId, String> {
    let base: Vec<(StableId, String)> = entries
        .into_iter()
        .map(|(id, name)| (id, slug(name, fallback)))
        .collect();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (_, slug) in &base {
        *counts.entry(slug.as_str()).or_default() += 1;
    }
    base.iter()
        .map(|(id, slug)| {
            let unique = if counts[slug.as_str()] > 1 {
                format!("{slug}-{}", id_suffix(*id))
            } else {
                slug.clone()
            };
            (*id, unique)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_ascii_and_collisions_get_id_suffixes() {
        assert_eq!(slug("Home Screen", "view"), "home-screen");
        assert_eq!(slug("  --Card__v2!! ", "c"), "card-v2");
        assert_eq!(slug("홈 화면", "view"), "view");
        assert_eq!(slug("홈 Home", "view"), "home");
        assert_eq!(slug(&"a".repeat(200), "x").len(), MAX_SLUG_CHARS);
        let a = StableId::new_v7(1, 0, 1);
        let b = StableId::new_v7(2, 0, 2);
        let c = StableId::new_v7(3, 0, 3);
        let slugs = unique_slugs([(a, "Card"), (b, "card"), (c, "Button")], "component");
        assert_eq!(slugs[&c], "button");
        assert_eq!(slugs[&a], format!("card-{}", id_suffix(a)));
        assert_eq!(slugs[&b], format!("card-{}", id_suffix(b)));
        assert_ne!(slugs[&a], slugs[&b]);
    }
}
