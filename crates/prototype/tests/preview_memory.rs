//! 프리뷰 상태는 세션 메모리에만 (XXX.3.6).

mod preview_support;
mod support;

use preview_support::*;
use stellar_prototype::session::SessionConfig;
use support::*;

#[test]
fn preview_never_writes_the_document() {
    // 검증 기준(XXX.3.6): 프리뷰 조작 후 save bytes 동일. 입력 로그·진행 중 motion은 세션 메모리에만 있습니다.
    let doc = document();
    let before = doc.state_bytes();
    {
        let mut s = session(&doc, SessionConfig::default());
        for inputs in script() {
            s.advance(inputs);
        }
        assert!(!s.input_log().is_empty());
    }
    assert_eq!(doc.state_bytes(), before);
}
