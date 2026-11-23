//! CSS layout longhand → Compose layout 규칙 (XXII.4.2, ADR-0107 결정 3·4, ADR-0121 결정 1).
//!
//! layout longhand 40개와 local transform의 Compose 규칙은 XXII.1.4 capability matrix의 Android 열입니다(항목 규칙
//! [`crate::matrix::css::CSS_LOWERINGS`], 규칙 문장과 값별 진단 [`crate::matrix::dialect`]). 생성기는 그 규칙대로만
//! 낮추고 1:1이 아닌 값을 [`crate::matrix::dialect::finding`]으로 냅니다. 단위: CSS `px` 1은 `1.dp`입니다.
