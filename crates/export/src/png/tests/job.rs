//! worker 실행 테스트 (XX.2.3).

use super::*;

#[test]
fn worker_job_renders_encodes_and_reports_gamut() {
    let capture = capture_of(
        fx::basic_roots(&fx::Basic::default()),
        request(2.0, ExportBackground::Transparent),
    );
    let export = ConfirmedExport::assume_confirmed(capture);
    let options = PngOptions::default();
    let job = PngJob::spawn(export.clone(), software(), options).unwrap();
    let file = job.wait().unwrap();
    let direct = render_png(&export, software().as_ref(), options, &CancelToken::new()).unwrap();
    assert_eq!(file, direct, "worker와 호출 thread 결과가 같음");
    assert_eq!((file.width(), file.height()), (200, 160));
    assert!(file.gamut().is_clean());
    let decoded = decode_png(file.bytes(), DecodeLimits::default()).unwrap();
    assert_eq!((decoded.width, decoded.height), (200, 160));
    // P3 빨강을 sRGB로 내보내면 잘린 색으로 보고합니다.
    let config = fx::Basic {
        shape_fill: stellar_core_types::color::Color::new(
            ColorSpace::DisplayP3,
            1.0,
            0.0,
            0.0,
            1.0,
        )
        .unwrap(),
        ..fx::Basic::default()
    };
    let capture = capture_of(
        fx::basic_roots(&config),
        request(1.0, ExportBackground::Transparent),
    );
    let export = ConfirmedExport::assume_confirmed(capture);
    let srgb_file = render_png(&export, software().as_ref(), options, &CancelToken::new()).unwrap();
    assert_eq!(srgb_file.gamut().clipped_colors, 1);
    assert_eq!(srgb_file.gamut().clipped_nodes, vec![fx::node_id(2)]);
    let p3 = PngOptions::new(OutputProfile::DisplayP3, PngBitDepth::Eight).unwrap();
    let p3_file = render_png(&export, software().as_ref(), p3, &CancelToken::new()).unwrap();
    assert!(p3_file.gamut().is_clean());
}

#[test]
fn job_rejects_wrong_format_and_honours_cancel() {
    let svg = capture_of(
        fx::basic_roots(&fx::Basic::default()),
        fx::view_request(ExportFormat::Svg),
    );
    let export = ConfirmedExport::assume_confirmed(svg);
    assert_eq!(
        PngJob::spawn(export, software(), PngOptions::default()).unwrap_err(),
        PngExportError::WrongFormat(ExportFormat::Svg)
    );
    let capture = capture_of(
        fx::basic_roots(&fx::Basic::default()),
        request(4.0, ExportBackground::Transparent),
    );
    let export = ConfirmedExport::assume_confirmed(capture);
    let job = PngJob::spawn(export, software(), PngOptions::default()).unwrap();
    job.cancel();
    assert_eq!(job.wait(), Err(PngExportError::Cancelled));
    // 렌더 오류는 그대로 전달합니다.
    let capture = capture_of(
        fx::basic_roots(&fx::Basic::default()),
        request(1.0, ExportBackground::Transparent),
    );
    let export = ConfirmedExport::assume_confirmed(capture);
    let empty = Arc::new(SoftwareTarget::new(
        Arc::new(Assets::default()),
        Arc::new(Glyphs::default()),
    ));
    let job = PngJob::spawn(export, empty, PngOptions::default()).unwrap();
    assert!(matches!(
        job.wait(),
        Err(PngExportError::Render(OffscreenError::MissingImage { .. }))
    ));
}
