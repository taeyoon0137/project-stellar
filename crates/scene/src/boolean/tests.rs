use super::*;

/// 결과 path golden(연산 이름 → command 줄). `STELLAR_UPDATE_SCENE_GOLDEN=1`이면 다시 씁니다.
const GOLDEN: &str = include_str!("../../tests/fixtures/boolean_v1.golden");
const GOLDEN_PATH: &str = "tests/fixtures/boolean_v1.golden";

fn v(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y).unwrap()
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> PathGeometry {
    PathGeometry::new(vec![
        PathCommand::MoveTo(v(x, y)),
        PathCommand::LineTo(v(x + w, y)),
        PathCommand::LineTo(v(x + w, y + h)),
        PathCommand::LineTo(v(x, y + h)),
        PathCommand::Close,
    ])
    .unwrap()
}

/// 반지름 r 원(cubic 4개, k = 0.5523).
fn circle(cx: f32, cy: f32, r: f32) -> PathGeometry {
    let k = 0.552_284_8 * r;
    PathGeometry::new(vec![
        PathCommand::MoveTo(v(cx + r, cy)),
        PathCommand::CubicTo {
            c1: v(cx + r, cy + k),
            c2: v(cx + k, cy + r),
            to: v(cx, cy + r),
        },
        PathCommand::CubicTo {
            c1: v(cx - k, cy + r),
            c2: v(cx - r, cy + k),
            to: v(cx - r, cy),
        },
        PathCommand::CubicTo {
            c1: v(cx - r, cy - k),
            c2: v(cx - k, cy - r),
            to: v(cx, cy - r),
        },
        PathCommand::CubicTo {
            c1: v(cx + k, cy - r),
            c2: v(cx + r, cy - k),
            to: v(cx + r, cy),
        },
        PathCommand::Close,
    ])
    .unwrap()
}

fn operand(geometry: PathGeometry) -> BooleanOperand {
    BooleanOperand {
        geometry,
        fill_rule: FillRule::NonZero,
        transform: Transform2D::identity(),
    }
}

fn area(op: BooleanOp, operands: &[BooleanOperand]) -> f64 {
    signed_area(&evaluate_boolean(op, operands).unwrap()).abs()
}

fn squares() -> [BooleanOperand; 2] {
    // A = [0,10]², B = [5,15]²: 겹침 25.
    [
        operand(rect(0.0, 0.0, 10.0, 10.0)),
        operand(rect(5.0, 5.0, 10.0, 10.0)),
    ]
}

#[test]
fn four_operations_on_overlapping_squares_match_analytic_area() {
    // 검증 기준(XII.10.30): 4개 연산 analytic fixture 허용 오차.
    let ops = squares();
    assert_eq!(area(BooleanOp::Union, &ops), 175.0);
    assert_eq!(area(BooleanOp::Intersect, &ops), 25.0);
    assert_eq!(area(BooleanOp::Subtract, &ops), 75.0);
    assert_eq!(area(BooleanOp::Exclude, &ops), 150.0);
}

#[test]
fn subtract_uses_the_first_operand_as_base() {
    let [a, b] = squares();
    let ab = evaluate_boolean(BooleanOp::Subtract, &[a.clone(), b.clone()]).unwrap();
    let ba = evaluate_boolean(BooleanOp::Subtract, &[b, a]).unwrap();
    assert_ne!(ab, ba);
    // A − B는 원점 쪽 L자, B − A는 반대쪽 L자.
    assert!(ab.commands().contains(&PathCommand::MoveTo(v(0.0, 0.0))));
    assert!(!ba
        .commands()
        .iter()
        .any(|c| *c == PathCommand::LineTo(v(0.0, 0.0)) || *c == PathCommand::MoveTo(v(0.0, 0.0))));
    // Union·Intersect는 순서와 무관합니다.
    let [a, b] = squares();
    assert_eq!(
        evaluate_boolean(BooleanOp::Union, &[a.clone(), b.clone()]).unwrap(),
        evaluate_boolean(BooleanOp::Union, &[b.clone(), a.clone()]).unwrap()
    );
    assert_eq!(
        evaluate_boolean(BooleanOp::Intersect, &[a.clone(), b.clone()]).unwrap(),
        evaluate_boolean(BooleanOp::Intersect, &[b, a]).unwrap()
    );
}

#[test]
fn circles_match_analytic_area_within_tolerance() {
    // 두 원(r = 10, 중심 거리 10): 교집합 넓이 = 2r²·acos(d/2r) − (d/2)·√(4r² − d²).
    let r = 10.0f64;
    let d = 10.0f64;
    let lens = 2.0 * r * r * (d / (2.0 * r)).acos() - (d / 2.0) * (4.0 * r * r - d * d).sqrt();
    let disc = std::f64::consts::PI * r * r;
    let ops = [
        operand(circle(0.0, 0.0, 10.0)),
        operand(circle(10.0, 0.0, 10.0)),
    ];
    // cubic 원 근사(반지름 오차 약 0.03%)와 선분화(0.05px) 오차를 합쳐 1% 안.
    let close = |got: f64, want: f64| (got - want).abs() / want < 0.01;
    assert!(close(area(BooleanOp::Intersect, &ops), lens));
    assert!(close(area(BooleanOp::Union, &ops), 2.0 * disc - lens));
    assert!(close(area(BooleanOp::Subtract, &ops), disc - lens));
    assert!(close(
        area(BooleanOp::Exclude, &ops),
        2.0 * disc - 2.0 * lens
    ));
}

#[test]
fn same_input_is_bit_stable() {
    // 검증 기준(XII.10.30): 같은 입력 bit-stable 결과.
    let ops = [
        operand(circle(0.0, 0.0, 10.0)),
        operand(rect(3.0, -4.0, 12.0, 8.0)),
        operand(circle(6.0, 6.0, 4.5)),
    ];
    for op in [
        BooleanOp::Union,
        BooleanOp::Subtract,
        BooleanOp::Intersect,
        BooleanOp::Exclude,
    ] {
        let first = evaluate_boolean(op, &ops).unwrap().canonical_bytes();
        for _ in 0..3 {
            assert_eq!(evaluate_boolean(op, &ops).unwrap().canonical_bytes(), first);
        }
    }
}

#[test]
fn operand_transform_moves_geometry_into_result_space() {
    let mut b = operand(rect(0.0, 0.0, 10.0, 10.0));
    b.transform = Transform2D::new(1.0, 0.0, 0.0, 1.0, 5.0, 5.0).unwrap();
    let ops = [operand(rect(0.0, 0.0, 10.0, 10.0)), b];
    assert_eq!(area(BooleanOp::Intersect, &ops), 25.0);
}

#[test]
fn shared_edges_and_disjoint_operands() {
    // 변을 공유하는 두 사각형의 합집합은 사각형 하나입니다.
    let ops = [
        operand(rect(0.0, 0.0, 10.0, 10.0)),
        operand(rect(10.0, 0.0, 10.0, 10.0)),
    ];
    let union = evaluate_boolean(BooleanOp::Union, &ops).unwrap();
    assert_eq!(union.commands().len(), 5, "{union:?}");
    assert_eq!(signed_area(&union).abs(), 200.0);
    assert!(evaluate_boolean(BooleanOp::Intersect, &ops)
        .unwrap()
        .is_empty());
    // 겹치지 않는 교집합은 빈 path.
    let apart = [
        operand(rect(0.0, 0.0, 1.0, 1.0)),
        operand(rect(5.0, 5.0, 1.0, 1.0)),
    ];
    assert!(evaluate_boolean(BooleanOp::Intersect, &apart)
        .unwrap()
        .is_empty());
    assert_eq!(area(BooleanOp::Union, &apart), 2.0);
}

#[test]
fn even_odd_operand_hole_is_respected() {
    // EvenOdd 고리(바깥 10, 안 4)와 가운데 사각형의 교집합은 고리 안쪽 구멍에서 비어 있습니다.
    let ring = PathGeometry::new(
        rect(0.0, 0.0, 10.0, 10.0)
            .commands()
            .iter()
            .chain(rect(3.0, 3.0, 4.0, 4.0).commands())
            .copied()
            .collect(),
    )
    .unwrap();
    let ops = [
        BooleanOperand {
            geometry: ring,
            fill_rule: FillRule::EvenOdd,
            transform: Transform2D::identity(),
        },
        operand(rect(2.0, 2.0, 6.0, 6.0)),
    ];
    assert_eq!(area(BooleanOp::Intersect, &ops), 36.0 - 16.0);
}

#[test]
fn rejects_out_of_range_and_empty_input() {
    assert_eq!(
        evaluate_boolean(BooleanOp::Union, &[]),
        Err(BooleanError::NoOperands)
    );
    let far = operand(rect(40_000.0, 0.0, 1.0, 1.0));
    assert_eq!(
        evaluate_boolean(BooleanOp::Union, &[far]),
        Err(BooleanError::OutOfRange { operand: 0 })
    );
}

fn describe(geometry: &PathGeometry) -> String {
    let mut out = String::new();
    for command in geometry.commands() {
        let line = match command {
            PathCommand::MoveTo(p) => format!("M {} {}", p.x(), p.y()),
            PathCommand::LineTo(p) => format!("L {} {}", p.x(), p.y()),
            PathCommand::CubicTo { to, .. } => format!("C {} {}", to.x(), to.y()),
            PathCommand::Close => "Z".to_owned(),
        };
        out.push_str(&line);
        out.push(' ');
    }
    out.trim_end().to_owned()
}

#[test]
fn result_paths_match_golden() {
    // lane 검증: Boolean 결과 Path golden.
    let ops = squares();
    let mut text = String::from("# Boolean 결과 path golden v1. A=[0,10]², B=[5,15]².\n");
    for (name, op) in [
        ("union", BooleanOp::Union),
        ("subtract", BooleanOp::Subtract),
        ("intersect", BooleanOp::Intersect),
        ("exclude", BooleanOp::Exclude),
    ] {
        text.push_str(&format!(
            "{name}: {}\n",
            describe(&evaluate_boolean(op, &ops).unwrap())
        ));
    }
    if std::env::var_os("STELLAR_UPDATE_SCENE_GOLDEN").is_some() {
        std::fs::write(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN_PATH),
            &text,
        )
        .unwrap();
        return;
    }
    assert_eq!(text, GOLDEN);
}
