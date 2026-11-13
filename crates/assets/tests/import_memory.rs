//! 대형 파일 import가 파일 전체를 메모리에 올리지 않는지 검증합니다 (XIII.5.2).
//!
//! 이 test binary 전용 global allocator로 heap 사용량 최고치를 측정합니다. import 중
//! 최고치 증가량이 파일 크기보다 훨씬 작은 상한(1 MiB) 이하여야 합니다 — streaming buffer
//! (64 KiB)와 경로·record 같은 작은 할당만 허용됩니다.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use stellar_assets::hash_stream::{StreamingHasher, STREAM_BUFFER_SIZE};
use stellar_assets::media_type::MediaType;
use stellar_assets::store::{AssetStore, ImportRequest};

struct CountingAllocator;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: 실제 할당은 System에 위임하고, 크기 집계만 원자적으로 갱신합니다.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: 호출자가 GlobalAlloc::alloc 계약을 지킵니다.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            PEAK.fetch_max(now, Ordering::SeqCst);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr은 같은 layout으로 이 allocator가 할당한 값입니다.
        unsafe { System.dealloc(ptr, layout) };
        CURRENT.fetch_sub(layout.size(), Ordering::SeqCst);
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// import 대상 크기(16 MiB). 상한(1 MiB)의 16배라 전체 로드 시 반드시 실패합니다.
const FILE_SIZE: u64 = 16 * 1024 * 1024;
/// import 중 허용하는 heap 최고치 증가량.
const HEAP_BUDGET: usize = 1024 * 1024;

#[test]
fn large_file_import_does_not_load_whole_file_into_memory() {
    let base = std::env::temp_dir().join(format!("stellar-assets-memory-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let project = base.join("project");
    std::fs::create_dir_all(project.join("assets")).unwrap();
    let source = base.join("large.bin");

    // sparse 파일 대신 패턴 블록을 이어 써서 실제 내용 있는 대형 파일을 만듭니다.
    let mut block = vec![0u8; STREAM_BUFFER_SIZE];
    let mut expected = StreamingHasher::new();
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&source).unwrap();
        let mut written = 0u64;
        let mut seed = 0u8;
        while written < FILE_SIZE {
            for (i, byte) in block.iter_mut().enumerate() {
                *byte = (i as u8).wrapping_mul(7).wrapping_add(seed);
            }
            file.write_all(&block).unwrap();
            expected.update(&block);
            written += block.len() as u64;
            seed = seed.wrapping_add(1);
        }
    }
    drop(block);
    let expected = expected.finish().unwrap();

    let store = AssetStore::new(&project);
    let request = ImportRequest::new(&source, MediaType::new("application/octet-stream").unwrap())
        .without_source_metadata();

    let baseline = CURRENT.load(Ordering::SeqCst);
    PEAK.store(baseline, Ordering::SeqCst);
    let imported = store.import_file(&request).unwrap();
    let peak_growth = PEAK.load(Ordering::SeqCst) - baseline;

    assert_eq!(imported.record.size, FILE_SIZE);
    assert_eq!(imported.record.content_hash, expected);
    let stored = std::fs::metadata(store.content_path(&imported.object_path)).unwrap();
    assert_eq!(stored.len(), FILE_SIZE);
    assert!(
        peak_growth <= HEAP_BUDGET,
        "import heap 최고치 증가 {peak_growth} bytes > 상한 {HEAP_BUDGET} (파일 {FILE_SIZE})"
    );
    eprintln!("import heap 최고치 증가: {peak_growth} bytes (파일 {FILE_SIZE} bytes)");

    let _ = std::fs::remove_dir_all(&base);
}
