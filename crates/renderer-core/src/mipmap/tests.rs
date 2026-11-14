//! mipmap 생성·texture upload 테스트 (XI.7.2).

use super::*;
use std::thread;

use crate::backend::mock::MockBackend;
use crate::backend::tests::device_config;
use crate::backend::{BackendCall, CheckedBackend};
use crate::digest::fnv1a64;
use crate::thread_role::{bind_thread_role, ThreadRole};

/// worker thread에서 `f`를 실행합니다.
fn on_worker<T: Send + 'static>(f: impl FnOnce(&DecodeWorkerScope) -> T + Send + 'static) -> T {
    thread::spawn(move || {
        let _role = bind_thread_role(ThreadRole::Worker).unwrap();
        let scope = DecodeWorkerScope::current().unwrap();
        f(&scope)
    })
    .join()
    .unwrap()
}

fn solid(width: u32, height: u32, rgba: [u8; 4]) -> DecodedImage {
    let pixels = rgba.repeat((width * height) as usize);
    DecodedImage::new(width, height, ColorSpace::Srgb, pixels).unwrap()
}

#[test]
fn expected_mip_count_by_size() {
    // 검증 기준(XI.7.2): 크기별 expected mip count.
    let table: [((u32, u32), u32); 12] = [
        ((1, 1), 1),
        ((2, 2), 2),
        ((2, 1), 2),
        ((3, 5), 3),
        ((4, 4), 3),
        ((17, 9), 5),
        ((256, 1), 9),
        ((255, 255), 8),
        ((1000, 1), 10),
        ((1024, 768), 11),
        ((1025, 3), 11),
        ((16_384, 16_384), 15),
    ];
    for ((w, h), count) in table {
        assert_eq!(mip_level_count(w, h), count, "{w}x{h}");
        // 마지막 level은 1×1이고 그 전 level은 1×1이 아닙니다.
        assert_eq!(mip_level_size(w, h, count - 1), (1, 1), "{w}x{h}");
        if count > 1 {
            assert_ne!(mip_level_size(w, h, count - 2), (1, 1), "{w}x{h}");
        }
    }
    assert_eq!(mip_level_count(0, 0), 0);
    assert_eq!(mip_level_size(3, 5, 1), (1, 2));
    assert_eq!(mip_level_size(17, 9, 2), (4, 2));
    assert_eq!(mip_level_size(7, 7, 40), (1, 1));

    // 실제 생성한 chain의 level 수·크기도 같습니다(작은 크기만 생성).
    let sizes: Vec<((u32, u32), u32)> = table
        .iter()
        .copied()
        .filter(|((w, h), _)| w * h <= 1024 * 768)
        .collect();
    let chains = on_worker(move |scope| {
        sizes
            .iter()
            .map(|((w, h), count)| {
                let image = solid(*w, *h, [10, 20, 30, 255]);
                let full = generate_mip_chain(&image, MipmapMode::Full, scope);
                let none = generate_mip_chain(&image, MipmapMode::None, scope);
                ((*w, *h), *count, full, none)
            })
            .collect::<Vec<_>>()
    });
    for ((w, h), count, full, none) in chains {
        assert_eq!(full.level_count(), count, "{w}x{h}");
        for (index, level) in full.levels().iter().enumerate() {
            assert_eq!(
                (level.width(), level.height()),
                mip_level_size(w, h, index as u32)
            );
            assert_eq!(
                level.pixels().len(),
                (level.width() * level.height() * 4) as usize
            );
        }
        assert_eq!(none.level_count(), 1);
        assert_eq!((none.width(), none.height()), (w, h));
    }
}

#[test]
fn transfer_table_matches_core_curve_and_round_trips() {
    for code in 0..=255u8 {
        let table = f32::from(DECODE_U16[code as usize]) / 65535.0;
        let reference = ColorSpace::Srgb.decode_channel(f32::from(code) / 255.0);
        assert!(
            (table - reference).abs() <= 1.0 / 65535.0 + 1e-5,
            "code {code}: {table} vs {reference}"
        );
        assert_eq!(encode_u16(DECODE_U16[code as usize]), code);
    }
    assert!(DECODE_U16.windows(2).all(|pair| pair[0] < pair[1]));
    // 선형 0.5 → sRGB 0.7354 → 188.
    assert_eq!(encode_u16(32_768), 188);
    assert_eq!(encode_u16(0), 0);
    assert_eq!(encode_u16(u16::MAX), 255);
}

#[test]
fn filtering_is_premultiplied_linear_box() {
    let results = on_worker(|scope| {
        // 불투명 검정 + 불투명 흰색 → 선형 평균 0.5 → 188(감마 공간 평균 128이 아님).
        let bw = DecodedImage::new(
            2,
            1,
            ColorSpace::Srgb,
            vec![0, 0, 0, 255, 255, 255, 255, 255],
        )
        .unwrap();
        // 불투명 빨강 + 완전 투명(색 쓰레기값 초록) → 초록이 번지지 않습니다.
        let edge = DecodedImage::new(
            2,
            1,
            ColorSpace::DisplayP3,
            vec![255, 0, 0, 255, 0, 255, 0, 0],
        )
        .unwrap();
        // 홀수 3×1: 마지막 pixel까지 3개 평균.
        let odd = DecodedImage::new(
            3,
            1,
            ColorSpace::Srgb,
            vec![255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255],
        )
        .unwrap();
        // 반투명 base level은 premultiplied로 저장됩니다.
        let half = solid(1, 1, [255, 255, 255, 128]);
        [bw, edge, odd, half].map(|image| generate_mip_chain(&image, MipmapMode::Full, scope))
    });
    let [bw, edge, odd, half] = results;
    assert_eq!(bw.levels()[1].pixel(0, 0), Some([188, 188, 188, 255]));
    assert_eq!(edge.levels()[1].pixel(0, 0), Some([188, 0, 0, 128]));
    assert_eq!(edge.color_space(), ColorSpace::DisplayP3);
    // (1 + 1 + 0) / 3 = 2/3 선형 → encode.
    let two_thirds = encode_u16(((2 * 65_535u32 + 1) / 3) as u16);
    assert_eq!(
        odd.levels()[1].pixel(0, 0),
        Some([two_thirds, two_thirds, two_thirds, 255])
    );
    // 흰색 × 알파 128/255 ≈ 선형 0.502 → 188.
    assert_eq!(half.levels()[0].pixel(0, 0), Some([188, 188, 188, 128]));
}

#[test]
fn uniform_color_is_exact_at_every_level_and_output_is_deterministic() {
    let (uniform, noisy) = on_worker(|scope| {
        let uniform =
            generate_mip_chain(&solid(37, 21, [200, 100, 50, 255]), MipmapMode::Full, scope);
        // 결정적 잡음 이미지.
        let mut state = 0x0007_0002u64;
        let mut pixels = Vec::with_capacity(33 * 17 * 4);
        for _ in 0..33 * 17 * 4 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            pixels.push((state >> 24) as u8);
        }
        let image = DecodedImage::new(33, 17, ColorSpace::Srgb, pixels).unwrap();
        (uniform, generate_mip_chain(&image, MipmapMode::Full, scope))
    });
    for level in uniform.levels() {
        for y in 0..level.height() {
            for x in 0..level.width() {
                assert_eq!(level.pixel(x, y), Some([200, 100, 50, 255]));
            }
        }
    }
    // 정수 연산만 쓰므로 golden digest가 고정됩니다(플랫폼 간 동일 구성, ADR-0006).
    let mut bytes = Vec::new();
    for level in noisy.levels() {
        bytes.extend_from_slice(level.pixels());
    }
    assert_eq!(noisy.level_count(), 6);
    assert_eq!(fnv1a64(&bytes), NOISY_GOLDEN);
}

/// `uniform_color_is_exact_at_every_level_and_output_is_deterministic`의 잡음 이미지 chain digest.
const NOISY_GOLDEN: u64 = 0x7f06_c56d_80ad_42aa;

#[test]
fn generation_requires_worker_thread() {
    let image = solid(4, 4, [1, 2, 3, 4]);
    let result = thread::spawn(move || generate_mip_chain_on_worker(&image, MipmapMode::Full))
        .join()
        .unwrap();
    assert_eq!(result, Err(MipmapError::NotOnWorkerThread));
    let image = solid(4, 4, [1, 2, 3, 4]);
    let chain = thread::spawn(move || {
        let _role = bind_thread_role(ThreadRole::Worker).unwrap();
        generate_mip_chain_on_worker(&image, MipmapMode::Full)
    })
    .join()
    .unwrap()
    .unwrap();
    assert_eq!(chain.level_count(), 3);
}

#[test]
fn upload_task_creates_texture_and_uploads_every_level() {
    let chain = on_worker(|scope| {
        generate_mip_chain(&solid(5, 3, [9, 8, 7, 255]), MipmapMode::Full, scope)
    });
    let task = TextureUploadTask::new(&chain);
    let desc = task.desc();
    assert_eq!(
        (desc.width, desc.height, desc.mip_levels, desc.format),
        (5, 3, 3, TextureFormat::Rgba8UnormSrgb)
    );
    assert!(desc.usage.sampled && !desc.usage.render_target);
    let regions: Vec<TextureRegion> = task.regions().collect();
    assert_eq!(
        regions
            .iter()
            .map(|r| (r.mip_level, r.width, r.height))
            .collect::<Vec<_>>(),
        vec![(0, 5, 3), (1, 2, 1), (2, 1, 1)]
    );
    assert_eq!(task.total_bytes(), (15 + 2 + 1) * 4);

    let mut backend = CheckedBackend::new(MockBackend::new());
    let caps = backend.init(&device_config(16, 16)).unwrap();
    task.check_device(&caps).unwrap();
    let mut queue = UploadQueue::new();
    let summary = task.start(&mut backend, &caps, &mut queue).unwrap();
    assert_eq!((summary.requests, summary.bytes), (3, 72));
    assert_eq!(queue.len(), 3);
    // 작은 예산으로 여러 frame에 나눠 upload합니다(base level 먼저).
    let first = queue.flush(&mut backend, 1).unwrap();
    assert_eq!((first.issued, first.remaining), (1, 2));
    let rest = queue.flush(&mut backend, 1024).unwrap();
    assert_eq!((rest.issued, rest.remaining), (2, 0));
    let updates = backend
        .inner()
        .calls
        .iter()
        .filter(|call| **call == BackendCall::UpdateTexture)
        .count();
    assert_eq!(updates, 3);

    // device 최대 크기 초과.
    let small = DeviceCapabilities {
        max_texture_dimension: 4,
        ..caps
    };
    assert_eq!(
        task.check_device(&small),
        Err(MipmapError::ExceedsDevice {
            width: 5,
            height: 3,
            max: 4
        })
    );
}

#[test]
fn start_rejects_texture_larger_than_device_before_creating_it() {
    // 리뷰 회귀(XI.7.2): start는 device 최대 크기를 넘는 texture를 backend 오류에 기대지 않고 만들기 전에
    // 거부합니다(mock backend는 크기 상한을 검사하지 않음).
    let chain = on_worker(|scope| {
        generate_mip_chain(&solid(5, 3, [9, 8, 7, 255]), MipmapMode::Full, scope)
    });
    let task = TextureUploadTask::new(&chain);
    let mut mock = MockBackend::new();
    mock.caps.max_texture_dimension = 4;
    let mut backend = CheckedBackend::new(mock);
    let caps = backend.init(&device_config(16, 16)).unwrap();
    let mut queue = UploadQueue::new();
    assert_eq!(
        task.start(&mut backend, &caps, &mut queue),
        Err(TextureUploadError::Device(MipmapError::ExceedsDevice {
            width: 5,
            height: 3,
            max: 4
        }))
    );
    assert!(!backend.inner().calls.contains(&BackendCall::CreateTexture));
    assert_eq!(queue.len(), 0);
    // 크기를 넘지 않는 texture는 같은 backend에서 정상 시작합니다.
    let fits = on_worker(|scope| {
        generate_mip_chain(&solid(4, 4, [1, 2, 3, 255]), MipmapMode::Full, scope)
    });
    let summary = TextureUploadTask::new(&fits)
        .start(&mut backend, &caps, &mut queue)
        .unwrap();
    assert_eq!(summary.requests, 3);
    assert_eq!(
        backend
            .inner()
            .calls
            .iter()
            .filter(|call| **call == BackendCall::CreateTexture)
            .count(),
        1
    );
}
