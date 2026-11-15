//! 큰 목록 선형성 검사: adapter가 형제 수(목록 항목 수)에 제곱으로 늘지 않는지 봅니다.
//!
//! 항목마다 부모 entry의 하위 범위를 다시 훑으면(자식 목록·listbox 여부를 항목마다 계산) 항목 수의 제곱으로
//! 늘어 상한을 크게 넘습니다. 상한은 debug build와 동시 빌드 부하를 고려한 여유 값이고 성능 목표가 아닙니다.

use std::time::{Duration, Instant};

use crate::apple::{export_apple, AppleDialect, AppleOptions};
use crate::fixture as fx;
use crate::web::aria::map_accessibility;
use crate::web::check::{check_against_ir, check_document};

/// 목록 항목 수.
const ITEMS: u32 = 100_000;

/// 단계 하나의 시간 상한.
const LIMIT: Duration = Duration::from_secs(30);

fn timed<T>(name: &str, run: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let value = run();
    let elapsed = started.elapsed();
    assert!(elapsed < LIMIT, "{name}: {elapsed:?} (상한 {LIMIT:?})");
    value
}

#[test]
fn adapters_stay_linear_on_large_lists() {
    let fixture = fx::list_fixture(ITEMS);
    let ios = timed("IR", || fixture.ir(&fx::ios_profile()));
    assert_eq!(ios.accessibility.len(), ITEMS as usize + 1);

    let web = timed("Web 매핑", || map_accessibility(&ios));
    let options = web
        .document
        .elements()
        .iter()
        .filter(|element| element.attr("role") == Some("option"))
        .count();
    assert_eq!(options, ITEMS as usize);
    timed("Web 정적 검사", || {
        assert_eq!(check_document(&web.document), Vec::new());
        assert_eq!(check_against_ir(&ios, &web.document), Vec::new());
    });

    let swiftui = timed("SwiftUI", || {
        export_apple(&ios, &AppleOptions::new(AppleDialect::SwiftUi)).expect("export")
    });
    assert_eq!(
        swiftui.source.matches(": ViewModifier {").count(),
        ITEMS as usize + 1
    );
}
