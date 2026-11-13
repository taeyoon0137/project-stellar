//! CPU/GPU 메모리 예산 provider (XVI.5.1).
//!
//! 캐시 예산(아키텍처 §10.4 "시스템 CPU·GPU 메모리 압박을 감지해 예산을 동적으로 조정")의 입력을
//! 운영체제별 메모리 정보에서 얻어 **정규화한 bucket**([`MemoryBucket`] Low·Medium·High)으로
//! 돌려줍니다. 캐시 관리자(XVI.5.2~)는 bytes 절대값이 아니라 bucket으로 정책을 고르므로, 장비
//! 크기·운영체제가 달라도 같은 정책 표를 씁니다.
//!
//! - [`MemoryBudgetProvider`]: 운영체제 질의를 숨기는 interface. 테스트·replay는
//!   [`MockMemoryProvider`]의 low/medium/high fixture를 씁니다.
//! - [`SystemMemoryProvider`]: macOS에서 `sysctlbyname("hw.memsize")`, `host_statistics64
//!   (HOST_VM_INFO64)`, `host_page_size`, `sysctlbyname("kern.memorystatus_vm_pressure_level")`을
//!   최소 FFI로 부릅니다. 그 밖의 운영체제는 [`MemoryQueryError::Unsupported`]입니다(Windows 보류,
//!   ADR-0006).
//! - [`BucketThresholds`]: 정규화 규칙. 가용 비율(‰)과 절대 bytes를 함께 보고, 운영체제 메모리 압박
//!   단계([`MemoryPressure`])가 있으면 그보다 높은 bucket을 주지 않습니다.
//!
//! **macOS 가용 메모리**는 보수적으로 `free + inactive` page × page 크기입니다. `free`에는 이미
//! speculative page가 들어 있고(SDK `vm_statistics.h` 주석), inactive는 재사용 가능한 page입니다.
//! purgeable·compressor page는 다른 집계와 겹칠 수 있어 더하지 않습니다.
//!
//! **GPU**: Apple Silicon은 통합 메모리라 GPU가 CPU와 같은 물리 pool을 씁니다([`GpuMemory::Unified`]).
//! GPU 권장 working set 상한(Metal `recommendedMaxWorkingSetSize`)은 renderer가 알기 때문에, 이 crate는
//! 상한을 받는 자리만 두고 renderer가 채웁니다([`SystemMemoryProvider::with_gpu_working_set_limit`]).
//!
//! 이 모듈의 값(bytes, bucket)은 프로젝트 내용·경로를 담지 않는 비민감 수치입니다.

use std::fmt;

/// 1 GiB.
pub const GIB: u64 = 1024 * 1024 * 1024;

/// 정규화 메모리 bucket. 순서는 여유가 적은 쪽부터입니다(`Low < Medium < High`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemoryBucket {
    /// 여유가 적음: 비필수 캐시를 줄이고 새 캐시 할당을 미룹니다.
    Low,
    /// 보통.
    Medium,
    /// 여유가 많음.
    High,
}

impl MemoryBucket {
    /// 모든 bucket(여유가 적은 쪽부터).
    pub const ALL: [MemoryBucket; 3] =
        [MemoryBucket::Low, MemoryBucket::Medium, MemoryBucket::High];

    /// 진단용 정적 이름.
    pub fn name(self) -> &'static str {
        match self {
            MemoryBucket::Low => "low",
            MemoryBucket::Medium => "medium",
            MemoryBucket::High => "high",
        }
    }
}

/// 운영체제가 알려 주는 메모리 압박 단계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemoryPressure {
    /// 정상.
    Normal,
    /// 경고: 캐시를 줄이기 시작해야 함.
    Warning,
    /// 위험: 즉시 줄여야 함.
    Critical,
}

/// 메모리 질의 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryQueryError {
    /// 이 운영체제에서는 질의를 지원하지 않습니다.
    Unsupported,
    /// 운영체제 호출 실패.
    Os {
        /// 실패한 호출 이름.
        call: &'static str,
        /// 반환 코드(kern_return_t 또는 errno).
        code: i32,
    },
    /// 값이 일관되지 않습니다(전체 0, 가용 > 전체 등).
    Invalid {
        /// 이유.
        reason: &'static str,
    },
}

impl fmt::Display for MemoryQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryQueryError::Unsupported => {
                f.write_str("이 운영체제에서는 메모리 질의를 지원하지 않습니다")
            }
            MemoryQueryError::Os { call, code } => {
                write!(f, "메모리 질의 {call} 실패(코드 {code})")
            }
            MemoryQueryError::Invalid { reason } => write!(f, "메모리 값 오류: {reason}"),
        }
    }
}

impl std::error::Error for MemoryQueryError {}

/// 한 memory pool의 전체·가용 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemoryReading {
    total_bytes: u64,
    available_bytes: u64,
}

impl MemoryReading {
    /// 전체 > 0, 가용 ≤ 전체인 값만 만듭니다.
    pub fn new(total_bytes: u64, available_bytes: u64) -> Result<MemoryReading, MemoryQueryError> {
        if total_bytes == 0 {
            return Err(MemoryQueryError::Invalid {
                reason: "전체 메모리가 0입니다",
            });
        }
        if available_bytes > total_bytes {
            return Err(MemoryQueryError::Invalid {
                reason: "가용 메모리가 전체보다 큽니다",
            });
        }
        Ok(MemoryReading {
            total_bytes,
            available_bytes,
        })
    }

    /// 전체 bytes.
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    /// 가용 bytes.
    pub fn available_bytes(&self) -> u64 {
        self.available_bytes
    }

    /// 가용 비율(‰, 내림).
    pub fn available_permille(&self) -> u16 {
        ((u128::from(self.available_bytes) * 1000) / u128::from(self.total_bytes)) as u16
    }
}

/// GPU 메모리 정보.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuMemory {
    /// 통합 메모리: GPU가 CPU와 같은 물리 pool을 씁니다. `working_set_limit`은 GPU 권장 working set
    /// 상한(bytes)이며 renderer가 알려 주기 전에는 `None`입니다.
    Unified {
        /// GPU 권장 working set 상한.
        working_set_limit: Option<u64>,
    },
    /// 전용 VRAM.
    Dedicated(MemoryReading),
}

/// 한 시점의 메모리 정보.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemorySnapshot {
    /// 시스템(CPU) 메모리.
    pub cpu: MemoryReading,
    /// GPU 메모리.
    pub gpu: GpuMemory,
    /// 운영체제 압박 단계(알 수 없으면 `None`).
    pub pressure: Option<MemoryPressure>,
}

impl MemorySnapshot {
    /// GPU가 쓸 수 있는 pool을 CPU 정보와 합쳐 하나의 reading으로 만듭니다.
    ///
    /// 통합 메모리: 전체 = min(CPU 전체, working set 상한), 가용 = min(CPU 가용, 그 전체).
    pub fn gpu_reading(&self) -> MemoryReading {
        match self.gpu {
            GpuMemory::Dedicated(reading) => reading,
            GpuMemory::Unified { working_set_limit } => {
                let total = working_set_limit
                    .filter(|limit| *limit > 0)
                    .map_or(self.cpu.total_bytes, |limit| {
                        limit.min(self.cpu.total_bytes)
                    });
                MemoryReading {
                    total_bytes: total,
                    available_bytes: self.cpu.available_bytes.min(total),
                }
            }
        }
    }
}

/// bucket 정규화 규칙.
///
/// - Low: 가용 비율 < `low_below_permille` 또는 가용 < `low_below_bytes`, 또는 압박 Critical.
/// - High: 가용 비율 ≥ `high_at_or_above_permille` 그리고 가용 ≥ `high_at_or_above_bytes`, 그리고 압박이
///   Warning·Critical이 아님.
/// - 그 밖은 Medium. 압박 Warning은 High를 Medium으로 낮춥니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BucketThresholds {
    /// Low 비율 경계(‰).
    pub low_below_permille: u16,
    /// Low 절대 경계(bytes).
    pub low_below_bytes: u64,
    /// High 비율 경계(‰).
    pub high_at_or_above_permille: u16,
    /// High 절대 경계(bytes).
    pub high_at_or_above_bytes: u64,
}

impl BucketThresholds {
    /// 기본 규칙: Low = 가용 15% 미만 또는 1 GiB 미만, High = 가용 40% 이상이고 4 GiB 이상.
    ///
    /// 기준 장비(M1 16 GB, 아키텍처 §5.2)에서 Low 경계는 2.4 GB, High 경계는 6.4 GB입니다.
    pub const DEFAULT: BucketThresholds = BucketThresholds {
        low_below_permille: 150,
        low_below_bytes: GIB,
        high_at_or_above_permille: 400,
        high_at_or_above_bytes: 4 * GIB,
    };

    /// 경계가 일관된지 확인합니다(비율 ≤ 1000‰, Low 경계 < High 경계).
    pub fn validate(&self) -> Result<(), MemoryQueryError> {
        if self.high_at_or_above_permille > 1000 {
            return Err(MemoryQueryError::Invalid {
                reason: "비율 경계는 1000‰ 이하여야 합니다",
            });
        }
        if self.low_below_permille > self.high_at_or_above_permille
            || self.low_below_bytes > self.high_at_or_above_bytes
        {
            return Err(MemoryQueryError::Invalid {
                reason: "Low 경계가 High 경계보다 큽니다",
            });
        }
        Ok(())
    }

    /// reading과 압박 단계를 bucket으로 정규화합니다.
    pub fn classify(
        &self,
        reading: &MemoryReading,
        pressure: Option<MemoryPressure>,
    ) -> MemoryBucket {
        // 비율은 내림한 ‰가 아니라 정확한 비교(available × 1000 vs 경계‰ × total)로 판정합니다.
        let available = reading.available_bytes();
        let scaled = u128::from(available) * 1000;
        let total = u128::from(reading.total_bytes());
        let below_low = scaled < u128::from(self.low_below_permille) * total;
        let at_high = scaled >= u128::from(self.high_at_or_above_permille) * total;
        let by_amount = if below_low || available < self.low_below_bytes {
            MemoryBucket::Low
        } else if at_high && available >= self.high_at_or_above_bytes {
            MemoryBucket::High
        } else {
            MemoryBucket::Medium
        };
        let cap = match pressure {
            Some(MemoryPressure::Critical) => MemoryBucket::Low,
            Some(MemoryPressure::Warning) => MemoryBucket::Medium,
            Some(MemoryPressure::Normal) | None => MemoryBucket::High,
        };
        by_amount.min(cap)
    }
}

impl Default for BucketThresholds {
    fn default() -> BucketThresholds {
        BucketThresholds::DEFAULT
    }
}

/// 정규화된 메모리 예산 입력.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemoryBudget {
    /// CPU bucket.
    pub cpu: MemoryBucket,
    /// GPU bucket.
    pub gpu: MemoryBucket,
    /// 판정에 쓴 snapshot.
    pub snapshot: MemorySnapshot,
}

impl MemoryBudget {
    /// snapshot을 규칙으로 정규화합니다. 통합 메모리 GPU는 같은 pool이라 OS 압박 단계를 함께
    /// 적용하고, 전용 VRAM은 OS 압박 단계와 무관하게 판정합니다.
    pub fn from_snapshot(snapshot: MemorySnapshot, thresholds: &BucketThresholds) -> MemoryBudget {
        let cpu = thresholds.classify(&snapshot.cpu, snapshot.pressure);
        let gpu_pressure = match snapshot.gpu {
            GpuMemory::Unified { .. } => snapshot.pressure,
            GpuMemory::Dedicated(_) => None,
        };
        let gpu = thresholds.classify(&snapshot.gpu_reading(), gpu_pressure);
        MemoryBudget { cpu, gpu, snapshot }
    }
}

/// CPU/GPU 메모리 예산 provider interface.
pub trait MemoryBudgetProvider {
    /// 현재 메모리 정보를 질의합니다.
    fn snapshot(&self) -> Result<MemorySnapshot, MemoryQueryError>;

    /// 현재 정보를 bucket으로 정규화합니다.
    fn budget(&self, thresholds: &BucketThresholds) -> Result<MemoryBudget, MemoryQueryError> {
        thresholds.validate()?;
        Ok(MemoryBudget::from_snapshot(self.snapshot()?, thresholds))
    }
}

/// 고정 snapshot을 돌려주는 provider(테스트·replay fixture).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MockMemoryProvider {
    snapshot: MemorySnapshot,
}

impl MockMemoryProvider {
    /// 주어진 snapshot을 돌려줍니다.
    pub fn new(snapshot: MemorySnapshot) -> MockMemoryProvider {
        MockMemoryProvider { snapshot }
    }

    fn reference(available: u64, pressure: MemoryPressure) -> MockMemoryProvider {
        // 기준 장비(M1 16 GB, 통합 메모리). 상수라 MemoryReading 검사를 통과합니다.
        MockMemoryProvider::new(MemorySnapshot {
            cpu: MemoryReading {
                total_bytes: 16 * GIB,
                available_bytes: available,
            },
            gpu: GpuMemory::Unified {
                working_set_limit: None,
            },
            pressure: Some(pressure),
        })
    }

    /// Low fixture: 16 GiB 중 512 MiB 가용, 압박 Warning.
    pub fn low() -> MockMemoryProvider {
        MockMemoryProvider::reference(GIB / 2, MemoryPressure::Warning)
    }

    /// Medium fixture: 16 GiB 중 6 GiB 가용(37.5%), 압박 Normal.
    pub fn medium() -> MockMemoryProvider {
        MockMemoryProvider::reference(6 * GIB, MemoryPressure::Normal)
    }

    /// High fixture: 16 GiB 중 12 GiB 가용(75%), 압박 Normal.
    pub fn high() -> MockMemoryProvider {
        MockMemoryProvider::reference(12 * GIB, MemoryPressure::Normal)
    }
}

impl MemoryBudgetProvider for MockMemoryProvider {
    fn snapshot(&self) -> Result<MemorySnapshot, MemoryQueryError> {
        Ok(self.snapshot)
    }
}

/// 운영체제 메모리 provider. macOS만 지원합니다(Windows 보류, ADR-0006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SystemMemoryProvider {
    gpu_working_set_limit: Option<u64>,
}

impl SystemMemoryProvider {
    /// GPU working set 상한을 모르는 provider.
    pub fn new() -> SystemMemoryProvider {
        SystemMemoryProvider::default()
    }

    /// renderer가 알려 준 GPU 권장 working set 상한(bytes)을 함께 씁니다.
    pub fn with_gpu_working_set_limit(limit: u64) -> SystemMemoryProvider {
        SystemMemoryProvider {
            gpu_working_set_limit: Some(limit),
        }
    }
}

impl MemoryBudgetProvider for SystemMemoryProvider {
    #[cfg(target_os = "macos")]
    fn snapshot(&self) -> Result<MemorySnapshot, MemoryQueryError> {
        let total = macos::physical_memory()?;
        let available = macos::available_memory()?.min(total);
        Ok(MemorySnapshot {
            cpu: MemoryReading::new(total, available)?,
            gpu: GpuMemory::Unified {
                working_set_limit: self.gpu_working_set_limit,
            },
            pressure: macos::pressure_level(),
        })
    }

    #[cfg(not(target_os = "macos"))]
    fn snapshot(&self) -> Result<MemorySnapshot, MemoryQueryError> {
        Err(MemoryQueryError::Unsupported)
    }
}

/// `kern.memorystatus_vm_pressure_level` 값(Darwin dispatch memory pressure 규약: 1 normal,
/// 2 warning, 4 critical)을 압박 단계로 바꿉니다. 알 수 없는 값은 `None`입니다.
pub fn pressure_from_darwin_level(level: i32) -> Option<MemoryPressure> {
    match level {
        1 => Some(MemoryPressure::Normal),
        2 => Some(MemoryPressure::Warning),
        4 => Some(MemoryPressure::Critical),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
mod macos {
    //! macOS 메모리 질의 최소 FFI(libSystem). 선언은 macOS SDK 헤더(`mach/mach_host.h`,
    //! `mach/mach_init.h`, `mach/host_info.h`, `mach/vm_statistics.h`, `sys/sysctl.h`)를 따릅니다.

    use std::ffi::{c_char, c_int, c_uint, c_void, CStr};

    use super::{pressure_from_darwin_level, MemoryPressure, MemoryQueryError};

    type KernReturn = c_int;
    type MachPort = c_uint;

    const KERN_SUCCESS: KernReturn = 0;
    /// `HOST_VM_INFO64` flavor.
    const HOST_VM_INFO64: c_int = 4;

    /// `struct vm_statistics64`의 rev1 prefix(`swapped_count` 앞까지). 커널은 요청한 count만큼만
    /// 채우므로, rev1 count를 넘겨 이후 SDK에서 늘어난 field와 무관하게 같은 layout을 읽습니다.
    #[repr(C, align(8))]
    #[derive(Default)]
    #[allow(dead_code)] // FFI가 채우는 layout. 일부 field만 읽습니다.
    struct VmStatistics64Rev1 {
        free_count: u32,
        active_count: u32,
        inactive_count: u32,
        wire_count: u32,
        zero_fill_count: u64,
        reactivations: u64,
        pageins: u64,
        pageouts: u64,
        faults: u64,
        cow_faults: u64,
        lookups: u64,
        hits: u64,
        purges: u64,
        purgeable_count: u32,
        speculative_count: u32,
        decompressions: u64,
        compressions: u64,
        swapins: u64,
        swapouts: u64,
        compressor_page_count: u32,
        throttled_count: u32,
        external_page_count: u32,
        internal_page_count: u32,
        total_uncompressed_pages_in_compressor: u64,
    }

    /// `offsetof(vm_statistics64_data_t, swapped_count)`.
    const REV1_BYTES: usize = 152;
    const _: () = assert!(std::mem::size_of::<VmStatistics64Rev1>() == REV1_BYTES);
    /// `HOST_VM_INFO64_REV1_COUNT`(integer_t 단위).
    const HOST_VM_INFO64_REV1_COUNT: c_uint = (REV1_BYTES / std::mem::size_of::<c_int>()) as c_uint;

    extern "C" {
        fn mach_host_self() -> MachPort;
        static mach_task_self_: MachPort;
        fn mach_port_deallocate(task: MachPort, name: MachPort) -> KernReturn;
        fn host_page_size(host: MachPort, out_page_size: *mut usize) -> KernReturn;
        fn host_statistics64(
            host: MachPort,
            flavor: c_int,
            host_info64_out: *mut c_int,
            host_info64_out_count: *mut c_uint,
        ) -> KernReturn;
        fn sysctlbyname(
            name: *const c_char,
            oldp: *mut c_void,
            oldlenp: *mut usize,
            newp: *mut c_void,
            newlen: usize,
        ) -> c_int;
    }

    fn last_errno() -> i32 {
        std::io::Error::last_os_error().raw_os_error().unwrap_or(-1)
    }

    /// 4 또는 8 byte 정수 sysctl을 읽습니다.
    fn sysctl_unsigned(name: &CStr, call: &'static str) -> Result<u64, MemoryQueryError> {
        let mut value: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        // SAFETY: name은 NUL 종단 문자열이고, oldp는 len bytes 쓰기 가능한 지역 변수입니다. newp는
        // NULL(읽기 전용 질의)입니다.
        let rc = unsafe {
            sysctlbyname(
                name.as_ptr(),
                (&mut value as *mut u64).cast::<c_void>(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 {
            return Err(MemoryQueryError::Os {
                call,
                code: last_errno(),
            });
        }
        match len {
            8 => Ok(value),
            // little-endian(arm64)에서 앞 4 bytes만 쓰였으므로 하위 32비트가 값입니다.
            4 => Ok(value & 0xFFFF_FFFF),
            _ => Err(MemoryQueryError::Invalid {
                reason: "sysctl 값 크기가 4·8 bytes가 아닙니다",
            }),
        }
    }

    /// 물리 메모리 bytes(`hw.memsize`).
    pub(super) fn physical_memory() -> Result<u64, MemoryQueryError> {
        sysctl_unsigned(c"hw.memsize", "sysctlbyname(hw.memsize)")
    }

    /// 압박 단계(`kern.memorystatus_vm_pressure_level`). 읽지 못하면 `None`.
    pub(super) fn pressure_level() -> Option<MemoryPressure> {
        let level = sysctl_unsigned(
            c"kern.memorystatus_vm_pressure_level",
            "sysctlbyname(kern.memorystatus_vm_pressure_level)",
        )
        .ok()?;
        pressure_from_darwin_level(i32::try_from(level).ok()?)
    }

    /// 보수적 가용 bytes: (free + inactive) page × page 크기.
    pub(super) fn available_memory() -> Result<u64, MemoryQueryError> {
        // SAFETY: 인자 없는 libSystem 함수이며 send right를 돌려줍니다(아래에서 반환).
        let host = unsafe { mach_host_self() };
        let result = query_vm(host);
        // SAFETY: mach_task_self_는 libSystem이 초기화한 현재 task port이고, host는 위에서 받은
        // send right입니다. 실패해도 질의 결과에는 영향이 없어 반환 코드를 무시합니다.
        unsafe {
            let _ = mach_port_deallocate(mach_task_self_, host);
        }
        result
    }

    fn query_vm(host: MachPort) -> Result<u64, MemoryQueryError> {
        let mut page_size: usize = 0;
        // SAFETY: out 포인터는 쓰기 가능한 지역 변수입니다.
        let kr = unsafe { host_page_size(host, &mut page_size) };
        if kr != KERN_SUCCESS {
            return Err(MemoryQueryError::Os {
                call: "host_page_size",
                code: kr,
            });
        }
        if page_size == 0 {
            return Err(MemoryQueryError::Invalid {
                reason: "page 크기가 0입니다",
            });
        }
        let mut stats = VmStatistics64Rev1::default();
        let mut count = HOST_VM_INFO64_REV1_COUNT;
        // SAFETY: stats는 rev1 layout(152 bytes, 8-byte 정렬)의 쓰기 가능한 지역 변수이고, count는 그
        // 크기를 integer_t 단위로 알립니다. 커널은 count 이하만 씁니다.
        let kr = unsafe {
            host_statistics64(
                host,
                HOST_VM_INFO64,
                (&mut stats as *mut VmStatistics64Rev1).cast::<c_int>(),
                &mut count,
            )
        };
        if kr != KERN_SUCCESS {
            return Err(MemoryQueryError::Os {
                call: "host_statistics64",
                code: kr,
            });
        }
        let pages = u64::from(stats.free_count) + u64::from(stats.inactive_count);
        pages
            .checked_mul(page_size as u64)
            .ok_or(MemoryQueryError::Invalid {
                reason: "가용 메모리 계산이 u64 범위를 넘습니다",
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(total: u64, available: u64) -> MemoryReading {
        MemoryReading::new(total, available).unwrap()
    }

    #[test]
    fn mock_fixtures_map_to_low_medium_high() {
        let t = BucketThresholds::DEFAULT;
        let budget = |p: MockMemoryProvider| p.budget(&t).unwrap();
        let low = budget(MockMemoryProvider::low());
        let medium = budget(MockMemoryProvider::medium());
        let high = budget(MockMemoryProvider::high());
        assert_eq!((low.cpu, low.gpu), (MemoryBucket::Low, MemoryBucket::Low));
        assert_eq!(
            (medium.cpu, medium.gpu),
            (MemoryBucket::Medium, MemoryBucket::Medium)
        );
        assert_eq!(
            (high.cpu, high.gpu),
            (MemoryBucket::High, MemoryBucket::High)
        );
        assert_eq!(
            MemoryBucket::ALL.map(MemoryBucket::name),
            ["low", "medium", "high"]
        );
    }

    #[test]
    fn thresholds_are_applied_at_exact_boundaries() {
        let t = BucketThresholds::DEFAULT;
        let total = 16 * GIB;
        // 15% 경계: 2.4 GiB 바로 아래 Low, 이상 Medium.
        let low_edge = (total * 150).div_ceil(1000);
        assert_eq!(
            t.classify(&reading(total, low_edge - 1), None),
            MemoryBucket::Low
        );
        assert_eq!(
            t.classify(&reading(total, low_edge), None),
            MemoryBucket::Medium
        );
        // 40% 경계: 6.4 GiB 바로 아래 Medium, 이상 High.
        let high_edge = (total * 400).div_ceil(1000);
        assert_eq!(
            t.classify(&reading(total, high_edge - 1), None),
            MemoryBucket::Medium
        );
        assert_eq!(
            t.classify(&reading(total, high_edge), None),
            MemoryBucket::High
        );
        // 작은 장비: 비율이 높아도 절대량 1 GiB 미만이면 Low, 4 GiB 미만이면 High가 아닙니다.
        assert_eq!(
            t.classify(&reading(2 * GIB, GIB - 1), None),
            MemoryBucket::Low
        );
        assert_eq!(
            t.classify(&reading(4 * GIB, 3 * GIB), None),
            MemoryBucket::Medium
        );
        // 큰 장비: 비율이 낮으면 절대량이 커도 Low.
        assert_eq!(
            t.classify(&reading(256 * GIB, 30 * GIB), None),
            MemoryBucket::Low
        );
    }

    #[test]
    fn os_pressure_caps_the_bucket() {
        let t = BucketThresholds::DEFAULT;
        let plenty = reading(16 * GIB, 12 * GIB);
        assert_eq!(
            t.classify(&plenty, Some(MemoryPressure::Normal)),
            MemoryBucket::High
        );
        assert_eq!(
            t.classify(&plenty, Some(MemoryPressure::Warning)),
            MemoryBucket::Medium
        );
        assert_eq!(
            t.classify(&plenty, Some(MemoryPressure::Critical)),
            MemoryBucket::Low
        );
        // 압박이 bucket을 올리지는 않습니다.
        let scarce = reading(16 * GIB, GIB / 2);
        assert_eq!(
            t.classify(&scarce, Some(MemoryPressure::Normal)),
            MemoryBucket::Low
        );
        assert_eq!(pressure_from_darwin_level(1), Some(MemoryPressure::Normal));
        assert_eq!(pressure_from_darwin_level(2), Some(MemoryPressure::Warning));
        assert_eq!(
            pressure_from_darwin_level(4),
            Some(MemoryPressure::Critical)
        );
        assert_eq!(pressure_from_darwin_level(3), None);
    }

    #[test]
    fn gpu_reading_follows_unified_or_dedicated_memory() {
        let t = BucketThresholds::DEFAULT;
        let cpu = reading(16 * GIB, 12 * GIB);
        // 통합 메모리 + working set 상한 8 GiB: 가용 8 GiB / 8 GiB → High.
        let unified = MemorySnapshot {
            cpu,
            gpu: GpuMemory::Unified {
                working_set_limit: Some(8 * GIB),
            },
            pressure: Some(MemoryPressure::Warning),
        };
        assert_eq!(unified.gpu_reading(), reading(8 * GIB, 8 * GIB));
        // 같은 pool이라 OS 압박(Warning)이 GPU에도 적용됩니다.
        let budget = MemoryBudget::from_snapshot(unified, &t);
        assert_eq!(
            (budget.cpu, budget.gpu),
            (MemoryBucket::Medium, MemoryBucket::Medium)
        );
        // 상한 0은 무시합니다.
        let zero_limit = MemorySnapshot {
            gpu: GpuMemory::Unified {
                working_set_limit: Some(0),
            },
            ..unified
        };
        assert_eq!(zero_limit.gpu_reading(), cpu);
        // 전용 VRAM: GPU reading은 VRAM, OS 압박과 무관.
        let dedicated = MemorySnapshot {
            cpu,
            gpu: GpuMemory::Dedicated(reading(8 * GIB, GIB / 4)),
            pressure: Some(MemoryPressure::Warning),
        };
        let budget = MemoryBudget::from_snapshot(dedicated, &t);
        assert_eq!(
            (budget.cpu, budget.gpu),
            (MemoryBucket::Medium, MemoryBucket::Low)
        );
    }

    #[test]
    fn invalid_values_are_rejected() {
        assert!(MemoryReading::new(0, 0).is_err());
        assert!(MemoryReading::new(10, 11).is_err());
        assert_eq!(reading(10, 10).available_permille(), 1000);
        assert_eq!(reading(3, 1).available_permille(), 333);
        assert_eq!(reading(u64::MAX, u64::MAX).available_permille(), 1000);
        let bad = BucketThresholds {
            low_below_permille: 500,
            ..BucketThresholds::DEFAULT
        };
        assert!(bad.validate().is_err());
        assert!(MockMemoryProvider::high().budget(&bad).is_err());
        let over = BucketThresholds {
            high_at_or_above_permille: 1001,
            ..BucketThresholds::DEFAULT
        };
        assert!(over.validate().is_err());
        assert!(BucketThresholds::default().validate().is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_system_provider_reports_consistent_values() {
        let snapshot = SystemMemoryProvider::new()
            .snapshot()
            .expect("macOS 메모리 질의");
        let cpu = snapshot.cpu;
        // hw.memsize는 page 단위의 양수이고, 가용은 전체 이하입니다(MemoryReading 불변식).
        assert!(
            cpu.total_bytes() >= GIB,
            "물리 메모리 {}",
            cpu.total_bytes()
        );
        assert!(cpu.available_bytes() <= cpu.total_bytes());
        assert!(cpu.available_bytes() > 0);
        assert_eq!(
            snapshot.gpu,
            GpuMemory::Unified {
                working_set_limit: None
            }
        );
        let budget = SystemMemoryProvider::with_gpu_working_set_limit(cpu.total_bytes() / 2)
            .budget(&BucketThresholds::DEFAULT)
            .expect("bucket");
        assert!(MemoryBucket::ALL.contains(&budget.cpu));
        assert!(budget.snapshot.gpu_reading().total_bytes() <= cpu.total_bytes() / 2);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn other_platforms_are_unsupported() {
        assert_eq!(
            SystemMemoryProvider::new().snapshot(),
            Err(MemoryQueryError::Unsupported)
        );
    }
}
